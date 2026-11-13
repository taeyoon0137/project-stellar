//! Git unmerged index 수집 (XIV.11.1~).
//!
//! `git ls-files -u -z --full-name -- :/`로 병합되지 않은 index 항목(stage 1 = base, 2 = ours,
//! 3 = theirs)의 mode와 blob ID를 경로별로 모읍니다. 레코드 형식은 `ls-files --stage`와 같으므로
//! XIV.2.5 해석기([`parse_ls_files_stage`])를 그대로 씁니다.
//!
//! 한쪽에 없는 stage(추가/추가 충돌의 base, 삭제/수정 충돌의 한쪽)는 `None`입니다. 같은 경로·stage가
//! 두 번 나오거나 stage 0이 섞이면 해석 오류입니다. 이 모듈은 index를 읽기만 합니다.

use std::path::Path;

use crate::decode::GitPath;
use crate::object::{FileMode, ObjectId};
use crate::ops::{run_ok, GitOpError, ParseFailure};
use crate::runner::argv;
use crate::snapshot::index::{parse_ls_files_stage, StagedEntry};
use crate::trust::GatedGitRunner;

/// 한 stage의 mode와 blob ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StageBlob {
    /// mode.
    pub mode: FileMode,
    /// blob ID(gitlink면 commit ID).
    pub oid: ObjectId,
}

/// 병합되지 않은 경로 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmergedPath {
    /// 경로.
    pub path: GitPath,
    /// stage 1(공통 조상).
    pub base: Option<StageBlob>,
    /// stage 2(ours, 현재 branch).
    pub ours: Option<StageBlob>,
    /// stage 3(theirs, 병합 대상).
    pub theirs: Option<StageBlob>,
}

impl UnmergedPath {
    /// stage 번호(1·2·3)의 blob.
    pub fn stage(&self, stage: u8) -> Option<&StageBlob> {
        match stage {
            1 => self.base.as_ref(),
            2 => self.ours.as_ref(),
            3 => self.theirs.as_ref(),
            _ => None,
        }
    }
}

const COMMAND: &str = "ls-files -u -z";

/// stage 항목(1·2·3)을 경로별로 묶습니다(Git index 순서 유지).
pub fn group_unmerged(entries: Vec<StagedEntry>) -> Result<Vec<UnmergedPath>, ParseFailure> {
    let mut paths: Vec<UnmergedPath> = Vec::new();
    for entry in entries {
        let bad = |detail: &str| ParseFailure::new(COMMAND, detail, entry.path.as_bytes());
        let index = match paths.iter().position(|p| p.path == entry.path) {
            Some(i) => i,
            None => {
                paths.push(UnmergedPath {
                    path: entry.path.clone(),
                    base: None,
                    ours: None,
                    theirs: None,
                });
                paths.len() - 1
            }
        };
        let slot = match entry.stage {
            1 => &mut paths[index].base,
            2 => &mut paths[index].ours,
            3 => &mut paths[index].theirs,
            _ => return Err(bad("unmerged 목록에 stage 0 항목")),
        };
        if slot.is_some() {
            return Err(bad("같은 경로·stage 중복"));
        }
        *slot = Some(StageBlob {
            mode: entry.mode,
            oid: entry.oid,
        });
    }
    Ok(paths)
}

/// 병합되지 않은 경로를 읽습니다(trust gate 적용, 읽기 전용).
pub fn read_unmerged_paths(
    git: &GatedGitRunner,
    worktree: &Path,
) -> Result<Vec<UnmergedPath>, GitOpError> {
    let output = run_ok(
        git,
        worktree,
        &argv(["ls-files", "-u", "-z", "--full-name", "--", ":/"]),
    )?;
    Ok(group_unmerged(parse_ls_files_stage(&output.stdout)?)?)
}

/// raw Git conflict 표시(XIV.11.4, Semantic Merge로 해석하지 않음).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawConflict {
    /// 경로.
    pub path: GitPath,
    /// Git 2글자 unmerged 상태(`UU`, `AA`, `DU`, `UD`, `AU`, `UA`, `DD`), stage 존재로 결정.
    pub xy: &'static str,
    /// stage 원본(사용자가 외부 도구로 해결할 때 참고).
    pub stages: UnmergedPath,
}

/// stage 존재 여부를 Git unmerged `XY`로 바꿉니다(`git status`와 같은 표기).
pub fn unmerged_xy(path: &UnmergedPath) -> &'static str {
    match (
        path.base.is_some(),
        path.ours.is_some(),
        path.theirs.is_some(),
    ) {
        (true, true, true) => "UU",
        (false, true, true) => "AA",
        (true, false, true) => "DU",
        (true, true, false) => "UD",
        (false, true, false) => "AU",
        (false, false, true) => "UA",
        (true, false, false) | (false, false, false) => "DD",
    }
}

/// Semantic Merge 대상과 raw conflict로 나눈 결과.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConflictPartition {
    /// Stellar 프로젝트 파일 conflict(Semantic Merge 입력).
    pub semantic: Vec<UnmergedPath>,
    /// non-Stellar 파일 conflict(raw 표시 전용).
    pub raw: Vec<RawConflict>,
}

/// conflict를 Stellar 파일(semantic)과 그 밖(raw)으로 나눕니다([`crate::classify`] 분류).
pub fn partition_conflicts(paths: Vec<UnmergedPath>) -> ConflictPartition {
    use crate::classify::{classify_project_path, ProjectFileClass};
    let mut out = ConflictPartition::default();
    for path in paths {
        if classify_project_path(path.path.as_bytes()) == ProjectFileClass::NonStellar {
            out.raw.push(RawConflict {
                path: path.path.clone(),
                xy: unmerged_xy(&path),
                stages: path,
            });
        } else {
            out.semantic.push(path);
        }
    }
    out
}

/// 현재 conflict를 읽어 semantic/raw로 나눕니다(읽기 전용).
pub fn read_conflict_partition(
    git: &GatedGitRunner,
    worktree: &Path,
) -> Result<ConflictPartition, GitOpError> {
    Ok(partition_conflicts(read_unmerged_paths(git, worktree)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const B1: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
    const B2: &str = "8baef1b4abc478178b004d62031cf7fe6db6f903";

    #[test]
    fn groups_stages_per_path_with_missing_sides() {
        let bytes = format!(
            "100644 {B1} 1\tboth.txt\0100644 {B2} 2\tboth.txt\0100755 {B1} 3\tboth.txt\0\
             100644 {B1} 2\tadded.txt\0100644 {B2} 3\tadded.txt\0\
             100644 {B1} 1\tdel.txt\0100644 {B2} 3\tdel.txt\0"
        );
        let paths = group_unmerged(parse_ls_files_stage(bytes.as_bytes()).unwrap()).unwrap();
        assert_eq!(paths.len(), 3);
        assert_eq!(paths[0].path.as_bytes(), b"both.txt");
        assert_eq!(paths[0].theirs.as_ref().unwrap().mode, FileMode::EXECUTABLE);
        assert!(paths[1].base.is_none());
        assert!(paths[1].ours.is_some() && paths[1].theirs.is_some());
        assert!(paths[2].ours.is_none());
        assert_eq!(paths[2].stage(1).unwrap().oid.as_str(), B1);
        assert_eq!(paths[2].stage(0), None);
    }

    #[test]
    fn duplicate_stage_or_stage0_is_rejected() {
        for bytes in [
            format!("100644 {B1} 2\ta\0100644 {B2} 2\ta\0"),
            format!("100644 {B1} 0\ta\0"),
        ] {
            assert!(group_unmerged(parse_ls_files_stage(bytes.as_bytes()).unwrap()).is_err());
        }
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::runner::GitRunner;
        use crate::test_support::{
            commit_file, git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use crate::trust::GitTrust;

        /// Git이 직접 해석한 `:<stage>:<path>` blob ID.
        fn stage_oid(runner: &GitRunner, repo: &Path, stage: u8, path: &[u8]) -> Option<String> {
            let spec = format!(":{stage}:{}", String::from_utf8_lossy(path));
            let out = runner
                .run(&argv(["rev-parse", "-q", "--verify", spec.as_str()]), repo)
                .unwrap();
            out.success()
                .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        }

        fn assert_matches_git(runner: &GitRunner, repo: &Path, paths: &[UnmergedPath]) {
            for p in paths {
                for stage in 1..=3u8 {
                    assert_eq!(
                        p.stage(stage).map(|b| b.oid.as_str().to_string()),
                        stage_oid(runner, repo, stage, p.path.as_bytes()),
                        "{:?} stage {stage}",
                        p.path
                    );
                }
            }
        }

        /// main과 other branch가 같은 파일을 다르게 바꾼 뒤 merge(충돌)합니다.
        fn conflicted(
            tag: &str,
            base: &[(&str, &[u8])],
            ours: &dyn Fn(&GitRunner, &Path),
            theirs: &dyn Fn(&GitRunner, &Path),
        ) -> Option<(TempDir, GitRunner)> {
            let git = system_git()?;
            let dir = TempDir::new(tag);
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            for (p, b) in base {
                commit_file(&runner, repo, p, b);
            }
            git_ok(&runner, repo, ["switch", "-q", "-c", "other"]);
            theirs(&runner, repo);
            git_ok(&runner, repo, ["switch", "-q", "main"]);
            ours(&runner, repo);
            let merge = runner.run(&argv(["merge", "-q", "other"]), repo).unwrap();
            assert!(!merge.success(), "충돌이 생기지 않음");
            Some((dir, runner))
        }

        #[test]
        fn text_and_binary_conflict_fixture() {
            let binary_base: Vec<u8> = (0..=255u8).collect();
            let Some((dir, runner)) = conflicted(
                "conflict-text-bin",
                &[
                    ("design/a.txt", b"base\n"),
                    ("assets/img.bin", &binary_base),
                ],
                &|r, repo| {
                    commit_file(r, repo, "design/a.txt", b"ours\n");
                    commit_file(r, repo, "assets/img.bin", &[0u8, 1, 2, 0, 255]);
                },
                &|r, repo| {
                    commit_file(r, repo, "design/a.txt", b"theirs\n");
                    commit_file(r, repo, "assets/img.bin", &[9u8, 0, 9, 0]);
                },
            ) else {
                return; // git 미설치 환경 건너뜀.
            };
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let paths = read_unmerged_paths(&gate, dir.path()).unwrap();
            let names: Vec<&[u8]> = paths.iter().map(|p| p.path.as_bytes()).collect();
            assert_eq!(names, vec![&b"assets/img.bin"[..], b"design/a.txt"]);
            assert!(paths
                .iter()
                .all(|p| p.base.is_some() && p.ours.is_some() && p.theirs.is_some()));
            assert_matches_git(&runner, dir.path(), &paths);
            // 하위 디렉터리 cwd에서도 root 기준 전체.
            assert_eq!(
                read_unmerged_paths(&gate, &dir.path().join("design")).unwrap(),
                paths
            );
        }

        #[test]
        fn rename_conflict_fixture() {
            let Some((dir, runner)) = conflicted(
                "conflict-rename",
                &[("design/old.txt", b"line 1\nline 2\nline 3\nline 4\n")],
                &|r, repo| {
                    git_ok(r, repo, ["mv", "design/old.txt", "design/ours.txt"]);
                    git_ok(r, repo, ["commit", "-q", "-m", "ours rename"]);
                },
                &|r, repo| {
                    git_ok(r, repo, ["mv", "design/old.txt", "design/theirs.txt"]);
                    git_ok(r, repo, ["commit", "-q", "-m", "theirs rename"]);
                },
            ) else {
                return;
            };
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let paths = read_unmerged_paths(&gate, dir.path()).unwrap();
            assert!(!paths.is_empty(), "rename/rename 충돌 항목 없음");
            // 각 stage blob이 Git의 `:<stage>:<path>` 해석과 정확히 일치.
            assert_matches_git(&runner, dir.path(), &paths);
            // rename 충돌은 경로마다 일부 stage만 가질 수 있음(세 stage 모두 있는 경로가 없어도 됨).
            assert!(paths
                .iter()
                .all(|p| p.base.is_some() || p.ours.is_some() || p.theirs.is_some()));
        }

        #[test]
        fn delete_modify_conflict_has_missing_side() {
            let Some((dir, runner)) = conflicted(
                "conflict-delmod",
                &[("design/x.txt", b"base\n")],
                &|r, repo| {
                    git_ok(r, repo, ["rm", "-q", "design/x.txt"]);
                    git_ok(r, repo, ["commit", "-q", "-m", "rm"]);
                },
                &|r, repo| commit_file(r, repo, "design/x.txt", b"theirs\n"),
            ) else {
                return;
            };
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let paths = read_unmerged_paths(&gate, dir.path()).unwrap();
            assert_eq!(paths.len(), 1);
            assert!(
                paths[0].base.is_some() && paths[0].ours.is_none() && paths[0].theirs.is_some()
            );
            assert_matches_git(&runner, dir.path(), &paths);
        }

        #[test]
        fn readme_conflict_stays_raw_and_is_not_semantic_input() {
            use crate::status::{read_status, StatusEntry, StatusOptions};
            let Some((dir, runner)) = conflicted(
                "conflict-readme",
                &[
                    ("README.md", b"base\n"),
                    ("design/current.stellarref", b"gen-base\n"),
                    ("docs/notes.md", b"n\n"),
                ],
                &|r, repo| {
                    commit_file(r, repo, "README.md", b"ours\n");
                    commit_file(r, repo, "design/current.stellarref", b"gen-ours\n");
                    git_ok(r, repo, ["rm", "-q", "docs/notes.md"]);
                    git_ok(r, repo, ["commit", "-q", "-m", "rm notes"]);
                },
                &|r, repo| {
                    commit_file(r, repo, "README.md", b"theirs\n");
                    commit_file(r, repo, "design/current.stellarref", b"gen-theirs\n");
                    commit_file(r, repo, "docs/notes.md", b"edited\n");
                },
            ) else {
                return;
            };
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let partition = read_conflict_partition(&gate, dir.path()).unwrap();
            let semantic: Vec<&[u8]> = partition
                .semantic
                .iter()
                .map(|p| p.path.as_bytes())
                .collect();
            assert_eq!(semantic, vec![&b"design/current.stellarref"[..]]);
            let raw: Vec<(&[u8], &str)> = partition
                .raw
                .iter()
                .map(|r| (r.path.as_bytes(), r.xy))
                .collect();
            assert_eq!(
                raw,
                vec![(&b"README.md"[..], "UU"), (b"docs/notes.md", "DU")]
            );
            // raw 표기는 git status의 unmerged XY와 같음.
            let status = read_status(&gate, dir.path(), StatusOptions::default()).unwrap();
            for r in &partition.raw {
                let kind = status
                    .entries
                    .iter()
                    .find_map(|e| match e {
                        StatusEntry::Unmerged(u) if u.path == r.path => Some(u.kind),
                        _ => None,
                    })
                    .unwrap();
                assert_eq!(kind.as_str(), r.xy);
            }
            // stage 원본은 보존(외부 도구 해결용).
            assert!(partition.raw[0].stages.ours.is_some());
        }
    }

    #[test]
    fn unmerged_xy_matches_stage_presence() {
        let blob = StageBlob {
            mode: FileMode::REGULAR,
            oid: ObjectId::parse(B1).unwrap(),
        };
        let path = |b: bool, o: bool, t: bool| UnmergedPath {
            path: GitPath::from_bytes(b"x"),
            base: b.then(|| blob.clone()),
            ours: o.then(|| blob.clone()),
            theirs: t.then(|| blob.clone()),
        };
        let table = [
            ((true, true, true), "UU"),
            ((false, true, true), "AA"),
            ((true, false, true), "DU"),
            ((true, true, false), "UD"),
            ((false, true, false), "AU"),
            ((false, false, true), "UA"),
            ((true, false, false), "DD"),
        ];
        for ((b, o, t), xy) in table {
            assert_eq!(unmerged_xy(&path(b, o, t)), xy);
        }
        let parts = partition_conflicts(vec![
            UnmergedPath {
                path: GitPath::from_bytes(b"stellar.yaml"),
                ..path(true, true, true)
            },
            UnmergedPath {
                path: GitPath::from_bytes(b"LICENSE"),
                ..path(false, true, true)
            },
        ]);
        assert_eq!(parts.semantic.len(), 1);
        assert_eq!(parts.raw[0].xy, "AA");
    }
}

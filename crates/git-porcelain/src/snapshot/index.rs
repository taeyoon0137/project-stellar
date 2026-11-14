//! Git index provider (XIV.2.5).
//!
//! index의 파일 목록과 blob ID를 `git ls-files --stage -z --full-name -- :/`로 읽습니다. 레코드는
//! `<mode> SP <oid> SP <stage> TAB <path>` + NUL입니다. `--full-name`과 root pathspec(`:/`)으로 cwd가
//! 하위 디렉터리여도 저장소 root 기준 전체 index를 받습니다.
//!
//! - **stage 0**: 일반 index 항목 → [`IndexSnapshot::entries`](Index snapshot의 원본).
//! - **stage 1·2·3**: 병합되지 않은(unmerged) 항목 → [`IndexSnapshot::unmerged`]. Index snapshot에
//!   섞지 않고 별도 conflict provider(XIV.11)로 넘깁니다. unmerged 경로는 stage 0 목록에 없습니다.
//!
//! Git index가 staged 상태의 유일한 원본입니다. 이 provider는 index를 읽기만 합니다.

use std::path::Path;

use super::SnapshotEntry;
use crate::decode::{split_nul_terminated, GitPath};
use crate::object::{FileMode, ObjectId};
use crate::ops::{run_ok, GitOpError, ParseFailure};
use crate::runner::argv;
use crate::trust::GatedGitRunner;

const COMMAND: &str = "ls-files --stage -z";

/// index stage 번호가 붙은 항목.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StagedEntry {
    /// 경로.
    pub path: GitPath,
    /// mode.
    pub mode: FileMode,
    /// blob ID(gitlink면 commit ID).
    pub oid: ObjectId,
    /// stage(0 = 일반, 1 = base, 2 = ours, 3 = theirs).
    pub stage: u8,
}

/// Index snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexSnapshot {
    /// stage 0 항목(Git index 순서).
    pub entries: Vec<SnapshotEntry>,
    /// stage 1·2·3 항목(conflict provider로 넘길 항목, Git index 순서).
    pub unmerged: Vec<StagedEntry>,
}

impl IndexSnapshot {
    /// unmerged 항목이 있는지.
    pub fn has_conflicts(&self) -> bool {
        !self.unmerged.is_empty()
    }
}

/// `git ls-files --stage -z`(또는 `-u -z`) 출력을 해석합니다.
pub fn parse_ls_files_stage(stdout: &[u8]) -> Result<Vec<StagedEntry>, ParseFailure> {
    let records = split_nul_terminated(stdout)
        .map_err(|e| ParseFailure::new(COMMAND, e.to_string(), stdout))?;
    let mut entries = Vec::with_capacity(records.len());
    for record in records {
        let bad = |detail: &str| ParseFailure::new(COMMAND, detail, record);
        let tab = record
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| bad("TAB 구분자 없음"))?;
        let (meta, path) = (&record[..tab], &record[tab + 1..]);
        if path.is_empty() {
            return Err(bad("빈 경로"));
        }
        let mut fields = meta.split(|b| *b == b' ');
        let (Some(mode), Some(oid), Some(stage), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(bad("필드 수가 3개가 아님"));
        };
        let mode = FileMode::from_octal_bytes(mode)
            .filter(|m| *m != FileMode::ABSENT && *m != FileMode::TREE)
            .ok_or_else(|| bad("잘못된 mode"))?;
        let oid = ObjectId::from_hex_bytes(oid).ok_or_else(|| bad("잘못된 object ID"))?;
        let stage = match stage {
            [d @ b'0'..=b'3'] => d - b'0',
            _ => return Err(bad("잘못된 stage")),
        };
        entries.push(StagedEntry {
            path: GitPath::from_bytes(path),
            mode,
            oid,
            stage,
        });
    }
    Ok(entries)
}

/// stage 항목을 stage 0 snapshot과 unmerged 목록으로 나눕니다.
pub fn split_index_entries(staged: Vec<StagedEntry>) -> IndexSnapshot {
    let mut snapshot = IndexSnapshot::default();
    for entry in staged {
        if entry.stage == 0 {
            snapshot.entries.push(SnapshotEntry {
                path: entry.path,
                mode: entry.mode,
                oid: entry.oid,
            });
        } else {
            snapshot.unmerged.push(entry);
        }
    }
    snapshot
}

/// 현재 index를 읽습니다(trust gate 적용, 읽기 전용).
pub fn read_index_snapshot(
    git: &GatedGitRunner,
    worktree: &Path,
) -> Result<IndexSnapshot, GitOpError> {
    let output = run_ok(
        git,
        worktree,
        &argv(["ls-files", "--stage", "-z", "--full-name", "--", ":/"]),
    )?;
    Ok(split_index_entries(parse_ls_files_stage(&output.stdout)?))
}

/// 동일 provider를 취소 가능한 Git 읽기로 실행합니다.
pub fn read_index_snapshot_cancellable(
    git: &GatedGitRunner,
    worktree: &Path,
    cancel: &crate::cancel::GitCancelToken,
) -> Result<IndexSnapshot, GitOpError> {
    let output = crate::ops::require_success(git.run_cancellable(
        &argv(["ls-files", "--stage", "-z", "--full-name", "--", ":/"]),
        worktree,
        cancel,
    )?)?;
    Ok(split_index_entries(parse_ls_files_stage(&output.stdout)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const B1: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
    const B2: &str = "8baef1b4abc478178b004d62031cf7fe6db6f903";

    #[test]
    fn parses_stages_and_splits_unmerged() {
        let bytes = format!(
            "100644 {B1} 0\ta b.txt\0100755 {B1} 0\trun.sh\0120000 {B2} 0\tlink\0\
             100644 {B1} 1\tc.txt\0100644 {B2} 2\tc.txt\0100644 {B1} 3\tc.txt\0"
        );
        let staged = parse_ls_files_stage(bytes.as_bytes()).unwrap();
        assert_eq!(staged.len(), 6);
        let snap = split_index_entries(staged);
        assert_eq!(snap.entries.len(), 3);
        assert_eq!(snap.entries[0].path.as_bytes(), b"a b.txt");
        assert_eq!(snap.entries[1].mode, FileMode::EXECUTABLE);
        assert_eq!(snap.entries[2].mode, FileMode::SYMLINK);
        assert!(snap.has_conflicts());
        let stages: Vec<u8> = snap.unmerged.iter().map(|e| e.stage).collect();
        assert_eq!(stages, vec![1, 2, 3]);
        assert!(snap.unmerged.iter().all(|e| e.path.as_bytes() == b"c.txt"));
        assert!(!snap.entries.iter().any(|e| e.path.as_bytes() == b"c.txt"));
    }

    #[test]
    fn malformed_records_are_rejected() {
        for bad in [
            format!("100644 {B1} 0 a\0"),
            format!("100644 {B1} 0\t\0"),
            format!("100644 {B1}\ta\0"),
            format!("100644 {B1} 0 x\ta\0"),
            format!("100645 {B1} 0\ta\0"),
            format!("000000 {B1} 0\ta\0"),
            "100644 xyz 0\ta\0".to_string(),
            format!("100644 {B1} 4\ta\0"),
            format!("100644 {B1} 01\ta\0"),
            format!("100644 {B1} 0\ta"),
        ] {
            assert!(parse_ls_files_stage(bad.as_bytes()).is_err(), "{bad:?}");
        }
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::snapshot::{read_tree_snapshot, SnapshotSource};
        use crate::test_support::{
            commit_file, git_ok, hermetic_runner, init_repo_with_identity, system_git, write_blob,
            TempDir,
        };
        use crate::trust::{GatedRunError, GitTrust};

        fn oid_of<'a>(entries: &'a [SnapshotEntry], path: &[u8]) -> Option<&'a SnapshotEntry> {
            entries.iter().find(|e| e.path.as_bytes() == path)
        }

        #[test]
        fn partial_staged_fixture_is_exact() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("index-partial");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            std::fs::create_dir_all(repo.join("design")).unwrap();
            for (p, body) in [
                ("design/a.txt", "v1\n"),
                ("design/keep.txt", "keep\n"),
                ("gone.txt", "gone\n"),
                ("mode.sh", "#!/bin/sh\n"),
            ] {
                std::fs::write(repo.join(p), body).unwrap();
            }
            git_ok(&runner, repo, ["add", "-A"]);
            git_ok(&runner, repo, ["commit", "-q", "-m", "base"]);

            // a: HEAD=v1, Index=v2, WorkingTree=v3 (같은 파일 부분 staging).
            std::fs::write(repo.join("design/a.txt"), "v2\n").unwrap();
            git_ok(&runner, repo, ["add", "design/a.txt"]);
            std::fs::write(repo.join("design/a.txt"), "v3\n").unwrap();
            // staged 삭제, staged 새 파일, staged mode 변경, unstaged 새 파일.
            git_ok(&runner, repo, ["rm", "-q", "--cached", "gone.txt"]);
            std::fs::write(repo.join("new.txt"), "new\n").unwrap();
            git_ok(&runner, repo, ["add", "new.txt"]);
            git_ok(&runner, repo, ["update-index", "--chmod=+x", "mode.sh"]);
            std::fs::write(repo.join("untracked.txt"), "u\n").unwrap();

            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let index = read_index_snapshot(&gate, repo).unwrap();
            assert!(!index.has_conflicts());
            let paths: Vec<&[u8]> = index.entries.iter().map(|e| e.path.as_bytes()).collect();
            // staged 삭제(gone.txt)는 없고 staged 새 파일은 있으며 untracked는 없음.
            assert_eq!(
                paths,
                vec![
                    &b"design/a.txt"[..],
                    b"design/keep.txt",
                    b"mode.sh",
                    b"new.txt"
                ]
            );
            let a = oid_of(&index.entries, b"design/a.txt").unwrap();
            assert_eq!(a.oid.as_str(), write_blob(&runner, repo, b"v2\n"));
            assert_eq!(
                oid_of(&index.entries, b"mode.sh").unwrap().mode,
                FileMode::EXECUTABLE
            );
            assert!(oid_of(&index.entries, b"untracked.txt").is_none());

            // HEAD는 여전히 v1·삭제 전·mode 변경 전.
            let head = read_tree_snapshot(&gate, repo, &SnapshotSource::Head).unwrap();
            let ha = oid_of(&head.entries, b"design/a.txt").unwrap();
            assert_eq!(ha.oid.as_str(), write_blob(&runner, repo, b"v1\n"));
            assert!(oid_of(&head.entries, b"gone.txt").is_some());
            assert_eq!(
                oid_of(&head.entries, b"mode.sh").unwrap().mode,
                FileMode::REGULAR
            );

            // 하위 디렉터리 cwd에서도 root 기준 전체 index.
            let sub = read_index_snapshot(&gate, &repo.join("design")).unwrap();
            assert_eq!(sub, index);
        }

        #[test]
        fn unmerged_entries_go_to_conflict_list_not_stage0() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("index-unmerged");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            commit_file(&runner, repo, "c.txt", b"base\n");
            commit_file(&runner, repo, "ok.txt", b"ok\n");
            git_ok(&runner, repo, ["switch", "-q", "-c", "other"]);
            commit_file(&runner, repo, "c.txt", b"theirs\n");
            git_ok(&runner, repo, ["switch", "-q", "main"]);
            commit_file(&runner, repo, "c.txt", b"ours\n");
            assert!(!runner
                .run(&argv(["merge", "-q", "other"]), repo)
                .unwrap()
                .success());

            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let index = read_index_snapshot(&gate, repo).unwrap();
            assert!(index.has_conflicts());
            assert!(oid_of(&index.entries, b"c.txt").is_none());
            assert!(oid_of(&index.entries, b"ok.txt").is_some());
            let stages: Vec<(u8, String)> = index
                .unmerged
                .iter()
                .map(|e| (e.stage, e.oid.as_str().to_string()))
                .collect();
            assert_eq!(
                stages,
                vec![
                    (1, write_blob(&runner, repo, b"base\n")),
                    (2, write_blob(&runner, repo, b"ours\n")),
                    (3, write_blob(&runner, repo, b"theirs\n")),
                ]
            );
        }

        #[test]
        fn untrusted_index_read_spawns_nothing() {
            let Some(git) = system_git() else {
                return;
            };
            let runner = hermetic_runner(git);
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted);
            assert_eq!(
                read_index_snapshot(&gate, &std::env::temp_dir()),
                Err(GitOpError::Gate(GatedRunError::ProjectNotTrusted))
            );
            assert_eq!(runner.spawn_count(), 0);
        }
    }
}

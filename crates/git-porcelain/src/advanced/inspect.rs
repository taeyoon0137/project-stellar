//! tags·notes·bisect·sparse-checkout·submodule 상태 읽기 (XXVI.4.1).
//!
//! 로드맵 기능의 **읽기 전용** core API입니다. 쓰기(생성·삭제·변경)는 기능별 별도 작업·승인 후에만
//! 추가하며 기본 비활성 feature flag 뒤에 둡니다(XXVI.4.2). 모든 조회는 상태를 바꾸지 않는 Git 명령만 씁니다.
//!
//! | 대상 | 명령 |
//! |---|---|
//! | tags | `for-each-ref refs/tags`(이름·종류·대상·annotated tag object·제목) |
//! | notes | `notes list`(기본 notes ref의 note blob ↔ 대상 object) |
//! | bisect | `for-each-ref refs/bisect/` + `--git-path BISECT_START` 존재 |
//! | sparse-checkout | `config --bool core.sparseCheckout`·`core.sparseCheckoutCone` + `sparse-checkout list` |
//! | submodule | index gitlink(`ls-files --stage`) + `.gitmodules`(`config --file`) + local config 초기화 여부 |
//!
//! submodule 조회는 submodule 저장소 안에서 Git을 실행하지 않습니다(중첩 저장소 config를 읽지 않음).
//! submodule URL은 민감값([`RemoteUrl`])으로 다룹니다.

use std::path::Path;

use crate::decode::{split_nul_terminated, GitPath};
use crate::object::{FileMode, ObjectId};
use crate::ops::{failure_of, run_ok, GitOpError, ParseFailure};
use crate::remote::RemoteUrl;
use crate::runner::argv;
use crate::snapshot::index::read_index_snapshot;
use crate::trust::GatedGitRunner;

/// tag 정보.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagInfo {
    /// tag 이름(`refs/tags/` 제외).
    pub name: String,
    /// annotated tag object(lightweight면 `None`).
    pub tag_object: Option<ObjectId>,
    /// tag가 가리키는 object(annotated는 한 단계 peel한 대상).
    pub target: ObjectId,
    /// 제목(annotated는 tag 메시지, lightweight는 대상 commit 제목).
    pub subject: String,
}

/// tag 목록(이름 순).
pub fn list_tags(git: &GatedGitRunner, worktree: &Path) -> Result<Vec<TagInfo>, GitOpError> {
    let out = run_ok(
        git,
        worktree,
        &argv([
            "for-each-ref",
            "--format=%(refname)%00%(objecttype)%00%(objectname)%00%(*objectname)%00%(subject)",
            "refs/tags",
        ]),
    )?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut tags = Vec::new();
    for line in text.lines() {
        let bad = |d: &str| {
            GitOpError::Parse(ParseFailure::new(
                "for-each-ref refs/tags",
                d,
                line.as_bytes(),
            ))
        };
        let fields: Vec<&str> = line.splitn(5, '\0').collect();
        let [refname, kind, oid, peeled, subject] = fields[..] else {
            return Err(bad("필드 수가 5개가 아님"));
        };
        let name = refname
            .strip_prefix("refs/tags/")
            .ok_or_else(|| bad("refs/tags/ 아님"))?;
        let oid = ObjectId::parse(oid).ok_or_else(|| bad("잘못된 object ID"))?;
        let (tag_object, target) = if kind == "tag" {
            let peeled = ObjectId::parse(peeled).ok_or_else(|| bad("annotated 대상 없음"))?;
            (Some(oid), peeled)
        } else {
            (None, oid)
        };
        tags.push(TagInfo {
            name: name.to_string(),
            tag_object,
            target,
            subject: subject.to_string(),
        });
    }
    Ok(tags)
}

/// note 하나(기본 notes ref).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteEntry {
    /// note blob.
    pub note: ObjectId,
    /// note가 붙은 object.
    pub object: ObjectId,
}

/// 기본 notes ref(`refs/notes/commits`)의 note 목록.
pub fn list_notes(git: &GatedGitRunner, worktree: &Path) -> Result<Vec<NoteEntry>, GitOpError> {
    let out = run_ok(git, worktree, &argv(["notes", "list"]))?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .map(|line| {
            let (note, object) = line.split_once(' ').unwrap_or((line, ""));
            match (ObjectId::parse(note), ObjectId::parse(object)) {
                (Some(note), Some(object)) => Ok(NoteEntry { note, object }),
                _ => Err(GitOpError::Parse(ParseFailure::new(
                    "notes list",
                    "잘못된 줄",
                    line.as_bytes(),
                ))),
            }
        })
        .collect()
}

/// bisect 상태(기본 용어 bad/good 및 사용자 용어 ref 보존).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BisectState {
    /// bisect 진행 중(`BISECT_START` 있음).
    pub active: bool,
    /// `refs/bisect/bad`.
    pub bad: Option<ObjectId>,
    /// `refs/bisect/good-*`.
    pub good: Vec<ObjectId>,
    /// `refs/bisect/skip-*`.
    pub skipped: Vec<ObjectId>,
    /// 그 밖의 `refs/bisect/*`(사용자 용어 등) 이름과 값.
    pub other: Vec<(String, ObjectId)>,
}

/// bisect 상태를 읽습니다.
pub fn read_bisect_state(git: &GatedGitRunner, worktree: &Path) -> Result<BisectState, GitOpError> {
    let out = run_ok(
        git,
        worktree,
        &argv([
            "for-each-ref",
            "--format=%(refname)%00%(objectname)",
            "refs/bisect/",
        ]),
    )?;
    let mut state = BisectState::default();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let (name, oid) = line.split_once('\0').unwrap_or((line, ""));
        let oid = ObjectId::parse(oid).ok_or_else(|| {
            GitOpError::Parse(ParseFailure::new(
                "for-each-ref refs/bisect",
                "잘못된 줄",
                line.as_bytes(),
            ))
        })?;
        let short = name.strip_prefix("refs/bisect/").unwrap_or(name);
        if short == "bad" {
            state.bad = Some(oid);
        } else if short.starts_with("good-") {
            state.good.push(oid);
        } else if short.starts_with("skip-") {
            state.skipped.push(oid);
        } else {
            state.other.push((short.to_string(), oid));
        }
    }
    let start = run_ok(
        git,
        worktree,
        &argv([
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "BISECT_START",
        ]),
    )?;
    let path_bytes = start.stdout.strip_suffix(b"\n").unwrap_or(&start.stdout);
    state.active = crate::decode::path_from_git_bytes(path_bytes).is_some_and(|p| p.exists());
    Ok(state)
}

/// sparse-checkout 상태.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SparseCheckoutState {
    /// `core.sparseCheckout`.
    pub enabled: bool,
    /// cone mode(`core.sparseCheckoutCone`).
    pub cone: bool,
    /// `git sparse-checkout list` 결과(enabled일 때만).
    pub patterns: Vec<String>,
}

fn config_bool(
    git: &GatedGitRunner,
    worktree: &Path,
    key: &str,
) -> Result<Option<bool>, GitOpError> {
    let out = git.run(&argv(["config", "--get", "--bool", key]), worktree)?;
    match out.exit_code {
        Some(0) => Ok(Some(out.stdout.trim_ascii() == b"true")),
        Some(1) => Ok(None),
        _ => Err(GitOpError::Git(failure_of(out))),
    }
}

/// sparse-checkout 상태를 읽습니다.
pub fn read_sparse_checkout(
    git: &GatedGitRunner,
    worktree: &Path,
) -> Result<SparseCheckoutState, GitOpError> {
    let enabled = config_bool(git, worktree, "core.sparseCheckout")?.unwrap_or(false);
    if !enabled {
        return Ok(SparseCheckoutState::default());
    }
    let cone = config_bool(git, worktree, "core.sparseCheckoutCone")?.unwrap_or(false);
    let out = run_ok(git, worktree, &argv(["sparse-checkout", "list"]))?;
    let patterns = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect();
    Ok(SparseCheckoutState {
        enabled,
        cone,
        patterns,
    })
}

/// submodule 정보.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmoduleInfo {
    /// 경로.
    pub path: GitPath,
    /// index에 기록된 commit.
    pub recorded_commit: ObjectId,
    /// `.gitmodules`의 이름(없으면 `None`).
    pub name: Option<String>,
    /// `.gitmodules`의 URL(민감값).
    pub url: Option<RemoteUrl>,
    /// local config에 `submodule.<name>.url`이 있음(`git submodule init`됨).
    pub initialized: bool,
    /// 작업 트리에 checkout되어 있음(경로에 `.git` 항목 존재).
    pub checked_out: bool,
}

/// `git config -z --get-regexp` 출력 → (key, value) 목록.
fn config_pairs(stdout: &[u8]) -> Vec<(String, String)> {
    split_nul_terminated(stdout)
        .unwrap_or_default()
        .into_iter()
        .map(|record| {
            let text = String::from_utf8_lossy(record);
            match text.split_once('\n') {
                Some((k, v)) => (k.to_string(), v.to_string()),
                None => (text.into_owned(), String::new()),
            }
        })
        .collect()
}

fn regexp_config(
    git: &GatedGitRunner,
    worktree: &Path,
    file: Option<&str>,
    pattern: &str,
) -> Result<Vec<(String, String)>, GitOpError> {
    let mut args = argv(["config", "-z"]);
    if let Some(file) = file {
        args.extend(argv(["--file", file]));
    }
    args.extend(argv(["--get-regexp", pattern]));
    let out = git.run(&args, worktree)?;
    match out.exit_code {
        Some(0) => Ok(config_pairs(&out.stdout)),
        Some(1) => Ok(Vec::new()),
        _ => Err(GitOpError::Git(failure_of(out))),
    }
}

/// submodule 상태를 읽습니다(submodule 저장소 안에서 Git을 실행하지 않음). `worktree`는 root여야 합니다.
pub fn list_submodules(
    git: &GatedGitRunner,
    worktree: &Path,
) -> Result<Vec<SubmoduleInfo>, GitOpError> {
    let index = read_index_snapshot(git, worktree)?;
    let gitlinks: Vec<_> = index
        .entries
        .into_iter()
        .filter(|e| e.mode == FileMode::GITLINK)
        .collect();
    if gitlinks.is_empty() {
        return Ok(Vec::new());
    }
    let has_gitmodules = worktree.join(".gitmodules").is_file();
    let modules = if has_gitmodules {
        regexp_config(
            git,
            worktree,
            Some(".gitmodules"),
            r"^submodule\..*\.(path|url)$",
        )?
    } else {
        Vec::new()
    };
    let local_urls = regexp_config(git, worktree, None, r"^submodule\..*\.url$")?;
    let name_of = |key: &str, suffix: &str| {
        key.strip_prefix("submodule.")
            .and_then(|k| k.strip_suffix(suffix))
            .map(str::to_string)
    };
    let mut out = Vec::new();
    for link in gitlinks {
        let path_text = link.path.text().display().to_string();
        let name = modules
            .iter()
            .find(|(k, v)| k.ends_with(".path") && *v == path_text)
            .and_then(|(k, _)| name_of(k, ".path"));
        let url = name.as_ref().and_then(|n| {
            modules
                .iter()
                .find(|(k, _)| name_of(k, ".url").as_deref() == Some(n.as_str()))
                .map(|(_, v)| RemoteUrl::new(v.clone()))
        });
        let initialized = name.as_ref().is_some_and(|n| {
            local_urls
                .iter()
                .any(|(k, _)| name_of(k, ".url").as_deref() == Some(n.as_str()))
        });
        let checked_out = link
            .path
            .to_path_buf()
            .is_some_and(|p| std::fs::symlink_metadata(worktree.join(p).join(".git")).is_ok());
        out.push(SubmoduleInfo {
            path: link.path,
            recorded_commit: link.oid,
            name,
            url,
            initialized,
            checked_out,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_pairs_parse_nul_records() {
        let pairs = config_pairs(
            b"submodule.libs/a.path\nlibs/a\0submodule.x.url\nhttps://h/x.git\0flag\0",
        );
        assert_eq!(
            pairs,
            vec![
                ("submodule.libs/a.path".to_string(), "libs/a".to_string()),
                ("submodule.x.url".to_string(), "https://h/x.git".to_string()),
                ("flag".to_string(), String::new()),
            ]
        );
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::test_support::{
            commit_file, git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use crate::trust::GitTrust;

        #[test]
        fn read_only_inventory_fixture() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("adv-inspect");
            let runner = hermetic_runner(git);
            let repo = dir.path().join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            init_repo_with_identity(&runner, &repo);
            commit_file(&runner, &repo, "design/a.txt", b"1\n");
            commit_file(&runner, &repo, "assets/b.txt", b"2\n");
            commit_file(&runner, &repo, "design/a.txt", b"3\n");
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let rev = |r: &str| {
                ObjectId::from_hex_bytes(
                    git_ok(&runner, &repo, ["rev-parse", r]).stdout.trim_ascii(),
                )
                .unwrap()
            };

            // 아무것도 없을 때.
            assert!(list_tags(&gate, &repo).unwrap().is_empty());
            assert!(list_notes(&gate, &repo).unwrap().is_empty());
            assert_eq!(
                read_bisect_state(&gate, &repo).unwrap(),
                BisectState::default()
            );
            assert_eq!(
                read_sparse_checkout(&gate, &repo).unwrap(),
                SparseCheckoutState::default()
            );
            assert!(list_submodules(&gate, &repo).unwrap().is_empty());

            // tags: lightweight + annotated.
            git_ok(&runner, &repo, ["tag", "light", "HEAD~1"]);
            git_ok(
                &runner,
                &repo,
                ["tag", "-a", "v1.0", "-m", "첫 릴리스", "HEAD"],
            );
            let tags = list_tags(&gate, &repo).unwrap();
            assert_eq!(tags.len(), 2);
            assert_eq!(tags[0].name, "light");
            assert_eq!(tags[0].tag_object, None);
            assert_eq!(tags[0].target, rev("HEAD~1"));
            assert_eq!(tags[1].name, "v1.0");
            assert_eq!(tags[1].tag_object, Some(rev("v1.0")));
            assert_eq!(tags[1].target, rev("HEAD"));
            assert_eq!(tags[1].subject, "첫 릴리스");

            // notes.
            git_ok(&runner, &repo, ["notes", "add", "-m", "검토 완료", "HEAD"]);
            let notes = list_notes(&gate, &repo).unwrap();
            assert_eq!(notes.len(), 1);
            assert_eq!(notes[0].object, rev("HEAD"));

            // bisect.
            git_ok(&runner, &repo, ["bisect", "start"]);
            git_ok(&runner, &repo, ["bisect", "bad", "HEAD"]);
            git_ok(&runner, &repo, ["bisect", "good", "HEAD~2"]);
            let bisect = read_bisect_state(&gate, &repo).unwrap();
            assert!(bisect.active);
            assert_eq!(bisect.bad, Some(rev("refs/bisect/bad")));
            assert_eq!(bisect.good.len(), 1);
            git_ok(&runner, &repo, ["bisect", "reset"]);
            assert!(!read_bisect_state(&gate, &repo).unwrap().active);

            // sparse-checkout(cone).
            git_ok(
                &runner,
                &repo,
                ["sparse-checkout", "set", "--cone", "design"],
            );
            let sparse = read_sparse_checkout(&gate, &repo).unwrap();
            assert!(sparse.enabled && sparse.cone);
            assert_eq!(sparse.patterns, vec!["design".to_string()]);
            assert!(!repo.join("assets/b.txt").exists());
            git_ok(&runner, &repo, ["sparse-checkout", "disable"]);

            // submodule(로컬 저장소에서 추가).
            let lib = dir.path().join("lib");
            std::fs::create_dir_all(&lib).unwrap();
            init_repo_with_identity(&runner, &lib);
            commit_file(&runner, &lib, "x.txt", b"x\n");
            git_ok(
                &runner,
                &repo,
                [
                    "-c",
                    "protocol.file.allow=always",
                    "submodule",
                    "add",
                    "-q",
                    lib.to_str().unwrap(),
                    "libs/lib",
                ],
            );
            let subs = list_submodules(&gate, &repo).unwrap();
            assert_eq!(subs.len(), 1);
            assert_eq!(subs[0].path.as_bytes(), b"libs/lib");
            assert_eq!(subs[0].name.as_deref(), Some("libs/lib"));
            assert_eq!(
                subs[0].url.as_ref().unwrap().expose(),
                lib.to_str().unwrap()
            );
            assert!(subs[0].initialized && subs[0].checked_out);
            let lib_head = git_ok(&runner, &lib, ["rev-parse", "HEAD"]);
            assert_eq!(
                subs[0].recorded_commit.as_str().as_bytes(),
                lib_head.stdout.trim_ascii()
            );
            assert!(
                !format!("{subs:?}").contains(lib.to_str().unwrap()),
                "URL 노출"
            );

            // 읽기 API들은 상태를 바꾸지 않음.
            let status_before = git_ok(&runner, &repo, ["status", "--porcelain=v2", "-z"]).stdout;
            let _ = list_tags(&gate, &repo).unwrap();
            let _ = list_submodules(&gate, &repo).unwrap();
            let _ = read_bisect_state(&gate, &repo).unwrap();
            assert_eq!(
                git_ok(&runner, &repo, ["status", "--porcelain=v2", "-z"]).stdout,
                status_before
            );
        }
    }
}

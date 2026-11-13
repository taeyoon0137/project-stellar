//! Git worktree 목록 (XXVI.3.1~).
//!
//! `git worktree list --porcelain -z`(Git 2.36+)를 해석합니다. 속성 줄은 NUL로 끝나고 worktree 레코드는
//! 빈 줄(연속 NUL)로 구분됩니다. 속성: `worktree <path>`, `HEAD <oid>`, `branch <ref>`, `detached`,
//! `bare`, `locked [<reason>]`, `prunable [<reason>]`. 첫 레코드가 주 worktree입니다. 경로는 원시 바이트로
//! 보존하고, 모르는 속성은 Git 문서대로 무시하되 보존합니다.

use std::path::{Path, PathBuf};

use crate::decode::path_from_git_bytes;
use crate::object::ObjectId;
use crate::ops::{run_ok, GitOpError, ParseFailure};
use crate::runner::argv;
use crate::trust::GatedGitRunner;

/// lock·prunable 표시(이유는 선택).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Annotation {
    /// 표시 없음.
    No,
    /// 표시 있음(이유가 있으면 텍스트).
    Yes {
        /// 이유.
        reason: Option<String>,
    },
}

impl Annotation {
    /// 표시가 있는지.
    pub fn is_set(&self) -> bool {
        matches!(self, Annotation::Yes { .. })
    }
}

/// worktree 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    /// worktree 경로.
    pub path: PathBuf,
    /// 주 worktree인지(목록 첫 항목).
    pub is_main: bool,
    /// HEAD commit(bare·unborn이면 `None`).
    pub head: Option<ObjectId>,
    /// branch ref(`refs/heads/...`).
    pub branch: Option<String>,
    /// detached HEAD.
    pub detached: bool,
    /// bare 저장소.
    pub bare: bool,
    /// lock 상태.
    pub locked: Annotation,
    /// prune 대상 여부.
    pub prunable: Annotation,
    /// 해석하지 않은 속성 원문.
    pub unknown: Vec<String>,
}

const COMMAND: &str = "worktree list --porcelain -z";

fn annotation(rest: Option<&str>) -> Annotation {
    Annotation::Yes {
        reason: rest.filter(|r| !r.is_empty()).map(str::to_string),
    }
}

/// `git worktree list --porcelain -z` 출력을 해석합니다.
pub fn parse_worktree_list(stdout: &[u8]) -> Result<Vec<WorktreeEntry>, ParseFailure> {
    if stdout.is_empty() {
        return Ok(Vec::new());
    }
    if !stdout.ends_with(b"\0\0") {
        return Err(ParseFailure::new(
            COMMAND,
            "레코드가 빈 줄로 끝나지 않음",
            stdout,
        ));
    }
    let body = &stdout[..stdout.len() - 2];
    let mut out = Vec::new();
    for record in body
        .split(|b| *b == 0)
        .collect::<Vec<_>>()
        .split(|l| l.is_empty())
    {
        let bad = |d: &str| ParseFailure::new(COMMAND, d, record.concat().as_slice());
        let (first, rest) = record.split_first().ok_or_else(|| bad("빈 레코드"))?;
        let path = first
            .strip_prefix(b"worktree ")
            .filter(|p| !p.is_empty())
            .and_then(path_from_git_bytes)
            .ok_or_else(|| bad("worktree 줄 없음"))?;
        let mut entry = WorktreeEntry {
            path,
            is_main: out.is_empty(),
            head: None,
            branch: None,
            detached: false,
            bare: false,
            locked: Annotation::No,
            prunable: Annotation::No,
            unknown: Vec::new(),
        };
        for line in rest {
            let text = String::from_utf8_lossy(line);
            let (key, value) = match text.split_once(' ') {
                Some((k, v)) => (k, Some(v)),
                None => (text.as_ref(), None),
            };
            match (key, value) {
                ("HEAD", Some(oid)) => {
                    entry.head = Some(ObjectId::parse(oid).ok_or_else(|| bad("잘못된 HEAD"))?)
                }
                ("branch", Some(r)) if r.starts_with("refs/") => entry.branch = Some(r.to_string()),
                ("detached", None) => entry.detached = true,
                ("bare", None) => entry.bare = true,
                ("locked", reason) => entry.locked = annotation(reason),
                ("prunable", reason) => entry.prunable = annotation(reason),
                ("worktree", _) => return Err(bad("레코드 안의 두 번째 worktree 줄")),
                _ => entry.unknown.push(text.into_owned()),
            }
        }
        out.push(entry);
    }
    Ok(out)
}

/// worktree 목록을 읽습니다(trust gate 적용, 읽기 전용).
pub fn list_worktrees(git: &GatedGitRunner, cwd: &Path) -> Result<Vec<WorktreeEntry>, GitOpError> {
    let out = run_ok(git, cwd, &argv(["worktree", "list", "--porcelain", "-z"]))?;
    Ok(parse_worktree_list(&out.stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const E1: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";

    #[test]
    fn parses_main_linked_locked_prunable_and_bare() {
        let out = format!(
            "worktree /repo\0HEAD {E1}\0branch refs/heads/main\0\0\
             worktree /wt/feature x\0HEAD {E1}\0branch refs/heads/feature\0locked 작업 중\0\0\
             worktree /wt/detached\0HEAD {E1}\0detached\0locked\0\0\
             worktree /wt/gone\0HEAD {E1}\0detached\0prunable gitdir file points to non-existent location\0future-attr x\0\0"
        );
        let list = parse_worktree_list(out.as_bytes()).unwrap();
        assert_eq!(list.len(), 4);
        assert!(list[0].is_main && !list[1].is_main);
        assert_eq!(list[1].path, PathBuf::from("/wt/feature x"));
        assert_eq!(list[1].branch.as_deref(), Some("refs/heads/feature"));
        assert_eq!(
            list[1].locked,
            Annotation::Yes {
                reason: Some("작업 중".into())
            }
        );
        assert!(list[2].detached);
        assert_eq!(list[2].locked, Annotation::Yes { reason: None });
        assert!(list[3].prunable.is_set());
        assert_eq!(list[3].unknown, vec!["future-attr x".to_string()]);
        let bare = parse_worktree_list(b"worktree /srv/r.git\0bare\0\0").unwrap();
        assert!(bare[0].bare && bare[0].head.is_none());
        assert!(parse_worktree_list(b"").unwrap().is_empty());
    }

    #[test]
    fn malformed_lists_are_rejected() {
        for bad in [
            format!("worktree /a\0HEAD {E1}\0"),
            format!("HEAD {E1}\0\0"),
            "worktree /a\0HEAD zz\0\0".to_string(),
            "worktree /a\0worktree /b\0\0".to_string(),
            "worktree \0\0".to_string(),
        ] {
            assert!(parse_worktree_list(bad.as_bytes()).is_err(), "{bad:?}");
        }
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::test_support::{
            canonical, commit_file, git_ok, hermetic_runner, init_repo_with_identity, system_git,
            TempDir,
        };
        use crate::trust::GitTrust;

        #[test]
        fn main_linked_locked_prunable_fixture() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("adv-wtlist");
            let runner = hermetic_runner(git);
            let repo = dir.path().join("main repo");
            std::fs::create_dir_all(&repo).unwrap();
            init_repo_with_identity(&runner, &repo);
            commit_file(&runner, &repo, "stellar.yaml", b"v: 1\n");
            let linked = dir.path().join("linked wt");
            let locked = dir.path().join("locked");
            let gone = dir.path().join("gone");
            for (path, extra) in [
                (&linked, vec!["-b", "feature"]),
                (&locked, vec!["--detach"]),
                (&gone, vec!["-b", "temp"]),
            ] {
                let mut args = vec!["worktree", "add", "-q"];
                args.extend(extra);
                args.push(path.to_str().unwrap());
                git_ok(&runner, &repo, args);
            }
            git_ok(
                &runner,
                &repo,
                [
                    "worktree",
                    "lock",
                    "--reason",
                    "외장 디스크",
                    locked.to_str().unwrap(),
                ],
            );
            std::fs::remove_dir_all(&gone).unwrap();

            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let list = list_worktrees(&gate, &repo).unwrap();
            assert_eq!(list.len(), 4);
            assert!(list[0].is_main);
            assert_eq!(canonical(&list[0].path), canonical(&repo));
            assert_eq!(list[0].branch.as_deref(), Some("refs/heads/main"));
            let find = |p: &Path| {
                list.iter()
                    .find(|e| e.path.file_name() == p.file_name())
                    .unwrap()
            };
            assert_eq!(find(&linked).branch.as_deref(), Some("refs/heads/feature"));
            let l = find(&locked);
            assert!(l.detached);
            assert_eq!(
                l.locked,
                Annotation::Yes {
                    reason: Some("외장 디스크".into())
                }
            );
            assert!(find(&gone).prunable.is_set());
            // linked worktree 안에서 읽어도 같은 목록.
            let from_linked = list_worktrees(&gate, &linked).unwrap();
            assert_eq!(from_linked.len(), 4);
            assert_eq!(canonical(&from_linked[0].path), canonical(&repo));
        }
    }
}

//! `RepositoryStatus` — HEAD·upstream·변경 수·conflict 상태 (XIV.2.2).
//!
//! [`StatusReport`](XIV.2.1)의 branch header와 항목에서 저장소 요약 상태를 만듭니다.
//!
//! - **HEAD**: branch(commit 있음), detached(commit), unborn(아직 commit 없는 branch)을 구분합니다.
//! - **upstream**: 없음, 추적 중(ahead/behind), 사라짐(설정은 있으나 Git이 `branch.ab`를 생략)을
//!   구분합니다.
//! - **변경 수**: staged(HEAD→index 변경), unstaged(index→worktree 변경), untracked, ignored,
//!   conflicted(unmerged 경로). `MM`처럼 양쪽이 바뀐 항목은 staged와 unstaged에 모두 셉니다.
//! - **conflict 상태**: unmerged 경로 수와 진행 중인 merge·cherry-pick·revert·rebase·am. 진행 상태는
//!   pseudo-ref를 `git rev-parse -q --verify`로, rebase·am 상태 디렉터리는 Git이 알려 주는
//!   `--git-path` 경로로 확인합니다(`.git` 경로를 추측하지 않음, 읽기만 함).
//!
//! 이 상태는 **저장된 파일 기준**입니다(Git status는 디스크의 파일만 봅니다, XIV.2.7).

use std::path::{Path, PathBuf};

use crate::decode::path_from_git_bytes;
use crate::object::ObjectId;
use crate::ops::{failure_of, run_ok, GitOpError, ParseFailure};
use crate::runner::argv;
use crate::status::{read_status, BranchHead, BranchOid, StatusEntry, StatusOptions, StatusReport};
use crate::trust::GatedGitRunner;

/// HEAD 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadState {
    /// branch가 commit을 가리킴.
    Branch {
        /// branch 이름(원시 바이트).
        name: Vec<u8>,
        /// 현재 commit.
        commit: ObjectId,
    },
    /// detached HEAD.
    Detached {
        /// 현재 commit.
        commit: ObjectId,
    },
    /// 아직 commit이 없는 branch(unborn).
    Unborn {
        /// branch 이름.
        branch: Vec<u8>,
    },
}

impl HeadState {
    /// 현재 commit(unborn이면 `None`).
    pub fn commit(&self) -> Option<&ObjectId> {
        match self {
            HeadState::Branch { commit, .. } | HeadState::Detached { commit } => Some(commit),
            HeadState::Unborn { .. } => None,
        }
    }
}

/// upstream 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpstreamState {
    /// upstream 미설정(또는 detached).
    None,
    /// upstream 추적 중.
    Tracking {
        /// upstream 이름(예: `origin/main`).
        name: Vec<u8>,
        /// upstream보다 앞선 commit 수.
        ahead: u64,
        /// upstream보다 뒤처진 commit 수.
        behind: u64,
    },
    /// upstream이 설정되어 있으나 ref가 사라짐.
    Gone {
        /// 설정된 upstream 이름.
        name: Vec<u8>,
    },
}

/// 변경 수.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChangeCounts {
    /// HEAD→index 변경 경로 수.
    pub staged: u64,
    /// index→worktree 변경 경로 수.
    pub unstaged: u64,
    /// untracked 파일 수.
    pub untracked: u64,
    /// ignored 파일 수(ignored를 요청했을 때만).
    pub ignored: u64,
    /// unmerged 경로 수.
    pub conflicted: u64,
}

/// 진행 중인 Git 작업.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum InProgressOperation {
    /// merge(`MERGE_HEAD`).
    Merge,
    /// cherry-pick(`CHERRY_PICK_HEAD`).
    CherryPick,
    /// revert(`REVERT_HEAD`).
    Revert,
    /// rebase(`rebase-merge` 또는 `rebase-apply` 상태).
    Rebase,
    /// `git am`(`rebase-apply/applying`).
    ApplyMailbox,
}

/// conflict 상태.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConflictState {
    /// unmerged 경로 수.
    pub unmerged_paths: u64,
    /// 진행 중인 작업(고정 순서).
    pub operations: Vec<InProgressOperation>,
}

impl ConflictState {
    /// unmerged 경로가 있거나 작업이 진행 중인지.
    pub fn is_active(&self) -> bool {
        self.unmerged_paths > 0 || !self.operations.is_empty()
    }
}

/// 저장소 요약 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryStatus {
    /// HEAD.
    pub head: HeadState,
    /// upstream.
    pub upstream: UpstreamState,
    /// 변경 수.
    pub counts: ChangeCounts,
    /// conflict 상태.
    pub conflict: ConflictState,
}

const COMMAND: &str = "status --porcelain=v2 --branch";

/// HEAD 상태를 branch header에서 만듭니다.
fn head_state(oid: &BranchOid, head: &BranchHead) -> Result<HeadState, ParseFailure> {
    match (oid, head) {
        (BranchOid::Commit(commit), BranchHead::Branch(name)) => Ok(HeadState::Branch {
            name: name.clone(),
            commit: commit.clone(),
        }),
        (BranchOid::Commit(commit), BranchHead::Detached) => Ok(HeadState::Detached {
            commit: commit.clone(),
        }),
        (BranchOid::Initial, BranchHead::Branch(name)) => Ok(HeadState::Unborn {
            branch: name.clone(),
        }),
        (BranchOid::Initial, BranchHead::Detached) => {
            Err(ParseFailure::new(COMMAND, "commit 없는 detached HEAD", b""))
        }
    }
}

/// status 항목에서 변경 수를 셉니다.
pub fn count_changes(entries: &[StatusEntry]) -> ChangeCounts {
    let mut counts = ChangeCounts::default();
    for entry in entries {
        let changed = match entry {
            StatusEntry::Ordinary(e) => Some(e),
            StatusEntry::Renamed(e) => Some(&e.entry),
            StatusEntry::Unmerged(_) => {
                counts.conflicted += 1;
                None
            }
            StatusEntry::Untracked(_) => {
                counts.untracked += 1;
                None
            }
            StatusEntry::Ignored(_) => {
                counts.ignored += 1;
                None
            }
        };
        if let Some(e) = changed {
            if e.index.is_changed() {
                counts.staged += 1;
            }
            if e.worktree.is_changed() {
                counts.unstaged += 1;
            }
        }
    }
    counts
}

impl RepositoryStatus {
    /// 이 상태의 기준: 디스크에 **저장된** 파일(편집기 저장 대기 상태 제외, XIV.2.7).
    pub fn basis(&self) -> crate::snapshot::SnapshotBasis {
        crate::snapshot::SnapshotBasis::SavedFilesOnDisk
    }

    /// `--branch`로 얻은 [`StatusReport`]와 진행 중 작업 목록에서 상태를 만듭니다.
    pub fn from_report(
        report: &StatusReport,
        operations: Vec<InProgressOperation>,
    ) -> Result<RepositoryStatus, ParseFailure> {
        let branch = report
            .branch
            .as_ref()
            .ok_or_else(|| ParseFailure::new(COMMAND, "branch header 없음(--branch 필요)", b""))?;
        let head = head_state(&branch.oid, &branch.head)?;
        let upstream = match (&branch.upstream, branch.ahead_behind) {
            (None, _) => UpstreamState::None,
            (Some(name), Some(ab)) => UpstreamState::Tracking {
                name: name.clone(),
                ahead: ab.ahead,
                behind: ab.behind,
            },
            (Some(name), None) => UpstreamState::Gone { name: name.clone() },
        };
        let counts = count_changes(&report.entries);
        let mut operations = operations;
        operations.sort();
        operations.dedup();
        Ok(RepositoryStatus {
            head,
            upstream,
            counts,
            conflict: ConflictState {
                unmerged_paths: counts.conflicted,
                operations,
            },
        })
    }
}

/// pseudo-ref가 존재하는지(`git rev-parse -q --verify <ref>`: 0=있음, 1=없음).
fn pseudo_ref_exists(
    git: &GatedGitRunner,
    worktree: &Path,
    name: &str,
) -> Result<bool, GitOpError> {
    let output = git.run(&argv(["rev-parse", "-q", "--verify", name]), worktree)?;
    match output.exit_code {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(GitOpError::Git(failure_of(output))),
    }
}

/// Git이 알려 주는 Git dir 내부 경로(`--git-path`, 절대 경로).
fn git_path(git: &GatedGitRunner, worktree: &Path, name: &str) -> Result<PathBuf, GitOpError> {
    let out = run_ok(
        git,
        worktree,
        &argv(["rev-parse", "--path-format=absolute", "--git-path", name]),
    )?;
    let bytes = out.stdout.strip_suffix(b"\n").unwrap_or(&out.stdout);
    path_from_git_bytes(bytes)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| {
            GitOpError::Parse(ParseFailure::new(
                "rev-parse --git-path",
                "절대 경로 아님",
                &out.stdout,
            ))
        })
}

/// 진행 중인 merge·cherry-pick·revert·rebase·am을 확인합니다(읽기 전용).
pub fn read_in_progress_operations(
    git: &GatedGitRunner,
    worktree: &Path,
) -> Result<Vec<InProgressOperation>, GitOpError> {
    let mut ops = Vec::new();
    for (name, op) in [
        ("MERGE_HEAD", InProgressOperation::Merge),
        ("CHERRY_PICK_HEAD", InProgressOperation::CherryPick),
        ("REVERT_HEAD", InProgressOperation::Revert),
    ] {
        if pseudo_ref_exists(git, worktree, name)? {
            ops.push(op);
        }
    }
    if git_path(git, worktree, "rebase-merge")?.is_dir() {
        ops.push(InProgressOperation::Rebase);
    }
    let apply = git_path(git, worktree, "rebase-apply")?;
    if apply.is_dir() {
        ops.push(if apply.join("applying").exists() {
            InProgressOperation::ApplyMailbox
        } else {
            InProgressOperation::Rebase
        });
    }
    Ok(ops)
}

/// status와 진행 중 작업을 읽어 [`RepositoryStatus`]를 만듭니다(trust gate 적용).
pub fn read_repository_status(
    git: &GatedGitRunner,
    worktree: &Path,
) -> Result<RepositoryStatus, GitOpError> {
    let report = read_status(git, worktree, StatusOptions::default())?;
    let operations = read_in_progress_operations(git, worktree)?;
    Ok(RepositoryStatus::from_report(&report, operations)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::parse_status_v2;

    const E1: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";

    #[test]
    fn head_and_upstream_states_from_headers() {
        let r = parse_status_v2(b"# branch.oid (initial)\0# branch.head main\0").unwrap();
        let s = RepositoryStatus::from_report(&r, Vec::new()).unwrap();
        assert_eq!(
            s.head,
            HeadState::Unborn {
                branch: b"main".to_vec()
            }
        );
        assert_eq!(s.head.commit(), None);
        assert_eq!(s.upstream, UpstreamState::None);

        let bytes = format!("# branch.oid {E1}\0# branch.head (detached)\0");
        let s = RepositoryStatus::from_report(&parse_status_v2(bytes.as_bytes()).unwrap(), vec![])
            .unwrap();
        assert_eq!(
            s.head,
            HeadState::Detached {
                commit: ObjectId::parse(E1).unwrap()
            }
        );

        let bytes = format!(
            "# branch.oid {E1}\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +1 -3\0"
        );
        let s = RepositoryStatus::from_report(&parse_status_v2(bytes.as_bytes()).unwrap(), vec![])
            .unwrap();
        assert_eq!(
            s.upstream,
            UpstreamState::Tracking {
                name: b"origin/main".to_vec(),
                ahead: 1,
                behind: 3
            }
        );
        let bytes = format!("# branch.oid {E1}\0# branch.head main\0# branch.upstream origin/x\0");
        let s = RepositoryStatus::from_report(&parse_status_v2(bytes.as_bytes()).unwrap(), vec![])
            .unwrap();
        assert_eq!(
            s.upstream,
            UpstreamState::Gone {
                name: b"origin/x".to_vec()
            }
        );
    }

    #[test]
    fn counts_and_conflict_state() {
        let z = "0".repeat(40);
        let bytes = format!(
            "# branch.oid {E1}\0# branch.head main\0\
             1 MM N... 100644 100644 100644 {E1} {E1} a\0\
             1 M. N... 100644 100644 100644 {E1} {E1} b\0\
             1 .D N... 100644 100644 000000 {E1} {E1} c\0\
             2 R. N... 100644 100644 100644 {E1} {E1} R100 d\0e\0\
             u UU N... 100644 100644 100644 100644 {E1} {E1} {E1} f\0\
             u AA N... 000000 100644 100644 100644 {z} {E1} {E1} g\0\
             ? h\0? i\0! j\0"
        );
        let r = parse_status_v2(bytes.as_bytes()).unwrap();
        let s = RepositoryStatus::from_report(
            &r,
            vec![InProgressOperation::Merge, InProgressOperation::Merge],
        )
        .unwrap();
        assert_eq!(
            s.counts,
            ChangeCounts {
                staged: 3,
                unstaged: 2,
                untracked: 2,
                ignored: 1,
                conflicted: 2
            }
        );
        assert_eq!(
            s.conflict,
            ConflictState {
                unmerged_paths: 2,
                operations: vec![InProgressOperation::Merge]
            }
        );
        assert!(s.conflict.is_active());
    }

    #[test]
    fn missing_or_impossible_headers_fail() {
        let r = parse_status_v2(b"? a\0").unwrap();
        assert!(RepositoryStatus::from_report(&r, vec![]).is_err());
        let r = parse_status_v2(b"# branch.oid (initial)\0# branch.head (detached)\0").unwrap();
        assert!(RepositoryStatus::from_report(&r, vec![]).is_err());
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::runner::GitRunner;
        use crate::test_support::{
            commit_file, git_ok, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use crate::trust::GitTrust;

        fn gate(runner: GitRunner) -> GatedGitRunner {
            GatedGitRunner::new(runner, GitTrust::Trusted)
        }

        #[test]
        fn unborn_branch_fixture() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("rstatus-unborn");
            let runner = hermetic_runner(git);
            init_repo_with_identity(&runner, dir.path());
            std::fs::write(dir.path().join("staged.txt"), b"s\n").unwrap();
            git_ok(&runner, dir.path(), ["add", "staged.txt"]);
            std::fs::write(dir.path().join("untracked.txt"), b"u\n").unwrap();
            let s = read_repository_status(&gate(runner), dir.path()).unwrap();
            assert_eq!(
                s.head,
                HeadState::Unborn {
                    branch: b"main".to_vec()
                }
            );
            assert_eq!(s.upstream, UpstreamState::None);
            assert_eq!(s.counts.staged, 1);
            assert_eq!(s.counts.untracked, 1);
            assert!(!s.conflict.is_active());
        }

        #[test]
        fn detached_head_fixture() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("rstatus-detached");
            let runner = hermetic_runner(git);
            init_repo_with_identity(&runner, dir.path());
            commit_file(&runner, dir.path(), "a.txt", b"1\n");
            let head = git_ok(&runner, dir.path(), ["rev-parse", "HEAD"]);
            let head = ObjectId::from_hex_bytes(head.stdout.trim_ascii()).unwrap();
            git_ok(&runner, dir.path(), ["switch", "-q", "--detach"]);
            std::fs::write(dir.path().join("a.txt"), b"2\n").unwrap();
            let s = read_repository_status(&gate(runner), dir.path()).unwrap();
            assert_eq!(s.head, HeadState::Detached { commit: head });
            assert_eq!(s.upstream, UpstreamState::None);
            assert_eq!(s.counts.unstaged, 1);
            assert_eq!(s.counts.staged, 0);
        }

        #[test]
        fn upstream_tracking_and_gone_fixture() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("rstatus-upstream");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            commit_file(&runner, repo, "a.txt", b"1\n");
            git_ok(&runner, repo, ["branch", "base"]);
            commit_file(&runner, repo, "a.txt", b"2\n");
            commit_file(&runner, repo, "a.txt", b"3\n");
            git_ok(&runner, repo, ["branch", "--set-upstream-to=base"]);
            let g = gate(runner.clone());
            let s = read_repository_status(&g, repo).unwrap();
            assert_eq!(
                s.upstream,
                UpstreamState::Tracking {
                    name: b"base".to_vec(),
                    ahead: 2,
                    behind: 0
                }
            );
            git_ok(&runner, repo, ["branch", "-D", "base"]);
            let s = read_repository_status(&g, repo).unwrap();
            assert_eq!(
                s.upstream,
                UpstreamState::Gone {
                    name: b"base".to_vec()
                }
            );
        }

        #[test]
        fn merge_and_cherry_pick_conflict_fixture() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("rstatus-conflict");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            commit_file(&runner, repo, "c.txt", b"base\n");
            git_ok(&runner, repo, ["switch", "-q", "-c", "other"]);
            commit_file(&runner, repo, "c.txt", b"theirs\n");
            git_ok(&runner, repo, ["switch", "-q", "main"]);
            commit_file(&runner, repo, "c.txt", b"ours\n");
            assert!(!runner
                .run(&argv(["merge", "-q", "other"]), repo)
                .unwrap()
                .success());
            let g = gate(runner.clone());
            let s = read_repository_status(&g, repo).unwrap();
            assert_eq!(s.conflict.unmerged_paths, 1);
            assert_eq!(s.conflict.operations, vec![InProgressOperation::Merge]);
            assert_eq!(s.counts.conflicted, 1);

            git_ok(&runner, repo, ["merge", "--abort"]);
            assert!(!runner
                .run(&argv(["cherry-pick", "other"]), repo)
                .unwrap()
                .success());
            let s = read_repository_status(&g, repo).unwrap();
            assert_eq!(s.conflict.operations, vec![InProgressOperation::CherryPick]);
            git_ok(&runner, repo, ["cherry-pick", "--abort"]);

            assert!(!runner
                .run(&argv(["rebase", "-q", "other"]), repo)
                .unwrap()
                .success());
            let s = read_repository_status(&g, repo).unwrap();
            assert!(
                s.conflict.operations.contains(&InProgressOperation::Rebase),
                "{:?}",
                s.conflict
            );
            git_ok(&runner, repo, ["rebase", "--abort"]);
            let s = read_repository_status(&g, repo).unwrap();
            assert!(!s.conflict.is_active());
        }
    }
}

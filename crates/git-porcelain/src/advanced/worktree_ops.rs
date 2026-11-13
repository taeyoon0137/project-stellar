//! worktree add/remove/move/lock/unlock/prune/repair (XXVI.3.2).
//!
//! 모든 연산은 시스템 Git `git worktree <sub>`를 [`CommandPlan`]으로 실행합니다. Git의 일반 worktree 동작을
//! 그대로 쓰며 `.git` 내부 파일을 직접 고치지 않습니다.
//!
//! | 연산 | 명령 | 수준 |
//! |---|---|---|
//! | add | `worktree add [-b <new> \| --detach] -- <path> [<commit-ish>]` | Safe |
//! | remove | `worktree remove -- <worktree>`(**force 없음**) | [`DestructiveLevel::DiscardsWork`](ignore된 파일과 디렉터리를 지움) |
//! | move | `worktree move -- <worktree> <new-path>` | Safe |
//! | lock | `worktree lock [--reason <r>] -- <worktree>` | Safe |
//! | unlock | `worktree unlock -- <worktree>` | Safe |
//! | prune | `worktree prune` | [`DestructiveLevel::DiscardsRefs`](사라진 worktree 관리 정보 삭제) |
//! | repair | `worktree repair [<path>...]` | Safe |
//!
//! `remove`는 변경·untracked 파일이 있으면 Git이 거부하고, Stellar는 `--force`를 제공하지 않습니다(XXVI.3.4).
//! `prune`은 먼저 [`preview_prune`](`--dry-run --verbose`)로 지울 대상을 보여 줄 수 있습니다.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::{CommandPlan, DestructiveLevel, PlanError};
use crate::object::ObjectId;
use crate::ops::{run_ok, GitOpError};
use crate::refname::is_valid_ref_body;
use crate::runner::argv;
use crate::trust::GatedGitRunner;

/// 새 worktree가 checkout할 대상.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddTarget {
    /// 새 branch를 만들어 checkout(`-b <name> [<start>]`).
    NewBranch {
        /// 새 branch 이름.
        name: String,
        /// 시작 지점(없으면 HEAD).
        start: Option<String>,
    },
    /// 기존 branch checkout.
    ExistingBranch(String),
    /// detached HEAD(`--detach <commit-ish>`).
    Detached(String),
}

fn path_arg(path: &Path) -> Result<OsString, PlanError> {
    if path.as_os_str().is_empty() {
        return Err(PlanError::InvalidInput("worktree 경로"));
    }
    Ok(path.as_os_str().to_os_string())
}

fn rev_ok(rev: &str) -> bool {
    rev == "HEAD" || is_valid_ref_body(rev) || ObjectId::parse(rev).is_some()
}

/// worktree add 계획.
pub fn plan_add(path: &Path, target: &AddTarget) -> Result<CommandPlan, PlanError> {
    let mut args = argv(["worktree", "add"]);
    let mut tail: Vec<OsString> = Vec::new();
    match target {
        AddTarget::NewBranch { name, start } => {
            if !is_valid_ref_body(name) {
                return Err(PlanError::InvalidInput("branch 이름"));
            }
            args.extend(argv(["-b", name.as_str()]));
            if let Some(start) = start {
                if !rev_ok(start) {
                    return Err(PlanError::InvalidInput("시작 지점"));
                }
                tail.push(OsString::from(start));
            }
        }
        AddTarget::ExistingBranch(name) => {
            if !is_valid_ref_body(name) {
                return Err(PlanError::InvalidInput("branch 이름"));
            }
            tail.push(OsString::from(name));
        }
        AddTarget::Detached(rev) => {
            if !rev_ok(rev) {
                return Err(PlanError::InvalidInput("commit"));
            }
            args.extend(argv(["--detach"]));
            tail.push(OsString::from(rev));
        }
    }
    args.push(OsString::from("--"));
    args.push(path_arg(path)?);
    args.extend(tail);
    Ok(CommandPlan::new(args, DestructiveLevel::Safe))
}

/// worktree remove 계획(force 없음, 확인 필요).
pub fn plan_remove(worktree: &Path) -> Result<CommandPlan, PlanError> {
    let mut args = argv(["worktree", "remove", "--"]);
    args.push(path_arg(worktree)?);
    Ok(CommandPlan::new(args, DestructiveLevel::DiscardsWork))
}

/// worktree move 계획.
pub fn plan_move(worktree: &Path, new_path: &Path) -> Result<CommandPlan, PlanError> {
    let mut args = argv(["worktree", "move", "--"]);
    args.push(path_arg(worktree)?);
    args.push(path_arg(new_path)?);
    Ok(CommandPlan::new(args, DestructiveLevel::Safe))
}

/// worktree lock 계획.
pub fn plan_lock(worktree: &Path, reason: Option<&str>) -> Result<CommandPlan, PlanError> {
    let mut args = argv(["worktree", "lock"]);
    if let Some(reason) = reason {
        if reason.contains('\0') || reason.contains('\n') {
            return Err(PlanError::InvalidInput("lock 이유"));
        }
        args.extend(argv(["--reason", reason]));
    }
    args.push(OsString::from("--"));
    args.push(path_arg(worktree)?);
    Ok(CommandPlan::new(args, DestructiveLevel::Safe))
}

/// worktree unlock 계획.
pub fn plan_unlock(worktree: &Path) -> Result<CommandPlan, PlanError> {
    let mut args = argv(["worktree", "unlock", "--"]);
    args.push(path_arg(worktree)?);
    Ok(CommandPlan::new(args, DestructiveLevel::Safe))
}

/// worktree prune 계획(확인 필요).
pub fn plan_prune() -> CommandPlan {
    CommandPlan::new(argv(["worktree", "prune"]), DestructiveLevel::DiscardsRefs)
}

/// worktree repair 계획(경로가 없으면 현재 저장소의 모든 worktree).
pub fn plan_repair(paths: &[PathBuf]) -> Result<CommandPlan, PlanError> {
    let mut args = argv(["worktree", "repair"]);
    if !paths.is_empty() {
        args.push(OsString::from("--"));
        for p in paths {
            args.push(path_arg(p)?);
        }
    }
    Ok(CommandPlan::new(args, DestructiveLevel::Safe))
}

/// worktree 제거 실패(XXVI.3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoveError {
    /// 확인 필요·입력 오류·실행 실패.
    Plan(PlanError),
    /// Git이 제거를 거부함(변경·untracked 파일, lock 등). Git 메시지를 그대로 보여 주며 force는 제공하지 않습니다.
    RefusedByGit {
        /// 종료 코드.
        exit_code: Option<i32>,
        /// Git stderr(UTF-8 우선 표시 텍스트).
        message: String,
    },
}

impl std::fmt::Display for RemoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RemoveError::Plan(e) => write!(f, "{e}"),
            RemoveError::RefusedByGit { message, .. } => {
                write!(f, "Git이 worktree 제거를 거부했습니다: {}", message.trim())
            }
        }
    }
}

impl std::error::Error for RemoveError {}

/// 확인된 remove plan을 실행합니다. Git이 거부하면 그 결과를 [`RemoveError::RefusedByGit`]로 그대로 돌려줍니다.
///
/// 이 API에는 force 경로가 없습니다. untracked·변경 파일이 있는 worktree는 사용자가 직접 정리한 뒤 다시
/// 시도하거나 외부 터미널에서 Git을 사용합니다.
pub fn remove_worktree(
    git: &GatedGitRunner,
    cwd: &Path,
    plan: &CommandPlan,
    confirmation: Option<&super::Confirmation>,
) -> Result<(), RemoveError> {
    let is_remove = plan.args().get(1).is_some_and(|a| a == "remove")
        && !plan.args().iter().any(|a| a == "--force" || a == "-f");
    if !is_remove {
        return Err(RemoveError::Plan(PlanError::InvalidInput(
            "remove plan 아님",
        )));
    }
    match super::execute_plan(git, cwd, plan, confirmation) {
        Ok(_) => Ok(()),
        Err(PlanError::Op(GitOpError::Git(failure))) => Err(RemoveError::RefusedByGit {
            exit_code: failure.exit_code,
            message: crate::decode::GitText::from_bytes(&failure.stderr)
                .display()
                .to_string(),
        }),
        Err(other) => Err(RemoveError::Plan(other)),
    }
}

/// prune이 지울 대상을 미리 봅니다(`git worktree prune --dry-run --verbose`, 변경 없음).
pub fn preview_prune(git: &GatedGitRunner, cwd: &Path) -> Result<Vec<String>, GitOpError> {
    let out = run_ok(
        git,
        cwd,
        &argv(["worktree", "prune", "--dry-run", "--verbose"]),
    )?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok(text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_matrix_levels_and_argv() {
        let p = Path::new("/wt/a b");
        let cases: Vec<(CommandPlan, &str, DestructiveLevel)> = vec![
            (
                plan_add(
                    p,
                    &AddTarget::NewBranch {
                        name: "feat".into(),
                        start: Some("main".into()),
                    },
                )
                .unwrap(),
                "git worktree add -b feat -- '/wt/a b' main",
                DestructiveLevel::Safe,
            ),
            (
                plan_add(p, &AddTarget::Detached("HEAD".into())).unwrap(),
                "git worktree add --detach -- '/wt/a b' HEAD",
                DestructiveLevel::Safe,
            ),
            (
                plan_remove(p).unwrap(),
                "git worktree remove -- '/wt/a b'",
                DestructiveLevel::DiscardsWork,
            ),
            (
                plan_move(p, Path::new("/wt/c")).unwrap(),
                "git worktree move -- '/wt/a b' /wt/c",
                DestructiveLevel::Safe,
            ),
            (
                plan_lock(p, Some("외장 디스크")).unwrap(),
                "git worktree lock --reason '외장 디스크' -- '/wt/a b'",
                DestructiveLevel::Safe,
            ),
            (
                plan_unlock(p).unwrap(),
                "git worktree unlock -- '/wt/a b'",
                DestructiveLevel::Safe,
            ),
            (
                plan_prune(),
                "git worktree prune",
                DestructiveLevel::DiscardsRefs,
            ),
            (
                plan_repair(&[]).unwrap(),
                "git worktree repair",
                DestructiveLevel::Safe,
            ),
        ];
        for (plan, preview, level) in cases {
            assert_eq!(plan.preview(), preview);
            assert_eq!(plan.level(), level);
        }
        // remove에는 force가 없음.
        assert!(!plan_remove(p)
            .unwrap()
            .args()
            .iter()
            .any(|a| a == "--force" || a == "-f"));
        assert!(plan_add(p, &AddTarget::ExistingBranch("-f".into())).is_err());
        assert!(plan_add(Path::new(""), &AddTarget::Detached("HEAD".into())).is_err());
        assert!(plan_lock(p, Some("a\nb")).is_err());
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::advanced::worktree::{list_worktrees, Annotation};
        use crate::advanced::{execute_plan, Confirmation};
        use crate::test_support::{
            commit_file, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use crate::trust::GitTrust;

        /// 있으면 실경로, 없으면(prunable worktree) 그대로.
        fn canon(path: &Path) -> PathBuf {
            std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        }

        #[test]
        fn command_matrix_fixture() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("adv-wtops");
            let runner = hermetic_runner(git);
            let repo = dir.path().join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            init_repo_with_identity(&runner, &repo);
            commit_file(&runner, &repo, "stellar.yaml", b"v: 1\n");
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let run = |plan: &CommandPlan| {
                execute_plan(&gate, &repo, plan, Some(&Confirmation::confirm(plan))).unwrap();
            };
            let names = || -> Vec<PathBuf> {
                list_worktrees(&gate, &repo)
                    .unwrap()
                    .into_iter()
                    .map(|e| canon(&e.path))
                    .collect()
            };

            // add: 새 branch·detached.
            let a = dir.path().join("wt a");
            let b = dir.path().join("wt-b");
            run(&plan_add(
                &a,
                &AddTarget::NewBranch {
                    name: "feature".into(),
                    start: None,
                },
            )
            .unwrap());
            run(&plan_add(&b, &AddTarget::Detached("HEAD".into())).unwrap());
            assert_eq!(names().len(), 3);
            assert!(a.join("stellar.yaml").exists());

            // move.
            let moved = dir.path().join("moved");
            run(&plan_move(&b, &moved).unwrap());
            assert!(names().contains(&canon(&moved)));

            // lock: lock된 worktree는 move를 Git이 거부.
            run(&plan_lock(&moved, Some("보관")).unwrap());
            let entry = list_worktrees(&gate, &repo)
                .unwrap()
                .into_iter()
                .find(|e| canon(&e.path) == canon(&moved))
                .unwrap();
            assert_eq!(
                entry.locked,
                Annotation::Yes {
                    reason: Some("보관".into())
                }
            );
            assert!(execute_plan(&gate, &repo, &plan_move(&moved, &b).unwrap(), None).is_err());
            run(&plan_unlock(&moved).unwrap());

            // remove: 확인 필요, 확인 후 깨끗한 worktree 제거.
            let remove = plan_remove(&moved).unwrap();
            let before = runner.spawn_count();
            assert!(matches!(
                execute_plan(&gate, &repo, &remove, None),
                Err(PlanError::ConfirmationRequired { .. })
            ));
            assert_eq!(runner.spawn_count(), before);
            run(&remove);
            assert!(!moved.exists());

            // prune: 디렉터리를 지운 worktree → 미리보기에 나타나고, 확인 후 정리.
            std::fs::remove_dir_all(&a).unwrap();
            let preview = preview_prune(&gate, &repo).unwrap();
            assert!(!preview.is_empty(), "prune 미리보기 비어 있음");
            assert_eq!(names().len(), 2, "dry-run이 정리함");
            assert!(matches!(
                execute_plan(&gate, &repo, &plan_prune(), None),
                Err(PlanError::ConfirmationRequired { .. })
            ));
            run(&plan_prune());
            assert_eq!(names(), vec![canon(&repo)]);

            // repair: 정상 상태에서 성공.
            run(&plan_repair(&[]).unwrap());
        }

        #[test]
        fn untracked_or_modified_worktree_remove_is_refused_as_is() {
            let Some(git) = system_git() else {
                return;
            };
            let dir = TempDir::new("adv-wtremove");
            let runner = hermetic_runner(git);
            let repo = dir.path().join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            init_repo_with_identity(&runner, &repo);
            commit_file(&runner, &repo, "stellar.yaml", b"v: 1\n");
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let wt = dir.path().join("linked");
            let add = plan_add(&wt, &AddTarget::Detached("HEAD".into())).unwrap();
            execute_plan(&gate, &repo, &add, None).unwrap();

            // untracked 파일 → Git 거부, 메시지 보존, worktree·파일 유지.
            std::fs::write(wt.join("scratch.txt"), b"keep me\n").unwrap();
            let remove = plan_remove(&wt).unwrap();
            let confirm = Confirmation::confirm(&remove);
            match remove_worktree(&gate, &repo, &remove, Some(&confirm)) {
                Err(RemoveError::RefusedByGit { exit_code, message }) => {
                    assert_ne!(exit_code, Some(0));
                    assert!(!message.trim().is_empty());
                    assert!(RemoveError::RefusedByGit {
                        exit_code,
                        message: message.clone()
                    }
                    .to_string()
                    .contains(message.trim()));
                }
                other => panic!("{other:?}"),
            }
            assert_eq!(std::fs::read(wt.join("scratch.txt")).unwrap(), b"keep me\n");
            assert_eq!(list_worktrees(&gate, &repo).unwrap().len(), 2);

            // tracked 변경도 거부.
            std::fs::remove_file(wt.join("scratch.txt")).unwrap();
            std::fs::write(wt.join("stellar.yaml"), b"v: changed\n").unwrap();
            assert!(matches!(
                remove_worktree(&gate, &repo, &remove, Some(&confirm)),
                Err(RemoveError::RefusedByGit { .. })
            ));
            assert!(wt.join("stellar.yaml").exists());

            // force plan은 이 API로 실행할 수 없음(수동 조립도 거부).
            let forced = CommandPlan::new(
                argv(["worktree", "remove", "--force", "--", wt.to_str().unwrap()]),
                DestructiveLevel::DiscardsWork,
            );
            assert!(matches!(
                remove_worktree(&gate, &repo, &forced, Some(&Confirmation::confirm(&forced))),
                Err(RemoveError::Plan(PlanError::InvalidInput(_)))
            ));
            assert!(wt.exists());

            // 정리 후에는 제거 성공.
            crate::test_support::git_ok(&runner, &wt, ["checkout", "--", "stellar.yaml"]);
            remove_worktree(&gate, &repo, &remove, Some(&confirm)).unwrap();
            assert!(!wt.exists());
        }
    }
}

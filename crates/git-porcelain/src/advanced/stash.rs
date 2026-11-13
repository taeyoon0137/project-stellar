//! stash list/create/apply/pop/drop (XXVI.2.1).
//!
//! - 목록: `git stash list --format=%gd%x00%H%x00%gs`(selector·commit·subject, 줄 단위 + NUL 필드).
//! - 만들기: `git stash push [--include-untracked] [-m <message>]`. message는 argv로 넘기며(shell 없음) NUL은 거부합니다.
//! - 적용: `git stash apply <stash@{n}>`(stash 유지), `git stash pop <stash@{n}>`(적용 성공 시에만 Git이 제거).
//!   conflict·작업 트리 충돌은 Git 결과를 그대로 돌려줍니다.
//! - 삭제: `git stash drop <stash@{n}>`은 [`DestructiveLevel::DiscardsRefs`](reflog·dangling commit으로만
//!   복구 가능)이라 사용자 확인이 필요합니다. 확인한 뒤 stash 목록이 바뀌어 같은 selector가 다른 stash를
//!   가리키면 지우지 않습니다([`StashError::StashChanged`]).

use std::path::Path;

use super::{execute_plan, CommandPlan, Confirmation, DestructiveLevel, PlanError};
use crate::object::ObjectId;
use crate::ops::{run_ok, GitOpError, ParseFailure};
use crate::runner::argv;
use crate::trust::GatedGitRunner;

/// stash 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashEntry {
    /// 위치(0이 최신).
    pub index: usize,
    /// selector(`stash@{n}`).
    pub selector: String,
    /// stash commit.
    pub commit: ObjectId,
    /// reflog subject(`On main: message` 등).
    pub subject: String,
}

/// stash 만들기 옵션.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StashOptions {
    /// 메시지.
    pub message: Option<String>,
    /// untracked 파일 포함.
    pub include_untracked: bool,
}

/// stash 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StashError {
    /// plan 실행 실패(확인 필요·입력 오류·Git 결과).
    Plan(PlanError),
    /// 확인 후 stash 목록이 바뀌어 대상이 달라짐(삭제 안 함).
    StashChanged,
}

impl std::fmt::Display for StashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StashError::Plan(e) => write!(f, "{e}"),
            StashError::StashChanged => {
                f.write_str("stash 목록이 바뀌었습니다. 다시 확인한 뒤 삭제하세요")
            }
        }
    }
}

impl std::error::Error for StashError {}

impl From<PlanError> for StashError {
    fn from(e: PlanError) -> StashError {
        StashError::Plan(e)
    }
}

impl From<GitOpError> for StashError {
    fn from(e: GitOpError) -> StashError {
        StashError::Plan(PlanError::Op(e))
    }
}

const COMMAND: &str = "stash list";

/// stash 목록 출력을 해석합니다.
pub fn parse_stash_list(stdout: &[u8]) -> Result<Vec<StashEntry>, ParseFailure> {
    let text = String::from_utf8_lossy(stdout);
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let bad = |d: &str| ParseFailure::new(COMMAND, d, line.as_bytes());
        let mut fields = line.splitn(3, '\0');
        let (Some(selector), Some(oid), Some(subject)) =
            (fields.next(), fields.next(), fields.next())
        else {
            return Err(bad("필드 수가 3개가 아님"));
        };
        if selector != format!("stash@{{{n}}}") {
            return Err(bad("selector 순서 불일치"));
        }
        out.push(StashEntry {
            index: n,
            selector: selector.to_string(),
            commit: ObjectId::parse(oid).ok_or_else(|| bad("잘못된 object ID"))?,
            subject: subject.to_string(),
        });
    }
    Ok(out)
}

/// stash 목록(최신 먼저).
pub fn list_stashes(git: &GatedGitRunner, worktree: &Path) -> Result<Vec<StashEntry>, GitOpError> {
    let out = run_ok(
        git,
        worktree,
        &argv(["stash", "list", "--format=%gd%x00%H%x00%gs"]),
    )?;
    Ok(parse_stash_list(&out.stdout)?)
}

/// stash를 만듭니다. 저장할 변경이 없으면 `None`.
pub fn create_stash(
    git: &GatedGitRunner,
    worktree: &Path,
    options: &StashOptions,
) -> Result<Option<StashEntry>, StashError> {
    let mut args = argv(["stash", "push"]);
    if options.include_untracked {
        args.extend(argv(["--include-untracked"]));
    }
    if let Some(message) = &options.message {
        if message.contains('\0') {
            return Err(PlanError::InvalidInput("stash 메시지에 NUL").into());
        }
        args.extend(argv(["-m", message.as_str()]));
    }
    let before = list_stashes(git, worktree)?;
    execute_plan(
        git,
        worktree,
        &CommandPlan::new(args, DestructiveLevel::Safe),
        None,
    )?;
    let after = list_stashes(git, worktree)?;
    Ok(if after.len() > before.len() {
        after.into_iter().next()
    } else {
        None
    })
}

fn selector(index: usize) -> String {
    format!("stash@{{{index}}}")
}

/// stash를 적용합니다(stash 유지).
pub fn apply_stash(git: &GatedGitRunner, worktree: &Path, index: usize) -> Result<(), StashError> {
    let sel = selector(index);
    let plan = CommandPlan::new(
        argv(["stash", "apply", sel.as_str()]),
        DestructiveLevel::Safe,
    );
    execute_plan(git, worktree, &plan, None)?;
    Ok(())
}

/// stash를 적용하고 성공하면 제거합니다(Git `stash pop`, 적용 실패 시 Git이 stash를 남김).
pub fn pop_stash(git: &GatedGitRunner, worktree: &Path, index: usize) -> Result<(), StashError> {
    let sel = selector(index);
    let plan = CommandPlan::new(argv(["stash", "pop", sel.as_str()]), DestructiveLevel::Safe);
    execute_plan(git, worktree, &plan, None)?;
    Ok(())
}

/// stash 삭제 계획(확인 대상 commit에 묶임).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashDropPlan {
    /// 확인 대상 plan(`git stash drop <selector>`, `DiscardsRefs`).
    pub plan: CommandPlan,
    /// 사용자가 본 stash.
    pub expected: StashEntry,
}

/// stash 삭제 계획을 만듭니다.
pub fn plan_drop_stash(entry: &StashEntry) -> StashDropPlan {
    StashDropPlan {
        plan: CommandPlan::new(
            argv(["stash", "drop", entry.selector.as_str()]),
            DestructiveLevel::DiscardsRefs,
        ),
        expected: entry.clone(),
    }
}

/// 확인된 계획으로 stash를 지웁니다. 확인이 없으면 프로세스를 만들지 않습니다.
pub fn drop_stash(
    git: &GatedGitRunner,
    worktree: &Path,
    drop: &StashDropPlan,
    confirmation: Option<&Confirmation>,
) -> Result<(), StashError> {
    let confirmed = confirmation.is_some_and(|c| *c == Confirmation::confirm(&drop.plan));
    if !confirmed {
        return Err(PlanError::ConfirmationRequired {
            level: drop.plan.level(),
            preview: drop.plan.preview(),
        }
        .into());
    }
    let current = list_stashes(git, worktree)?;
    if current.get(drop.expected.index) != Some(&drop.expected) {
        return Err(StashError::StashChanged);
    }
    execute_plan(git, worktree, &drop.plan, confirmation)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const E1: &str = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";

    #[test]
    fn parses_stash_list() {
        let out =
            format!("stash@{{0}}\0{E1}\0On main: 두 번째\nstash@{{1}}\0{E1}\0WIP on main: abc x\n");
        let list = parse_stash_list(out.as_bytes()).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].subject, "On main: 두 번째");
        assert_eq!(list[1].selector, "stash@{1}");
        for bad in [
            format!("stash@{{1}}\0{E1}\0x\n"),
            "stash@{0}\0zz\0x\n".to_string(),
            format!("stash@{{0}}\0{E1}\n"),
        ] {
            assert!(parse_stash_list(bad.as_bytes()).is_err());
        }
    }

    #[cfg(unix)]
    mod real_git {
        use super::*;
        use crate::test_support::{
            commit_file, hermetic_runner, init_repo_with_identity, system_git, TempDir,
        };
        use crate::trust::GitTrust;

        #[test]
        fn stash_lifecycle_fixture() {
            let Some(git) = system_git() else {
                return; // git 미설치 환경 건너뜀.
            };
            let dir = TempDir::new("adv-stash");
            let runner = hermetic_runner(git);
            let repo = dir.path();
            init_repo_with_identity(&runner, repo);
            commit_file(&runner, repo, "stellar.yaml", b"v: 1\n");
            let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
            let read = || std::fs::read(repo.join("stellar.yaml")).unwrap();

            // 변경이 없으면 None.
            assert_eq!(
                create_stash(&gate, repo, &StashOptions::default()).unwrap(),
                None
            );

            std::fs::write(repo.join("stellar.yaml"), b"v: 2\n").unwrap();
            std::fs::write(repo.join("new.txt"), b"untracked\n").unwrap();
            let entry = create_stash(
                &gate,
                repo,
                &StashOptions {
                    message: Some("토큰 실험 -- $(x)".into()),
                    include_untracked: true,
                },
            )
            .unwrap()
            .unwrap();
            assert!(
                entry.subject.ends_with("토큰 실험 -- $(x)"),
                "{}",
                entry.subject
            );
            assert_eq!(read(), b"v: 1\n");
            assert!(!repo.join("new.txt").exists());

            apply_stash(&gate, repo, 0).unwrap();
            assert_eq!(read(), b"v: 2\n");
            assert_eq!(list_stashes(&gate, repo).unwrap().len(), 1);
            // 같은 변경이 이미 있는 상태에서 다시 apply하면 Git이 거부(결과 그대로).
            assert!(matches!(
                apply_stash(&gate, repo, 0),
                Err(StashError::Plan(PlanError::Op(GitOpError::Git(_))))
            ));
            crate::test_support::git_ok(&runner, repo, ["checkout", "--", "stellar.yaml"]);
            std::fs::remove_file(repo.join("new.txt")).unwrap();

            pop_stash(&gate, repo, 0).unwrap();
            assert_eq!(read(), b"v: 2\n");
            assert!(list_stashes(&gate, repo).unwrap().is_empty());

            // 두 개 만든 뒤 오래된 것(1)을 지움.
            create_stash(
                &gate,
                repo,
                &StashOptions {
                    message: Some("first".into()),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
            std::fs::write(repo.join("stellar.yaml"), b"v: 3\n").unwrap();
            create_stash(
                &gate,
                repo,
                &StashOptions {
                    message: Some("second".into()),
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();
            let list = list_stashes(&gate, repo).unwrap();
            assert!(list[1].subject.ends_with("first"));
            let drop = plan_drop_stash(&list[1]);
            assert_eq!(drop.plan.level(), DestructiveLevel::DiscardsRefs);
            let before = runner.spawn_count();
            assert!(matches!(
                drop_stash(&gate, repo, &drop, None),
                Err(StashError::Plan(PlanError::ConfirmationRequired { .. }))
            ));
            assert_eq!(runner.spawn_count(), before, "확인 없이 실행됨");
            let confirm = Confirmation::confirm(&drop.plan);

            // 확인 뒤 목록이 바뀌면(새 stash로 index 이동) 지우지 않음.
            std::fs::write(repo.join("stellar.yaml"), b"v: 4\n").unwrap();
            create_stash(&gate, repo, &StashOptions::default())
                .unwrap()
                .unwrap();
            assert_eq!(
                drop_stash(&gate, repo, &drop, Some(&confirm)),
                Err(StashError::StashChanged)
            );
            assert_eq!(list_stashes(&gate, repo).unwrap().len(), 3);

            // 최신 목록으로 다시 계획·확인하면 삭제.
            let list = list_stashes(&gate, repo).unwrap();
            let target = list.iter().find(|e| e.subject.ends_with("first")).unwrap();
            let drop = plan_drop_stash(target);
            drop_stash(&gate, repo, &drop, Some(&Confirmation::confirm(&drop.plan))).unwrap();
            let left: Vec<String> = list_stashes(&gate, repo)
                .unwrap()
                .into_iter()
                .map(|e| e.subject)
                .collect();
            assert_eq!(left.len(), 2);
            assert!(!left.iter().any(|s| s.ends_with("first")));
        }
    }
}

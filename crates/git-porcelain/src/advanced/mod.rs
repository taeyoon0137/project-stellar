//! 로드맵 고급 Git 기능의 core API (XXVI.*).
//!
//! Branch·Fetch·Pull·Stash·Reset·Reflog·Worktree 등 아키텍처 §24.4 로드맵 기능을 **시스템 Git 명령**으로
//! 연결하는 core API입니다. UI는 이 crate에 없으며, 초기 빌드에 노출하지 않는 정책(XIV.13.7)은 UI·command
//! registry 쪽에서 적용합니다.
//!
//! 파괴적일 수 있는 명령은 실행 전에 [`CommandPlan`](argv 미리보기 + [`DestructiveLevel`])을 만들고,
//! [`DestructiveLevel::Safe`]가 아니면 사용자가 그 plan을 확인한 [`Confirmation`]이 있어야만 실행합니다.
//! 확인은 plan의 argv·level digest에 묶이므로 다른 plan의 확인으로 실행할 수 없습니다. 확인이 없거나
//! 맞지 않으면 프로세스를 만들지 않고 [`PlanError::ConfirmationRequired`]를 반환합니다.
//!
//! 모든 실행은 trust gate([`GatedGitRunner`])를 거치고, Git 결과(실패 포함)를 그대로 돌려줍니다.

use std::ffi::OsString;
use std::path::Path;

use crate::ops::{require_success, GitOpError};
use crate::runner::GitOutput;
use crate::trust::GatedGitRunner;

/// branch 목록·전환·생성·삭제(XXVI.1.1).
pub mod branch;

/// fetch와 진행률(XXVI.1.2).
pub mod fetch;

/// pull 전략 선택(XXVI.1.3).
pub mod pull;

/// stash list/create/apply/pop/drop(XXVI.2.1).
pub mod stash;

/// reset/restore 파괴 수준별 확인(XXVI.2.3).
pub mod reset;

/// reflog 조회와 선택 항목 복구(XXVI.2.4).
pub mod reflog;

/// Git worktree 목록(XXVI.3.1).
pub mod worktree;

/// worktree add/remove/move/lock/unlock/prune/repair(XXVI.3.2).
pub mod worktree_ops;

/// tags·notes·bisect·sparse-checkout·submodule 읽기(XXVI.4.1).
pub mod inspect;

/// 고급 Git 쓰기 기능 feature flag(기본 비활성, XXVI.4.2).
pub mod features;

/// 명령의 파괴 수준.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DestructiveLevel {
    /// 데이터 손실 없음(확인 불필요).
    Safe,
    /// ref·stash 등을 지움. reflog·dangling object로 한동안 복구 가능(확인 필요).
    DiscardsRefs,
    /// 커밋되지 않은 작업(작업 트리·index 변경)을 버림. 복구 불가(확인 필요).
    DiscardsWork,
}

impl DestructiveLevel {
    /// 실행 전에 사용자 확인이 필요한지.
    pub fn requires_confirmation(self) -> bool {
        self != DestructiveLevel::Safe
    }
}

/// 실행할 Git 명령 계획(미리보기 가능한 argv와 파괴 수준).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPlan {
    args: Vec<OsString>,
    level: DestructiveLevel,
}

impl CommandPlan {
    /// argv와 수준으로 계획을 만듭니다.
    pub fn new(args: Vec<OsString>, level: DestructiveLevel) -> CommandPlan {
        CommandPlan { args, level }
    }

    /// 파괴 수준.
    pub fn level(&self) -> DestructiveLevel {
        self.level
    }

    /// Git에 넘길 argv.
    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    /// 사용자에게 보여 줄 명령 미리보기(`git <argv...>`, 공백 포함 인자는 작은따옴표).
    pub fn preview(&self) -> String {
        let mut out = String::from("git");
        for arg in &self.args {
            let text = arg.to_string_lossy();
            out.push(' ');
            if text.is_empty() || text.chars().any(|c| c.is_whitespace() || c == '\'') {
                out.push('\'');
                out.push_str(&text.replace('\'', "'\\''"));
                out.push('\'');
            } else {
                out.push_str(&text);
            }
        }
        out
    }

    /// argv·수준에 묶인 digest(FNV-1a 64, 확인 대조용).
    fn digest(&self) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |bytes: &[u8]| {
            for b in bytes {
                hash ^= u64::from(*b);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        };
        feed(&[self.level as u8]);
        for arg in &self.args {
            feed(arg.as_encoded_bytes());
            feed(&[0]);
        }
        hash
    }
}

/// 사용자가 특정 [`CommandPlan`]을 확인했다는 증표.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Confirmation {
    digest: u64,
    level: DestructiveLevel,
}

impl Confirmation {
    /// UI가 미리보기를 보여 주고 사용자가 확인했을 때 만듭니다.
    pub fn confirm(plan: &CommandPlan) -> Confirmation {
        Confirmation {
            digest: plan.digest(),
            level: plan.level,
        }
    }
}

/// plan 실행 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// 확인이 없거나 다른 plan의 확인임(프로세스 생성 0).
    ConfirmationRequired {
        /// 필요한 수준.
        level: DestructiveLevel,
        /// 명령 미리보기.
        preview: String,
    },
    /// 입력 검증 실패(프로세스 생성 0).
    InvalidInput(&'static str),
    /// Git 실행 실패 또는 비영 종료(결과 보존).
    Op(GitOpError),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::ConfirmationRequired { level, preview } => {
                write!(f, "확인이 필요한 명령입니다({level:?}): {preview}")
            }
            PlanError::InvalidInput(what) => write!(f, "잘못된 입력: {what}"),
            PlanError::Op(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PlanError {}

impl From<GitOpError> for PlanError {
    fn from(e: GitOpError) -> PlanError {
        PlanError::Op(e)
    }
}

/// plan을 실행합니다. 확인이 필요한데 맞는 확인이 없으면 프로세스를 만들지 않습니다.
pub fn execute_plan(
    git: &GatedGitRunner,
    worktree: &Path,
    plan: &CommandPlan,
    confirmation: Option<&Confirmation>,
) -> Result<GitOutput, PlanError> {
    if plan.level.requires_confirmation() {
        let ok = confirmation.is_some_and(|c| c.digest == plan.digest() && c.level == plan.level);
        if !ok {
            return Err(PlanError::ConfirmationRequired {
                level: plan.level,
                preview: plan.preview(),
            });
        }
    }
    let output = git.run(plan.args(), worktree).map_err(GitOpError::Gate)?;
    Ok(require_success(output)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{argv, GitRunner};
    use crate::trust::GitTrust;

    fn gate() -> (GatedGitRunner, GitRunner) {
        let runner = GitRunner::new(
            std::path::PathBuf::from("/nonexistent/git"),
            Vec::new(),
            std::time::Duration::from_secs(5),
        );
        (
            GatedGitRunner::new(runner.clone(), GitTrust::Trusted),
            runner,
        )
    }

    #[test]
    fn destructive_plan_needs_matching_confirmation_before_spawn() {
        let (gate, runner) = gate();
        let dir = std::env::temp_dir();
        let plan = CommandPlan::new(
            argv(["reset", "--hard", "HEAD"]),
            DestructiveLevel::DiscardsWork,
        );
        assert_eq!(plan.preview(), "git reset --hard HEAD");
        assert!(matches!(
            execute_plan(&gate, &dir, &plan, None),
            Err(PlanError::ConfirmationRequired {
                level: DestructiveLevel::DiscardsWork,
                ..
            })
        ));
        // 다른 plan의 확인으로는 실행 불가.
        let other = CommandPlan::new(
            argv(["reset", "--hard", "HEAD~1"]),
            DestructiveLevel::DiscardsWork,
        );
        let wrong = Confirmation::confirm(&other);
        assert!(matches!(
            execute_plan(&gate, &dir, &plan, Some(&wrong)),
            Err(PlanError::ConfirmationRequired { .. })
        ));
        assert_eq!(runner.spawn_count(), 0);
        // 맞는 확인이면 실행 시도(여기서는 executable이 없어 spawn 오류).
        let right = Confirmation::confirm(&plan);
        assert!(matches!(
            execute_plan(&gate, &dir, &plan, Some(&right)),
            Err(PlanError::Op(GitOpError::Gate(_)))
        ));
        assert_eq!(runner.spawn_count(), 1);
    }

    #[test]
    fn safe_plan_runs_without_confirmation_and_preview_quotes() {
        let (gate, runner) = gate();
        let plan = CommandPlan::new(argv(["branch", "--", "my branch"]), DestructiveLevel::Safe);
        assert_eq!(plan.preview(), "git branch -- 'my branch'");
        let _ = execute_plan(&gate, &std::env::temp_dir(), &plan, None);
        assert_eq!(runner.spawn_count(), 1);
        assert!(DestructiveLevel::DiscardsRefs.requires_confirmation());
        assert!(!DestructiveLevel::Safe.requires_confirmation());
    }
}

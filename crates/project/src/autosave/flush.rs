//! 창 닫기·앱 비활성화 flush 결과와 Git UI 전 확인 (XIII.1.6·XIV.2.8, ADR-0083 결정 6·7).
//!
//! flush는 제한 시간 안에 저장을 시도하고, 끝나지 못하거나 저장할 수 없으면 **창 닫기 결정 상태**를 셸에
//! 돌려줍니다([`FlushOutcome::Blocked`]). 코어는 어떤 경우에도 미저장 변경을 자동으로 버리지 않습니다. 버릴지는
//! 셸이 사용자에게 물은 뒤 정합니다(D6 시트: 다시 시도·저장하지 않고 닫기·취소).
//!
//! Git UI를 열기 전에는 대기 중인 자동 저장을 flush할지 호출자에게 상태를 돌려줍니다([`GitUiPreflight`]). 호출자는
//! [`GitUiChoice::SaveFirst`](저장 뒤 Git UI) 또는 [`GitUiChoice::ContinueWithDisk`](저장하지 않고 디스크 상태로
//! Git UI)를 고릅니다. 코어는 사용자가 고르기 전에 메모리 편집을 저장하지 않습니다.

use std::fmt;

use stellar_core_types::revision::DocumentRevision;

use crate::window_session::StaleResolution;

/// flush가 창을 닫지 못하게 하는 이유. 셸은 이 상태로 사용자에게 결정을 묻습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlushBlock {
    /// 제한 시간 안에 저장을 끝내지 못했습니다. 진행 중인 쓰기는 계속되며 편집은 메모리에 남습니다.
    TimedOut,
    /// 저장이 실패했습니다(잠금 경합·디스크 등). 메시지에는 경로를 넣지 않습니다.
    Failed(String),
    /// 다른 writer가 먼저 저장했습니다. 두 선택지 또는 reconciliation 대기입니다.
    Stale(StaleResolution),
    /// 제스처(drag·scrub)가 진행 중이라 미완료 상태를 저장하지 않았습니다.
    GestureInProgress,
}

impl FlushBlock {
    /// 안정 문자열 code.
    pub fn code(&self) -> &'static str {
        match self {
            FlushBlock::TimedOut => "timed-out",
            FlushBlock::Failed(_) => "failed",
            FlushBlock::Stale(_) => "stale",
            FlushBlock::GestureInProgress => "gesture-in-progress",
        }
    }
}

impl fmt::Display for FlushBlock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlushBlock::TimedOut => f.write_str("제한 시간 안에 저장을 끝내지 못했습니다"),
            FlushBlock::Failed(message) => write!(f, "저장하지 못했습니다: {message}"),
            FlushBlock::Stale(_) => f.write_str("다른 writer가 먼저 저장했습니다"),
            FlushBlock::GestureInProgress => f.write_str("진행 중인 조작이 끝나지 않았습니다"),
        }
    }
}

/// flush 결과(창 닫기 결정 상태).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlushOutcome {
    /// 저장할 것이 없었습니다. 닫아도 됩니다.
    Clean,
    /// 저장을 마쳤습니다. 닫아도 됩니다.
    Saved {
        /// 저장 뒤 loaded generation 번호.
        generation: u64,
    },
    /// 닫기 전에 사용자의 결정이 필요합니다. 미저장 편집은 메모리에 그대로 있습니다.
    Blocked(FlushBlock),
}

impl FlushOutcome {
    /// 셸이 확인 없이 창을 닫아도 되는지.
    pub fn may_close(&self) -> bool {
        matches!(self, FlushOutcome::Clean | FlushOutcome::Saved { .. })
    }
}

/// Git UI를 열기 전 창 하나의 저장 상태(XIV.2.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitUiPreflight {
    /// 메모리와 디스크가 같습니다. 바로 열어도 됩니다.
    Clean,
    /// 저장되지 않은 완료 편집이 있습니다. 호출자가 [`GitUiChoice`]를 고릅니다.
    PendingEdits {
        /// 저장되지 않은 작업 문서 revision.
        revision: DocumentRevision,
    },
    /// 저장할 수 없는 상태입니다(stale·실패·제스처). 편집은 메모리에 남습니다.
    Blocked(FlushBlock),
}

/// Git UI 전 호출자의 선택(XIV.2.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitUiChoice {
    /// 대기 중인 편집을 지금 저장한 뒤 Git UI를 엽니다.
    SaveFirst,
    /// 저장하지 않고 디스크 상태로 Git UI를 엽니다. 메모리 편집은 그대로 남습니다.
    ContinueWithDisk,
}

impl GitUiChoice {
    /// 안정 문자열 code.
    pub fn code(self) -> &'static str {
        match self {
            GitUiChoice::SaveFirst => "save-first",
            GitUiChoice::ContinueWithDisk => "continue-with-disk",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_clean_and_saved_may_close_and_codes_are_frozen() {
        assert!(FlushOutcome::Clean.may_close());
        assert!(FlushOutcome::Saved { generation: 3 }.may_close());
        let blocks = [
            FlushBlock::TimedOut,
            FlushBlock::Failed("디스크".into()),
            FlushBlock::Stale(StaleResolution::Choose),
            FlushBlock::GestureInProgress,
        ];
        let codes: Vec<_> = blocks.iter().map(FlushBlock::code).collect();
        assert_eq!(
            codes,
            ["timed-out", "failed", "stale", "gesture-in-progress"]
        );
        for block in blocks {
            assert!(!FlushOutcome::Blocked(block).may_close());
        }
        assert_eq!(GitUiChoice::SaveFirst.code(), "save-first");
        assert_eq!(GitUiChoice::ContinueWithDisk.code(), "continue-with-disk");
    }
}

//! 병합 결과를 한쪽에 적용할지 판정합니다 (XIII.3.4, XXIII.3, ADR-0110).
//!
//! 열린 창의 외부 변경 반영(LoadedBase·CurrentMemory·ExternalDisk)과 Merge Composer 결과 적용은 같은 질문을
//! 합니다. "base·ours·theirs를 병합한 결과를 쓸 수 있는가, 쓴다면 무엇인가". 이 모듈은 그 판정 하나를 둡니다.
//!
//! | 입력 관계 | 결과 |
//! |---|---|
//! | theirs = base(외부 의미 변경 없음) | [`Reconciliation::KeepOurs`] |
//! | ours = base 또는 ours = theirs(로컬 의미 변경 없음·같은 변경) | [`Reconciliation::TakeTheirs`] |
//! | 그 밖, conflict 0·새 snapshot 위반 0 | [`Reconciliation::Merged`] |
//! | conflict 있음 | [`Reconciliation::Conflicted`] |
//! | conflict 0이지만 병합 결과에 새 snapshot 위반 | [`Reconciliation::Blocked`] |
//!
//! 병합은 [`CandidateSnapshot::build`](XIV.10 엔진)이고, 쓰기 조건은 [`CandidateSnapshot::validate`]입니다
//! (XXIII.2.4). 이 모듈은 새 병합 규칙이나 검사를 만들지 않습니다. 결과 snapshot을 파일·문서로 되돌리는 일은
//! 호출자(ops·runtime)가 기존 역투영으로 합니다.

use stellar_semantic_diff::snapshot::SemanticSnapshot;

use crate::candidate::{CandidateBlocked, CandidateSnapshot};
use crate::model::{MergeError, MergeInput, SemanticConflict};

/// 병합 결과 적용 판정.
#[derive(Debug, Clone)]
pub enum Reconciliation {
    /// 상대 쪽이 base와 의미가 같습니다. 결과는 ours입니다.
    KeepOurs(MergeInput),
    /// 우리 쪽이 base와 같거나 두 쪽이 같습니다. 결과는 theirs입니다.
    TakeTheirs(MergeInput),
    /// 겹치지 않는 변경을 자동 병합했고 검증을 통과했습니다.
    Merged(CandidateSnapshot),
    /// 겹치는 변경이 있습니다. C는 conflict 대상을 base로 둔 상태입니다.
    Conflicted(CandidateSnapshot),
    /// conflict는 없지만 병합 결과가 입력에 없던 snapshot 위반을 만듭니다.
    Blocked {
        /// 병합 candidate.
        candidate: CandidateSnapshot,
        /// 쓰기를 막는 이유.
        blocked: CandidateBlocked,
    },
}

impl Reconciliation {
    /// 쓸 수 있는 결과 snapshot. conflict·위반이면 `None`입니다.
    pub fn result(&self) -> Option<&SemanticSnapshot> {
        match self {
            Reconciliation::KeepOurs(input) => Some(&input.ours),
            Reconciliation::TakeTheirs(input) => Some(&input.theirs),
            Reconciliation::Merged(candidate) => Some(candidate.snapshot()),
            Reconciliation::Conflicted(_) | Reconciliation::Blocked { .. } => None,
        }
    }

    /// 남은 conflict(엔진의 `(StableId, 대상)` 순서). 쓸 수 있는 결과면 비어 있습니다.
    pub fn conflicts(&self) -> &[SemanticConflict] {
        match self {
            Reconciliation::Conflicted(candidate) => candidate.conflicts(),
            _ => &[],
        }
    }

    /// 안정 이름(진단·보고용).
    pub fn name(&self) -> &'static str {
        match self {
            Reconciliation::KeepOurs(_) => "keep-ours",
            Reconciliation::TakeTheirs(_) => "take-theirs",
            Reconciliation::Merged(_) => "merged",
            Reconciliation::Conflicted(_) => "conflicted",
            Reconciliation::Blocked { .. } => "blocked",
        }
    }
}

/// base·ours·theirs의 적용 판정을 냅니다(모듈 문서 표). 같은 입력이면 같은 결과입니다.
pub fn reconcile(input: MergeInput) -> Result<Reconciliation, MergeError> {
    if input.theirs == input.base {
        return Ok(Reconciliation::KeepOurs(input));
    }
    if input.ours == input.base || input.ours == input.theirs {
        return Ok(Reconciliation::TakeTheirs(input));
    }
    let candidate = CandidateSnapshot::build(input)?;
    if !candidate.conflicts().is_empty() {
        return Ok(Reconciliation::Conflicted(candidate));
    }
    match candidate.validate() {
        Ok(_) => Ok(Reconciliation::Merged(candidate)),
        Err(blocked) => Ok(Reconciliation::Blocked { candidate, blocked }),
    }
}

#[cfg(test)]
mod tests;

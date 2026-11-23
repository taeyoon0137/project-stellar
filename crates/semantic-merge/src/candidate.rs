//! 독립 `CandidateSnapshot C` (XXIII.2.1~).
//!
//! Semantic Merge 결과를 입력(Source A = ours, Source B = theirs, base)과 분리된 메모리 값으로 둡니다
//! (아키텍처 §27.4 "결과 C는 A·B를 직접 변경하지 않는 가상 Snapshot"). C는 입력 세 snapshot을 복사해
//! 가지므로 A·B는 바뀌지 않고, 파일·Git index도 읽거나 쓰지 않습니다. 해결(XIV.12)은 C에만 적용합니다.

use stellar_semantic_diff::snapshot::SemanticSnapshot;

use crate::merge::merge;
use crate::model::{MergeError, MergeInput, MergeResult, SemanticConflict};

/// 병합 결과 candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateSnapshot {
    input: MergeInput,
    snapshot: SemanticSnapshot,
    conflicts: Vec<SemanticConflict>,
}

impl CandidateSnapshot {
    /// 입력을 병합해 C를 만듭니다(XXIII.2.1).
    pub fn build(input: MergeInput) -> Result<CandidateSnapshot, MergeError> {
        let result = merge(&input)?;
        Ok(CandidateSnapshot::from_result(input, result))
    }

    /// 이미 계산한 병합 결과로 C를 만듭니다. `result`는 `input`의 [`merge`] 결과여야 합니다.
    pub fn from_result(input: MergeInput, result: MergeResult) -> CandidateSnapshot {
        CandidateSnapshot {
            input,
            snapshot: result.merged,
            conflicts: result.conflicts,
        }
    }

    /// 병합 입력(base·A·B). C를 바꿔도 그대로입니다.
    pub fn input(&self) -> &MergeInput {
        &self.input
    }

    /// 현재 candidate snapshot.
    pub fn snapshot(&self) -> &SemanticSnapshot {
        &self.snapshot
    }

    /// 엔진이 남긴 conflict(`(StableId, 대상)` 순서).
    pub fn conflicts(&self) -> &[SemanticConflict] {
        &self.conflicts
    }

    /// 아직 해결하지 않은 conflict 수.
    pub fn unresolved_count(&self) -> usize {
        self.conflicts.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::tests::{base, edit, set};
    use crate::model::tests::text;
    use stellar_semantic_diff::keys;

    #[test]
    fn candidate_is_independent_of_sources() {
        let base = base();
        let ours = edit(&base, 11, |o| set(o, keys::DISPLAY_NAME, text("ours")));
        let theirs = edit(&base, 12, |o| set(o, keys::DISPLAY_NAME, text("theirs")));
        let (before_a, before_b) = (ours.clone(), theirs.clone());
        let candidate = CandidateSnapshot::build(MergeInput {
            base: base.clone(),
            ours: ours.clone(),
            theirs: theirs.clone(),
        })
        .unwrap();
        assert_eq!(candidate.unresolved_count(), 0);
        assert_eq!((&ours, &theirs), (&before_a, &before_b));
        assert_eq!(candidate.input().ours, before_a);
        assert_ne!(candidate.snapshot(), &before_a);
        assert_ne!(candidate.snapshot(), &before_b);
    }
}

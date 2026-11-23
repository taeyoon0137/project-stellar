//! 독립 `CandidateSnapshot C`와 해결 모델 (XXIII.2, XIV.12.1~XIV.12.3).
//!
//! Semantic Merge 결과를 입력(Source A = ours, Source B = theirs, base)과 분리된 메모리 값으로 둡니다
//! (아키텍처 §27.4 "결과 C는 A·B를 직접 변경하지 않는 가상 Snapshot"). C는 입력 세 snapshot을 복사해
//! 가지므로 A·B는 바뀌지 않고, 파일·Git index도 읽거나 쓰지 않습니다. 해결(XIV.12)은 C에만 적용합니다.
//!
//! **단위와 출처(XXIII.2.2)**: 단위는 `(StableId, 대상)`입니다(대상 = 객체 존재·위치·속성 경로, conflict 대상과
//! 같은 [`ConflictTarget`]). C는 base와 다른 단위와 conflict 단위마다 출처 [`Source`]를 기록합니다: A·B 한쪽
//! 상태와 같으면 그쪽, 양쪽과 같거나(같은 변경) 엔진이 계산한 값(key 재배정·정의 목록 병합)이면 `Auto`,
//! conflict로 base를 유지하면 `Base`, 사용자가 직접 넣은 값이면 `Custom`입니다. 기록이 없는 단위는 base 그대로입니다.

use std::collections::BTreeMap;

use stellar_semantic_diff::change::{Position, SemanticChange};
use stellar_semantic_diff::core_types::ids::StableId;
use stellar_semantic_diff::diff::diff;
use stellar_semantic_diff::snapshot::SemanticSnapshot;

use crate::merge::merge;
use crate::model::{ConflictTarget, SideState};
use crate::model::{MergeError, MergeInput, MergeResult, SemanticConflict};

/// 단위 `(StableId, 대상)`.
pub type UnitKey = (StableId, ConflictTarget);

/// 단위 값의 출처(XXIII.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Source {
    /// base 상태(conflict로 유지했거나 base를 골랐음).
    Base = 0,
    /// Source A(ours).
    A = 1,
    /// Source B(theirs).
    B = 2,
    /// 양쪽이 같게 바꿨거나 엔진이 계산함.
    Auto = 3,
    /// 사용자가 직접 넣은 값.
    Custom = 4,
}

impl Source {
    /// 진단용 이름.
    pub fn name(self) -> &'static str {
        match self {
            Source::Base => "base",
            Source::A => "a",
            Source::B => "b",
            Source::Auto => "auto",
            Source::Custom => "custom",
        }
    }
}

/// snapshot에서 단위 하나의 상태. 객체가 없으면 `Missing`.
pub fn unit_state(snapshot: &SemanticSnapshot, (object, target): &UnitKey) -> SideState {
    let Some(found) = snapshot.get(*object) else {
        return SideState::Missing;
    };
    match target {
        ConflictTarget::Object => SideState::Object(Box::new(found.clone())),
        ConflictTarget::Position => SideState::Position(Position::of(found)),
        ConflictTarget::Property(path) => SideState::Value(found.property(path).cloned()),
    }
}

/// 변경 하나의 단위.
fn unit_of(change: &SemanticChange) -> UnitKey {
    use stellar_semantic_diff::change::SemanticChangeKind as K;
    let target = match change.kind() {
        K::AddObject | K::RemoveObject | K::AddToken | K::RemoveToken => ConflictTarget::Object,
        K::MoveObject | K::ReorderObject => ConflictTarget::Position,
        _ => change
            .property_path()
            .map_or(ConflictTarget::Object, |path| {
                ConflictTarget::Property(path.clone())
            }),
    };
    (change.object_id(), target)
}

/// 병합 결과 candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateSnapshot {
    input: MergeInput,
    snapshot: SemanticSnapshot,
    conflicts: Vec<SemanticConflict>,
    provenance: BTreeMap<UnitKey, Source>,
}

impl CandidateSnapshot {
    /// 입력을 병합해 C를 만듭니다(XXIII.2.1).
    pub fn build(input: MergeInput) -> Result<CandidateSnapshot, MergeError> {
        let result = merge(&input)?;
        Ok(CandidateSnapshot::from_result(input, result))
    }

    /// 이미 계산한 병합 결과로 C를 만듭니다. `result`는 `input`의 [`merge`] 결과여야 합니다.
    pub fn from_result(input: MergeInput, result: MergeResult) -> CandidateSnapshot {
        let provenance = initial_provenance(&input, &result);
        CandidateSnapshot {
            input,
            snapshot: result.merged,
            conflicts: result.conflicts,
            provenance,
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

    /// 단위의 출처(기록이 없으면 base 그대로).
    pub fn provenance(&self, object: StableId, target: &ConflictTarget) -> Source {
        self.provenance
            .get(&(object, target.clone()))
            .copied()
            .unwrap_or(Source::Base)
    }

    /// 기록한 출처(단위 순서).
    pub fn provenance_entries(&self) -> impl Iterator<Item = (&UnitKey, Source)> + '_ {
        self.provenance.iter().map(|(key, source)| (key, *source))
    }
}

/// 초기 출처: base와 다른 단위는 A·B 상태 비교, conflict 단위는 base.
fn initial_provenance(input: &MergeInput, result: &MergeResult) -> BTreeMap<UnitKey, Source> {
    let mut out = BTreeMap::new();
    for change in diff(&input.base, &result.merged).iter() {
        let key = unit_of(change);
        let state = unit_state(&result.merged, &key);
        let a = state == unit_state(&input.ours, &key);
        let b = state == unit_state(&input.theirs, &key);
        let source = match (a, b) {
            (true, false) => Source::A,
            (false, true) => Source::B,
            _ => Source::Auto,
        };
        out.insert(key, source);
    }
    for conflict in &result.conflicts {
        out.entry((conflict.object, conflict.target.clone()))
            .or_insert(Source::Base);
    }
    out
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

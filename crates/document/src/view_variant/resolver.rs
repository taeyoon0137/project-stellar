//! Variant resolver (IX.7.4~IX.7.6).
//!
//! - **matching·정렬**(IX.7.4): 현재 변수 할당과 조건이 맞는 모든 Variant를 찾아 **구체성 오름차순**(일반 → 구체),
//!   같은 구체성은 Variant StableId 오름차순으로 정렬합니다. 적용 순서는 `Base → 일반 Variant → 더 구체적인 조합
//!   Variant`입니다(기술 아키텍처 16장).

use std::fmt;

use stellar_core_types::ids::StableId;

use super::condition::{Assignment, VariantCondition};
use super::variable::ViewVariableSet;
use crate::component::variant_override::VariantOverrideSet;

/// View Variant 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewVariantId(StableId);

impl ViewVariantId {
    /// `StableId`로부터 만듭니다.
    pub const fn new(id: StableId) -> ViewVariantId {
        ViewVariantId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }
}

/// View Variant 하나: 조건 + 희소 override(IX.7.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewVariant {
    /// 식별자.
    pub id: ViewVariantId,
    /// 조건.
    pub condition: VariantCondition,
    /// override.
    pub overrides: VariantOverrideSet,
}

impl ViewVariant {
    /// Variant를 만듭니다.
    pub fn new(
        id: ViewVariantId,
        condition: VariantCondition,
        overrides: VariantOverrideSet,
    ) -> ViewVariant {
        ViewVariant {
            id,
            condition,
            overrides,
        }
    }

    /// 구체성(`Equals` term 수).
    pub fn specificity(&self) -> usize {
        self.condition.specificity()
    }
}

impl fmt::Display for ViewVariantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 할당과 맞는 Variant를 적용 순서(구체성 오름차순, 같으면 id 오름차순)로 돌려줍니다.
pub fn matching_variants<'a>(
    variants: impl IntoIterator<Item = &'a ViewVariant>,
    assignment: &Assignment,
    variables: &ViewVariableSet,
) -> Vec<&'a ViewVariant> {
    let mut matched: Vec<&ViewVariant> = variants
        .into_iter()
        .filter(|variant| variant.condition.matches(assignment, variables))
        .collect();
    matched.sort_by_key(|variant| (variant.specificity(), variant.id));
    matched
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! 기술 아키텍처 16장 예제 Variant.

    use super::super::variable::fixtures::*;
    use super::*;

    /// Variant id.
    pub(crate) fn variant_id(ts: u64) -> ViewVariantId {
        ViewVariantId::new(stable(ts))
    }

    /// `조건 → 빈 override` Variant.
    pub(crate) fn variant(ts: u64, condition: VariantCondition) -> ViewVariant {
        ViewVariant::new(variant_id(ts), condition, VariantOverrideSet::new())
    }

    pub(crate) const BASE_ANY: u64 = 7_100;
    pub(crate) const COMPACT: u64 = 7_101;
    pub(crate) const ERROR: u64 = 7_102;
    pub(crate) const COMPACT_ERROR: u64 = 7_103;
    pub(crate) const DARK_ON: u64 = 7_104;

    pub(crate) fn compact() -> VariantCondition {
        VariantCondition::new().equals(vid(DEVICE), device(0))
    }

    pub(crate) fn error() -> VariantCondition {
        VariantCondition::new().equals(vid(PAYMENT), payment(2))
    }
}

#[cfg(test)]
mod tests {
    use super::super::condition::ConditionTerm;
    use super::super::variable::fixtures::*;
    use super::super::variable::VariableValue;
    use super::fixtures::*;
    use super::*;
    use stellar_core_types::typed_value::TypedValue;

    #[test]
    fn general_and_specific_variants_are_ordered() {
        // 검증 기준(IX.7.4): 일반+구체 조합 fixture 순서 정확.
        let set = variable_set();
        let variants = vec![
            variant(COMPACT_ERROR, compact().equals(vid(PAYMENT), payment(2))),
            variant(ERROR, error()),
            variant(
                DARK_ON,
                VariantCondition::new()
                    .equals(vid(DARK), VariableValue::Value(TypedValue::Boolean(true))),
            ),
            variant(COMPACT, compact()),
            variant(
                BASE_ANY,
                VariantCondition::new().with(vid(DEVICE), ConditionTerm::Any),
            ),
        ];
        let ids = |assignment: &Assignment| {
            matching_variants(&variants, assignment, &set)
                .into_iter()
                .map(|v| v.id)
                .collect::<Vec<_>>()
        };
        // compact + error: Any(0) → compact(1)·error(1, id 순) → compact+error(2). dark=false라 DARK_ON 제외.
        let compact_error = Assignment::new()
            .with(vid(DEVICE), device(0))
            .with(vid(PAYMENT), payment(2));
        assert_eq!(
            ids(&compact_error),
            vec![
                variant_id(BASE_ANY),
                variant_id(COMPACT),
                variant_id(ERROR),
                variant_id(COMPACT_ERROR),
            ]
        );
        // regular + error: 조합 variant 없음.
        let regular_error = Assignment::new().with(vid(PAYMENT), payment(2));
        assert_eq!(
            ids(&regular_error),
            vec![variant_id(BASE_ANY), variant_id(ERROR)]
        );
        // 입력 순서와 무관하게 같은 결과.
        let mut reversed = variants.clone();
        reversed.reverse();
        assert_eq!(
            matching_variants(&reversed, &compact_error, &set)
                .into_iter()
                .map(|v| v.id)
                .collect::<Vec<_>>(),
            ids(&compact_error)
        );
        // dark = true면 DARK_ON(1)이 compact(1)·error(1)과 id 순으로 섞입니다.
        let all = compact_error.with(vid(DARK), VariableValue::Value(TypedValue::Boolean(true)));
        assert_eq!(
            ids(&all),
            vec![
                variant_id(BASE_ANY),
                variant_id(COMPACT),
                variant_id(ERROR),
                variant_id(DARK_ON),
                variant_id(COMPACT_ERROR),
            ]
        );
    }
}

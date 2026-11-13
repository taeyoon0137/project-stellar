//! Variant resolver (IX.7.4~IX.7.6).
//!
//! - **matching·정렬**(IX.7.4): 현재 변수 할당과 조건이 맞는 모든 Variant를 찾아 **구체성 오름차순**(일반 → 구체),
//!   같은 구체성은 Variant StableId 오름차순으로 정렬합니다. 적용 순서는 `Base → 일반 Variant → 더 구체적인 조합
//!   Variant`입니다(기술 아키텍처 16장).
//! - **충돌**(IX.7.5): 같은 patch 대상(PropertyPath·구조 patch node·(인스턴스, slot))을 지정한 Variant 중 **가장
//!   구체적인 group**만 값을 정합니다. 그 group 안에서 값이 서로 다르면 [`VariantConflict`]를 만들고 그 대상은
//!   **적용하지 않습니다** — 정렬 순서(id)로 임의 선택하지 않습니다. 더 구체적인 Variant가 같은 대상을 지정하면
//!   아래 group의 모호함은 해소됩니다(16장: "더 구체적인 Variant가 같은 속성을 지정하면 우선").
//! - **적용**(IX.7.6): 구체 Variant 값이 일반 Variant 값을 덮어쓰고, 지정하지 않은 속성은 Base 또는 일반 Variant 값을
//!   상속합니다. 정의되지 않은 조합(예: compact + processing)에는 추가 override가 없으므로 Base + 그 조합에 일치하는
//!   일반 Variant만 적용됩니다. 충돌 대상은 Variant 값을 적용하지 않고 Base 값을 유지합니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::typed_value::TypedValue;

use super::condition::{Assignment, VariantCondition};
use super::variable::ViewVariableSet;
use crate::component::definition::ComponentSlotId;
use crate::component::variant_override::{SlotChange, StructurePatch, VariantOverrideSet};

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

/// patch 대상.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PatchKey {
    /// 속성 경로.
    Property(PropertyPath),
    /// 구조 patch 대상 node(Add면 새 node id).
    Structure(StableId),
    /// (인스턴스, slot).
    Slot(StableId, ComponentSlotId),
}

/// patch 값.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchValue {
    /// 속성 값.
    Property(TypedValue),
    /// 구조 patch.
    Structure(StructurePatch),
    /// slot change.
    Slot(SlotChange),
}

/// 같은 구체성 Variant끼리 같은 대상을 다르게 지정한 충돌.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantConflict {
    /// 대상.
    pub key: PatchKey,
    /// 충돌한 group의 구체성.
    pub specificity: usize,
    /// 그 group에서 대상을 지정한 Variant(id 오름차순).
    pub variants: Vec<ViewVariantId>,
}

/// resolve 결과.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VariantResolution {
    /// 적용 순서의 일치 Variant.
    pub matched: Vec<ViewVariantId>,
    /// 대상별 적용 값과 그 값을 정한 Variant(대상 오름차순). 충돌 대상은 없습니다.
    pub applied: BTreeMap<PatchKey, (ViewVariantId, PatchValue)>,
    /// 충돌(대상 오름차순).
    pub conflicts: Vec<VariantConflict>,
}

impl VariantResolution {
    /// 충돌이 있는지(모호한 조합).
    pub fn is_ambiguous(&self) -> bool {
        !self.conflicts.is_empty()
    }

    /// 적용 값을 하나의 희소 override로 합칩니다(충돌 대상 제외). scene·layout 생성이 base 위에 이 set을 적용합니다.
    pub fn effective_overrides(&self) -> VariantOverrideSet {
        let mut set = VariantOverrideSet::new();
        for (key, (_, value)) in &self.applied {
            match (key, value) {
                (PatchKey::Property(path), PatchValue::Property(value)) => {
                    set.set_property(path.clone(), value.clone());
                }
                (PatchKey::Structure(node), PatchValue::Structure(patch)) => {
                    set.set_structure(*node, patch.clone());
                }
                (PatchKey::Slot(instance, slot), PatchValue::Slot(change)) => {
                    set.set_slot(*instance, *slot, change.clone());
                }
                // key와 value 갈래는 resolve_variants가 항상 같게 만듭니다.
                _ => {}
            }
        }
        set
    }
}

/// 속성 값의 출처.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueSource {
    /// Base View 값.
    Base,
    /// 그 값을 정한 Variant.
    Variant(ViewVariantId),
}

/// 적용 결과 속성 값.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedValue {
    /// 값.
    pub value: TypedValue,
    /// 출처.
    pub source: ValueSource,
}

/// Base 속성 값에 resolve 결과를 적용합니다(경로 오름차순). Base에 없는 경로라도 Variant가 지정하면 포함합니다.
pub fn resolve_properties(
    base: &BTreeMap<PropertyPath, TypedValue>,
    resolution: &VariantResolution,
) -> BTreeMap<PropertyPath, ResolvedValue> {
    let mut out: BTreeMap<PropertyPath, ResolvedValue> = base
        .iter()
        .map(|(path, value)| {
            (
                path.clone(),
                ResolvedValue {
                    value: value.clone(),
                    source: ValueSource::Base,
                },
            )
        })
        .collect();
    for (key, (variant, value)) in &resolution.applied {
        if let (PatchKey::Property(path), PatchValue::Property(value)) = (key, value) {
            out.insert(
                path.clone(),
                ResolvedValue {
                    value: value.clone(),
                    source: ValueSource::Variant(*variant),
                },
            );
        }
    }
    out
}

fn patches(variant: &ViewVariant) -> Vec<(PatchKey, PatchValue)> {
    let overrides = &variant.overrides;
    let mut out: Vec<(PatchKey, PatchValue)> = overrides
        .property_patches()
        .map(|(path, value)| {
            (
                PatchKey::Property(path.clone()),
                PatchValue::Property(value.clone()),
            )
        })
        .collect();
    out.extend(overrides.structure_patches().map(|(node, patch)| {
        (
            PatchKey::Structure(node),
            PatchValue::Structure(patch.clone()),
        )
    }));
    out.extend(overrides.slot_changes().map(|(instance, slot, change)| {
        (
            PatchKey::Slot(instance, slot),
            PatchValue::Slot(change.clone()),
        )
    }));
    out
}

/// 할당과 맞는 Variant를 정렬하고 대상별 적용 값·충돌을 계산합니다.
pub fn resolve_variants<'a>(
    variants: impl IntoIterator<Item = &'a ViewVariant>,
    assignment: &Assignment,
    variables: &ViewVariableSet,
) -> VariantResolution {
    let matched = matching_variants(variants, assignment, variables);
    // 대상 → (구체성, id, 값) 목록(적용 순서).
    let mut by_key: BTreeMap<PatchKey, Vec<(usize, ViewVariantId, PatchValue)>> = BTreeMap::new();
    for variant in &matched {
        for (key, value) in patches(variant) {
            by_key
                .entry(key)
                .or_default()
                .push((variant.specificity(), variant.id, value));
        }
    }
    let mut resolution = VariantResolution {
        matched: matched.iter().map(|v| v.id).collect(),
        ..VariantResolution::default()
    };
    for (key, entries) in by_key {
        let Some(top) = entries.iter().map(|(spec, _, _)| *spec).max() else {
            continue;
        };
        let group: Vec<&(usize, ViewVariantId, PatchValue)> =
            entries.iter().filter(|(spec, _, _)| *spec == top).collect();
        let first = group[0];
        if group.iter().all(|(_, _, value)| *value == first.2) {
            resolution.applied.insert(key, (first.1, first.2.clone()));
        } else {
            resolution.conflicts.push(VariantConflict {
                key,
                specificity: top,
                variants: group.iter().map(|(_, id, _)| *id).collect(),
            });
        }
    }
    resolution
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

    use stellar_core_types::property::PropertyKey;
    use stellar_core_types::value::FiniteF32;

    fn padding(node: u64) -> PropertyPath {
        PropertyPath::with_keys(stable(node), vec![PropertyKey(0x0110)])
    }

    fn number(v: f32) -> TypedValue {
        TypedValue::Number(FiniteF32::new(v).unwrap())
    }

    fn patched(ts: u64, condition: VariantCondition, value: f32) -> ViewVariant {
        let mut v = variant(ts, condition);
        v.overrides.set_property(padding(1), number(value));
        v
    }

    #[test]
    fn same_specificity_conflict_is_reported_and_not_applied() {
        // 검증 기준(IX.7.5): ambiguity fixture 실패. compact(1)와 error(1)가 같은 padding을 8·12로 지정.
        let set = variable_set();
        let variants = vec![
            patched(COMPACT, compact(), 8.0),
            patched(ERROR, error(), 12.0),
        ];
        let compact_error = Assignment::new()
            .with(vid(DEVICE), device(0))
            .with(vid(PAYMENT), payment(2));
        let resolution = resolve_variants(&variants, &compact_error, &set);
        assert!(resolution.is_ambiguous());
        assert_eq!(
            resolution.conflicts,
            vec![VariantConflict {
                key: PatchKey::Property(padding(1)),
                specificity: 1,
                variants: vec![variant_id(COMPACT), variant_id(ERROR)],
            }]
        );
        assert!(
            resolution.applied.is_empty(),
            "임의 순서로 적용하지 않음(id 순서상 앞인 값도 적용 안 함)"
        );
        // 한쪽만 일치하면 충돌 없음.
        let regular_error = Assignment::new().with(vid(PAYMENT), payment(2));
        let resolution = resolve_variants(&variants, &regular_error, &set);
        assert!(!resolution.is_ambiguous());
        assert_eq!(
            resolution.applied[&PatchKey::Property(padding(1))],
            (variant_id(ERROR), PatchValue::Property(number(12.0)))
        );
        // 같은 값이면 충돌이 아닙니다.
        let same = vec![
            patched(COMPACT, compact(), 8.0),
            patched(ERROR, error(), 8.0),
        ];
        assert!(!resolve_variants(&same, &compact_error, &set).is_ambiguous());
        // 더 구체적인 조합 Variant가 같은 대상을 지정하면 모호함이 해소됩니다.
        let mut resolved = variants.clone();
        resolved.push(patched(
            COMPACT_ERROR,
            compact().equals(vid(PAYMENT), payment(2)),
            4.0,
        ));
        let resolution = resolve_variants(&resolved, &compact_error, &set);
        assert!(!resolution.is_ambiguous());
        assert_eq!(
            resolution.applied[&PatchKey::Property(padding(1))],
            (variant_id(COMPACT_ERROR), PatchValue::Property(number(4.0)))
        );
    }

    // --- IX.7.6: 기술 아키텍처 16장 요구 예제 ---

    fn color(node: u64) -> PropertyPath {
        PropertyPath::with_keys(stable(node), vec![PropertyKey(0x0120)])
    }

    fn text(value: &str) -> TypedValue {
        TypedValue::String(value.into())
    }

    /// compact: padding 8 / error: color "red" / compact+error: padding 4.
    fn example_variants() -> Vec<ViewVariant> {
        let mut error_variant = variant(ERROR, error());
        error_variant.overrides.set_property(color(1), text("red"));
        vec![
            patched(COMPACT, compact(), 8.0),
            error_variant,
            patched(
                COMPACT_ERROR,
                compact().equals(vid(PAYMENT), payment(2)),
                4.0,
            ),
        ]
    }

    fn example_base() -> BTreeMap<PropertyPath, TypedValue> {
        [(padding(1), number(16.0)), (color(1), text("black"))]
            .into_iter()
            .collect()
    }

    fn resolved(assignment: &Assignment) -> BTreeMap<PropertyPath, (TypedValue, ValueSource)> {
        let set = variable_set();
        let resolution = resolve_variants(&example_variants(), assignment, &set);
        assert!(!resolution.is_ambiguous());
        resolve_properties(&example_base(), &resolution)
            .into_iter()
            .map(|(path, value)| (path, (value.value, value.source)))
            .collect()
    }

    #[test]
    fn specific_overrides_general_and_missing_combinations_use_general_only() {
        // 검증 기준(IX.7.6): 요구 예제 fixture 통과.
        let regular = |payment_value| {
            Assignment::new()
                .with(vid(DEVICE), device(1))
                .with(vid(PAYMENT), payment_value)
        };
        let compact_with = |payment_value| {
            Assignment::new()
                .with(vid(DEVICE), device(0))
                .with(vid(PAYMENT), payment_value)
        };
        // regular + idle: Base만.
        let r = resolved(&regular(payment(0)));
        assert_eq!(r[&padding(1)], (number(16.0), ValueSource::Base));
        assert_eq!(r[&color(1)], (text("black"), ValueSource::Base));
        // compact + processing: 정의되지 않은 조합 → Base + 일반 compact만.
        let r = resolved(&compact_with(payment(1)));
        assert_eq!(
            r[&padding(1)],
            (number(8.0), ValueSource::Variant(variant_id(COMPACT)))
        );
        assert_eq!(r[&color(1)], (text("black"), ValueSource::Base));
        // regular + error: Base + 일반 error.
        let r = resolved(&regular(payment(2)));
        assert_eq!(r[&padding(1)], (number(16.0), ValueSource::Base));
        assert_eq!(
            r[&color(1)],
            (text("red"), ValueSource::Variant(variant_id(ERROR)))
        );
        // compact + error: 구체 조합이 compact의 padding을 덮어쓰고, color는 일반 error 값을 상속.
        let r = resolved(&compact_with(payment(2)));
        assert_eq!(
            r[&padding(1)],
            (number(4.0), ValueSource::Variant(variant_id(COMPACT_ERROR)))
        );
        assert_eq!(
            r[&color(1)],
            (text("red"), ValueSource::Variant(variant_id(ERROR)))
        );
        // 합친 override set도 같은 결과를 담습니다.
        let set = variable_set();
        let effective = resolve_variants(&example_variants(), &compact_with(payment(2)), &set)
            .effective_overrides();
        assert_eq!(
            effective.property_patches().collect::<Vec<_>>(),
            vec![(&padding(1), &number(4.0)), (&color(1), &text("red"))]
        );
    }
}

//! Component Context headless 조립 (XXX.7.9·XXX.7.11, ADR-0099 결정 9·11).
//!
//! - [`resolve_context_overrides`]: provision 값 경로에 Target Profile override와 View Variant override를 기존 우선순위
//!   (공통값 → profile override → variant override → profile × variant, `resolve_for_profile`)로 적용한 ContextSet을
//!   만듭니다. 같은 구체성 Variant 충돌은 variant resolver의 충돌을 그대로 진단으로 돌려줍니다.
//! - 조회(인스펙터용)는 문서 crate `context::query`가 정하고, 이 모듈은 열린 프로젝트에서 같은 함수를 부릅니다.

use std::collections::BTreeMap;

use stellar_document::context::overrides::{
    common_values, provision_of_path, variant_conflicts, variant_values, with_overrides,
    ContextOverrideConflict, ContextOverrideError,
};
use stellar_document::context::set::{ContextKeys, ContextSet};
use stellar_document::view_variant::resolver::VariantResolution;
use stellar_semantic_diff::core_types::ids::StableId;
use stellar_target_profile::overrides::{
    resolve_for_profile, CommonValues, ProfileOverrideSet, ValueOrigin, VariantLayer,
};
use stellar_target_profile::spec::TargetProfile;

/// override를 적용한 Context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextOverrideResult {
    /// provision 값에 override를 넣은 set.
    pub context: ContextSet,
    /// provision별 값 출처(공통값·profile·variant·profile × variant).
    pub origins: BTreeMap<StableId, ValueOrigin>,
    /// 같은 구체성 Variant 충돌(그 provision은 Variant 값을 적용하지 않음).
    pub conflicts: Vec<ContextOverrideConflict>,
}

/// provision 값에 profile·variant override를 적용합니다(XXX.7.9).
pub fn resolve_context_overrides(
    set: &ContextSet,
    keys: &ContextKeys<'_>,
    overrides: &ProfileOverrideSet,
    profile: &TargetProfile,
    variants: &VariantResolution,
) -> Result<ContextOverrideResult, ContextOverrideError> {
    let mut common = CommonValues::new();
    for (path, value) in common_values(set) {
        common.set(path, value);
    }
    let layer = VariantLayer::new(
        variants.matched.iter().map(|id| id.stable_id()).collect(),
        variant_values(variants),
    );
    let resolved = resolve_for_profile(&common, overrides, profile, &layer);
    let provision_values: Vec<_> = resolved
        .values
        .iter()
        .filter(|(path, _)| provision_of_path(path).is_some())
        .collect();
    let context = with_overrides(
        set,
        keys,
        provision_values
            .iter()
            .map(|(path, value)| (*path, &value.value)),
    )?;
    let origins = provision_values
        .iter()
        .filter_map(|(path, value)| provision_of_path(path).map(|id| (id, value.origin)))
        .collect();
    Ok(ContextOverrideResult {
        context,
        origins,
        conflicts: variant_conflicts(variants),
    })
}

#[cfg(test)]
mod tests {
    use stellar_document::component::variant_override::VariantOverrideSet;
    use stellar_document::context::fixtures::world::*;
    use stellar_document::context::fixtures::*;
    use stellar_document::context::resolve::{
        ContextEvaluator, ContextResolver, ContextSite, NoInputs,
    };
    use stellar_document::expressions::types::{ExpressionType, TypeSpec};
    use stellar_document::view_variant::condition::{Assignment, VariantCondition};
    use stellar_document::view_variant::resolver::{resolve_variants, ViewVariant, ViewVariantId};
    use stellar_document::view_variant::variable::{
        VariableValue, ViewVariable, ViewVariableId, ViewVariableSet,
    };
    use stellar_semantic_diff::core_types::typed_value::TypedValue;
    use stellar_semantic_diff::core_types::value::FiniteF32;
    use stellar_target_profile::overrides::OverrideScope;
    use stellar_target_profile::profile::TargetProfileId;
    use stellar_target_profile::registry::ProfileRegistryVersion;
    use stellar_target_profile::spec::{
        CapabilitySetVersion, ProfileSpec, WebBaseline, WebProfile,
    };

    use super::*;

    fn num(v: f32) -> TypedValue {
        TypedValue::Number(FiniteF32::new(v).unwrap())
    }

    fn web() -> TargetProfile {
        TargetProfile::new(
            TargetProfileId::new("web").unwrap(),
            ProfileRegistryVersion::new(1).unwrap(),
            ProfileSpec::Web(WebProfile {
                baseline: WebBaseline::new(2024).unwrap(),
                capability_set_version: CapabilitySetVersion::new(1).unwrap(),
            }),
        )
    }

    fn flag(id: u64) -> ViewVariable {
        ViewVariable::new(
            ViewVariableId::new(sid(id)),
            format!("v{id}"),
            TypeSpec::required(ExpressionType::Boolean),
            VariableValue::Value(TypedValue::Boolean(false)),
            [],
        )
        .unwrap()
    }

    fn on(_variable: u64) -> VariableValue {
        VariableValue::Value(TypedValue::Boolean(true))
    }

    /// dark(spec 1)·compact(spec 1)·dark+compact(spec 2) Variant가 View root N provision 값을 덮어씁니다.
    fn variants(conflict: bool) -> (Vec<ViewVariant>, ViewVariableSet) {
        let mut set = ViewVariableSet::new(sid(VIEW));
        set.insert(flag(8001)).unwrap();
        set.insert(flag(8002)).unwrap();
        let path = stellar_document::context::provision::ContextProvision::new(
            sid(P_ROOT_N),
            stellar_document::context::provision::ContextOwner::View(sid(VIEW)),
            sid(ROOT),
            sid(N),
            number(1, 0.0),
        )
        .unwrap()
        .value_path();
        let patch = |value: f32| {
            let mut overrides = VariantOverrideSet::new();
            overrides.set_property(path.clone(), num(value));
            overrides
        };
        let dark = VariantCondition::new().equals(ViewVariableId::new(sid(8001)), on(8001));
        let compact = VariantCondition::new().equals(ViewVariableId::new(sid(8002)), on(8002));
        let mut out = vec![
            ViewVariant::new(ViewVariantId::new(sid(8101)), dark.clone(), patch(20.0)),
            ViewVariant::new(
                ViewVariantId::new(sid(8102)),
                compact.clone(),
                patch(if conflict { 30.0 } else { 20.0 }),
            ),
        ];
        if !conflict {
            out.push(ViewVariant::new(
                ViewVariantId::new(sid(8103)),
                dark.equals(ViewVariableId::new(sid(8002)), on(8002)),
                patch(40.0),
            ));
        }
        (out, set)
    }

    fn value_at_frame(
        model: &stellar_document::model::DocumentModel,
        set: &ContextSet,
    ) -> Option<TypedValue> {
        let (_, library) = sample_world();
        let resolver = ContextResolver::with_context(model, &library, set);
        let keys = ContextKeys::new(set);
        ContextEvaluator::new(&resolver, &keys, &NoInputs)
            .value(sid(N), &ContextSite::scene(sid(FRAME)))
            .unwrap()
    }

    #[test]
    fn provision_overrides_follow_common_profile_variant_priority() {
        // 검증 기준(XXX.7.9): 우선순위 조합 fixture 통과.
        let (model, _) = sample_world();
        let set = model.context().clone();
        let keys = ContextKeys::new(&set);
        let profile = web();
        let path = set.provision(sid(P_ROOT_N)).unwrap().value_path();
        let (all, variables) = variants(false);
        let none = resolve_variants(&all, &Assignment::new(), &variables);
        let dark = resolve_variants(
            &all,
            &Assignment::new().with(ViewVariableId::new(sid(8001)), on(8001)),
            &variables,
        );
        let both = resolve_variants(
            &all,
            &Assignment::new()
                .with(ViewVariableId::new(sid(8001)), on(8001))
                .with(ViewVariableId::new(sid(8002)), on(8002)),
            &variables,
        );
        let mut profiles = ProfileOverrideSet::new();
        // 1) 공통값만.
        let result = resolve_context_overrides(&set, &keys, &profiles, &profile, &none).unwrap();
        assert_eq!(result.origins[&sid(P_ROOT_N)], ValueOrigin::Common);
        assert_eq!(value_at_frame(&model, &result.context), Some(num(10.0)));
        // 2) profile override가 공통값을 이깁니다.
        profiles
            .profile_mut(&profile.id)
            .set(OverrideScope::Always, path.clone(), num(11.0));
        let result = resolve_context_overrides(&set, &keys, &profiles, &profile, &none).unwrap();
        assert_eq!(result.origins[&sid(P_ROOT_N)], ValueOrigin::Profile);
        assert_eq!(value_at_frame(&model, &result.context), Some(num(11.0)));
        // 3) variant override가 profile override를 이깁니다.
        let result = resolve_context_overrides(&set, &keys, &profiles, &profile, &dark).unwrap();
        assert_eq!(
            result.origins[&sid(P_ROOT_N)],
            ValueOrigin::Variant(sid(8101))
        );
        assert_eq!(value_at_frame(&model, &result.context), Some(num(20.0)));
        // 4) 더 구체적인 Variant가 이기고, Card 안의 같은 key 재읽기도 새 값을 씁니다(40 + 5).
        let result = resolve_context_overrides(&set, &keys, &profiles, &profile, &both).unwrap();
        assert_eq!(value_at_frame(&model, &result.context), Some(num(40.0)));
        let (_, library) = sample_world();
        let resolver = ContextResolver::with_context(&model, &library, &result.context);
        let keys2 = ContextKeys::new(&result.context);
        let inner = ContextEvaluator::new(&resolver, &keys2, &NoInputs)
            .value(
                sid(N),
                &ContextSite::virtual_node(vec![sid(CARD_INSTANCE)], sid(CARD_FRAME)),
            )
            .unwrap();
        assert_eq!(inner, Some(num(45.0)));
        // 5) profile × variant override가 variant override를 이깁니다.
        let dark_id = sid(8101);
        profiles.profile_mut(&profile.id).set(
            OverrideScope::Variant(dark_id),
            path.clone(),
            num(21.0),
        );
        let result = resolve_context_overrides(&set, &keys, &profiles, &profile, &dark).unwrap();
        assert_eq!(
            result.origins[&sid(P_ROOT_N)],
            ValueOrigin::ProfileVariant(dark_id)
        );
        assert_eq!(value_at_frame(&model, &result.context), Some(num(21.0)));
        assert!(result.conflicts.is_empty());
    }

    #[test]
    fn same_specificity_variant_conflict_is_reported_and_not_applied() {
        // 검증 기준(XXX.7.9): 같은 specificity 충돌 diagnostic.
        let (model, _) = sample_world();
        let set = model.context().clone();
        let keys = ContextKeys::new(&set);
        let (all, variables) = variants(true);
        let both = resolve_variants(
            &all,
            &Assignment::new()
                .with(ViewVariableId::new(sid(8001)), on(8001))
                .with(ViewVariableId::new(sid(8002)), on(8002)),
            &variables,
        );
        let result =
            resolve_context_overrides(&set, &keys, &ProfileOverrideSet::new(), &web(), &both)
                .unwrap();
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].provision, sid(P_ROOT_N));
        assert_eq!(result.conflicts[0].specificity, 1);
        assert_eq!(
            result.conflicts[0].variants,
            vec![ViewVariantId::new(sid(8101)), ViewVariantId::new(sid(8102))]
        );
        // 충돌 대상은 Variant 값을 적용하지 않고 공통값입니다.
        assert_eq!(result.origins[&sid(P_ROOT_N)], ValueOrigin::Common);
        assert_eq!(value_at_frame(&model, &result.context), Some(num(10.0)));
        // 타입이 다른 override 값은 거부합니다.
        let mut profiles = ProfileOverrideSet::new();
        let profile = web();
        profiles.profile_mut(&profile.id).set(
            OverrideScope::Always,
            set.provision(sid(P_ROOT_N)).unwrap().value_path(),
            TypedValue::Boolean(true),
        );
        let none = resolve_variants(&all, &Assignment::new(), &variables);
        assert_eq!(
            resolve_context_overrides(&set, &keys, &profiles, &profile, &none),
            Err(ContextOverrideError::TypeMismatch {
                provision: sid(P_ROOT_N)
            })
        );
    }
}

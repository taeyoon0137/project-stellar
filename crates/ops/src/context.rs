//! Component Context headless 조립 (XXX.7.9·XXX.7.11, ADR-0099 결정 9·11).
//!
//! - [`resolve_context_overrides`]: provision 값 경로에 Target Profile override와 View Variant override를 기존 우선순위
//!   (공통값 → profile override → variant override → profile × variant, `resolve_for_profile`)로 적용한 ContextSet을
//!   만듭니다. 같은 구체성 Variant 충돌은 variant resolver의 충돌을 그대로 진단으로 돌려줍니다.
//! - 조회(인스펙터용)는 문서 crate `context::query`가 정하고, 이 모듈은 열린 프로젝트에서 같은 함수를 부릅니다.
//! - [`read_context`]: 저장된 프로젝트의 문서 하나에서 Context 선언(key·provision·requirement·Slot 공개 목록)과 validator
//!   진단을, `site`를 주면 그 위치에서 key마다 해석 값·출처·provider 경로와 Value Binding 선택기 후보를 돌려줍니다
//!   (`stellar context`, MCP `document.context`, ADR-0114 결정 6). 값·식·타입 문장은 Context 편집 입력과 같은 텍스트
//!   원본 문법입니다. binding은 아직 문서 원본에 저장되지 않으므로 소비 binding 단위 조회(`inspect_binding`)는 앱
//!   경로의 몫이고, headless 조회는 위치 단위입니다.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

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

use stellar_document::context::query::binding_candidates;
/// 조회 위치(인스턴스 chain + node, ADR-0099 결정 4).
pub use stellar_document::context::resolve::ContextSite;
use stellar_document::context::resolve::{
    ContextEvaluator, ContextOrigin, ContextResolver, NoInputs, ResolveError,
};
use stellar_document::context::validate::{validate_context, ContextSeverity};
use stellar_project::document_io::source_v4::context::{
    expression_text, owner_text, scope_text, type_name, value_text,
};

use crate::error::{DocumentReadError, OpenError};
use crate::revision::ProjectRevision;
use crate::snapshot::{open_snapshot, ProjectSnapshot};

/// key 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextKeyRow {
    /// key.
    pub id: StableId,
    /// 이름.
    pub name: String,
    /// scope 문장(`project`·`library <id>`·`document <id>`).
    pub scope: String,
    /// 값 타입 문장.
    pub value_type: String,
    /// default 식 문장.
    pub default: String,
}

/// provision 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextProvisionRow {
    /// provision.
    pub id: StableId,
    /// owner 문장(`view <id>`·`component <id>`).
    pub owner: String,
    /// provider node.
    pub provider: StableId,
    /// key.
    pub key: StableId,
    /// 값 식 문장.
    pub value: String,
}

/// 진단 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextDiagnosticRow {
    /// 안정 코드(`context.*`).
    pub code: &'static str,
    /// 수준(`error`·`orphan`·`warning`·`info`).
    pub severity: &'static str,
    /// 설명.
    pub message: String,
}

/// 해석 출처.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextSource {
    /// key default.
    Default,
    /// provision.
    Provision {
        /// provision.
        provision: StableId,
        /// provider node.
        provider: StableId,
        /// provider를 펼친 인스턴스 chain.
        instances: Vec<StableId>,
    },
}

impl From<&ContextOrigin> for ContextSource {
    fn from(origin: &ContextOrigin) -> ContextSource {
        match origin {
            ContextOrigin::Default => ContextSource::Default,
            ContextOrigin::Provision {
                provision,
                provider,
                instances,
            } => ContextSource::Provision {
                provision: *provision,
                provider: *provider,
                instances: instances.clone(),
            },
        }
    }
}

/// 위치에서 key 하나의 해석.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextResolutionRow {
    /// key.
    pub key: StableId,
    /// key 이름.
    pub name: String,
    /// 해석 값 문장(값 없음·리터럴로 쓰지 않는 타입·평가 실패면 `None`).
    pub value: Option<String>,
    /// 평가 실패 원인.
    pub error: Option<String>,
    /// 출처.
    pub source: ContextSource,
    /// 지나간 위치(소비 지점 → provider, default면 최상위까지).
    pub path: Vec<ContextSite>,
    /// Slot 공개 목록 경유.
    pub via_slots: Vec<StableId>,
}

/// Value Binding 선택기 후보 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCandidateRow {
    /// key.
    pub key: StableId,
    /// 이름.
    pub name: String,
    /// 값 타입 문장.
    pub value_type: String,
    /// 소유 정의에 requirement를 더해야 하는지.
    pub needs_declaration: bool,
    /// 이 위치의 출처.
    pub source: ContextSource,
}

/// 위치 하나의 조회.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSiteReport {
    /// 위치.
    pub site: ContextSite,
    /// key마다 해석(key 이름·StableId 순).
    pub resolutions: Vec<ContextResolutionRow>,
    /// 선택기 후보(이름·StableId 순).
    pub candidates: Vec<ContextCandidateRow>,
}

/// 문서 하나의 Context 조회 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextReport {
    /// 읽은 revision.
    pub revision: ProjectRevision,
    /// 문서.
    pub document: StableId,
    /// key(StableId 순).
    pub keys: Vec<ContextKeyRow>,
    /// provision(StableId 순).
    pub provisions: Vec<ContextProvisionRow>,
    /// 컴포넌트 requirement(컴포넌트 StableId 순, key 정렬).
    pub requirements: Vec<(StableId, Vec<StableId>)>,
    /// Slot 공개 목록(Slot StableId 순, key 정렬).
    pub slot_keys: Vec<(StableId, Vec<StableId>)>,
    /// validator 진단(종류·대상 순).
    pub diagnostics: Vec<ContextDiagnosticRow>,
    /// 위치 조회(`site`를 줬을 때만).
    pub site: Option<ContextSiteReport>,
}

/// 조회 실패.
#[derive(Debug)]
pub enum ContextReadError {
    /// 프로젝트를 열 수 없습니다.
    Open(OpenError),
    /// 문서를 읽을 수 없습니다.
    Document(DocumentReadError),
    /// 프로젝트 컴포넌트 원본을 읽을 수 없습니다.
    Components(crate::text_source::TextSourceOperationError),
    /// 위치를 해석할 수 없습니다(scene·정의 tree에 없는 node, 잘못된 인스턴스 chain).
    Site(ResolveError),
}

impl fmt::Display for ContextReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextReadError::Open(e) => write!(f, "{e}"),
            ContextReadError::Document(e) => write!(f, "{e}"),
            ContextReadError::Components(e) => write!(f, "컴포넌트 원본: {e}"),
            ContextReadError::Site(e) => write!(f, "위치: {e}"),
        }
    }
}

impl std::error::Error for ContextReadError {}

fn severity_name(severity: ContextSeverity) -> &'static str {
    match severity {
        ContextSeverity::Error => "error",
        ContextSeverity::Orphan => "orphan",
        ContextSeverity::Warning => "warning",
        ContextSeverity::Info => "info",
    }
}

/// `path` 프로젝트의 문서 하나를 읽어 [`read_snapshot_context`]를 부릅니다.
pub fn read_context(
    path: &Path,
    document: StableId,
    site: Option<&ContextSite>,
) -> Result<ContextReport, ContextReadError> {
    let snapshot = open_snapshot(path).map_err(ContextReadError::Open)?;
    read_snapshot_context(&snapshot, document, site)
}

/// 열린 snapshot의 문서 하나에서 Context를 조회합니다(모듈 문서). 결과는 입력의 함수입니다.
pub fn read_snapshot_context(
    snapshot: &ProjectSnapshot,
    document: StableId,
    site: Option<&ContextSite>,
) -> Result<ContextReport, ContextReadError> {
    let model = snapshot
        .document_model(document)
        .map_err(ContextReadError::Document)?;
    let (library, _) =
        crate::component_source::project_library(snapshot).map_err(ContextReadError::Components)?;
    let set = model.context().live(&model);
    let keys = ContextKeys::new(&set);
    fn members<'s>(
        entries: impl Iterator<Item = (StableId, &'s std::collections::BTreeSet<StableId>)>,
    ) -> Vec<(StableId, Vec<StableId>)> {
        entries
            .map(|(owner, keys)| (owner, keys.iter().copied().collect()))
            .collect()
    }
    let diagnostics = validate_context(&model, &library, &keys, &[], &NoInputs)
        .into_iter()
        .map(|diagnostic| ContextDiagnosticRow {
            code: diagnostic.kind.code(),
            severity: severity_name(diagnostic.severity()),
            message: diagnostic.kind.to_string(),
        })
        .collect();
    let site = match site {
        None => None,
        Some(site) => {
            let resolver = ContextResolver::with_context(&model, &library, &set);
            let evaluator = ContextEvaluator::new(&resolver, &keys, &NoInputs);
            let mut resolutions = Vec::new();
            let mut ordered: Vec<_> = keys.iter().collect();
            ordered.sort_by(|a, b| (a.name(), a.id()).cmp(&(b.name(), b.id())));
            for key in ordered {
                let resolution = resolver
                    .resolve(key.id(), site)
                    .map_err(ContextReadError::Site)?;
                let (value, error) = match evaluator.value_of(&resolution) {
                    Ok(value) => (value.as_ref().and_then(value_text), None),
                    Err(error) => (None, Some(error.to_string())),
                };
                resolutions.push(ContextResolutionRow {
                    key: key.id(),
                    name: key.name().to_owned(),
                    value,
                    error,
                    source: ContextSource::from(&resolution.origin),
                    path: resolution.path.clone(),
                    via_slots: resolution.via_slots.clone(),
                });
            }
            let candidates = binding_candidates(&resolver, &keys, site, None)
                .map_err(ContextReadError::Site)?
                .into_iter()
                .map(|candidate| ContextCandidateRow {
                    key: candidate.key,
                    name: candidate.name,
                    value_type: type_name(candidate.value_type),
                    needs_declaration: candidate.needs_declaration,
                    source: ContextSource::from(&candidate.origin),
                })
                .collect();
            Some(ContextSiteReport {
                site: site.clone(),
                resolutions,
                candidates,
            })
        }
    };
    Ok(ContextReport {
        revision: snapshot.revision(),
        document,
        keys: set
            .keys()
            .map(|key| ContextKeyRow {
                id: key.id(),
                name: key.name().to_owned(),
                scope: scope_text(key.scope()),
                value_type: type_name(key.value_type()),
                default: expression_text(key.default_expression().ast()),
            })
            .collect(),
        provisions: set
            .provisions()
            .map(|provision| ContextProvisionRow {
                id: provision.id(),
                owner: owner_text(provision.owner()),
                provider: provision.provider(),
                key: provision.key(),
                value: expression_text(provision.value()),
            })
            .collect(),
        requirements: members(set.all_requirements()),
        slot_keys: members(set.all_slot_keys()),
        diagnostics,
        site,
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

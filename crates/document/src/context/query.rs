//! 인스펙터용 Context 조회 API (XXX.7.11, ADR-0099 결정 11).
//!
//! - [`inspect_binding`]: 선택 속성의 binding이 Context를 읽으면 key마다 이름, 해석 값, 출처(provision 또는 default),
//!   소비 지점부터 provider까지의 node 경로, Slot 공개 목록 경유를 돌려줍니다. Context를 읽지 않으면 빈 목록입니다.
//! - [`binding_candidates`]: Value Binding 선택기 후보로 그 위치에서 소비할 수 있는 key와 선언 필요 여부를 돌려줍니다.
//!   순서는 (이름, StableId)로 고정입니다. 다른 문서 scope의 key는 후보가 아닙니다.
//!
//! 셸 연결(XXX.7.12)은 UI 재설계 때 합니다. 이 모듈은 셸 FFI를 만들지 않습니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;
use stellar_expressions::types::TypeSpec;

use super::key::ContextScope;
use super::resolve::{
    ContextEvalError, ContextEvaluator, ContextInputs, ContextOrigin, ContextResolver, ContextSite,
    ResolveError,
};
use super::set::ContextKeys;
use super::validate::ContextConsumer;
use crate::component::typed_expr::assignable;

/// binding 하나의 key 하나 조회 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextBindingInfo {
    /// key.
    pub key: StableId,
    /// key 표시 이름(key가 없으면 `None`).
    pub name: Option<String>,
    /// 해석 값(평가 실패면 `None`, 원인은 `error`).
    pub value: Option<Option<TypedValue>>,
    /// 평가 실패 원인.
    pub error: Option<ContextEvalError>,
    /// 출처.
    pub origin: ContextOrigin,
    /// 제공한 provision node까지의 경로(소비 지점 → provider). default면 최상위까지 지난 경로.
    pub path: Vec<ContextSite>,
    /// Slot 공개 목록 경유.
    pub via_slots: Vec<StableId>,
}

/// 선택 binding의 Context 조회(key StableId 순).
pub fn inspect_binding(
    resolver: &ContextResolver<'_>,
    keys: &ContextKeys<'_>,
    consumer: &ContextConsumer,
    inputs: &dyn ContextInputs,
) -> Result<Vec<ContextBindingInfo>, ResolveError> {
    let evaluator = ContextEvaluator::new(resolver, keys, inputs);
    let mut out = Vec::new();
    for key in consumer.reads() {
        let resolution = resolver.resolve(key, &consumer.site)?;
        let (value, error) = match evaluator.value_of(&resolution) {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        };
        out.push(ContextBindingInfo {
            key,
            name: keys.key(key).map(|k| k.name().to_owned()),
            value,
            error,
            origin: resolution.origin.clone(),
            path: resolution.path.clone(),
            via_slots: resolution.via_slots.clone(),
        });
    }
    Ok(out)
}

/// Value Binding 선택기 후보 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCandidate {
    /// key.
    pub key: StableId,
    /// 표시 이름.
    pub name: String,
    /// scope.
    pub scope: ContextScope,
    /// 값 타입.
    pub value_type: TypeSpec,
    /// 이 위치에서 쓰려면 소유 정의에 requirement를 더해야 하는지.
    pub needs_declaration: bool,
    /// 이 위치에서 해석되는 출처.
    pub origin: ContextOrigin,
}

/// `site`에서 소비할 수 있는 key 후보. `expected`가 있으면 그 타입 위치에 둘 수 있는 key만 고릅니다.
pub fn binding_candidates(
    resolver: &ContextResolver<'_>,
    keys: &ContextKeys<'_>,
    site: &ContextSite,
    expected: Option<TypeSpec>,
) -> Result<Vec<ContextCandidate>, ResolveError> {
    let document = resolver.model().document_id();
    let owner = resolver.library().owner_of(site.node).map(|c| c.id());
    let mut out = Vec::new();
    for key in keys.iter() {
        if matches!(key.scope(), ContextScope::Document(id) if id != document) {
            continue;
        }
        if expected.is_some_and(|expected| !assignable(key.value_type(), expected)) {
            continue;
        }
        let needs_declaration = owner.is_some_and(|component| {
            !resolver
                .context()
                .requirements(component)
                .contains(&key.id())
        });
        out.push(ContextCandidate {
            key: key.id(),
            name: key.name().to_owned(),
            scope: key.scope(),
            value_type: key.value_type(),
            needs_declaration,
            origin: resolver.resolve(key.id(), site)?.origin,
        });
    }
    out.sort_by(|a, b| (a.name.as_str(), a.key).cmp(&(b.name.as_str(), b.key)));
    out.dedup_by(|a, b| a.key == b.key);
    Ok(out)
}

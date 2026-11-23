//! 사용 프로젝트 instance의 외부 component 해석과 외부 참조 사용 수집 (XV.5.3·XV.7.3, ADR-0119 결정 4·5).
//!
//! 문서 원본은 아직 instance payload에 [`ExternalComponentRef`](선언 이름 + LibraryId)를 저장하지 않습니다(XV.5.2
//! 차이). instance payload에는 component StableId([`ComponentRef`])만 있습니다. 그래서 사용 프로젝트 instance가
//! 외부 component를 가리키는지는 다음 규칙 하나로 정합니다([`classify_component`]).
//!
//! 1. 사용 프로젝트 자신의 ComponentLibrary에 그 component가 있으면 **로컬**입니다.
//! 2. 아니면 사용 프로젝트가 직접 선언한 의존성(lock root scope)의 snapshot 가운데 그 StableId를 export한 것을
//!    찾습니다. 정확히 하나면 그 선언 이름·LibraryId로 만든 **외부 참조**입니다.
//! 3. 둘 이상이면 어느 의존성인지 정할 수 없으므로 **모호**입니다(같은 library의 두 버전을 다른 이름으로 선언한 경우
//!    등). command 사전 검사는 모호한 대상을 거부합니다(fail closed).
//! 4. 어디에도 없으면 **미해결**입니다(offline으로 snapshot이 없거나 사라진 component). 이 판정은 library 편집이
//!    아니므로 문서 command 자신의 검증에 맡깁니다.
//!
//! 문서가 외부 참조를 저장하게 되면 2·3단계는 저장된 참조로 바뀌고, 나머지 경로(사전 검사·후보 검증)는 그대로
//! 씁니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_dependencies::candidate_check::{ExternalInstanceUsage, OverrideUsage};
use stellar_dependencies::declaration::LogicalName;
use stellar_dependencies::external_ref::ExternalComponentRef;
use stellar_dependencies::ref_resolution::{LibraryGraphInstance, RefScope};
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::component::instance::ComponentInstancePayload;
use stellar_document::model::DocumentModel;
use stellar_document::records::NodePayload;

use crate::library_exports::value_type_signature;

/// instance component 해석 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentOrigin {
    /// 사용 프로젝트 자신의 component.
    Local,
    /// 직접 선언한 의존성 하나가 export한 component.
    External(ExternalComponentRef),
    /// 둘 이상의 직접 의존성이 같은 StableId를 export함(선언 이름 순).
    Ambiguous(Vec<LogicalName>),
    /// 로컬에도 로드된 의존성에도 없음.
    Unresolved,
}

/// `component`의 출처를 정합니다(모듈 문서 규칙).
pub fn classify_component(
    own: &ComponentLibrary,
    libraries: &LibraryGraphInstance,
    component: ComponentRef,
) -> ComponentOrigin {
    if own.get(component).is_some() {
        return ComponentOrigin::Local;
    }
    let mut found: Vec<ExternalComponentRef> = Vec::new();
    for (name, node) in libraries.scopes().children_of(&RefScope::Project) {
        let Some(snapshot) = libraries.snapshot(node) else {
            continue;
        };
        if snapshot.component(component).is_some() {
            found.push(ExternalComponentRef::new(
                name.clone(),
                node.library_id,
                component,
            ));
        }
    }
    match found.len() {
        0 => ComponentOrigin::Unresolved,
        1 => ComponentOrigin::External(found.remove(0)),
        _ => ComponentOrigin::Ambiguous(found.iter().map(|r| r.dependency().clone()).collect()),
    }
}

/// instance payload의 direct override를 사용 기록으로 바꿉니다. nested 경로 override는 외부 component의 직접
/// property가 아니므로 넣지 않습니다(IX.4.2 nested 직접 수정 금지).
pub fn override_usages(payload: &ComponentInstancePayload) -> Vec<OverrideUsage> {
    let mut out: Vec<OverrideUsage> = payload
        .property_overrides()
        .filter(|(key, _)| key.is_direct())
        .map(|(key, value)| OverrideUsage {
            property: key.property().stable_id(),
            value_type: value_type_signature(value),
        })
        .collect();
    out.sort();
    out
}

/// 문서의 외부 component instance 사용을 모읍니다(instance StableId 순).
///
/// 모호한 instance는 어느 후보에 대어 볼지 정할 수 없으므로 `ambiguous`로 따로 돌려줍니다.
pub fn external_instance_usages(
    model: &DocumentModel,
    own: &ComponentLibrary,
    libraries: &LibraryGraphInstance,
) -> ExternalUsages {
    let mut usages = ExternalUsages::default();
    let mut ids: Vec<StableId> = model
        .columns()
        .iter_live()
        .filter_map(|handle| model.stable_id(handle))
        .collect();
    ids.sort();
    for id in ids {
        let Some(NodePayload::ComponentInstance(payload)) = model.payload(id) else {
            continue;
        };
        match classify_component(own, libraries, payload.component()) {
            ComponentOrigin::External(component) => usages.instances.push(ExternalInstanceUsage {
                instance: id,
                component,
                overrides: override_usages(payload),
            }),
            ComponentOrigin::Ambiguous(names) => usages.ambiguous.push((id, names)),
            ComponentOrigin::Local | ComponentOrigin::Unresolved => {}
        }
    }
    usages
}

/// 문서의 외부 참조 사용.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalUsages {
    /// 외부 component instance(instance 순).
    pub instances: Vec<ExternalInstanceUsage>,
    /// 출처를 정할 수 없는 instance와 후보 선언 이름.
    pub ambiguous: Vec<(StableId, Vec<LogicalName>)>,
}

/// 후보 library를 적용하기 전에 사용 프로젝트의 외부 instance·override를 후보에 대어 봅니다(XV.7.3).
///
/// 사용 수집은 현재 lock 기준 graph instance(`current`)로 하고, 판정은 후보 snapshot으로 합니다. 모호한 instance는
/// 후보 판정에서 빠지므로 결과와 함께 돌려줍니다.
pub fn check_update_candidate(
    model: &DocumentModel,
    own: &ComponentLibrary,
    current: &LibraryGraphInstance,
    candidate: &stellar_dependencies::library_update::LibraryUpdateCandidate,
) -> (
    stellar_dependencies::candidate_check::CandidateCheck,
    Vec<(StableId, Vec<LogicalName>)>,
) {
    let usages = external_instance_usages(model, own, current);
    (
        candidate.check_usages(&usages.instances, &[]),
        usages.ambiguous,
    )
}

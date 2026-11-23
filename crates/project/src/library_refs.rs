//! 사용 프로젝트 instance의 외부 component 해석과 외부 참조 사용 수집 (XV.5.2·XV.5.3·XV.7.3, ADR-0119 결정 4·5,
//! ADR-0137 결정 1).
//!
//! 문서 instance payload에는 component StableId([`ComponentRef`])만 있습니다. instance가 어느 의존성의 component를
//! 가리키는지는 사용 프로젝트 `stellar.yaml`이 **저장한** 외부 참조(의존성 선언 `components`, [`ComponentImports`])가
//! 정합니다. [`classify_component`]는 다음 순서로 판정합니다.
//!
//! 1. 저장된 참조가 정확히 하나면 그 참조가 가리키는 **외부** component입니다(로컬 정의보다 앞섭니다). 둘 이상의 선언이
//!    같은 StableId를 저장했으면 **모호**입니다.
//! 2. 저장된 참조가 없고 사용 프로젝트 자신의 ComponentLibrary에 있으면 **로컬**입니다.
//! 3. 저장된 참조가 없고 로컬에도 없으면, 직접 선언한 의존성(lock root scope)의 로드된 snapshot 가운데 그 StableId를
//!    export한 것을 찾습니다. 정확히 하나면 **저장되지 않은 외부 참조**([`ComponentOrigin::Undeclared`])입니다. 이 경우는
//!    ADR-0137 이전 프로젝트의 호환 읽기이고, 진단(`library-ref/undeclared`)이 `stellar library import`로 참조를 저장하도록
//!    알립니다. 읽기 전용 사전 검사는 외부와 같이 판정합니다(fail closed). 둘 이상이면 **모호**입니다.
//! 4. 어디에도 없으면 **미해결**입니다(offline으로 snapshot이 없거나 사라진 component). 이 판정은 library 편집이
//!    아니므로 문서 command 자신의 검증에 맡깁니다.
//!
//! 저장된 참조는 snapshot이 없어도(offline·받지 않음) 외부입니다. 그래서 받지 않은 library component의 override 설정은
//! 사전 검사에서 `LibraryUnavailable`로 거부되고(reset은 허용), 문서 참조 진단은 그 참조를 사용할 수 없다고 냅니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::reference::ComponentRef;
use stellar_dependencies::candidate_check::{
    check_candidate_usages, CandidateDiagnostic, CandidateIssue, ExternalInstanceUsage,
    OverrideUsage,
};
use stellar_dependencies::component_imports::ComponentImports;
use stellar_dependencies::declaration::LogicalName;
use stellar_dependencies::external_ref::{ExternalComponentRef, ExternalResourceRef};
use stellar_dependencies::library_load::{
    LoadedLibraries, ResourceResolution, UnresolvedExternalRef,
};
use stellar_dependencies::ref_resolution::{LibraryGraphInstance, RefResolutionError, RefScope};
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
    /// `stellar.yaml`이 저장한 외부 참조.
    External(ExternalComponentRef),
    /// 저장된 참조는 없지만 직접 선언한 의존성 하나가 export한 component(ADR-0137 이전 프로젝트의 호환 읽기).
    Undeclared(ExternalComponentRef),
    /// 둘 이상의 선언이 같은 StableId를 저장했거나, 저장된 참조 없이 둘 이상의 직접 의존성이 export함(선언 이름 순).
    Ambiguous(Vec<LogicalName>),
    /// 로컬에도 로드된 의존성에도 없음.
    Unresolved,
}

impl ComponentOrigin {
    /// 외부 component 참조(저장됐든 아니든). 로컬·모호·미해결이면 `None`입니다.
    pub fn external(&self) -> Option<&ExternalComponentRef> {
        match self {
            ComponentOrigin::External(r) | ComponentOrigin::Undeclared(r) => Some(r),
            _ => None,
        }
    }
}

/// `component`의 출처를 정합니다(모듈 문서 규칙). 저장된 참조는 `libraries.imports()`입니다.
pub fn classify_component(
    own: &ComponentLibrary,
    libraries: &LibraryGraphInstance,
    component: ComponentRef,
) -> ComponentOrigin {
    classify_with(own, libraries.imports(), libraries, component)
}

/// [`classify_component`]에 저장된 참조 표를 따로 넘깁니다.
pub fn classify_with(
    own: &ComponentLibrary,
    imports: &ComponentImports,
    libraries: &LibraryGraphInstance,
    component: ComponentRef,
) -> ComponentOrigin {
    match imports.get(component) {
        [] => {}
        [stored] => return ComponentOrigin::External(stored.clone()),
        many => {
            return ComponentOrigin::Ambiguous(
                many.iter().map(|r| r.dependency().clone()).collect(),
            )
        }
    }
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
        1 => ComponentOrigin::Undeclared(found.remove(0)),
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
        let origin = classify_component(own, libraries, payload.component());
        if matches!(origin, ComponentOrigin::Undeclared(_)) {
            usages.undeclared.push(id);
        }
        match origin {
            ComponentOrigin::External(component) | ComponentOrigin::Undeclared(component) => {
                usages.instances.push(ExternalInstanceUsage {
                    instance: id,
                    component,
                    overrides: override_usages(payload),
                })
            }
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
    /// `instances` 가운데 `stellar.yaml`에 저장되지 않은 참조로 판정한 instance(instance 순, ADR-0137 결정 1).
    pub undeclared: Vec<StableId>,
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

/// 문서 instance 하나의 외부 참조 문제(문서 참조 단위 진단의 입력, ADR-0137 결정 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentRefIssue {
    /// 참조한 library를 사용할 수 없음(offline·손상). 원래 참조 전체와 위치를 보존합니다.
    Unavailable(UnresolvedExternalRef),
    /// 의존성이 `stellar.lock.yaml`에 없음(선언만 있고 아직 받지 않음).
    NotLocked {
        /// instance.
        instance: StableId,
        /// 참조.
        reference: ExternalComponentRef,
    },
    /// library는 로드했지만 참조를 해석하지 못함(사라진 component·다른 library·다른 종류).
    Unresolved {
        /// instance.
        instance: StableId,
        /// 참조.
        reference: ExternalComponentRef,
        /// 해석 오류.
        error: RefResolutionError,
    },
    /// 참조가 `stellar.yaml`에 저장되지 않음(호환 읽기로 판정).
    Undeclared {
        /// instance.
        instance: StableId,
        /// 호환 읽기로 정한 참조.
        reference: ExternalComponentRef,
    },
    /// 어느 의존성의 component인지 정할 수 없음.
    Ambiguous {
        /// instance.
        instance: StableId,
        /// component.
        component: ComponentRef,
        /// 후보 선언 이름.
        dependencies: Vec<LogicalName>,
    },
    /// 현재 lock의 library에서 instance override가 orphan임(property 사라짐·비공개·타입 변경).
    OrphanOverride(CandidateDiagnostic),
}

impl DocumentRefIssue {
    /// 문제가 있는 instance.
    pub fn instance(&self) -> StableId {
        match self {
            DocumentRefIssue::Unavailable(r) => r.location().root(),
            DocumentRefIssue::NotLocked { instance, .. }
            | DocumentRefIssue::Unresolved { instance, .. }
            | DocumentRefIssue::Undeclared { instance, .. }
            | DocumentRefIssue::Ambiguous { instance, .. } => *instance,
            DocumentRefIssue::OrphanOverride(d) => match &d.issue {
                CandidateIssue::ComponentRemoved { instance }
                | CandidateIssue::LibraryChanged { instance, .. }
                | CandidateIssue::OrphanOverride { instance, .. } => *instance,
                CandidateIssue::TokenRemoved { location }
                | CandidateIssue::TokenRetyped { location } => location.root(),
            },
        }
    }
}

/// 문서의 외부 component 참조를 lock 기준으로 로드한 library에 대어 문제를 모읍니다(instance 순, ADR-0137 결정 2).
///
/// 판정은 기존 함수가 합니다: 출처는 [`classify_component`], 해석·사용할 수 없음 구분은 [`LoadedLibraries::resolve`], override
/// orphan은 후보 검증과 같은 [`check_candidate_usages`](현재 snapshot에 대어 봄)입니다. `loaded`가 없으면(lock 없음) 모든 외부
/// 참조가 [`DocumentRefIssue::NotLocked`]입니다.
pub fn document_ref_issues(
    model: &DocumentModel,
    own: &ComponentLibrary,
    instance: &LibraryGraphInstance,
    loaded: Option<&LoadedLibraries>,
) -> Vec<DocumentRefIssue> {
    let usages = external_instance_usages(model, own, instance);
    let mut issues: Vec<DocumentRefIssue> = usages
        .ambiguous
        .iter()
        .filter_map(|(id, dependencies)| match model.payload(*id) {
            Some(NodePayload::ComponentInstance(payload)) => Some(DocumentRefIssue::Ambiguous {
                instance: *id,
                component: payload.component(),
                dependencies: dependencies.clone(),
            }),
            _ => None,
        })
        .collect();
    for usage in &usages.instances {
        if usages.undeclared.contains(&usage.instance) {
            issues.push(DocumentRefIssue::Undeclared {
                instance: usage.instance,
                reference: usage.component.clone(),
            });
        }
        let reference = ExternalResourceRef::Component(usage.component.clone());
        let Some(loaded) = loaded else {
            issues.push(DocumentRefIssue::NotLocked {
                instance: usage.instance,
                reference: usage.component.clone(),
            });
            continue;
        };
        match loaded.resolve(&RefScope::Project, &reference) {
            ResourceResolution::Resolved(_) => {
                let node = loaded
                    .instance()
                    .node_for(&RefScope::Project, &reference)
                    .ok()
                    .and_then(|node| loaded.instance().snapshot(node));
                if let Some(snapshot) = node {
                    let check = check_candidate_usages(
                        usage.component.dependency(),
                        snapshot,
                        std::slice::from_ref(usage),
                        &[],
                    );
                    issues.extend(
                        check
                            .diagnostics()
                            .iter()
                            .cloned()
                            .map(DocumentRefIssue::OrphanOverride),
                    );
                }
            }
            ResourceResolution::Unavailable(missing) => issues.push(DocumentRefIssue::Unavailable(
                missing.unresolved_at(PropertyPath::new(usage.instance)),
            )),
            ResourceResolution::Unresolved(RefResolutionError::UnknownDependency { .. }) => issues
                .push(DocumentRefIssue::NotLocked {
                    instance: usage.instance,
                    reference: usage.component.clone(),
                }),
            ResourceResolution::Unresolved(error) => issues.push(DocumentRefIssue::Unresolved {
                instance: usage.instance,
                reference: usage.component.clone(),
                error,
            }),
        }
    }
    issues.sort_by_key(DocumentRefIssue::instance);
    issues
}

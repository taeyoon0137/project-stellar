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

use stellar_core_types::reference::ComponentRef;
use stellar_dependencies::declaration::LogicalName;
use stellar_dependencies::external_ref::ExternalComponentRef;
use stellar_dependencies::ref_resolution::{LibraryGraphInstance, RefScope};
use stellar_document::component::definition::ComponentLibrary;


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

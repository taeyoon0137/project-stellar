//! 사용 프로젝트 component command의 library 읽기 전용 사전 검사 (XV.5.3, ADR-0119 결정 4).
//!
//! 사용 프로젝트는 library 원본을 고치지 않습니다(아키텍처 30.2). component 정의를 바꾸는 document command
//! ([`PropertyCommand`])와 instance override command([`PropertyOverrideCommand`])를 적용하기 **전에** 이 검사를
//! 통과시킵니다. 대상이 외부 component면([`classify_component`]) command를 의존성 crate의 형식 중립 요청
//! ([`LibraryEditRequest`])으로 바꿔 [`check_library_edits`] 하나로 판정합니다. 한 건이라도 거부되면 transaction
//! 전체를 거부합니다.
//!
//! | command | 외부 component일 때 요청 | 판정 |
//! |---|---|---|
//! | `PropertyCommand::Create`·`Delete`·`Restore` | 정의 편집 | 거부 |
//! | `PropertyCommand::Rename`·`SetDefault`·`Retype`·`RestoreRecord` | property 정의 편집 | 거부 |
//! | `PropertyOverrideCommand::Set` | override 설정(값 타입 = [`value_type_signature`]) | 공개·타입 일치만 허용 |
//! | `PropertyOverrideCommand::Reset`·`Restore` | override reset | 허용(instance 쪽 sparse 값만 바꿈) |
//!
//! `Restore`는 `Reset`의 inverse로, 지운 instance 값을 그대로 되돌릴 뿐 library를 바꾸지 않으므로 reset과 같이
//! 봅니다(update 뒤 orphan이 된 override의 reset도 정확히 undo).
//!
//! 외부 판정은 `stellar.yaml`이 저장한 참조가 먼저 정합니다([`classify_component`], ADR-0137 결정 1). 저장된 참조의
//! library snapshot이 없으면(offline) override 설정은 `LibraryUnavailable`로 거부됩니다.
//!
//! 로컬 component와 출처를 정할 수 없는 미해결 component는 library 편집이 아니므로 통과시키고, command 자신의
//! 검증이 판정합니다. 모호한 component(둘 이상의 직접 의존성이 export)는 거부합니다.

use std::fmt;

use stellar_core_types::reference::ComponentRef;
use stellar_dependencies::declaration::LogicalName;
use stellar_dependencies::external_ref::ExternalComponentRef;
use stellar_dependencies::read_only::{
    check_library_edits, AllowedLibraryEdit, LibraryEditRejection, LibraryEditRequest,
};
use stellar_dependencies::ref_resolution::{LibraryGraphInstance, RefScope};
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::component::expansion::{SceneIndex, ScenePayloads};
use stellar_document::component::override_commands::PropertyOverrideCommand;
use stellar_document::component::property_commands::PropertyCommand;
use stellar_document::node_columns::NodeColumns;

use crate::library_exports::value_type_signature;
use crate::library_refs::{classify_component, ComponentOrigin};

/// 검사할 component command 하나.
#[derive(Debug, Clone, Copy)]
pub enum ComponentEdit<'a> {
    /// component property 정의 command.
    Property(&'a PropertyCommand),
    /// instance override command.
    Override(&'a PropertyOverrideCommand),
}

/// 사전 검사 거부.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryGuardRejection {
    /// library 읽기 전용 정책 거부(의존성 crate 판정).
    Library(LibraryEditRejection),
    /// 둘 이상의 직접 의존성이 같은 component를 export해 대상을 정할 수 없음.
    AmbiguousComponent {
        /// component.
        component: ComponentRef,
        /// 후보 선언 이름.
        dependencies: Vec<LogicalName>,
    },
}

impl fmt::Display for LibraryGuardRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LibraryGuardRejection::Library(e) => write!(f, "{e}"),
            LibraryGuardRejection::AmbiguousComponent {
                component,
                dependencies,
            } => {
                let names: Vec<&str> = dependencies.iter().map(LogicalName::as_str).collect();
                write!(
                    f,
                    "component {}를 export한 의존성이 여럿입니다({})",
                    component.id(),
                    names.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for LibraryGuardRejection {}

/// 검사 결과: 통과한 외부 편집(로컬·미해결 대상 command는 담지 않음).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuardedEdits {
    /// 허용된 외부 override 편집(command 순).
    pub allowed: Vec<AllowedLibraryEdit>,
}

/// command 묶음을 적용 전에 검사합니다. 거부되면 (command 위치, 사유)입니다.
///
/// `own`은 사용 프로젝트 자신의 component library, `libraries`는 lock 기준으로 로드한 graph instance(XV.5.5),
/// `columns`·`payloads`는 override 대상 instance를 찾는 scene입니다.
pub fn check_component_edits(
    own: &ComponentLibrary,
    libraries: &LibraryGraphInstance,
    columns: &NodeColumns,
    payloads: &ScenePayloads,
    edits: &[ComponentEdit<'_>],
) -> Result<GuardedEdits, (usize, LibraryGuardRejection)> {
    let index = SceneIndex::build(columns, payloads);
    let mut requests = Vec::new();
    let mut positions = Vec::new();
    for (position, edit) in edits.iter().enumerate() {
        let component = match edit {
            ComponentEdit::Property(command) => command.component(),
            ComponentEdit::Override(command) => match index.instance_payload(command.instance()) {
                Ok(payload) => payload.component(),
                // instance가 아니거나 없으면 command 자신의 검증이 거부합니다.
                Err(_) => continue,
            },
        };
        let external = match classify_component(own, libraries, component) {
            // 저장되지 않은 외부 참조도 같은 정책으로 판정합니다(fail closed, ADR-0137 결정 1).
            ComponentOrigin::External(external) | ComponentOrigin::Undeclared(external) => external,
            ComponentOrigin::Ambiguous(dependencies) => {
                return Err((
                    position,
                    LibraryGuardRejection::AmbiguousComponent {
                        component,
                        dependencies,
                    },
                ))
            }
            ComponentOrigin::Local | ComponentOrigin::Unresolved => continue,
        };
        requests.push(request_for(edit, external));
        positions.push(position);
    }
    let allowed = check_library_edits(&requests, |name| {
        libraries
            .scopes()
            .child(&RefScope::Project, name)
            .and_then(|node| libraries.snapshot(node))
    })
    .map_err(|(at, rejection)| (positions[at], LibraryGuardRejection::Library(rejection)))?;
    Ok(GuardedEdits { allowed })
}

/// command를 형식 중립 요청으로 바꿉니다(모듈 문서 표).
fn request_for(edit: &ComponentEdit<'_>, component: ExternalComponentRef) -> LibraryEditRequest {
    match edit {
        ComponentEdit::Property(command) => match command {
            PropertyCommand::Create { .. }
            | PropertyCommand::Delete { .. }
            | PropertyCommand::Restore { .. } => {
                LibraryEditRequest::EditComponentDefinition { component }
            }
            PropertyCommand::Rename { .. }
            | PropertyCommand::SetDefault { .. }
            | PropertyCommand::Retype { .. }
            | PropertyCommand::RestoreRecord { .. } => LibraryEditRequest::EditComponentProperty {
                component,
                property: command.property().stable_id(),
            },
        },
        ComponentEdit::Override(command) => match command {
            PropertyOverrideCommand::Set {
                instance,
                property,
                value,
            } => LibraryEditRequest::SetInstanceOverride {
                instance: *instance,
                component,
                property: property.stable_id(),
                value_type: value_type_signature(value),
            },
            PropertyOverrideCommand::Reset { instance, property }
            | PropertyOverrideCommand::Restore {
                instance, property, ..
            } => LibraryEditRequest::ResetInstanceOverride {
                instance: *instance,
                component,
                property: property.stable_id(),
            },
        },
    }
}

//! 사용 프로젝트의 library resource 읽기 전용 정책 (XV.5.3).
//!
//! 사용 프로젝트는 library 원본을 수정하지 않습니다(아키텍처 30.2). 원본 수정은 library 저장소나
//! worktree에서 하고, 사용 프로젝트는 commit 후 의존성을 update합니다. 사용 프로젝트 command가 외부
//! resource에 대해 요청할 수 있는 변경은 **자기 instance의 공개 Property override**뿐입니다.
//!
//! | 요청 | 판정 |
//! |---|---|
//! | component 정의 편집(이름·tree·property/slot 선언·local token) | 거부 |
//! | component property 정의 편집(이름·타입·default·공개 여부) | 거부 |
//! | token 정의 편집(이름·타입·default expression) | 거부 |
//! | library resource 삭제 | 거부 |
//! | instance override 설정 | 공개 property이고 타입 표기가 같을 때만 허용 |
//! | instance override reset | 항상 허용(instance 쪽 sparse 값 삭제. property가 사라졌거나 library를 쓸 수 없어도 정리 가능) |
//!
//! 판정은 snapshot(XV.5.1)만 읽습니다. reset은 snapshot을 읽지 않습니다. document·commands crate는 crate graph
//! 정책상 이 crate에 의존할 수 없습니다. 그래서 외부 참조를 대상으로 하는 command의 사전 검사는 이 crate와
//! document·commands를 함께 쓰는 project crate(또는 project를 거쳐 command를 실행하는 runtime) 계층이 맡습니다. 그
//! 계층이 command를 이 분류로 바꿔 transaction 적용 전에 [`check_library_edits`]를 호출하고, 한 건이라도 거부되면
//! transaction 전체를 거부합니다. 연결은 `stellar_project::library_guard`입니다(ADR-0119 결정 4).

use std::fmt;

use stellar_core_types::ids::StableId;

use crate::declaration::{LibraryId, LogicalName};
use crate::external_ref::{ExternalComponentRef, ExternalResourceRef, ExternalTokenRef};
use crate::library_snapshot::{
    ExportVisibility, LibraryResourceKey, LibraryResourceKind, LibrarySnapshot,
};

/// 사용 프로젝트 command가 외부 library resource에 요청하는 변경(형식 중립 분류).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryEditRequest {
    /// component 정의 편집(이름·root tree·property/slot 선언·local token scope).
    EditComponentDefinition {
        /// 대상 component.
        component: ExternalComponentRef,
    },
    /// component property 정의 편집(이름·타입·default·공개 여부).
    EditComponentProperty {
        /// 대상 component.
        component: ExternalComponentRef,
        /// 대상 property StableId.
        property: StableId,
    },
    /// token 정의 편집(이름·타입·default expression).
    EditToken {
        /// 대상 token.
        token: ExternalTokenRef,
    },
    /// library resource 삭제.
    DeleteResource {
        /// 대상 resource.
        resource: ExternalResourceRef,
    },
    /// 사용 프로젝트 instance의 property override 설정.
    SetInstanceOverride {
        /// 사용 프로젝트 instance node StableId.
        instance: StableId,
        /// instance가 가리키는 외부 component.
        component: ExternalComponentRef,
        /// override할 property StableId.
        property: StableId,
        /// override 값 타입 표기(snapshot property 타입 표기와 같은 규칙).
        value_type: String,
    },
    /// 사용 프로젝트 instance의 property override reset(sparse 값 삭제).
    ResetInstanceOverride {
        /// 사용 프로젝트 instance node StableId.
        instance: StableId,
        /// instance가 가리키는 외부 component.
        component: ExternalComponentRef,
        /// reset할 property StableId.
        property: StableId,
    },
}

impl LibraryEditRequest {
    /// 요청이 가리키는 의존성 logical name.
    pub fn dependency(&self) -> &LogicalName {
        match self {
            LibraryEditRequest::EditComponentDefinition { component }
            | LibraryEditRequest::EditComponentProperty { component, .. }
            | LibraryEditRequest::SetInstanceOverride { component, .. }
            | LibraryEditRequest::ResetInstanceOverride { component, .. } => component.dependency(),
            LibraryEditRequest::EditToken { token } => token.dependency(),
            LibraryEditRequest::DeleteResource { resource } => resource.dependency(),
        }
    }

    fn library_id(&self) -> LibraryId {
        match self {
            LibraryEditRequest::EditComponentDefinition { component }
            | LibraryEditRequest::EditComponentProperty { component, .. }
            | LibraryEditRequest::SetInstanceOverride { component, .. }
            | LibraryEditRequest::ResetInstanceOverride { component, .. } => component.library_id(),
            LibraryEditRequest::EditToken { token } => token.library_id(),
            LibraryEditRequest::DeleteResource { resource } => resource.library_id(),
        }
    }
}

/// 허용된 변경. 모두 사용 프로젝트 instance의 sparse override입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowedLibraryEdit {
    /// 공개 property override 설정.
    SetOverride {
        /// 사용 프로젝트 instance.
        instance: StableId,
        /// 대상 component 전역 key.
        component: LibraryResourceKey,
        /// property StableId.
        property: StableId,
    },
    /// override reset. snapshot과 무관하게 허용하므로 전역 key가 아니라 instance가 가진 외부 참조를
    /// 그대로 담습니다(offline·library 불일치에서도 같은 값).
    ResetOverride {
        /// 사용 프로젝트 instance.
        instance: StableId,
        /// instance가 가리키는 외부 component 참조.
        component: ExternalComponentRef,
        /// property StableId.
        property: StableId,
    },
}

/// 거부 사유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryEditRejection {
    /// library 원본 resource는 사용 프로젝트에서 읽기 전용입니다.
    ReadOnlyResource {
        /// 대상 resource 전역 key.
        key: LibraryResourceKey,
        /// 대상 종류.
        kind: LibraryResourceKind,
    },
    /// 의존성 이름의 snapshot이 없음(선언 없음·offline·로드 실패). override 공개 여부를 확인할 수 없음.
    LibraryUnavailable {
        /// 의존성 이름.
        dependency: LogicalName,
    },
    /// 참조의 library와 snapshot의 library가 다름.
    LibraryMismatch {
        /// 참조가 기대한 library.
        expected: LibraryId,
        /// snapshot library.
        found: LibraryId,
    },
    /// component가 snapshot에 없음.
    UnknownComponent {
        /// component 전역 key.
        component: LibraryResourceKey,
    },
    /// property가 component에 없음.
    UnknownProperty {
        /// component 전역 key.
        component: LibraryResourceKey,
        /// property.
        property: StableId,
    },
    /// 비공개 property는 override할 수 없음.
    PrivateProperty {
        /// component 전역 key.
        component: LibraryResourceKey,
        /// property.
        property: StableId,
    },
    /// override 값 타입이 property 타입과 다름.
    OverrideTypeMismatch {
        /// component 전역 key.
        component: LibraryResourceKey,
        /// property.
        property: StableId,
    },
}

impl fmt::Display for LibraryEditRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LibraryEditRejection::ReadOnlyResource { key, kind } => write!(
                f,
                "library {} {key}는 읽기 전용입니다(library 저장소에서 수정한 뒤 의존성을 update하세요)",
                kind.as_str()
            ),
            LibraryEditRejection::LibraryUnavailable { dependency } => {
                write!(f, "의존성 {dependency}의 library를 사용할 수 없습니다")
            }
            LibraryEditRejection::LibraryMismatch { expected, found } => write!(
                f,
                "참조한 library {expected}와 로드된 library {found}가 다릅니다"
            ),
            LibraryEditRejection::UnknownComponent { component } => {
                write!(f, "library component {component}가 없습니다")
            }
            LibraryEditRejection::UnknownProperty {
                component,
                property,
            } => write!(f, "component {component}에 property {property}가 없습니다"),
            LibraryEditRejection::PrivateProperty {
                component,
                property,
            } => write!(
                f,
                "component {component}의 property {property}는 공개 property가 아니라 override할 수 없습니다"
            ),
            LibraryEditRejection::OverrideTypeMismatch {
                component,
                property,
            } => write!(
                f,
                "component {component}의 property {property} 타입과 override 값 타입이 다릅니다"
            ),
        }
    }
}

impl std::error::Error for LibraryEditRejection {}

/// 요청 하나를 `snapshot`(요청 의존성 이름이 가리키는 library)에 대해 판정합니다.
///
/// 정의·property·token 편집과 삭제는 snapshot 내용과 무관하게 항상 거부합니다. override 설정은
/// component·property가 있고 property가 공개이며 타입 표기가 같을 때만 허용합니다. reset은
/// instance 쪽 값만 지우므로 snapshot을 보지 않고 허용합니다. component·property가 사라졌거나 참조
/// library가 선언과 달라졌어도 정리할 수 있습니다(update 후 orphan 정리).
pub fn check_library_edit(
    snapshot: &LibrarySnapshot,
    request: &LibraryEditRequest,
) -> Result<AllowedLibraryEdit, LibraryEditRejection> {
    let same_library = || {
        let expected = request.library_id();
        if expected == snapshot.library_id() {
            Ok(())
        } else {
            Err(LibraryEditRejection::LibraryMismatch {
                expected,
                found: snapshot.library_id(),
            })
        }
    };
    let read_only = |resource: StableId,
                     kind: LibraryResourceKind|
     -> Result<AllowedLibraryEdit, LibraryEditRejection> {
        same_library()?;
        Err(LibraryEditRejection::ReadOnlyResource {
            key: snapshot.key_of(resource),
            kind,
        })
    };
    match request {
        LibraryEditRequest::ResetInstanceOverride {
            instance,
            component,
            property,
        } => Ok(reset_edit(*instance, component, *property)),
        LibraryEditRequest::EditComponentDefinition { component }
        | LibraryEditRequest::EditComponentProperty { component, .. } => {
            read_only(component.component().id(), LibraryResourceKind::Component)
        }
        LibraryEditRequest::EditToken { token } => {
            read_only(token.token().id(), LibraryResourceKind::Token)
        }
        LibraryEditRequest::DeleteResource { resource } => {
            read_only(resource.resource(), resource.kind())
        }
        LibraryEditRequest::SetInstanceOverride {
            instance,
            component,
            property,
            value_type,
        } => {
            same_library()?;
            let found = snapshot.component(component.component()).ok_or_else(|| {
                LibraryEditRejection::UnknownComponent {
                    component: snapshot.key_of(component.component().id()),
                }
            })?;
            let key = found.key();
            let declared = found.export().property(*property).ok_or_else(|| {
                LibraryEditRejection::UnknownProperty {
                    component: key.clone(),
                    property: *property,
                }
            })?;
            if declared.visibility != ExportVisibility::Public {
                return Err(LibraryEditRejection::PrivateProperty {
                    component: key,
                    property: *property,
                });
            }
            if !override_type_accepted(&declared.type_signature, value_type) {
                return Err(LibraryEditRejection::OverrideTypeMismatch {
                    component: key,
                    property: *property,
                });
            }
            Ok(AllowedLibraryEdit::SetOverride {
                instance: *instance,
                component: key,
                property: *property,
            })
        }
    }
}

/// override 값 타입 표기 `value_type`(값이 있는 override이므로 항상 비-nullable 표기 `T`)을 property 타입 표기
/// `declared`가 받는지 판정합니다.
///
/// 타입 표기는 expressions `TypeSpec`의 표시 형식(`T`, `Optional<T>`)입니다(ADR-0119 결정 3). `T` 값은 `T`와
/// `Optional<T>` property 모두에 둘 수 있습니다(`TypeSpec::accepts`와 같은 규칙). library update 전 검증(XV.7.3)도
/// 이 함수로 판정합니다.
pub fn override_type_accepted(declared: &str, value_type: &str) -> bool {
    if declared == value_type {
        return true;
    }
    declared
        .strip_prefix("Optional<")
        .and_then(|rest| rest.strip_suffix('>'))
        .is_some_and(|inner| inner == value_type)
}

/// reset 허용 결과(snapshot을 읽지 않음).
fn reset_edit(
    instance: StableId,
    component: &ExternalComponentRef,
    property: StableId,
) -> AllowedLibraryEdit {
    AllowedLibraryEdit::ResetOverride {
        instance,
        component: component.clone(),
        property,
    }
}

/// reset 요청이면 snapshot 없이 허용 결과를 만듭니다.
fn allowed_reset(request: &LibraryEditRequest) -> Option<AllowedLibraryEdit> {
    match request {
        LibraryEditRequest::ResetInstanceOverride {
            instance,
            component,
            property,
        } => Some(reset_edit(*instance, component, *property)),
        _ => None,
    }
}

/// transaction의 외부 resource 변경 요청을 모두 판정합니다. 하나라도 거부되면 그 위치와 사유로 실패합니다.
///
/// `snapshot_for`는 요청의 의존성 이름을 그 scope의 snapshot으로 바꿉니다(없으면 `None`). reset은
/// snapshot 없이도 허용합니다(instance 쪽 값만 지움). 그 밖의 요청은 snapshot이 없으면
/// `LibraryUnavailable`로 거부합니다. 정의 편집·삭제는 어차피 허용되지 않고, override 설정은 공개 여부를
/// 확인할 수 없기 때문입니다.
pub fn check_library_edits<'s, F>(
    requests: &[LibraryEditRequest],
    mut snapshot_for: F,
) -> Result<Vec<AllowedLibraryEdit>, (usize, LibraryEditRejection)>
where
    F: FnMut(&LogicalName) -> Option<&'s LibrarySnapshot>,
{
    let mut allowed = Vec::with_capacity(requests.len());
    for (index, request) in requests.iter().enumerate() {
        if let Some(reset) = allowed_reset(request) {
            allowed.push(reset);
            continue;
        }
        let Some(snapshot) = snapshot_for(request.dependency()) else {
            return Err((
                index,
                LibraryEditRejection::LibraryUnavailable {
                    dependency: request.dependency().clone(),
                },
            ));
        };
        allowed.push(check_library_edit(snapshot, request).map_err(|e| (index, e))?);
    }
    Ok(allowed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::fixtures::{commit, library};
    use crate::external_ref::fixtures::{component_ref, token_ref};
    use crate::graph::DependencyNodeKey;
    use crate::library_snapshot::fixtures::{component, property, rid, snapshot, token, token_set};
    use crate::library_snapshot::LibraryExports;

    const BUTTON: u16 = 1;
    const LABEL: u16 = 2;
    const SECRET: u16 = 3;
    const BRAND: u16 = 5;

    fn ds() -> LibrarySnapshot {
        let mut secret = property(SECRET, "internal", "Number", b"0");
        secret.visibility = ExportVisibility::Private;
        snapshot(
            DependencyNodeKey::new(library(1), commit(0x11)),
            LibraryExports {
                components: vec![component(
                    BUTTON,
                    "Button",
                    vec![property(LABEL, "label", "String", b"\"OK\""), secret],
                )],
                token_sets: vec![token_set(4, vec![token(BRAND, "brand", "Color", b"#f00")])],
            },
        )
    }

    fn button() -> ExternalComponentRef {
        component_ref("ds", library(1), rid(BUTTON))
    }

    fn set_override(property: u16, value_type: &str) -> LibraryEditRequest {
        LibraryEditRequest::SetInstanceOverride {
            instance: rid(900),
            component: button(),
            property: rid(property),
            value_type: value_type.to_string(),
        }
    }

    #[test]
    fn override_type_follows_type_spec_display() {
        assert!(override_type_accepted("String", "String"));
        assert!(override_type_accepted("Optional<Number>", "Number"));
        assert!(!override_type_accepted("Number", "Optional<Number>"));
        assert!(!override_type_accepted("Optional<Number>", "String"));
        assert!(!override_type_accepted("Optional<Number", "Number"));
    }

    #[test]
    fn editing_library_definitions_is_rejected() {
        let snap = ds();
        let component_key = snap.key_of(rid(BUTTON));
        let component_rejection = LibraryEditRejection::ReadOnlyResource {
            key: component_key.clone(),
            kind: LibraryResourceKind::Component,
        };
        let read_only_component = Err(component_rejection.clone());
        assert_eq!(
            check_library_edit(
                &snap,
                &LibraryEditRequest::EditComponentDefinition {
                    component: button()
                }
            ),
            read_only_component
        );
        assert_eq!(
            check_library_edit(
                &snap,
                &LibraryEditRequest::EditComponentProperty {
                    component: button(),
                    property: rid(LABEL),
                }
            ),
            read_only_component
        );
        let brand = token_ref("ds", library(1), rid(BRAND));
        assert_eq!(
            check_library_edit(
                &snap,
                &LibraryEditRequest::EditToken {
                    token: brand.clone()
                }
            ),
            Err(LibraryEditRejection::ReadOnlyResource {
                key: snap.key_of(rid(BRAND)),
                kind: LibraryResourceKind::Token,
            })
        );
        for resource in [
            ExternalResourceRef::from(button()),
            ExternalResourceRef::from(brand),
        ] {
            let kind = resource.kind();
            let id = resource.resource();
            assert_eq!(
                check_library_edit(&snap, &LibraryEditRequest::DeleteResource { resource }),
                Err(LibraryEditRejection::ReadOnlyResource {
                    key: snap.key_of(id),
                    kind,
                })
            );
        }
        // 존재하지 않는 resource여도 편집은 읽기 전용으로 거부(새 resource 생성 경로 없음).
        let ghost = component_ref("ds", library(1), rid(77));
        assert!(matches!(
            check_library_edit(
                &snap,
                &LibraryEditRequest::EditComponentDefinition { component: ghost }
            ),
            Err(LibraryEditRejection::ReadOnlyResource { .. })
        ));
        let text = component_rejection.to_string();
        assert!(text.contains("읽기 전용"));
    }

    #[test]
    fn public_override_is_allowed_and_others_are_rejected() {
        let snap = ds();
        let key = snap.key_of(rid(BUTTON));
        assert_eq!(
            check_library_edit(&snap, &set_override(LABEL, "String")),
            Ok(AllowedLibraryEdit::SetOverride {
                instance: rid(900),
                component: key.clone(),
                property: rid(LABEL),
            })
        );
        assert_eq!(
            check_library_edit(&snap, &set_override(SECRET, "Number")),
            Err(LibraryEditRejection::PrivateProperty {
                component: key.clone(),
                property: rid(SECRET),
            })
        );
        assert_eq!(
            check_library_edit(&snap, &set_override(LABEL, "Number")),
            Err(LibraryEditRejection::OverrideTypeMismatch {
                component: key.clone(),
                property: rid(LABEL),
            })
        );
        assert_eq!(
            check_library_edit(&snap, &set_override(99, "String")),
            Err(LibraryEditRejection::UnknownProperty {
                component: key,
                property: rid(99),
            })
        );
        let missing = LibraryEditRequest::SetInstanceOverride {
            instance: rid(900),
            component: component_ref("ds", library(1), rid(77)),
            property: rid(LABEL),
            value_type: "String".to_string(),
        };
        assert_eq!(
            check_library_edit(&snap, &missing),
            Err(LibraryEditRejection::UnknownComponent {
                component: snap.key_of(rid(77)),
            })
        );
        // 다른 library를 가리키는 참조.
        let foreign = LibraryEditRequest::SetInstanceOverride {
            instance: rid(900),
            component: component_ref("ds", library(2), rid(BUTTON)),
            property: rid(LABEL),
            value_type: "String".to_string(),
        };
        assert_eq!(
            check_library_edit(&snap, &foreign),
            Err(LibraryEditRejection::LibraryMismatch {
                expected: library(2),
                found: library(1),
            })
        );
        // reset은 사라진 property여도 허용(instance 쪽 정리).
        let reset = LibraryEditRequest::ResetInstanceOverride {
            instance: rid(900),
            component: button(),
            property: rid(99),
        };
        assert_eq!(
            check_library_edit(&snap, &reset),
            Ok(AllowedLibraryEdit::ResetOverride {
                instance: rid(900),
                component: button(),
                property: rid(99),
            })
        );
    }

    #[test]
    fn reset_is_allowed_without_snapshot_or_with_mismatched_library() {
        // reset은 instance 쪽 sparse 값만 지우므로 library를 쓸 수 없거나(offline) 선언이 다른 library로
        // 바뀐 뒤에도 사용자가 자기 override를 정리할 수 있어야 합니다(리뷰 P3).
        let snap = ds();
        let lookup = |name: &LogicalName| (name.as_str() == "ds").then_some(&snap);
        let icons = component_ref("icons", library(1), rid(BUTTON));
        let offline_reset = [LibraryEditRequest::ResetInstanceOverride {
            instance: rid(900),
            component: icons.clone(),
            property: rid(LABEL),
        }];
        assert_eq!(
            check_library_edits(&offline_reset, lookup),
            Ok(vec![AllowedLibraryEdit::ResetOverride {
                instance: rid(900),
                component: icons.clone(),
                property: rid(LABEL),
            }])
        );
        let foreign = component_ref("ds", library(2), rid(BUTTON));
        let mismatched_reset = LibraryEditRequest::ResetInstanceOverride {
            instance: rid(900),
            component: foreign.clone(),
            property: rid(LABEL),
        };
        assert_eq!(
            check_library_edit(&snap, &mismatched_reset),
            Ok(AllowedLibraryEdit::ResetOverride {
                instance: rid(900),
                component: foreign,
                property: rid(LABEL),
            })
        );
        // reset과 offline override가 섞이면 override 위치에서 거부(reset만 통과시키지 않음).
        let mixed = [
            offline_reset[0].clone(),
            LibraryEditRequest::SetInstanceOverride {
                instance: rid(900),
                component: icons,
                property: rid(LABEL),
                value_type: "String".to_string(),
            },
        ];
        assert_eq!(
            check_library_edits(&mixed, lookup),
            Err((
                1,
                LibraryEditRejection::LibraryUnavailable {
                    dependency: LogicalName::new("icons").unwrap(),
                }
            ))
        );
    }

    #[test]
    fn transaction_with_any_library_edit_is_rejected_as_a_whole() {
        let snap = ds();
        let lookup = |name: &LogicalName| (name.as_str() == "ds").then_some(&snap);
        let overrides = [set_override(LABEL, "String"), set_override(LABEL, "String")];
        assert_eq!(
            check_library_edits(&overrides, lookup).map(|v| v.len()),
            Ok(2)
        );

        let mixed = [
            set_override(LABEL, "String"),
            LibraryEditRequest::EditComponentDefinition {
                component: button(),
            },
        ];
        let (index, rejection) = check_library_edits(&mixed, lookup).unwrap_err();
        assert_eq!(index, 1);
        assert!(matches!(
            rejection,
            LibraryEditRejection::ReadOnlyResource { .. }
        ));

        // snapshot이 없는 의존성(offline 등): override도 확인할 수 없어 거부.
        let offline = [LibraryEditRequest::SetInstanceOverride {
            instance: rid(900),
            component: component_ref("icons", library(1), rid(BUTTON)),
            property: rid(LABEL),
            value_type: "String".to_string(),
        }];
        assert_eq!(
            check_library_edits(&offline, lookup),
            Err((
                0,
                LibraryEditRejection::LibraryUnavailable {
                    dependency: LogicalName::new("icons").unwrap(),
                }
            ))
        );
    }
}

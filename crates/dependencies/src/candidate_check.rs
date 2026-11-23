//! 후보 library 적용 전 사용 프로젝트 외부 참조·instance override 검증 (XV.7.3).
//!
//! library update(XV.7.1)는 lock을 바꾸기 전에 후보 snapshot([`CandidateLibrarySnapshot`])을 만듭니다. 이 모듈은
//! 사용 프로젝트가 그 의존성에 대해 가진 외부 참조와 instance override를 후보 snapshot에 대어 보고, 후보를 적용하면
//! 끊기는 참조를 **orphan 진단**으로 돌려줍니다. 진단은 문서를 바꾸지 않습니다. override 값은 지우지 않고 남겨 두고
//! (IX.4.6 "저장에서 삭제하지 않음"과 같은 정책), 사용자가 update 승인(XV.7.4) 전에 볼 수 있게 합니다.
//!
//! override 판정은 사용 프로젝트 command 사전 검사와 **같은 함수**([`check_library_edit`])입니다. 후보 snapshot에서
//! 그 override 설정 요청이 거부되는 사유가 곧 orphan 원인입니다(이중 판정 금지, ADR-0119 결정 5).
//!
//! | 사전 검사 거부 사유 | orphan 원인 |
//! |---|---|
//! | `UnknownComponent` | [`CandidateIssue::ComponentRemoved`] |
//! | `UnknownProperty` | [`OrphanCause::PropertyRemoved`] |
//! | `PrivateProperty` | [`OrphanCause::PropertyPrivate`] |
//! | `OverrideTypeMismatch` | [`OrphanCause::PropertyRetyped`] |
//! | `LibraryMismatch` | [`CandidateIssue::LibraryChanged`] |
//!
//! [`CandidateLibrarySnapshot`]: crate::library_update::CandidateLibrarySnapshot

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;

use crate::declaration::{LibraryId, LogicalName};
use crate::external_ref::{ExternalComponentRef, ExternalTokenRef};
use crate::library_snapshot::LibrarySnapshot;
use crate::read_only::{
    check_library_edit, override_type_accepted, LibraryEditRejection, LibraryEditRequest,
};

/// instance 하나의 property override 사용.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OverrideUsage {
    /// override한 property StableId.
    pub property: StableId,
    /// override 값 타입 표기(`TypeSpec` 표시 형식의 비-nullable `T`, ADR-0119 결정 3).
    pub value_type: String,
}

/// 사용 프로젝트 instance 하나가 외부 component를 쓰는 방식.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalInstanceUsage {
    /// 사용 프로젝트 instance node StableId.
    pub instance: StableId,
    /// instance가 가리키는 외부 component.
    pub component: ExternalComponentRef,
    /// instance의 property override(property 순).
    pub overrides: Vec<OverrideUsage>,
}

/// 사용 프로젝트의 외부 token 참조 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalTokenUsage {
    /// 참조가 있는 속성 위치.
    pub location: PropertyPath,
    /// 외부 token 참조.
    pub token: ExternalTokenRef,
    /// 참조 위치가 기대하는 값 타입 표기(모르면 `None`, 그때는 존재만 확인).
    pub expected_type: Option<String>,
}

/// orphan override 원인.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OrphanCause {
    /// 후보 component에서 property가 사라짐.
    PropertyRemoved,
    /// 후보에서 property가 공개가 아님.
    PropertyPrivate,
    /// 후보 property 타입이 override 값 타입을 받지 않음.
    PropertyRetyped,
}

impl OrphanCause {
    fn as_str(self) -> &'static str {
        match self {
            OrphanCause::PropertyRemoved => "property가 사라졌습니다",
            OrphanCause::PropertyPrivate => "property가 공개가 아닙니다",
            OrphanCause::PropertyRetyped => "property 타입이 바뀌었습니다",
        }
    }
}

/// 후보를 적용하면 끊기는 참조.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandidateIssue {
    /// instance가 가리키는 component가 후보에 없음(instance 전체가 orphan 참조).
    ComponentRemoved {
        /// 사용 프로젝트 instance.
        instance: StableId,
    },
    /// 참조의 library와 후보 library가 다름.
    LibraryChanged {
        /// 사용 프로젝트 instance.
        instance: StableId,
        /// 참조가 기대한 library.
        expected: LibraryId,
        /// 후보 library.
        found: LibraryId,
    },
    /// instance override가 후보에서 orphan이 됨(값은 저장에 남음).
    OrphanOverride {
        /// 사용 프로젝트 instance.
        instance: StableId,
        /// override한 property.
        property: StableId,
        /// 원인.
        cause: OrphanCause,
    },
    /// 외부 token 참조가 후보에 없음.
    TokenRemoved {
        /// 참조 위치.
        location: PropertyPath,
    },
    /// 외부 token 타입이 참조 위치의 기대 타입과 다름.
    TokenRetyped {
        /// 참조 위치.
        location: PropertyPath,
    },
}

/// orphan 진단 하나.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CandidateDiagnostic {
    /// 대상 의존성 선언 이름.
    pub dependency: LogicalName,
    /// 끊기는 참조.
    pub issue: CandidateIssue,
}

impl fmt::Display for CandidateDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let dependency = &self.dependency;
        match &self.issue {
            CandidateIssue::ComponentRemoved { instance } => write!(
                f,
                "{dependency} 후보에 instance {instance}의 component가 없습니다(orphan 참조)"
            ),
            CandidateIssue::LibraryChanged {
                instance,
                expected,
                found,
            } => write!(
                f,
                "{dependency} 후보 library {found}가 instance {instance}의 참조 library {expected}와 다릅니다"
            ),
            CandidateIssue::OrphanOverride {
                instance,
                property,
                cause,
            } => write!(
                f,
                "{dependency} 후보에서 instance {instance}의 property {property} override가 orphan이 됩니다: {}",
                cause.as_str()
            ),
            CandidateIssue::TokenRemoved { location } => {
                write!(
                    f,
                    "{dependency} 후보에 node {}의 속성이 참조한 token이 없습니다",
                    location.root()
                )
            }
            CandidateIssue::TokenRetyped { location } => write!(
                f,
                "{dependency} 후보의 token 타입이 node {}의 속성이 기대한 타입과 다릅니다",
                location.root()
            ),
        }
    }
}

/// 후보 검증 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CandidateCheck {
    diagnostics: Vec<CandidateDiagnostic>,
}

impl CandidateCheck {
    /// orphan 진단(정렬).
    pub fn diagnostics(&self) -> &[CandidateDiagnostic] {
        &self.diagnostics
    }

    /// 끊기는 참조가 없는지.
    pub fn is_clean(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

/// `dependency`의 후보 snapshot에 사용 프로젝트 외부 참조·override를 대어 봅니다.
///
/// `dependency`가 아닌 의존성을 가리키는 사용은 건너뜁니다. 같은 instance의 component가 후보에 없으면 그 instance의
/// override는 따로 진단하지 않습니다(instance 전체가 orphan 참조).
pub fn check_candidate_usages(
    dependency: &LogicalName,
    candidate: &LibrarySnapshot,
    instances: &[ExternalInstanceUsage],
    tokens: &[ExternalTokenUsage],
) -> CandidateCheck {
    let mut diagnostics = Vec::new();
    let mut push = |issue| {
        diagnostics.push(CandidateDiagnostic {
            dependency: dependency.clone(),
            issue,
        })
    };
    for usage in instances
        .iter()
        .filter(|u| u.component.dependency() == dependency)
    {
        if usage.component.library_id() != candidate.library_id() {
            push(CandidateIssue::LibraryChanged {
                instance: usage.instance,
                expected: usage.component.library_id(),
                found: candidate.library_id(),
            });
            continue;
        }
        if candidate.component(usage.component.component()).is_none() {
            push(CandidateIssue::ComponentRemoved {
                instance: usage.instance,
            });
            continue;
        }
        for item in &usage.overrides {
            let request = LibraryEditRequest::SetInstanceOverride {
                instance: usage.instance,
                component: usage.component.clone(),
                property: item.property,
                value_type: item.value_type.clone(),
            };
            let cause = match check_library_edit(candidate, &request) {
                Ok(_) => continue,
                Err(LibraryEditRejection::UnknownProperty { .. }) => OrphanCause::PropertyRemoved,
                Err(LibraryEditRejection::PrivateProperty { .. }) => OrphanCause::PropertyPrivate,
                Err(LibraryEditRejection::OverrideTypeMismatch { .. }) => {
                    OrphanCause::PropertyRetyped
                }
                // component·library는 위에서 확인했습니다. 남은 사유(정의 편집 등)는 override 요청에서 나오지 않지만,
                // 나오면 사라진 것으로 봅니다(fail closed).
                Err(_) => OrphanCause::PropertyRemoved,
            };
            push(CandidateIssue::OrphanOverride {
                instance: usage.instance,
                property: item.property,
                cause,
            });
        }
    }
    for usage in tokens.iter().filter(|u| u.token.dependency() == dependency) {
        let found = (usage.token.library_id() == candidate.library_id())
            .then(|| candidate.token(usage.token.token()))
            .flatten();
        match (found, &usage.expected_type) {
            (None, _) => push(CandidateIssue::TokenRemoved {
                location: usage.location.clone(),
            }),
            (Some(token), Some(expected))
                if !override_type_accepted(expected, &token.export().type_signature) =>
            {
                push(CandidateIssue::TokenRetyped {
                    location: usage.location.clone(),
                })
            }
            _ => {}
        }
    }
    diagnostics.sort();
    CandidateCheck { diagnostics }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_hash::ContentHashText;
    use crate::declaration::fixtures::{commit, library};
    use crate::external_ref::fixtures::{component_ref, token_ref};
    use crate::graph::DependencyNodeKey;
    use crate::library_snapshot::fixtures::{component, property, rid, token, token_set};
    use crate::library_snapshot::{ExportVisibility, LibraryExports, SnapshotLimits};
    use stellar_core_types::property::PropertyKey;

    const CARD: u16 = 1;
    const TITLE: u16 = 2;
    const TONE: u16 = 3;
    const SECRET: u16 = 4;
    const GONE: u16 = 5;
    const BADGE: u16 = 9;
    const INK: u16 = 11;

    fn snapshot(properties: Vec<crate::library_snapshot::ExportedProperty>) -> LibrarySnapshot {
        LibrarySnapshot::new(
            DependencyNodeKey::new(library(1), commit(2)),
            ContentHashText::from_sha256_digest([2; 32]),
            LibraryExports {
                components: vec![component(CARD, "Card", properties)],
                token_sets: vec![token_set(10, vec![token(INK, "ink", "Color", b"#000")])],
            },
            &SnapshotLimits::default(),
        )
        .unwrap()
    }

    fn usage(overrides: &[(u16, &str)]) -> ExternalInstanceUsage {
        ExternalInstanceUsage {
            instance: rid(100),
            component: component_ref("ds", library(1), rid(CARD)),
            overrides: overrides
                .iter()
                .map(|(p, ty)| OverrideUsage {
                    property: rid(*p),
                    value_type: (*ty).to_string(),
                })
                .collect(),
        }
    }

    fn ds() -> LogicalName {
        LogicalName::new("ds").unwrap()
    }

    #[test]
    fn removed_property_override_is_orphan_diagnostic() {
        let mut secret = property(SECRET, "secret", "String", b"x");
        secret.visibility = ExportVisibility::Private;
        let candidate = snapshot(vec![
            property(TITLE, "title", "String", b"t"),
            property(TONE, "tone", "Optional<Number>", b"1"),
            secret,
        ]);
        let used = usage(&[
            (TITLE, "String"),
            (TONE, "Number"),
            (SECRET, "String"),
            (GONE, "String"),
        ]);
        let check = check_candidate_usages(&ds(), &candidate, &[used], &[]);
        let issues: Vec<_> = check.diagnostics().iter().map(|d| &d.issue).collect();
        assert_eq!(
            issues,
            [
                &CandidateIssue::OrphanOverride {
                    instance: rid(100),
                    property: rid(SECRET),
                    cause: OrphanCause::PropertyPrivate,
                },
                &CandidateIssue::OrphanOverride {
                    instance: rid(100),
                    property: rid(GONE),
                    cause: OrphanCause::PropertyRemoved,
                },
            ]
        );
        assert!(check.diagnostics()[1].to_string().contains("사라졌습니다"));

        // 타입이 바뀐 property.
        let retyped = snapshot(vec![property(TITLE, "title", "Number", b"1")]);
        let check = check_candidate_usages(&ds(), &retyped, &[usage(&[(TITLE, "String")])], &[]);
        assert_eq!(
            check.diagnostics()[0].issue,
            CandidateIssue::OrphanOverride {
                instance: rid(100),
                property: rid(TITLE),
                cause: OrphanCause::PropertyRetyped,
            }
        );
    }

    #[test]
    fn removed_component_library_change_and_tokens() {
        let candidate = snapshot(vec![]);
        let mut gone = usage(&[(TITLE, "String")]);
        gone.component = component_ref("ds", library(1), rid(BADGE));
        let mut moved = usage(&[]);
        moved.instance = rid(101);
        moved.component = component_ref("ds", library(7), rid(CARD));
        let mut other = usage(&[(GONE, "String")]);
        other.component = component_ref("icons", library(3), rid(CARD));
        let location = PropertyPath::with_keys(rid(200), vec![PropertyKey(1)]);
        let tokens = [
            ExternalTokenUsage {
                location: location.clone(),
                token: token_ref("ds", library(1), rid(INK)),
                expected_type: Some("Color".into()),
            },
            ExternalTokenUsage {
                location: location.clone(),
                token: token_ref("ds", library(1), rid(INK)),
                expected_type: Some("Number".into()),
            },
            ExternalTokenUsage {
                location: location.clone(),
                token: token_ref("ds", library(1), rid(77)),
                expected_type: None,
            },
        ];
        let check = check_candidate_usages(&ds(), &candidate, &[gone, moved, other], &tokens);
        let issues: Vec<_> = check
            .diagnostics()
            .iter()
            .map(|d| d.issue.clone())
            .collect();
        assert_eq!(
            issues,
            [
                CandidateIssue::ComponentRemoved { instance: rid(100) },
                CandidateIssue::LibraryChanged {
                    instance: rid(101),
                    expected: library(7),
                    found: library(1),
                },
                CandidateIssue::TokenRemoved {
                    location: location.clone()
                },
                CandidateIssue::TokenRetyped { location },
            ]
        );
        // 다른 의존성(icons)의 사용은 이 후보와 무관.
        assert!(check.diagnostics().iter().all(|d| d.dependency == ds()));
        // 끊기는 참조가 없으면 깨끗함.
        let clean = check_candidate_usages(&ds(), &snapshot(vec![]), &[usage(&[])], &[]);
        assert!(clean.is_clean());
    }
}

//! 인스턴스 PropertyOverride 생성·변경·reset command (IX.2.4).
//!
//! scene 인스턴스의 PropertyOverride map을 바꾸는 closed enum command입니다. 각 command는 적용 전에
//! 전부 검증하고(실패 시 scene 불변), 적용하면 정확히 되돌리는 **inverse command**를 돌려줍니다.
//!
//! - [`PropertyOverrideCommand::Set`]: 공개 property override를 만들거나 값을 바꿉니다. IX.2.3 규칙
//!   (공개·선언·타입 일치)을 검사합니다. 대상은 **이 인스턴스 정의의 property id**뿐이며 nested
//!   instance 경로를 받는 인자가 없습니다 — nested property를 상위 인스턴스에서 직접 바꾸는 API는
//!   제공하지 않습니다(IX.4.2).
//! - [`PropertyOverrideCommand::Reset`]: map entry를 **삭제**해 정의 default로 돌아갑니다. 희소 저장이므로
//!   reset 후 payload bytes는 override를 만든 적 없는 인스턴스와 같습니다. 정의에서 사라진 property의
//!   orphan override도 지울 수 있습니다.
//! - [`PropertyOverrideCommand::Restore`]: `Reset`의 inverse 전용입니다. 규칙을 다시 검사하지 않고
//!   지웠던 값을 그대로 되살립니다(orphan override의 undo도 정확히 복원). entry가 없는 상태에서만
//!   적용됩니다.
//!
//! Phase VII command 계층이 생기면 이 enum을 그 closed enum의 한 갈래로 등록합니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;

use super::definition::{ComponentLibrary, ComponentPropertyId};
use super::expansion::{ExpansionError, SceneIndex, ScenePayloads};
use super::instance::{ComponentInstancePayload, OverrideKey};
use super::override_rules::{check_override, PropertyOverrideError};
use crate::node_columns::NodeColumns;

/// PropertyOverride command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyOverrideCommand {
    /// 공개 property override를 만들거나 바꿉니다.
    Set {
        /// scene 인스턴스.
        instance: StableId,
        /// 인스턴스 정의의 property.
        property: ComponentPropertyId,
        /// 새 값.
        value: TypedValue,
    },
    /// override entry를 삭제해 default로 돌아갑니다.
    Reset {
        /// scene 인스턴스.
        instance: StableId,
        /// property.
        property: ComponentPropertyId,
    },
    /// `Reset`의 inverse: 지운 값을 규칙 재검사 없이 되살립니다(entry가 없을 때만).
    Restore {
        /// scene 인스턴스.
        instance: StableId,
        /// property.
        property: ComponentPropertyId,
        /// 되살릴 값.
        value: TypedValue,
    },
}

/// PropertyOverride command 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverrideCommandError {
    /// override 규칙 위반(IX.2.3).
    Rule(PropertyOverrideError),
    /// reset할 override가 없습니다.
    NotOverridden {
        /// 인스턴스.
        instance: StableId,
        /// property.
        property: ComponentPropertyId,
    },
    /// restore 대상 entry가 이미 있습니다(inverse 적용 전제 위반).
    AlreadyOverridden {
        /// 인스턴스.
        instance: StableId,
        /// property.
        property: ComponentPropertyId,
    },
}

impl fmt::Display for OverrideCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OverrideCommandError::Rule(err) => write!(f, "{err}"),
            OverrideCommandError::NotOverridden { instance, property } => write!(
                f,
                "인스턴스 {instance}에 property {} override가 없습니다",
                property.stable_id()
            ),
            OverrideCommandError::AlreadyOverridden { instance, property } => write!(
                f,
                "인스턴스 {instance}에 property {} override가 이미 있습니다",
                property.stable_id()
            ),
        }
    }
}

impl std::error::Error for OverrideCommandError {}

impl From<PropertyOverrideError> for OverrideCommandError {
    fn from(err: PropertyOverrideError) -> OverrideCommandError {
        OverrideCommandError::Rule(err)
    }
}

impl From<ExpansionError> for OverrideCommandError {
    fn from(err: ExpansionError) -> OverrideCommandError {
        OverrideCommandError::Rule(PropertyOverrideError::Expansion(err))
    }
}

impl PropertyOverrideCommand {
    /// 대상 인스턴스.
    pub fn instance(&self) -> StableId {
        match self {
            PropertyOverrideCommand::Set { instance, .. }
            | PropertyOverrideCommand::Reset { instance, .. }
            | PropertyOverrideCommand::Restore { instance, .. } => *instance,
        }
    }

    /// 대상 property.
    pub fn property(&self) -> ComponentPropertyId {
        match self {
            PropertyOverrideCommand::Set { property, .. }
            | PropertyOverrideCommand::Reset { property, .. }
            | PropertyOverrideCommand::Restore { property, .. } => *property,
        }
    }

    /// 적용 가능 여부를 검증하고 현재 payload를 돌려줍니다. 상태를 바꾸지 않습니다.
    fn check(
        &self,
        library: &ComponentLibrary,
        columns: &NodeColumns,
        payloads: &ScenePayloads,
    ) -> Result<ComponentInstancePayload, OverrideCommandError> {
        let index = SceneIndex::build(columns, payloads);
        let instance = self.instance();
        let property = self.property();
        let payload = index.instance_payload(instance)?;
        let key = OverrideKey::direct(property);
        match self {
            PropertyOverrideCommand::Set { value, .. } => {
                let entry = library.get(payload.component()).ok_or(
                    PropertyOverrideError::UnresolvedComponent {
                        instance,
                        component: payload.component(),
                    },
                )?;
                check_override(entry, instance, &key, value)?;
            }
            PropertyOverrideCommand::Reset { .. } => {
                if payload.property_override(&key).is_none() {
                    return Err(OverrideCommandError::NotOverridden { instance, property });
                }
            }
            PropertyOverrideCommand::Restore { .. } => {
                if payload.property_override(&key).is_some() {
                    return Err(OverrideCommandError::AlreadyOverridden { instance, property });
                }
            }
        }
        Ok(payload)
    }

    /// 적용 가능 여부를 검증합니다. 상태를 바꾸지 않습니다.
    pub fn validate(
        &self,
        library: &ComponentLibrary,
        columns: &NodeColumns,
        payloads: &ScenePayloads,
    ) -> Result<(), OverrideCommandError> {
        self.check(library, columns, payloads).map(|_| ())
    }

    /// 검증 후 적용하고 inverse command를 반환합니다. 검증 실패 시 scene은 바뀌지 않습니다.
    pub fn apply(
        &self,
        library: &ComponentLibrary,
        columns: &NodeColumns,
        payloads: &mut ScenePayloads,
    ) -> Result<PropertyOverrideCommand, OverrideCommandError> {
        let mut payload = self.check(library, columns, payloads)?;
        let instance = self.instance();
        let property = self.property();
        let key = OverrideKey::direct(property);
        let inverse = match self {
            PropertyOverrideCommand::Set { value, .. }
            | PropertyOverrideCommand::Restore { value, .. } => {
                match payload.set_property_override(key, value.clone()) {
                    Some(previous) => PropertyOverrideCommand::Set {
                        instance,
                        property,
                        value: previous,
                    },
                    None => PropertyOverrideCommand::Reset { instance, property },
                }
            }
            PropertyOverrideCommand::Reset { .. } => match payload.remove_property_override(&key) {
                Some(previous) => PropertyOverrideCommand::Restore {
                    instance,
                    property,
                    value: previous,
                },
                None => return Err(OverrideCommandError::NotOverridden { instance, property }),
            },
        };
        payloads.insert(instance, payload.encode());
        Ok(inverse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::override_rules::fixtures::*;

    fn title(value: &str) -> PropertyOverrideCommand {
        PropertyOverrideCommand::Set {
            instance: stable(1001),
            property: pid(TITLE),
            value: TypedValue::String(value.into()),
        }
    }

    #[test]
    fn set_change_reset_apply_and_inverse_with_sparse_storage() {
        // 검증 기준(IX.2.4): apply/inverse와 sparse 저장 확인.
        let library = library();
        let empty = ComponentInstancePayload::new(card());
        let (columns, mut payloads) = scene(&empty);
        let original = payloads.clone();

        // 생성 → inverse는 Reset.
        let undo_create = title("주문")
            .apply(&library, &columns, &mut payloads)
            .unwrap();
        assert_eq!(
            undo_create,
            PropertyOverrideCommand::Reset {
                instance: stable(1001),
                property: pid(TITLE)
            }
        );
        let created = payloads.clone();
        // 변경 → inverse는 이전 값 Set.
        let undo_change = title("결제")
            .apply(&library, &columns, &mut payloads)
            .unwrap();
        assert_eq!(undo_change, title("주문"));
        undo_change
            .apply(&library, &columns, &mut payloads)
            .unwrap();
        assert_eq!(payloads, created, "변경 undo");

        // reset → entry 삭제, bytes가 override 없는 인스턴스와 동일(희소 저장).
        let reset = PropertyOverrideCommand::Reset {
            instance: stable(1001),
            property: pid(TITLE),
        };
        let undo_reset = reset.apply(&library, &columns, &mut payloads).unwrap();
        assert_eq!(payloads, original, "reset 후 sparse bytes");
        assert_eq!(
            payloads[&stable(1001)].len(),
            16 + 4 + 4,
            "component id + 빈 map 두 개"
        );
        // reset의 inverse(Restore)로 값 복원, 다시 reset 가능.
        let redo_reset = undo_reset.apply(&library, &columns, &mut payloads).unwrap();
        assert_eq!(payloads, created);
        assert_eq!(redo_reset, reset);

        // 생성 undo도 원래 bytes로.
        let (columns, mut payloads) = scene(&empty);
        let undo = title("x").apply(&library, &columns, &mut payloads).unwrap();
        undo.apply(&library, &columns, &mut payloads).unwrap();
        assert_eq!(payloads, original);
    }

    #[test]
    fn invalid_commands_leave_scene_unchanged() {
        let library = library();
        let (columns, mut payloads) = scene(&ComponentInstancePayload::new(card()));
        let before = payloads.clone();
        // 타입 불일치·없는 property.
        for command in [
            PropertyOverrideCommand::Set {
                instance: stable(1001),
                property: pid(TITLE),
                value: TypedValue::Boolean(true),
            },
            PropertyOverrideCommand::Set {
                instance: stable(1001),
                property: pid(LABEL),
                value: TypedValue::String("nested Button label".into()),
            },
        ] {
            assert!(matches!(
                command.apply(&library, &columns, &mut payloads),
                Err(OverrideCommandError::Rule(_))
            ));
        }
        // 없는 override reset.
        assert_eq!(
            PropertyOverrideCommand::Reset {
                instance: stable(1001),
                property: pid(TITLE),
            }
            .apply(&library, &columns, &mut payloads)
            .unwrap_err(),
            OverrideCommandError::NotOverridden {
                instance: stable(1001),
                property: pid(TITLE),
            }
        );
        // 인스턴스가 아닌 node.
        assert!(matches!(
            PropertyOverrideCommand::Reset {
                instance: stable(1000),
                property: pid(TITLE),
            }
            .validate(&library, &columns, &payloads),
            Err(OverrideCommandError::Rule(
                PropertyOverrideError::Expansion(ExpansionError::NotAnInstance { .. })
            ))
        ));
        assert_eq!(payloads, before);
    }

    #[test]
    fn orphan_override_reset_is_exactly_undoable() {
        // 정의에서 사라진 property(9_999)의 orphan override도 reset하고 undo로 정확히 복원합니다.
        let library = library();
        let mut payload = ComponentInstancePayload::new(card());
        payload.set_property_override(
            OverrideKey::direct(pid(9_999)),
            TypedValue::String("orphan".into()),
        );
        let (columns, mut payloads) = scene(&payload);
        let before = payloads.clone();
        let undo = PropertyOverrideCommand::Reset {
            instance: stable(1001),
            property: pid(9_999),
        }
        .apply(&library, &columns, &mut payloads)
        .unwrap();
        // 같은 값을 Set으로는 다시 만들 수 없지만(규칙 위반) Restore는 정확히 되돌립니다.
        assert!(PropertyOverrideCommand::Set {
            instance: stable(1001),
            property: pid(9_999),
            value: TypedValue::String("orphan".into()),
        }
        .validate(&library, &columns, &payloads)
        .is_err());
        undo.apply(&library, &columns, &mut payloads).unwrap();
        assert_eq!(payloads, before);
        // entry가 있으면 Restore 거부.
        assert!(matches!(
            undo.validate(&library, &columns, &payloads),
            Err(OverrideCommandError::AlreadyOverridden { .. })
        ));
    }
}

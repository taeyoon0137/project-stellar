//! Component Property 생성·rename·default 변경·삭제 command (IX.1.3).
//!
//! [`PropertyCommand`]는 컴포넌트 정의의 Property 선언·record를 바꾸는 closed enum command입니다. 각 command는
//! 적용 전에 전부 검증하고(실패 시 library·drilling 모음 불변), 적용하면 정확히 되돌리는 **inverse command**를
//! 돌려줍니다.
//!
//! - [`PropertyCommand::Create`]: Property를 선언 목록의 지정 위치(기본: 끝)에 선언하고 record를 추가합니다.
//!   이름은 컴포넌트 안에서 유일해야 합니다. inverse는 `Delete`.
//! - [`PropertyCommand::Rename`]: 이름만 바꿉니다(식별은 StableId). inverse는 이전 이름 `Rename`.
//! - [`PropertyCommand::SetDefault`]: default expression을 바꿉니다. 닫힌 표현식·선언 타입 대입·평가 가능 규칙
//!   (IX.1.2)을 다시 검사합니다. inverse는 이전 AST `SetDefault`.
//! - [`PropertyCommand::Delete`]: 선언과 record를 지웁니다. 이 property를 쓰는 drilling binding(IX.4.1 — owner
//!   source, 경계 chain)은 **지우지 않고** orphan 정보([`DrillingOrphan`])를 붙여 남기고, 그 목록을 결과
//!   ([`PropertyCommandOutcome::orphaned`])로 돌려줍니다. inverse는 `Restore`.
//! - [`PropertyCommand::Restore`]: `Delete`의 inverse 전용입니다. record를 원래 위치에 되살리고 그 삭제가 orphan으로
//!   만든 binding의 orphan 표시만 지웁니다.
//! - [`PropertyCommand::Retype`](IX.4.6): 선언 타입과 default를 함께 바꿉니다. 바뀐 타입 때문에 값 흐름이 타입 규칙을
//!   어기는 drilling binding은 `PropertyRetyped`로 orphan이 됩니다. inverse는 `RestoreRecord`(inverse 전용: 이전
//!   record로 되돌리고 그 변경이 만든 orphan 표시를 지움).
//!
//! **상위 chain 전파**(IX.4.6): 삭제·타입 변경으로 직접 끊긴 binding뿐 아니라, 그 binding으로 값을 넘기는 상위 chain
//! 전체를 `UpstreamOfBreak { origin }`으로 orphan 표시합니다([`DrillingSet::orphan_chain`]). 아래쪽(끊긴 지점보다
//! 안쪽) binding은 그대로 live입니다. 어느 binding도 저장에서 지우지 않습니다.
//!
//! 삭제 거부: 같은 컴포넌트 상태 머신 표현식(initial·조건·action 값)이 property를 참조하거나, 다른 컴포넌트 정의
//! tree 안 nested 인스턴스가 그 property를 정적 override하면 거부합니다 — 둘 다 library 저장 검증(IX.5·IX.2.3)이
//! 받아들이지 않는 상태가 되기 때문입니다. scene 인스턴스의 override는 정의에서 사라진 property의 orphan
//! override로 남고(IX.2.4 reset으로 정리), 이 command는 scene을 보지 않습니다.
//!
//! Phase VII command 계층(`stellar-commands`) 등록은 후속입니다(document 계층 closed enum + inverse 반환).

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::ComponentRef;
use stellar_expressions::ast::{ExpressionAst, ReferenceTarget};
use stellar_expressions::types::TypeSpec;

use super::definition::{ComponentEntry, ComponentLibrary, ComponentPropertyId};
use super::drilling::{
    BreakPoint, DrillingBindingId, DrillingError, DrillingOrphan, DrillingSet, OrphanCause,
};
use super::instance::{ComponentInstancePayload, OverrideKey};
use super::property::{ComponentProperty, ComponentPropertyError};
use super::state_machine::TransitionAction;
use crate::node_kind::NodeKind;

/// Property command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyCommand {
    /// Property를 선언하고 record를 추가합니다.
    Create {
        /// 컴포넌트.
        component: ComponentRef,
        /// 새 property record.
        property: ComponentProperty,
        /// 선언 목록 위치(`None`이면 끝).
        index: Option<usize>,
    },
    /// 이름을 바꿉니다.
    Rename {
        /// 컴포넌트.
        component: ComponentRef,
        /// property.
        property: ComponentPropertyId,
        /// 새 이름.
        name: String,
    },
    /// default expression을 바꿉니다.
    SetDefault {
        /// 컴포넌트.
        component: ComponentRef,
        /// property.
        property: ComponentPropertyId,
        /// 새 default.
        default: ExpressionAst,
    },
    /// 선언과 record를 지웁니다. 이 property를 쓰는 drilling binding은 orphan이 됩니다.
    Delete {
        /// 컴포넌트.
        component: ComponentRef,
        /// property.
        property: ComponentPropertyId,
    },
    /// 선언 타입과 default를 함께 바꿉니다(IX.4.6). 끊긴 drilling chain은 orphan이 됩니다.
    Retype {
        /// 컴포넌트.
        component: ComponentRef,
        /// property.
        property: ComponentPropertyId,
        /// 새 선언 타입.
        value_type: TypeSpec,
        /// 새 타입에 맞는 default.
        default: ExpressionAst,
    },
    /// `Retype`의 inverse 전용: 이전 record로 되돌리고 그 변경이 만든 orphan 표시를 지웁니다.
    RestoreRecord {
        /// 컴포넌트.
        component: ComponentRef,
        /// 이전 record.
        property: ComponentProperty,
        /// orphan 표시를 지울 binding.
        revive: Vec<DrillingBindingId>,
    },
    /// `Delete`의 inverse 전용: record를 원래 위치에 되살리고 그 삭제가 만든 orphan 표시를 지웁니다.
    Restore {
        /// 컴포넌트.
        component: ComponentRef,
        /// 되살릴 record.
        property: ComponentProperty,
        /// 선언 목록 위치.
        index: usize,
        /// orphan 표시를 지울 binding.
        revive: Vec<DrillingBindingId>,
    },
}

/// command 적용 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyCommandOutcome {
    /// 정확히 되돌리는 command.
    pub inverse: PropertyCommand,
    /// 이 command가 orphan으로 만든 binding(id 오름차순).
    pub orphaned: Vec<(DrillingBindingId, DrillingOrphan)>,
}

/// Property command 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyCommandError {
    /// 컴포넌트가 없습니다.
    ComponentNotFound {
        /// 컴포넌트.
        component: ComponentRef,
    },
    /// 컴포넌트에 선언된 property가 아닙니다.
    UnknownProperty {
        /// 컴포넌트.
        component: ComponentRef,
        /// property.
        property: ComponentPropertyId,
    },
    /// 같은 id의 property가 이미 있습니다.
    DuplicateProperty {
        /// 컴포넌트.
        component: ComponentRef,
        /// property.
        property: ComponentPropertyId,
    },
    /// 같은 컴포넌트 안에 같은 이름의 다른 property가 있습니다.
    DuplicateName {
        /// 컴포넌트.
        component: ComponentRef,
        /// 이름.
        name: String,
    },
    /// 선언 목록 위치가 범위를 벗어났습니다.
    IndexOutOfRange {
        /// 요청 위치.
        index: usize,
        /// 현재 선언 수.
        len: usize,
    },
    /// 이름·default 규칙 위반(IX.1.2).
    Property(ComponentPropertyError),
    /// 같은 컴포넌트 상태 머신 표현식이 property를 참조합니다.
    ReferencedByStateMachine {
        /// 컴포넌트.
        component: ComponentRef,
        /// property.
        property: ComponentPropertyId,
    },
    /// 다른 컴포넌트 정의 tree 안 nested 인스턴스가 property를 정적 override합니다.
    OverriddenInDefinition {
        /// 정의 tree 소유 컴포넌트.
        owner: ComponentRef,
        /// nested 인스턴스.
        instance: StableId,
        /// property.
        property: ComponentPropertyId,
    },
    /// restore할 binding이 이 삭제로 orphan이 된 상태가 아닙니다(inverse 적용 전제 위반).
    NotRevivable {
        /// binding.
        binding: DrillingBindingId,
    },
    /// drilling 모음 오류.
    Drilling(DrillingError),
}

impl fmt::Display for PropertyCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PropertyCommandError::ComponentNotFound { component } => {
                write!(f, "컴포넌트 {}가 없습니다", component.id())
            }
            PropertyCommandError::UnknownProperty {
                component,
                property,
            } => write!(
                f,
                "컴포넌트 {}에 property {}가 없습니다",
                component.id(),
                property.stable_id()
            ),
            PropertyCommandError::DuplicateProperty {
                component,
                property,
            } => write!(
                f,
                "컴포넌트 {}에 property {}가 이미 있습니다",
                component.id(),
                property.stable_id()
            ),
            PropertyCommandError::DuplicateName { component, name } => write!(
                f,
                "컴포넌트 {}에 같은 이름의 property가 있습니다: {name}",
                component.id()
            ),
            PropertyCommandError::IndexOutOfRange { index, len } => {
                write!(f, "property 위치 {index}가 선언 수 {len}을 넘습니다")
            }
            PropertyCommandError::Property(err) => write!(f, "{err}"),
            PropertyCommandError::ReferencedByStateMachine {
                component,
                property,
            } => write!(
                f,
                "컴포넌트 {}의 상태 머신이 property {}를 참조합니다",
                component.id(),
                property.stable_id()
            ),
            PropertyCommandError::OverriddenInDefinition {
                owner,
                instance,
                property,
            } => write!(
                f,
                "컴포넌트 {} 정의의 인스턴스 {instance}가 property {}를 override합니다",
                owner.id(),
                property.stable_id()
            ),
            PropertyCommandError::NotRevivable { binding } => write!(
                f,
                "drilling binding {binding}는 이 삭제로 orphan이 된 상태가 아닙니다"
            ),
            PropertyCommandError::Drilling(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for PropertyCommandError {}

impl From<ComponentPropertyError> for PropertyCommandError {
    fn from(err: ComponentPropertyError) -> PropertyCommandError {
        PropertyCommandError::Property(err)
    }
}

impl From<DrillingError> for PropertyCommandError {
    fn from(err: DrillingError) -> PropertyCommandError {
        PropertyCommandError::Drilling(err)
    }
}

/// 상태 머신 표현식(initial·조건·SetState 값)이 property를 참조하는지.
pub(super) fn state_machine_uses(entry: &ComponentEntry, property: ComponentPropertyId) -> bool {
    let target = ReferenceTarget::ComponentProperty(property.stable_id());
    let machine = entry.state_machine();
    machine
        .variables()
        .any(|v| v.initial().references().contains(&target))
        || machine.transitions().iter().any(|t| {
            t.condition()
                .is_some_and(|c| c.references().contains(&target))
                || t.actions().iter().any(|a| match a {
                    TransitionAction::SetState { value, .. } => {
                        value.references().contains(&target)
                    }
                    TransitionAction::Emit { .. } => false,
                })
        })
}

/// 다른 컴포넌트 정의 tree 안에서 `component`의 `property`를 정적 override하는 첫 nested 인스턴스.
pub(super) fn definition_override_of(
    library: &ComponentLibrary,
    component: ComponentRef,
    property: ComponentPropertyId,
) -> Option<(ComponentRef, StableId)> {
    let key = OverrideKey::direct(property);
    library.iter().find_map(|entry| {
        entry.tree().iter().find_map(|node| {
            if node.kind != NodeKind::ComponentInstance {
                return None;
            }
            let payload = ComponentInstancePayload::decode(&node.payload).ok()?;
            (payload.component() == component && payload.property_override(&key).is_some())
                .then(|| (entry.definition().reference(), node.id))
        })
    })
}

fn entry_of(
    library: &ComponentLibrary,
    component: ComponentRef,
) -> Result<&ComponentEntry, PropertyCommandError> {
    library
        .get(component)
        .ok_or(PropertyCommandError::ComponentNotFound { component })
}

fn record_of(
    entry: &ComponentEntry,
    property: ComponentPropertyId,
) -> Result<&ComponentProperty, PropertyCommandError> {
    entry
        .property(property)
        .filter(|_| entry.definition().properties().contains(&property))
        .ok_or(PropertyCommandError::UnknownProperty {
            component: entry.definition().reference(),
            property,
        })
}

fn check_name_free(
    entry: &ComponentEntry,
    name: &str,
    except: Option<ComponentPropertyId>,
) -> Result<(), PropertyCommandError> {
    match entry.property_named(name) {
        Some(other) if Some(other.id()) != except => Err(PropertyCommandError::DuplicateName {
            component: entry.definition().reference(),
            name: name.to_string(),
        }),
        _ => Ok(()),
    }
}

fn check_insertable(
    entry: &ComponentEntry,
    property: &ComponentProperty,
    index: usize,
) -> Result<(), PropertyCommandError> {
    let component = entry.definition().reference();
    if entry.definition().properties().contains(&property.id())
        || entry.property(property.id()).is_some()
    {
        return Err(PropertyCommandError::DuplicateProperty {
            component,
            property: property.id(),
        });
    }
    check_name_free(entry, property.name(), None)?;
    let len = entry.definition().properties().len();
    if index > len {
        return Err(PropertyCommandError::IndexOutOfRange { index, len });
    }
    Ok(())
}

impl ComponentLibrary {
    /// property를 `index` 위치에 선언하고 record를 추가합니다(검사 후 적용).
    pub(super) fn insert_property_record(
        &mut self,
        component: ComponentRef,
        property: ComponentProperty,
        index: usize,
    ) -> Result<(), PropertyCommandError> {
        check_insertable(entry_of(self, component)?, &property, index)?;
        let entry = self
            .entry_mut(component)
            .ok_or(PropertyCommandError::ComponentNotFound { component })?;
        entry
            .definition_mut()
            .insert_property_at(index, property.id())
            .map_err(|_| PropertyCommandError::DuplicateProperty {
                component,
                property: property.id(),
            })?;
        entry.properties_map_mut().insert(property.id(), property);
        Ok(())
    }

    /// property 선언과 record를 지우고 (record, 선언 위치)를 돌려줍니다.
    pub(super) fn remove_property_record(
        &mut self,
        component: ComponentRef,
        property: ComponentPropertyId,
    ) -> Result<(ComponentProperty, usize), PropertyCommandError> {
        let entry = self
            .entry_mut(component)
            .ok_or(PropertyCommandError::ComponentNotFound { component })?;
        let index = entry
            .definition()
            .properties()
            .iter()
            .position(|p| *p == property)
            .ok_or(PropertyCommandError::UnknownProperty {
                component,
                property,
            })?;
        let record = entry.properties_map_mut().remove(&property).ok_or(
            PropertyCommandError::UnknownProperty {
                component,
                property,
            },
        )?;
        entry
            .definition_mut()
            .remove_property(property)
            .map_err(|_| PropertyCommandError::UnknownProperty {
                component,
                property,
            })?;
        Ok((record, index))
    }

    /// 같은 id의 record를 교체하고 이전 record를 돌려줍니다(이름 유일성 검사).
    pub(super) fn replace_property_record(
        &mut self,
        component: ComponentRef,
        record: ComponentProperty,
    ) -> Result<ComponentProperty, PropertyCommandError> {
        let entry = entry_of(self, component)?;
        record_of(entry, record.id())?;
        check_name_free(entry, record.name(), Some(record.id()))?;
        let entry = self
            .entry_mut(component)
            .ok_or(PropertyCommandError::ComponentNotFound { component })?;
        let previous = entry
            .properties_map_mut()
            .insert(record.id(), record.clone())
            .ok_or(PropertyCommandError::UnknownProperty {
                component,
                property: record.id(),
            })?;
        Ok(previous)
    }
}

impl PropertyCommand {
    /// 대상 컴포넌트.
    pub fn component(&self) -> ComponentRef {
        match self {
            PropertyCommand::Create { component, .. }
            | PropertyCommand::Rename { component, .. }
            | PropertyCommand::SetDefault { component, .. }
            | PropertyCommand::Delete { component, .. }
            | PropertyCommand::Retype { component, .. }
            | PropertyCommand::RestoreRecord { component, .. }
            | PropertyCommand::Restore { component, .. } => *component,
        }
    }

    /// 대상 property.
    pub fn property(&self) -> ComponentPropertyId {
        match self {
            PropertyCommand::Create { property, .. }
            | PropertyCommand::RestoreRecord { property, .. }
            | PropertyCommand::Restore { property, .. } => property.id(),
            PropertyCommand::Rename { property, .. }
            | PropertyCommand::SetDefault { property, .. }
            | PropertyCommand::Retype { property, .. }
            | PropertyCommand::Delete { property, .. } => *property,
        }
    }

    /// 적용 가능 여부를 검증합니다. 상태를 바꾸지 않습니다.
    pub fn validate(
        &self,
        library: &ComponentLibrary,
        drilling: &DrillingSet,
    ) -> Result<(), PropertyCommandError> {
        let component = self.component();
        let entry = entry_of(library, component)?;
        match self {
            PropertyCommand::Create {
                property, index, ..
            } => {
                let at = index.unwrap_or(entry.definition().properties().len());
                check_insertable(entry, property, at)
            }
            PropertyCommand::Rename { property, name, .. } => {
                let record = record_of(entry, *property)?;
                record.renamed(name.clone())?;
                check_name_free(entry, name, Some(*property))
            }
            PropertyCommand::SetDefault {
                property, default, ..
            } => {
                record_of(entry, *property)?.with_default(default.clone())?;
                Ok(())
            }
            PropertyCommand::Delete { property, .. } => {
                record_of(entry, *property)?;
                check_deletable(library, entry, *property)
            }
            PropertyCommand::Retype {
                property,
                value_type,
                default,
                ..
            } => {
                retyped(record_of(entry, *property)?, *value_type, default.clone())?;
                check_deletable(library, entry, *property)
            }
            PropertyCommand::RestoreRecord {
                property, revive, ..
            } => {
                record_of(entry, property.id())?;
                check_name_free(entry, property.name(), Some(property.id()))?;
                check_revivable(drilling, revive, |cause| {
                    matches!(cause, OrphanCause::PropertyRetyped { component: c, property: p, .. }
                        if *c == component && *p == property.id())
                })
            }
            PropertyCommand::Restore {
                property,
                index,
                revive,
                ..
            } => {
                check_insertable(entry, property, *index)?;
                check_revivable(drilling, revive, |cause| {
                    *cause
                        == OrphanCause::PropertyDeleted {
                            component,
                            property: property.id(),
                        }
                })
            }
        }
    }

    /// 검증 후 적용하고 inverse와 orphan 목록을 반환합니다. 검증 실패 시 상태는 바뀌지 않습니다.
    pub fn apply(
        &self,
        library: &mut ComponentLibrary,
        drilling: &mut DrillingSet,
    ) -> Result<PropertyCommandOutcome, PropertyCommandError> {
        self.validate(library, drilling)?;
        let component = self.component();
        let mut orphaned = Vec::new();
        let inverse = match self {
            PropertyCommand::Create {
                property, index, ..
            } => {
                let at = match index {
                    Some(at) => *at,
                    None => entry_of(library, component)?
                        .definition()
                        .properties()
                        .len(),
                };
                library.insert_property_record(component, property.clone(), at)?;
                PropertyCommand::Delete {
                    component,
                    property: property.id(),
                }
            }
            PropertyCommand::Rename { property, name, .. } => {
                let renamed =
                    record_of(entry_of(library, component)?, *property)?.renamed(name.clone())?;
                let previous = library.replace_property_record(component, renamed)?;
                PropertyCommand::Rename {
                    component,
                    property: *property,
                    name: previous.name().to_string(),
                }
            }
            PropertyCommand::SetDefault {
                property, default, ..
            } => {
                let updated = record_of(entry_of(library, component)?, *property)?
                    .with_default(default.clone())?;
                let previous = library.replace_property_record(component, updated)?;
                PropertyCommand::SetDefault {
                    component,
                    property: *property,
                    default: previous.default_expression().ast().clone(),
                }
            }
            PropertyCommand::Delete { property, .. } => {
                let fallback = record_of(entry_of(library, component)?, *property)?.value_type();
                let direct = typed_uses(library, drilling, component, *property, fallback);
                let (record, index) = library.remove_property_record(component, *property)?;
                orphaned = drilling.orphan_chain(
                    library,
                    direct,
                    OrphanCause::PropertyDeleted {
                        component,
                        property: *property,
                    },
                    None,
                )?;
                PropertyCommand::Restore {
                    component,
                    property: record,
                    index,
                    revive: orphaned.iter().map(|(id, _)| *id).collect(),
                }
            }
            PropertyCommand::Retype {
                property,
                value_type,
                default,
                ..
            } => {
                let current = record_of(entry_of(library, component)?, *property)?;
                let previous_type = current.value_type();
                let updated = retyped(current, *value_type, default.clone())?;
                let uses = typed_uses(library, drilling, component, *property, previous_type);
                let previous = library.replace_property_record(component, updated)?;
                // 바뀐 타입으로 다시 검증해 실패하는 사용처만 직접 끊긴 binding입니다.
                let direct: Vec<_> = uses
                    .into_iter()
                    .filter(|(id, _, _)| {
                        drilling
                            .get(*id)
                            .is_some_and(|binding| binding.validate(library, drilling).is_err())
                    })
                    .collect();
                orphaned = drilling.orphan_chain(
                    library,
                    direct,
                    OrphanCause::PropertyRetyped {
                        component,
                        property: *property,
                        previous: previous_type,
                        current: *value_type,
                    },
                    Some((component, *property, previous_type)),
                )?;
                PropertyCommand::RestoreRecord {
                    component,
                    property: previous,
                    revive: orphaned.iter().map(|(id, _)| *id).collect(),
                }
            }
            PropertyCommand::RestoreRecord {
                property, revive, ..
            } => {
                let current = library.replace_property_record(component, property.clone())?;
                for binding in revive {
                    drilling.clear_orphan(*binding);
                }
                PropertyCommand::Retype {
                    component,
                    property: property.id(),
                    value_type: current.value_type(),
                    default: current.default_expression().ast().clone(),
                }
            }
            PropertyCommand::Restore {
                property,
                index,
                revive,
                ..
            } => {
                library.insert_property_record(component, property.clone(), *index)?;
                for binding in revive {
                    drilling.clear_orphan(*binding);
                }
                PropertyCommand::Delete {
                    component,
                    property: property.id(),
                }
            }
        };
        Ok(PropertyCommandOutcome { inverse, orphaned })
    }
}

/// `property`를 직접 쓰는 live binding과 위치, 끊기기 전 target 타입.
fn typed_uses(
    library: &ComponentLibrary,
    drilling: &DrillingSet,
    component: ComponentRef,
    property: ComponentPropertyId,
    fallback: TypeSpec,
) -> Vec<(DrillingBindingId, BreakPoint, TypeSpec)> {
    drilling
        .uses_of(library, component, property)
        .into_iter()
        .map(|(binding, at)| {
            let last_valid_type = drilling.target_type(library, binding).unwrap_or(fallback);
            (binding, at, last_valid_type)
        })
        .collect()
}

/// 같은 id·이름·공개 여부로 타입과 default를 바꾼 record.
fn retyped(
    current: &ComponentProperty,
    value_type: TypeSpec,
    default: ExpressionAst,
) -> Result<ComponentProperty, PropertyCommandError> {
    Ok(ComponentProperty::new(
        current.id(),
        current.name().to_string(),
        value_type,
        default,
        current.visibility(),
    )?)
}

/// `revive`의 각 binding이 orphan이고, 원인이 `expected`이거나 `revive` 안 binding에서 전파된 것인지.
fn check_revivable(
    drilling: &DrillingSet,
    revive: &[DrillingBindingId],
    expected: impl Fn(&OrphanCause) -> bool,
) -> Result<(), PropertyCommandError> {
    for binding in revive {
        let revivable = drilling
            .orphan(*binding)
            .is_some_and(|orphan| match &orphan.cause {
                OrphanCause::UpstreamOfBreak { origin } => revive.contains(origin),
                cause => expected(cause),
            });
        if !revivable {
            return Err(PropertyCommandError::NotRevivable { binding: *binding });
        }
    }
    Ok(())
}

fn check_deletable(
    library: &ComponentLibrary,
    entry: &ComponentEntry,
    property: ComponentPropertyId,
) -> Result<(), PropertyCommandError> {
    let component = entry.definition().reference();
    if state_machine_uses(entry, property) {
        return Err(PropertyCommandError::ReferencedByStateMachine {
            component,
            property,
        });
    }
    if let Some((owner, instance)) = definition_override_of(library, component, property) {
        return Err(PropertyCommandError::OverriddenInDefinition {
            owner,
            instance,
            property,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::drilling::fixtures::*;
    use crate::component::drilling::{BreakPoint, DrillingBinding};
    use crate::component::property::PropertyVisibility;
    use crate::component::state_machine::fixtures as sm;
    use crate::component::typed_expr::test_util::literal;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_expressions::types::{ExpressionType, TypeSpec};

    /// 2단계 chain: C1.title(41_001) → I1.C2.label, C0.title(41_000) → I0.C1.title.
    fn drilled() -> (ComponentLibrary, DrillingSet) {
        let mut library = nested_library(2);
        declare(&mut library, 1, string_property(41_001, "title"));
        declare(&mut library, 0, string_property(41_000, "title"));
        let mut set = DrillingSet::new();
        set.insert(DrillingBinding::link(
            bid(1),
            component(1),
            pid(41_001),
            instance(1),
            pid(LABEL),
        ))
        .unwrap();
        set.insert(DrillingBinding::link(
            bid(2),
            component(0),
            pid(41_000),
            instance(0),
            pid(41_001),
        ))
        .unwrap();
        set.validate(&library).unwrap();
        (library, set)
    }

    fn snapshot(library: &ComponentLibrary, set: &DrillingSet) -> (Vec<u8>, Vec<u8>) {
        (library.encode(), set.encode())
    }

    #[test]
    fn create_rename_default_delete_apply_and_inverse() {
        // 검증 기준(IX.1.3): apply/inverse 통과.
        let (mut library, mut set) = drilled();
        let original = snapshot(&library, &set);
        let commands = [
            PropertyCommand::Create {
                component: component(2),
                property: string_property(42_000, "caption"),
                index: Some(0),
            },
            PropertyCommand::Rename {
                component: component(2),
                property: pid(42_000),
                name: "subtitle".into(),
            },
            PropertyCommand::SetDefault {
                component: component(2),
                property: pid(42_000),
                default: literal(TypedValue::String("부제".into())),
            },
            PropertyCommand::Delete {
                component: component(2),
                property: pid(42_000),
            },
        ];
        let mut states = vec![original.clone()];
        let mut inverses = Vec::new();
        for command in &commands {
            let outcome = command.apply(&mut library, &mut set).unwrap();
            assert!(outcome.orphaned.is_empty(), "inbound drilling 없음");
            inverses.push(outcome.inverse);
            states.push(snapshot(&library, &set));
            // 매 단계 library는 저장 검증을 통과합니다.
            ComponentLibrary::decode(&library.encode()).unwrap();
        }
        // 생성 위치·이름·default 반영 확인은 중간 상태로.
        let mut probe = library.clone();
        let mut probe_set = set.clone();
        inverses[3].apply(&mut probe, &mut probe_set).unwrap();
        let entry = probe.get(component(2)).unwrap();
        assert_eq!(entry.definition().properties()[0], pid(42_000));
        let record = entry.property(pid(42_000)).unwrap();
        assert_eq!(record.name(), "subtitle");
        assert_eq!(
            record.default_value(),
            Some(&TypedValue::String("부제".into()))
        );
        // inverse 역순 적용 → 각 단계 bytes 정확 복원.
        for (step, inverse) in inverses.iter().enumerate().rev() {
            let redo = inverse.apply(&mut library, &mut set).unwrap().inverse;
            assert_eq!(snapshot(&library, &set), states[step], "undo {step}");
            // redo는 원래 command와 같은 효과.
            let mut l = library.clone();
            let mut d = set.clone();
            redo.apply(&mut l, &mut d).unwrap();
            assert_eq!(snapshot(&l, &d), states[step + 1], "redo {step}");
        }
        assert_eq!(snapshot(&library, &set), original);
    }

    #[test]
    fn deleting_a_drilled_property_creates_orphan_information() {
        // 검증 기준(IX.1.3): inbound drilling이 있는 삭제는 orphan 정보 생성.
        let (mut library, mut set) = drilled();
        let original = snapshot(&library, &set);
        // 가장 안쪽 C2.label 삭제: C1 link(bid 1)가 경계 0에서 끊깁니다.
        let outcome = PropertyCommand::Delete {
            component: component(2),
            property: pid(LABEL),
        }
        .apply(&mut library, &mut set)
        .unwrap();
        let expected = DrillingOrphan {
            cause: OrphanCause::PropertyDeleted {
                component: component(2),
                property: pid(LABEL),
            },
            break_point: BreakPoint::Boundary(0),
            last_valid_type: TypeSpec::required(ExpressionType::String),
        };
        // 상위 chain(C0 link, bid 2)도 전파로 orphan이 됩니다(IX.4.6).
        let upstream = DrillingOrphan {
            cause: OrphanCause::UpstreamOfBreak { origin: bid(1) },
            break_point: BreakPoint::Boundary(0),
            last_valid_type: TypeSpec::required(ExpressionType::String),
        };
        assert_eq!(
            outcome.orphaned,
            vec![(bid(1), expected.clone()), (bid(2), upstream)]
        );
        // binding은 지워지지 않고 원래 ID·path·chain을 보존합니다.
        assert_eq!(set.len(), 2);
        assert_eq!(set.orphan(bid(1)), Some(&expected));
        assert_eq!(set.get(bid(1)).unwrap().target(), pid(LABEL));
        // orphan은 검증에서 제외되고 저장·복원됩니다.
        set.validate(&library).unwrap();
        let decoded = DrillingSet::decode(&set.encode()).unwrap();
        assert_eq!(decoded, set);
        // undo: record 원위치 + orphan 해제 → bytes 동일.
        outcome.inverse.apply(&mut library, &mut set).unwrap();
        assert_eq!(snapshot(&library, &set), original);

        // 중간 C1.title 삭제: C1 link는 source, C0 link는 경계 0에서 끊깁니다.
        let outcome = PropertyCommand::Delete {
            component: component(1),
            property: pid(41_001),
        }
        .apply(&mut library, &mut set)
        .unwrap();
        let points: Vec<_> = outcome
            .orphaned
            .iter()
            .map(|(id, orphan)| (*id, orphan.break_point))
            .collect();
        assert_eq!(
            points,
            vec![
                (bid(1), BreakPoint::Source),
                (bid(2), BreakPoint::Boundary(0))
            ]
        );
        assert!(outcome
            .orphaned
            .iter()
            .all(|(_, o)| o.last_valid_type == TypeSpec::required(ExpressionType::String)));
        // 다른 삭제가 이미 orphan으로 만든 binding은 덮어쓰지 않습니다.
        let second = PropertyCommand::Delete {
            component: component(2),
            property: pid(LABEL),
        }
        .apply(&mut library, &mut set)
        .unwrap();
        assert!(second.orphaned.is_empty());
        second.inverse.apply(&mut library, &mut set).unwrap();
        assert!(set.is_orphan(bid(1)), "C1.title 삭제의 orphan 유지");
        outcome.inverse.apply(&mut library, &mut set).unwrap();
        assert_eq!(snapshot(&library, &set), original);
    }

    /// 3단계 chain을 IX.4.5 적용으로 만듭니다: L0(C0.p0 → I0.p1), L1(C1.p1 → I1.p2), L2(C2.p2 → I2.label).
    fn applied_chain() -> (
        ComponentLibrary,
        DrillingSet,
        Vec<ComponentPropertyId>,
        Vec<DrillingBindingId>,
    ) {
        use crate::component::drilling_apply::apply_drilling_plan;
        use crate::component::drilling_plan::{DrillingPlan, DrillingRequest};
        let mut library = nested_library(3);
        let mut set = DrillingSet::new();
        let plan = DrillingPlan::compute(
            &library,
            &set,
            DrillingRequest {
                top: component(0),
                path: vec![instance(0), instance(1), instance(2)],
                property: pid(LABEL),
            },
        )
        .unwrap();
        let mut next = 70_000u64;
        let applied = apply_drilling_plan(&mut library, &mut set, &plan, &mut || {
            next += 1;
            stable(next)
        })
        .unwrap();
        let properties = applied.created_properties.iter().map(|(_, p)| *p).collect();
        (library, set, properties, applied.created_links)
    }

    fn summary(
        orphaned: &[(DrillingBindingId, DrillingOrphan)],
    ) -> Vec<(DrillingBindingId, BreakPoint, bool)> {
        orphaned
            .iter()
            .map(|(id, o)| {
                (
                    *id,
                    o.break_point,
                    matches!(o.cause, OrphanCause::UpstreamOfBreak { .. }),
                )
            })
            .collect()
    }

    #[test]
    fn partial_chain_delete_orphans_the_whole_upstream_chain() {
        // 검증 기준(IX.4.6): break point와 affected chain fixture 정확.
        let (mut library, mut set, props, links) = applied_chain();
        let (l0, l1, l2) = (links[0], links[1], links[2]);
        let original = snapshot(&library, &set);
        // 중간 C2.p2 삭제: L1(경계 0 target)·L2(source)가 직접 끊기고, L1로 값을 넘기는 L0가 전파 orphan.
        let outcome = PropertyCommand::Delete {
            component: component(2),
            property: props[2],
        }
        .apply(&mut library, &mut set)
        .unwrap();
        assert_eq!(
            summary(&outcome.orphaned),
            vec![
                (l0, BreakPoint::Boundary(0), true),
                (l1, BreakPoint::Boundary(0), false),
                (l2, BreakPoint::Source, false),
            ]
        );
        assert_eq!(
            set.orphan(l0).unwrap().cause,
            OrphanCause::UpstreamOfBreak { origin: l1 }
        );
        assert_eq!(set.affected_chain(l1), vec![l0, l1]);
        // 저장에서 삭제하지 않음: binding 3개와 orphan 정보가 bytes로 보존됩니다.
        assert_eq!(set.len(), 3);
        let decoded = DrillingSet::decode(&set.encode()).unwrap();
        assert_eq!(decoded, set);
        assert_eq!(decoded.orphans().count(), 3);
        outcome.inverse.apply(&mut library, &mut set).unwrap();
        assert_eq!(snapshot(&library, &set), original);

        // 가장 안쪽 C3.label 삭제: L2가 break point, 상위 L1·L0 전체가 전파 orphan(origin L2).
        let outcome = PropertyCommand::Delete {
            component: component(3),
            property: pid(LABEL),
        }
        .apply(&mut library, &mut set)
        .unwrap();
        assert_eq!(
            summary(&outcome.orphaned),
            vec![
                (l0, BreakPoint::Boundary(0), true),
                (l1, BreakPoint::Boundary(0), true),
                (l2, BreakPoint::Boundary(0), false),
            ]
        );
        assert_eq!(set.affected_chain(l2), vec![l0, l1, l2]);
        assert!(outcome
            .orphaned
            .iter()
            .all(|(_, o)| o.last_valid_type == TypeSpec::required(ExpressionType::String)));
        outcome.inverse.apply(&mut library, &mut set).unwrap();
        assert_eq!(snapshot(&library, &set), original);
    }

    #[test]
    fn type_change_orphans_broken_links_and_upstream_but_keeps_downstream_live() {
        let (mut library, mut set, props, links) = applied_chain();
        let (l0, l1, l2) = (links[0], links[1], links[2]);
        let original = snapshot(&library, &set);
        let number = TypeSpec::required(ExpressionType::Number);
        // C1.p1 String → Number: L1(source)·L0(경계 0 target) 직접 끊김, 아래쪽 L2는 live.
        let outcome = PropertyCommand::Retype {
            component: component(1),
            property: props[1],
            value_type: number,
            default: literal(TypedValue::Number(
                stellar_core_types::value::FiniteF32::new(1.0).unwrap(),
            )),
        }
        .apply(&mut library, &mut set)
        .unwrap();
        assert_eq!(
            summary(&outcome.orphaned),
            vec![
                (l0, BreakPoint::Boundary(0), false),
                (l1, BreakPoint::Source, false),
            ]
        );
        assert_eq!(
            set.orphan(l1).unwrap().cause,
            OrphanCause::PropertyRetyped {
                component: component(1),
                property: props[1],
                previous: TypeSpec::required(ExpressionType::String),
                current: number,
            }
        );
        // L0의 target이 바뀐 property이므로 마지막 유효 타입은 이전 타입(String).
        assert_eq!(
            set.orphan(l0).unwrap().last_valid_type,
            TypeSpec::required(ExpressionType::String)
        );
        assert!(!set.is_orphan(l2), "끊긴 지점보다 안쪽은 live");
        assert_eq!(DrillingSet::decode(&set.encode()).unwrap(), set);
        set.validate(&library).unwrap();
        ComponentLibrary::decode(&library.encode()).unwrap();
        outcome.inverse.apply(&mut library, &mut set).unwrap();
        assert_eq!(snapshot(&library, &set), original);

        // C2.p2 String → Optional String: L1(String → Optional)은 유효하지만 L2(Optional → String)가 끊겨
        // 상위 L1·L0가 전파 orphan.
        let outcome = PropertyCommand::Retype {
            component: component(2),
            property: props[2],
            value_type: TypeSpec::optional(ExpressionType::String),
            default: literal(TypedValue::String("확인".into())),
        }
        .apply(&mut library, &mut set)
        .unwrap();
        assert_eq!(
            summary(&outcome.orphaned),
            vec![
                (l0, BreakPoint::Boundary(0), true),
                (l1, BreakPoint::Boundary(0), true),
                (l2, BreakPoint::Source, false),
            ]
        );
        assert_eq!(set.affected_chain(l2), vec![l0, l1, l2]);
        assert_eq!(
            set.orphan(l1).unwrap().last_valid_type,
            TypeSpec::required(ExpressionType::String),
            "target이 바뀐 property면 이전 타입"
        );
        let redo = outcome.inverse.apply(&mut library, &mut set).unwrap();
        assert_eq!(snapshot(&library, &set), original);
        // redo는 같은 orphan 상태를 다시 만듭니다.
        let again = redo.inverse;
        assert!(matches!(again, PropertyCommand::Retype { .. }));
        let replay = again.apply(&mut library, &mut set).unwrap();
        assert_eq!(summary(&replay.orphaned), summary(&outcome.orphaned));
        replay.inverse.apply(&mut library, &mut set).unwrap();
        assert_eq!(snapshot(&library, &set), original);

        // 타입에 맞지 않는 default, 상태 머신 참조는 거부.
        assert!(matches!(
            PropertyCommand::Retype {
                component: component(1),
                property: props[1],
                value_type: number,
                default: literal(TypedValue::String("x".into())),
            }
            .apply(&mut library, &mut set),
            Err(PropertyCommandError::Property(_))
        ));
        let mut toggle = sm::toggle_library();
        let machine = sm::toggle_machine(&toggle);
        toggle.set_state_machine(sm::toggle(), machine).unwrap();
        assert!(matches!(
            PropertyCommand::Retype {
                component: sm::toggle(),
                property: sm::prop(sm::INITIALLY_ON),
                value_type: TypeSpec::optional(ExpressionType::Boolean),
                default: literal(TypedValue::Boolean(true)),
            }
            .apply(&mut toggle, &mut DrillingSet::new()),
            Err(PropertyCommandError::ReferencedByStateMachine { .. })
        ));
        assert_eq!(snapshot(&library, &set), original);
    }

    #[test]
    fn invalid_commands_leave_state_unchanged() {
        let (mut library, mut set) = drilled();
        let before = snapshot(&library, &set);
        let bad = [
            // 같은 이름.
            PropertyCommand::Create {
                component: component(2),
                property: string_property(42_000, "label"),
                index: None,
            },
            // 같은 id.
            PropertyCommand::Create {
                component: component(2),
                property: string_property(LABEL, "other"),
                index: None,
            },
            // 범위 밖 위치.
            PropertyCommand::Create {
                component: component(2),
                property: string_property(42_000, "caption"),
                index: Some(9),
            },
            // rename 이름 충돌·빈 이름.
            PropertyCommand::Rename {
                component: component(2),
                property: pid(LABEL),
                name: "secret".into(),
            },
            PropertyCommand::Rename {
                component: component(2),
                property: pid(LABEL),
                name: " ".into(),
            },
            // default 타입 불일치.
            PropertyCommand::SetDefault {
                component: component(2),
                property: pid(LABEL),
                default: literal(TypedValue::Boolean(true)),
            },
            // 없는 property·컴포넌트.
            PropertyCommand::Delete {
                component: component(2),
                property: pid(49_999),
            },
            PropertyCommand::Delete {
                component: component(9),
                property: pid(LABEL),
            },
            // 이 삭제로 orphan이 되지 않은 binding을 revive.
            PropertyCommand::Restore {
                component: component(2),
                property: string_property(42_000, "caption"),
                index: 0,
                revive: vec![bid(1)],
            },
        ];
        for command in &bad {
            assert!(
                command.apply(&mut library, &mut set).is_err(),
                "{command:?}"
            );
            assert_eq!(snapshot(&library, &set), before, "{command:?}");
        }
        // 이름을 자기 이름으로 바꾸는 것은 허용(no-op inverse).
        let outcome = PropertyCommand::Rename {
            component: component(2),
            property: pid(LABEL),
            name: "label".into(),
        }
        .apply(&mut library, &mut set)
        .unwrap();
        assert_eq!(snapshot(&library, &set), before);
        outcome.inverse.apply(&mut library, &mut set).unwrap();
    }

    #[test]
    fn delete_is_rejected_when_state_machine_or_definition_override_uses_it() {
        // 상태 머신 initial이 initiallyOn을 참조.
        let mut library = sm::toggle_library();
        let machine = sm::toggle_machine(&library);
        library.set_state_machine(sm::toggle(), machine).unwrap();
        let mut set = DrillingSet::new();
        assert_eq!(
            PropertyCommand::Delete {
                component: sm::toggle(),
                property: sm::prop(sm::INITIALLY_ON),
            }
            .apply(&mut library, &mut set)
            .unwrap_err(),
            PropertyCommandError::ReferencedByStateMachine {
                component: sm::toggle(),
                property: sm::prop(sm::INITIALLY_ON),
            }
        );
        // 참조 없는 caption은 삭제 가능.
        PropertyCommand::Delete {
            component: sm::toggle(),
            property: sm::prop(sm::CAPTION),
        }
        .apply(&mut library, &mut set)
        .unwrap();
        ComponentLibrary::decode(&library.encode()).unwrap();

        // C1 정의 tree의 C2 인스턴스가 label을 정적 override.
        let mut library = nested_library(2);
        let mut payload = ComponentInstancePayload::new(component(2));
        payload.set_property_override(
            OverrideKey::direct(pid(LABEL)),
            TypedValue::String("정적".into()),
        );
        library
            .entry_mut(component(1))
            .unwrap()
            .tree_mut()
            .set_payload(instance(1), payload.encode());
        assert_eq!(
            PropertyCommand::Delete {
                component: component(2),
                property: pid(LABEL),
            }
            .validate(&library, &set)
            .unwrap_err(),
            PropertyCommandError::OverriddenInDefinition {
                owner: component(1),
                instance: instance(1),
                property: pid(LABEL),
            }
        );
        // private property 생성도 가능합니다.
        PropertyCommand::Create {
            component: component(0),
            property: number_property(42_001, "count", PropertyVisibility::Private),
            index: None,
        }
        .apply(&mut library, &mut set)
        .unwrap();
        assert!(!library
            .get(component(0))
            .unwrap()
            .property(pid(42_001))
            .unwrap()
            .is_public());
    }
}

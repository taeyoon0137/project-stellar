//! Component Context 편집 command (XXX.7.8, ADR-0099 결정 8).
//!
//! [`ContextCommand`] 하나(`CommandKind::EditContext`, `0x00A0`)가 [`ContextEdit`] 연산 하나를 담습니다.
//!
//! | tag | 연산 | 역 |
//! |---|---|---|
//! | 1 | key 생성 | key 삭제 |
//! | 2 | key rename | 이전 이름 rename |
//! | 3 | key default(값 타입 포함) 변경 | 이전 default |
//! | 4 | key 삭제(그 key의 provision·requirement·공개 목록 항목도 함께) | 12 복원 |
//! | 5 | provision 생성 | provision 삭제 |
//! | 6 | provision 값 변경 | 이전 값 |
//! | 7 | provision 삭제 | provision 생성 |
//! | 8·9 | requirement 추가·삭제 | 삭제·추가 |
//! | 10·11 | Slot 공개 목록 추가·삭제 | 삭제·추가 |
//! | 12 | key 복원(삭제의 역) | key 삭제 |
//!
//! - 소비가 있는 key를 지우면 문서는 key를 지우고, 소비 binding은 binding 쪽 orphan validator(VIII.8.1,
//!   `stellar_bindings::context::ContextBindingWorld`)가 `OrphanedBinding`으로 바꿉니다. binding은 지우지 않습니다.
//! - provision만 지우면 다음 해석이 바깥 provision이나 key default로 갑니다(orphan 아님).
//! - View owner provision은 문서 모델로 owner 범위를 검사합니다. 컴포넌트 owner provision의 범위는 컴포넌트 library를 아는
//!   Context validator(XXX.7.5)가 검사합니다(command 계층은 library를 모름).

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::context::key::{
    type_spec_bytes, type_spec_from_bytes, ContextKey, ContextKeyError,
};
use stellar_document::context::provision::{
    check_provision_scope, ContextOwner, ContextProvision, ContextProvisionError, PROVISION_VALUE,
};
use stellar_document::context::set::{ContextSet, ContextSetError};
use stellar_document::expressions::ast::ExpressionAst;
use stellar_document::expressions::types::TypeSpec;
use stellar_document::model::DocumentModel;
use stellar_document::modifier_metadata::{InvalidationClass, InvalidationSet};

use crate::codec::{CommandCodecError, Reader, Writer};
use crate::command::CommandError;
use crate::edit::EditContext;

/// Context 값이 바뀌면 그 값을 읽는 속성의 배치·도형·그리기가 모두 바뀔 수 있습니다.
pub(crate) const CONTEXT_INVALIDATION: InvalidationSet = InvalidationSet::EMPTY
    .with(InvalidationClass::Layout)
    .with(InvalidationClass::Geometry)
    .with(InvalidationClass::Paint);

/// Context 속성 key(semantic 대역 `0xECxx`, ADR-0099 결정 10). change set의 경로로만 씁니다.
pub(crate) const CONTEXT_KEY_PATH: stellar_core_types::property::PropertyKey =
    stellar_core_types::property::PropertyKey(0xEC00);

/// Context 연산 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextEdit {
    /// key 생성.
    CreateKey(ContextKey),
    /// key rename.
    RenameKey {
        /// key.
        key: StableId,
        /// 새 이름.
        name: String,
    },
    /// key default(값 타입 포함) 변경.
    SetKeyDefault {
        /// key.
        key: StableId,
        /// 새 값 타입.
        value_type: TypeSpec,
        /// 새 닫힌 default.
        default: ExpressionAst,
    },
    /// key 삭제(그 key의 provision·requirement·Slot 공개 목록 항목도 지움).
    DeleteKey {
        /// key.
        key: StableId,
    },
    /// provision 생성.
    CreateProvision(ContextProvision),
    /// provision 값 변경.
    SetProvisionValue {
        /// provision.
        provision: StableId,
        /// 새 값 식.
        value: ExpressionAst,
    },
    /// provision 삭제.
    DeleteProvision {
        /// provision.
        provision: StableId,
    },
    /// requirement 추가.
    AddRequirement {
        /// 컴포넌트.
        component: StableId,
        /// key.
        key: StableId,
    },
    /// requirement 삭제.
    RemoveRequirement {
        /// 컴포넌트.
        component: StableId,
        /// key.
        key: StableId,
    },
    /// Slot 공개 목록 추가.
    AddSlotKey {
        /// Slot.
        slot: StableId,
        /// key.
        key: StableId,
    },
    /// Slot 공개 목록 삭제.
    RemoveSlotKey {
        /// Slot.
        slot: StableId,
        /// key.
        key: StableId,
    },
    /// key 복원(삭제의 역): key와 함께 지웠던 provision·requirement 컴포넌트·공개 목록 Slot.
    RestoreKey {
        /// key.
        key: ContextKey,
        /// 함께 지웠던 provision(StableId 순).
        provisions: Vec<ContextProvision>,
        /// 그 key를 requirement로 가졌던 컴포넌트(StableId 순).
        requirements: Vec<StableId>,
        /// 그 key를 공개했던 Slot(StableId 순).
        slots: Vec<StableId>,
    },
}

/// Context 편집 command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCommand {
    /// 연산.
    pub edit: ContextEdit,
}

fn invalid(reason: &'static str) -> CommandError {
    CommandError::InvalidRequest { reason }
}

fn set_error(error: ContextSetError) -> CommandError {
    match error {
        ContextSetError::DuplicateId { id } => CommandError::DuplicateId { id },
        ContextSetError::DuplicateName { .. } => {
            invalid("같은 scope에 같은 이름의 ContextKey가 있습니다")
        }
        ContextSetError::DuplicateProvision { .. } => {
            invalid("provider node에 같은 key provision이 이미 있습니다")
        }
        ContextSetError::UnknownKey { .. } => invalid("ContextKey가 없습니다"),
        ContextSetError::UnknownProvision { .. } => invalid("ContextProvision이 없습니다"),
        ContextSetError::Key(_) | ContextSetError::Malformed(_) => {
            invalid("ContextKey 정의가 잘못됐습니다")
        }
        ContextSetError::Provision(_) => invalid("ContextProvision 정의가 잘못됐습니다"),
    }
}

fn key_error(_: ContextKeyError) -> CommandError {
    invalid("ContextKey default는 선언 타입의 닫힌 식이어야 합니다")
}

fn provision_error(_: ContextProvisionError) -> CommandError {
    invalid("ContextProvision 값 식을 쓸 수 없습니다")
}

impl ContextCommand {
    /// 연산을 적용한 새 set을 계산합니다(모델은 바꾸지 않음).
    fn applied(&self, model: &DocumentModel) -> Result<ContextSet, CommandError> {
        let mut set = model.context().clone();
        match &self.edit {
            ContextEdit::CreateKey(key) => {
                if model.contains_persistent_id(key.id()) || model.contains(key.id()) {
                    return Err(CommandError::DuplicateId { id: key.id() });
                }
                set.insert_key(key.clone()).map_err(set_error)?;
            }
            ContextEdit::RenameKey { key, name } => {
                let current = set.key(*key).ok_or(invalid("ContextKey가 없습니다"))?;
                let renamed = current.renamed(name.clone()).map_err(key_error)?;
                set.replace_key(renamed).map_err(set_error)?;
            }
            ContextEdit::SetKeyDefault {
                key,
                value_type,
                default,
            } => {
                let current = set.key(*key).ok_or(invalid("ContextKey가 없습니다"))?;
                let next = current
                    .with_default(*value_type, default.clone())
                    .map_err(key_error)?;
                set.replace_key(next).map_err(set_error)?;
            }
            ContextEdit::DeleteKey { key } => {
                set.remove_key(*key)
                    .ok_or(invalid("ContextKey가 없습니다"))?;
                let provisions: Vec<StableId> = set.provisions_of(*key).map(|p| p.id()).collect();
                for provision in provisions {
                    set.remove_provision(provision);
                }
                let components: Vec<StableId> = set
                    .all_requirements()
                    .filter(|(_, keys)| keys.contains(key))
                    .map(|(c, _)| c)
                    .collect();
                for component in components {
                    set.remove_requirement(component, *key);
                }
                let slots: Vec<StableId> = set
                    .all_slot_keys()
                    .filter(|(_, keys)| keys.contains(key))
                    .map(|(s, _)| s)
                    .collect();
                for slot in slots {
                    set.remove_slot_key(slot, *key);
                }
            }
            ContextEdit::CreateProvision(provision) => {
                if model.contains_persistent_id(provision.id()) || model.contains(provision.id()) {
                    return Err(CommandError::DuplicateId { id: provision.id() });
                }
                if let ContextOwner::View(_) = provision.owner() {
                    check_provision_scope(provision, model, &ComponentLibrary::new())
                        .map_err(|_| invalid("provision provider가 owner View 밖에 있습니다"))?;
                }
                set.insert_provision(provision.clone()).map_err(set_error)?;
            }
            ContextEdit::SetProvisionValue { provision, value } => {
                let current = set
                    .provision(*provision)
                    .ok_or(invalid("ContextProvision이 없습니다"))?;
                let next = current.with_value(value.clone()).map_err(provision_error)?;
                set.replace_provision(next).map_err(set_error)?;
            }
            ContextEdit::DeleteProvision { provision } => {
                set.remove_provision(*provision)
                    .ok_or(invalid("ContextProvision이 없습니다"))?;
            }
            ContextEdit::AddRequirement { component, key } => {
                if !set.add_requirement(*component, *key) {
                    return Err(invalid("requirement가 이미 있습니다"));
                }
            }
            ContextEdit::RemoveRequirement { component, key } => {
                if !set.remove_requirement(*component, *key) {
                    return Err(invalid("requirement가 없습니다"));
                }
            }
            ContextEdit::AddSlotKey { slot, key } => {
                if !set.add_slot_key(*slot, *key) {
                    return Err(invalid("Slot 공개 목록에 key가 이미 있습니다"));
                }
            }
            ContextEdit::RemoveSlotKey { slot, key } => {
                if !set.remove_slot_key(*slot, *key) {
                    return Err(invalid("Slot 공개 목록에 key가 없습니다"));
                }
            }
            ContextEdit::RestoreKey {
                key,
                provisions,
                requirements,
                slots,
            } => {
                set.insert_key(key.clone()).map_err(set_error)?;
                for provision in provisions {
                    set.insert_provision(provision.clone()).map_err(set_error)?;
                }
                for component in requirements {
                    set.add_requirement(*component, key.id());
                }
                for slot in slots {
                    set.add_slot_key(*slot, key.id());
                }
            }
        }
        Ok(set)
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        self.applied(model).map(|_| ())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<ContextCommand, CommandError> {
        self.validate(model)?;
        let set = model.context();
        let edit = match &self.edit {
            ContextEdit::CreateKey(key) => ContextEdit::DeleteKey { key: key.id() },
            ContextEdit::RenameKey { key, .. } => ContextEdit::RenameKey {
                key: *key,
                name: set.key(*key).expect("validated").name().to_owned(),
            },
            ContextEdit::SetKeyDefault { key, .. } => {
                let current = set.key(*key).expect("validated");
                ContextEdit::SetKeyDefault {
                    key: *key,
                    value_type: current.value_type(),
                    default: current.default_expression().ast().clone(),
                }
            }
            ContextEdit::DeleteKey { key } => ContextEdit::RestoreKey {
                key: set.key(*key).expect("validated").clone(),
                provisions: set.provisions_of(*key).cloned().collect(),
                requirements: set
                    .all_requirements()
                    .filter(|(_, keys)| keys.contains(key))
                    .map(|(c, _)| c)
                    .collect(),
                slots: set
                    .all_slot_keys()
                    .filter(|(_, keys)| keys.contains(key))
                    .map(|(s, _)| s)
                    .collect(),
            },
            ContextEdit::CreateProvision(provision) => ContextEdit::DeleteProvision {
                provision: provision.id(),
            },
            ContextEdit::SetProvisionValue { provision, .. } => ContextEdit::SetProvisionValue {
                provision: *provision,
                value: set
                    .provision(*provision)
                    .expect("validated")
                    .value()
                    .clone(),
            },
            ContextEdit::DeleteProvision { provision } => {
                ContextEdit::CreateProvision(set.provision(*provision).expect("validated").clone())
            }
            ContextEdit::AddRequirement { component, key } => ContextEdit::RemoveRequirement {
                component: *component,
                key: *key,
            },
            ContextEdit::RemoveRequirement { component, key } => ContextEdit::AddRequirement {
                component: *component,
                key: *key,
            },
            ContextEdit::AddSlotKey { slot, key } => ContextEdit::RemoveSlotKey {
                slot: *slot,
                key: *key,
            },
            ContextEdit::RemoveSlotKey { slot, key } => ContextEdit::AddSlotKey {
                slot: *slot,
                key: *key,
            },
            ContextEdit::RestoreKey { key, .. } => ContextEdit::DeleteKey { key: key.id() },
        };
        Ok(ContextCommand { edit })
    }

    /// change set에 남길 (객체, 경로).
    fn touched(&self) -> (StableId, PropertyPath) {
        let key_path = |key: StableId| PropertyPath::with_keys(key, vec![CONTEXT_KEY_PATH]);
        match &self.edit {
            ContextEdit::CreateKey(key) => (key.id(), key_path(key.id())),
            ContextEdit::RenameKey { key, .. }
            | ContextEdit::SetKeyDefault { key, .. }
            | ContextEdit::DeleteKey { key } => (*key, key_path(*key)),
            ContextEdit::RestoreKey { key, .. } => (key.id(), key_path(key.id())),
            ContextEdit::CreateProvision(provision) => {
                (provision.provider(), provision.value_path())
            }
            ContextEdit::SetProvisionValue { provision, .. }
            | ContextEdit::DeleteProvision { provision } => (
                *provision,
                PropertyPath::with_keys(*provision, vec![PROVISION_VALUE]),
            ),
            ContextEdit::AddRequirement { component, key }
            | ContextEdit::RemoveRequirement { component, key } => (
                *component,
                PropertyPath::with_keys(*component, vec![CONTEXT_KEY_PATH])
                    .with_key_segment(&key.to_string()),
            ),
            ContextEdit::AddSlotKey { slot, key } | ContextEdit::RemoveSlotKey { slot, key } => (
                *slot,
                PropertyPath::with_keys(*slot, vec![CONTEXT_KEY_PATH])
                    .with_key_segment(&key.to_string()),
            ),
        }
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let next = self.applied(ctx.model())?;
        ctx.replace_context(next)?;
        let (object, path) = self.touched();
        ctx.changes().property(object, path, CONTEXT_INVALIDATION);
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        match &self.edit {
            ContextEdit::CreateKey(key) => {
                out.u8(1);
                out.bytes(&key.encode());
            }
            ContextEdit::RenameKey { key, name } => {
                out.u8(2);
                out.id(*key);
                out.str(name);
            }
            ContextEdit::SetKeyDefault {
                key,
                value_type,
                default,
            } => {
                out.u8(3);
                out.id(*key);
                out.bytes(&type_spec_bytes(*value_type));
                out.bytes(&default.to_canonical_bytes().unwrap_or_default());
            }
            ContextEdit::DeleteKey { key } => {
                out.u8(4);
                out.id(*key);
            }
            ContextEdit::CreateProvision(provision) => {
                out.u8(5);
                out.bytes(&provision.encode());
            }
            ContextEdit::SetProvisionValue { provision, value } => {
                out.u8(6);
                out.id(*provision);
                out.bytes(&value.to_canonical_bytes().unwrap_or_default());
            }
            ContextEdit::DeleteProvision { provision } => {
                out.u8(7);
                out.id(*provision);
            }
            ContextEdit::AddRequirement { component, key } => {
                out.u8(8);
                out.id(*component);
                out.id(*key);
            }
            ContextEdit::RemoveRequirement { component, key } => {
                out.u8(9);
                out.id(*component);
                out.id(*key);
            }
            ContextEdit::AddSlotKey { slot, key } => {
                out.u8(10);
                out.id(*slot);
                out.id(*key);
            }
            ContextEdit::RemoveSlotKey { slot, key } => {
                out.u8(11);
                out.id(*slot);
                out.id(*key);
            }
            ContextEdit::RestoreKey {
                key,
                provisions,
                requirements,
                slots,
            } => {
                out.u8(12);
                out.bytes(&key.encode());
                out.len(provisions.len());
                for provision in provisions {
                    out.bytes(&provision.encode());
                }
                out.len(requirements.len());
                for component in requirements {
                    out.id(*component);
                }
                out.len(slots.len());
                for slot in slots {
                    out.id(*slot);
                }
            }
        }
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<ContextCommand, CommandCodecError> {
        let malformed = |reason| CommandCodecError::Malformed { reason };
        let key =
            |bytes: &[u8]| ContextKey::decode(bytes).map_err(|_| malformed("ContextKey bytes"));
        let provision = |bytes: &[u8]| {
            ContextProvision::decode(bytes).map_err(|_| malformed("ContextProvision bytes"))
        };
        let ast = |bytes: &[u8]| {
            ExpressionAst::from_canonical_bytes(bytes).map_err(|_| malformed("expression bytes"))
        };
        let edit = match input.u8()? {
            1 => ContextEdit::CreateKey(key(input.bytes()?)?),
            2 => ContextEdit::RenameKey {
                key: input.id()?,
                name: input.string()?,
            },
            3 => ContextEdit::SetKeyDefault {
                key: input.id()?,
                value_type: type_spec_from_bytes(input.bytes()?)
                    .ok_or(malformed("TypeSpec bytes"))?,
                default: ast(input.bytes()?)?,
            },
            4 => ContextEdit::DeleteKey { key: input.id()? },
            5 => ContextEdit::CreateProvision(provision(input.bytes()?)?),
            6 => ContextEdit::SetProvisionValue {
                provision: input.id()?,
                value: ast(input.bytes()?)?,
            },
            7 => ContextEdit::DeleteProvision {
                provision: input.id()?,
            },
            8 => ContextEdit::AddRequirement {
                component: input.id()?,
                key: input.id()?,
            },
            9 => ContextEdit::RemoveRequirement {
                component: input.id()?,
                key: input.id()?,
            },
            10 => ContextEdit::AddSlotKey {
                slot: input.id()?,
                key: input.id()?,
            },
            11 => ContextEdit::RemoveSlotKey {
                slot: input.id()?,
                key: input.id()?,
            },
            12 => {
                let restored = key(input.bytes()?)?;
                let mut provisions = Vec::new();
                for _ in 0..input.len()? {
                    provisions.push(provision(input.bytes()?)?);
                }
                let mut requirements = Vec::new();
                for _ in 0..input.len()? {
                    requirements.push(input.id()?);
                }
                let mut slots = Vec::new();
                for _ in 0..input.len()? {
                    slots.push(input.id()?);
                }
                ContextEdit::RestoreKey {
                    key: restored,
                    provisions,
                    requirements,
                    slots,
                }
            }
            _ => return Err(malformed("모르는 Context 연산 tag")),
        };
        Ok(ContextCommand { edit })
    }
}

#[cfg(test)]
mod tests {
    use stellar_document::context::fixtures::world::*;
    use stellar_document::context::fixtures::{
        context_read, number, number_type, sample_world, sid,
    };
    use stellar_document::context::key::ContextScope;
    use stellar_document::context::resolve::{ContextResolver, ContextSite};

    use super::*;
    use crate::codec::{decode_command, encode_command};
    use crate::command::Command;

    fn apply(model: &mut DocumentModel, command: &Command) {
        let mut ctx = EditContext::new(model);
        command.apply(&mut ctx).expect("apply");
        let _ = ctx.into_changes();
    }

    fn run(model: &mut DocumentModel, edit: ContextEdit) -> Command {
        let command = Command::EditContext(Box::new(ContextCommand { edit }));
        let inverse = command.invert(model).expect("invert");
        apply(model, &command);
        inverse
    }

    #[test]
    fn every_context_edit_applies_and_inverts() {
        // 검증 기준(XXX.7.8): apply/inverse 통과, codec round-trip.
        let (mut model, _) = sample_world();
        let original = model.context().clone();
        let new_key = ContextKey::new(
            sid(960),
            "density",
            ContextScope::Document(sid(DOC)),
            number_type(),
            number(960, 1.0),
        )
        .unwrap();
        let edits = vec![
            ContextEdit::CreateKey(new_key),
            ContextEdit::RenameKey {
                key: sid(N),
                name: "n2".into(),
            },
            ContextEdit::SetKeyDefault {
                key: sid(D),
                value_type: number_type(),
                default: number(961, 4.0),
            },
            ContextEdit::DeleteKey { key: sid(D) },
            ContextEdit::CreateProvision(
                ContextProvision::new(
                    sid(962),
                    ContextOwner::View(sid(VIEW)),
                    sid(FRAME),
                    sid(N),
                    number(962, 2.0),
                )
                .unwrap(),
            ),
            ContextEdit::SetProvisionValue {
                provision: sid(P_ROOT_N),
                value: number(963, 8.0),
            },
            ContextEdit::DeleteProvision {
                provision: sid(P_ROOT_E),
            },
            ContextEdit::AddRequirement {
                component: sid(CARD),
                key: sid(D),
            },
            ContextEdit::RemoveRequirement {
                component: sid(CARD),
                key: sid(N),
            },
            ContextEdit::AddSlotKey {
                slot: sid(BODY),
                key: sid(N),
            },
            ContextEdit::RemoveSlotKey {
                slot: sid(LABEL),
                key: sid(D),
            },
        ];
        for edit in edits {
            let command = Command::EditContext(Box::new(ContextCommand { edit: edit.clone() }));
            assert_eq!(decode_command(&encode_command(&command)).unwrap(), command);
            let inverse = run(&mut model, edit.clone());
            assert_ne!(model.context(), &original, "{edit:?}");
            assert_eq!(decode_command(&encode_command(&inverse)).unwrap(), inverse);
            apply(&mut model, &inverse);
            assert_eq!(model.context(), &original, "{edit:?} 역");
        }
        // key 삭제는 그 key의 provision·requirement·공개 목록 항목도 지웁니다.
        run(&mut model, ContextEdit::DeleteKey { key: sid(D) });
        assert!(model.context().provisions_of(sid(D)).next().is_none());
        assert!(!model.context().slot_keys(sid(BODY)).contains(&sid(D)));
    }

    #[test]
    fn deleting_only_the_provision_falls_back_to_default_and_bad_edits_are_rejected() {
        // 검증 기준(XXX.7.8): provision만 삭제하면 default(orphan 아님).
        let (mut model, library) = sample_world();
        run(
            &mut model,
            ContextEdit::DeleteProvision {
                provision: sid(P_HOST_D),
            },
        );
        {
            let resolver = ContextResolver::new(&model, &library);
            let resolution = resolver
                .resolve(sid(D), &ContextSite::scene(sid(CONTENT)))
                .unwrap();
            assert!(resolution.is_default());
        }
        // 잘못된 편집: 없는 key rename, 범위 밖 View provision, 열린 default, 같은 id.
        let bad = [
            ContextEdit::RenameKey {
                key: sid(9999),
                name: "x".into(),
            },
            ContextEdit::CreateProvision(
                ContextProvision::new(
                    sid(970),
                    ContextOwner::View(sid(VIEW)),
                    sid(CARD_ROOT),
                    sid(N),
                    number(970, 1.0),
                )
                .unwrap(),
            ),
            ContextEdit::SetKeyDefault {
                key: sid(N),
                value_type: number_type(),
                default: context_read(971, sid(D)),
            },
            ContextEdit::CreateKey(
                ContextKey::new(
                    sid(ROOT),
                    "clash",
                    ContextScope::Project,
                    number_type(),
                    number(972, 1.0),
                )
                .unwrap(),
            ),
        ];
        for edit in bad {
            let command = Command::EditContext(Box::new(ContextCommand { edit: edit.clone() }));
            assert!(command.validate(&model).is_err(), "{edit:?}");
        }
        // 한 context에서 적용한 편집은 rollback으로 되돌아갑니다.
        let before = model.context().clone();
        let mut ctx = EditContext::new(&mut model);
        Command::EditContext(Box::new(ContextCommand {
            edit: ContextEdit::DeleteProvision {
                provision: sid(P_ROOT_N),
            },
        }))
        .apply(&mut ctx)
        .unwrap();
        ctx.rollback().unwrap();
        drop(ctx);
        assert_eq!(model.context(), &before);
    }
}

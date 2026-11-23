//! 가이드·레이아웃 그리드와 View 변수·Variant 편집 command (XII.10.19·XII.10.20·XVI.11.1, ADR-0117 결정 2.4·ADR-0126).
//!
//! - [`SetNodeGuides`]: Frame 하나의 가이드 추가·이동·삭제와 레이아웃 그리드 목록 교체([`GuideEdit`] 하나). 역은
//!   [`NodeGuides::apply`]가 돌려준 역 편집이라 제스처 하나 = history 항목 하나이고 undo가 정확히 되돌립니다.
//! - [`EditViewVariants`]: View 하나의 변수·Variant command([`ViewVariantCommand`], IX.7.8) 하나. 역은 문서 crate가
//!   돌려준 inverse command입니다.
//!
//! 두 값은 layout 입력이 아니므로(ADR-0074, ADR-0117 결정 4.4) 무효화는 비어 있습니다. 변경 경로는 semantic snapshot
//! 경로와 같습니다: 가이드는 `[owner, 0xED01] + 가이드 id`, 그리드는 `[owner, 0xED02]`, View 변수는
//! `[view, 0xED10] + 변수 id`, Variant는 `[Variant id, 0xED11]`(core-types `reserved::guides`).
//!
//! 잠금(D8) 판정은 다른 command와 같이 호출 쪽(`commands::lock`)이 합니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::reserved::guides::{
    SNAPSHOT_NODE_GUIDE, SNAPSHOT_NODE_LAYOUT_GRIDS, SNAPSHOT_VARIANT_CONDITION,
    SNAPSHOT_VIEW_VARIABLE,
};
use stellar_document::guides::{GuideEdit, GuideError, NodeGuides};
use stellar_document::model::DocumentModel;
use stellar_document::modifier_metadata::InvalidationSet;
use stellar_document::records::NodePayload;
use stellar_document::view_variables::{decode_command, encode_command};
use stellar_document::view_variant::commands::{
    VariantCommandError, ViewVariantCommand, ViewVariantModel,
};

use crate::codec::{CommandCodecError, Reader, Writer};
use crate::command::CommandError;
use crate::edit::EditContext;

/// 가이드 하나의 변경 경로 key.
pub const GUIDE_PATH: PropertyKey = PropertyKey(SNAPSHOT_NODE_GUIDE);
/// 레이아웃 그리드 목록의 변경 경로 key.
pub const GRIDS_PATH: PropertyKey = PropertyKey(SNAPSHOT_NODE_LAYOUT_GRIDS);
/// View 변수 하나의 변경 경로 key.
pub const VIEW_VARIABLE_PATH: PropertyKey = PropertyKey(SNAPSHOT_VIEW_VARIABLE);
/// Variant 하나의 변경 경로 key(경로 root = Variant id).
pub const VARIANT_CONDITION_PATH: PropertyKey = PropertyKey(SNAPSHOT_VARIANT_CONDITION);

fn guide_error(error: GuideError) -> CommandError {
    CommandError::InvalidRequest {
        reason: match error {
            GuideError::InvalidNumber(_) => "가이드·그리드 값이 유효하지 않습니다",
            GuideError::InvalidCount => "그리드 트랙 수가 유효하지 않습니다",
            GuideError::TooMany(_) => "가이드·그리드 수가 상한을 넘습니다",
            GuideError::DuplicateGuide(_) => "같은 id의 가이드가 이미 있습니다",
            GuideError::UnknownGuide(_) => "가이드가 없습니다",
            GuideError::NilId => "NIL id는 가이드 id로 쓸 수 없습니다",
            GuideError::Malformed(_) => "가이드 기록 형식이 잘못됐습니다",
        },
    }
}

fn variant_error(error: VariantCommandError) -> CommandError {
    CommandError::InvalidRequest {
        reason: match error {
            VariantCommandError::Variable(_) => "View 변수 규칙 위반입니다",
            VariantCommandError::Condition(_) => "Variant 조건이 현재 변수에 맞지 않습니다",
            VariantCommandError::DuplicateVariant { .. } => "같은 id의 Variant가 이미 있습니다",
            VariantCommandError::UnknownVariant { .. } => "Variant가 없습니다",
            VariantCommandError::Overrides(_) => "Variant override 형식이 잘못됐습니다",
            VariantCommandError::Malformed { .. } => "View 변수 command 형식이 잘못됐습니다",
        },
    }
}

// --- SetNodeGuides ---

/// Frame 하나의 가이드·레이아웃 그리드 편집.
#[derive(Debug, Clone, PartialEq)]
pub struct SetNodeGuides {
    /// 소유 Frame.
    pub owner: StableId,
    /// 편집.
    pub edit: GuideEdit,
}

impl SetNodeGuides {
    /// 편집을 적용한 새 기록과 역 편집(모델은 바꾸지 않음).
    fn applied(&self, model: &DocumentModel) -> Result<(NodeGuides, GuideEdit), CommandError> {
        match model.payload(self.owner) {
            None => return Err(CommandError::UnknownNode { id: self.owner }),
            Some(NodePayload::Frame(_)) => {}
            Some(_) => {
                return Err(CommandError::InvalidRequest {
                    reason: "가이드·레이아웃 그리드는 Frame에만 둡니다",
                })
            }
        }
        let mut record = model
            .node_guides(self.owner)
            .cloned()
            .unwrap_or_else(|| NodeGuides::new(self.owner));
        let inverse = record.apply(&self.edit).map_err(guide_error)?;
        Ok((record, inverse))
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        self.applied(model).map(|_| ())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<SetNodeGuides, CommandError> {
        let (_, inverse) = self.applied(model)?;
        Ok(SetNodeGuides {
            owner: self.owner,
            edit: inverse,
        })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let (record, _) = self.applied(ctx.model())?;
        ctx.set_node_guides(record)?;
        let path = match &self.edit {
            GuideEdit::Add(guide) => guide_path(self.owner, guide.id),
            GuideEdit::Move { id, .. } | GuideEdit::Remove { id } => guide_path(self.owner, *id),
            GuideEdit::SetGrids(_) => PropertyPath::with_keys(self.owner, vec![GRIDS_PATH]),
        };
        ctx.changes()
            .property(self.owner, path, InvalidationSet::EMPTY);
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.owner);
        out.bytes(&self.edit.encode());
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<SetNodeGuides, CommandCodecError> {
        let owner = input.id()?;
        let edit = GuideEdit::decode(input.bytes()?).map_err(|_| CommandCodecError::Malformed {
            reason: "가이드 편집 bytes",
        })?;
        Ok(SetNodeGuides { owner, edit })
    }
}

/// 가이드 하나의 변경 경로.
pub fn guide_path(owner: StableId, guide: StableId) -> PropertyPath {
    PropertyPath::with_keys(owner, vec![GUIDE_PATH]).with_key_segment(&guide.to_string())
}

/// View 변수 하나의 변경 경로.
pub fn variable_path(view: StableId, variable: StableId) -> PropertyPath {
    PropertyPath::with_keys(view, vec![VIEW_VARIABLE_PATH]).with_key_segment(&variable.to_string())
}

// --- EditViewVariants ---

/// View 하나의 변수·Variant command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditViewVariants {
    /// 대상 View.
    pub view: StableId,
    /// 문서 crate command(IX.7.8).
    pub command: ViewVariantCommand,
}

impl EditViewVariants {
    fn applied(
        &self,
        model: &DocumentModel,
    ) -> Result<(ViewVariantModel, ViewVariantCommand), CommandError> {
        if model.definition().view_position(self.view).is_none() {
            return Err(CommandError::InvalidRequest {
                reason: "View가 문서에 없습니다",
            });
        }
        let mut variants = model
            .view_variants(self.view)
            .cloned()
            .unwrap_or_else(|| ViewVariantModel::new(self.view));
        if let ViewVariantCommand::CreateVariable(_) | ViewVariantCommand::CreateVariant(_) =
            &self.command
        {
            let id = match &self.command {
                ViewVariantCommand::CreateVariable(variable) => variable.id().stable_id(),
                ViewVariantCommand::CreateVariant(variant) => variant.id.stable_id(),
                _ => unreachable!(),
            };
            // 새 변수·Variant id는 문서의 다른 영속 id와, 다른 View(또는 같은 View의 다른 종류)의 변수·Variant id와
            // 겹치지 않아야 합니다(ADR-0126 결정 1.4). Variant는 semantic snapshot에서 id로 객체가 됩니다.
            if model.contains_persistent_id(id) || model.has_view_variant_item(id) {
                return Err(CommandError::DuplicateId { id });
            }
        }
        let inverse = self.command.apply(&mut variants).map_err(variant_error)?;
        Ok((variants, inverse))
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        self.applied(model).map(|_| ())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<EditViewVariants, CommandError> {
        let (_, inverse) = self.applied(model)?;
        Ok(EditViewVariants {
            view: self.view,
            command: inverse,
        })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let (variants, _) = self.applied(ctx.model())?;
        ctx.set_view_variants(variants)?;
        let path = match &self.command {
            ViewVariantCommand::CreateVariable(variable)
            | ViewVariantCommand::ReplaceVariable(variable) => {
                variable_path(self.view, variable.id().stable_id())
            }
            ViewVariantCommand::DeleteVariable(id) => variable_path(self.view, id.stable_id()),
            ViewVariantCommand::CreateVariant(variant)
            | ViewVariantCommand::ReplaceVariant(variant)
            | ViewVariantCommand::RestoreVariant(variant) => {
                PropertyPath::with_keys(variant.id.stable_id(), vec![VARIANT_CONDITION_PATH])
            }
            ViewVariantCommand::DeleteVariant(id) => {
                PropertyPath::with_keys(id.stable_id(), vec![VARIANT_CONDITION_PATH])
            }
        };
        ctx.changes()
            .property(self.view, path, InvalidationSet::EMPTY);
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.view);
        out.bytes(&encode_command(&self.command));
    }

    pub(crate) fn decode_from(
        input: &mut Reader<'_>,
    ) -> Result<EditViewVariants, CommandCodecError> {
        let view = input.id()?;
        let command = decode_command(input.bytes()?).map_err(|_| CommandCodecError::Malformed {
            reason: "View 변수 command bytes",
        })?;
        Ok(EditViewVariants { view, command })
    }
}

#[cfg(test)]
#[path = "guides_tests.rs"]
mod tests;

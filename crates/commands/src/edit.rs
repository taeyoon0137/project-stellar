//! command 적용 context (VII.1.2).
//!
//! [`Command::apply`](crate::command::Command::apply)는 문서 모델을 직접 받지 않고 이
//! context의 **primitive 변경**만으로 문서를 바꿉니다. 모든 변경이 한 지점을 지나므로 이후
//! 작업이 변경 기록(VII.1.4)과 되돌리기 journal(VII.1.5)을 여기에 붙입니다.
//!
//! primitive는 [`DocumentModel`]의 공개 변경 API와 1:1입니다: node 삽입, leaf 삭제, payload
//! 교체(종류 유지), 표시 이름, modifier stack 교체.
//!
//! command는 의미 단위 변경(바뀐 node·속성·무효화)을 [`EditContext::changes`]에 기록합니다
//! (VII.1.4). primitive 자체는 기록하지 않습니다 — 예를 들어 이동을 삭제·재삽입 primitive로
//! 구현해도 결과는 "부모·순서 변경"으로 보고됩니다.

use stellar_core_types::ids::StableId;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::ModifierStack;
use stellar_document::records::{NodePayload, NodeRecord};

use crate::change::ChangeRecorder;
use crate::command::CommandError;

/// command 적용 중의 문서 변경 창구.
#[derive(Debug)]
pub struct EditContext<'a> {
    model: &'a mut DocumentModel,
    changes: ChangeRecorder,
}

impl<'a> EditContext<'a> {
    /// 모델 위에 context를 엽니다.
    pub fn new(model: &'a mut DocumentModel) -> EditContext<'a> {
        EditContext {
            model,
            changes: ChangeRecorder::new(),
        }
    }

    /// 현재(적용 중) 모델 읽기.
    pub fn model(&self) -> &DocumentModel {
        self.model
    }

    /// 의미 단위 변경 기록기(VII.1.4).
    pub fn changes(&mut self) -> &mut ChangeRecorder {
        &mut self.changes
    }

    /// 지금까지의 변경 기록을 꺼냅니다.
    pub fn into_changes(self) -> ChangeRecorder {
        self.changes
    }

    /// node를 삽입합니다.
    pub fn insert_node(&mut self, record: NodeRecord) -> Result<(), CommandError> {
        self.model.insert_node(record)?;
        Ok(())
    }

    /// 자식 없는 node를 지우고 그 레코드와(있었다면) modifier stack을 반환합니다.
    pub fn remove_leaf(
        &mut self,
        id: StableId,
    ) -> Result<(NodeRecord, Option<ModifierStack>), CommandError> {
        let stack = self.model.modifier_stack(id).cloned();
        let record = self.model.remove_leaf(id)?;
        Ok((record, stack))
    }

    /// payload를 교체합니다(종류 유지). 이전 payload를 반환합니다.
    pub fn set_payload(
        &mut self,
        id: StableId,
        payload: NodePayload,
    ) -> Result<NodePayload, CommandError> {
        let previous = self
            .model
            .payload(id)
            .cloned()
            .ok_or(CommandError::UnknownNode { id })?;
        self.model.set_payload(id, payload)?;
        Ok(previous)
    }

    /// 표시 이름을 바꿉니다. 이전 이름을 반환합니다.
    pub fn set_display_name(&mut self, id: StableId, name: String) -> Result<String, CommandError> {
        let previous = self
            .model
            .runtime_id(id)
            .and_then(|handle| self.model.columns().get(handle))
            .map(|fields| fields.display_name)
            .ok_or(CommandError::UnknownNode { id })?;
        self.model.set_display_name(id, name)?;
        Ok(previous)
    }

    /// node의 modifier stack을 교체합니다(빈 stack = 제거). 이전 stack을 반환합니다.
    pub fn set_modifier_stack(
        &mut self,
        stack: ModifierStack,
    ) -> Result<Option<ModifierStack>, CommandError> {
        let previous = self.model.modifier_stack(stack.node).cloned();
        self.model.set_modifier_stack(stack)?;
        Ok(previous)
    }
}

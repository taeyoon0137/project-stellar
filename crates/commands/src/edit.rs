//! command 적용 context (VII.1.2·VII.1.4·VII.1.5).
//!
//! [`Command::apply`](crate::command::Command::apply)는 문서 모델을 직접 받지 않고 이
//! context의 **primitive 변경**만으로 문서를 바꿉니다. 모든 변경이 한 지점을 지나므로 변경
//! 기록(VII.1.4)과 되돌리기 journal(VII.1.5)이 여기에 붙습니다.
//!
//! primitive는 [`DocumentModel`]의 공개 변경 API와 1:1입니다: node 삽입, leaf 삭제, payload
//! 교체(종류 유지), 표시 이름, modifier stack 교체, 문서 정의 교체(ADR-0079).
//!
//! - **변경 기록**: command는 의미 단위 변경(바뀐 node·속성·무효화)을 [`EditContext::changes`]에
//!   기록합니다. primitive 자체는 기록하지 않습니다 — 예를 들어 이동을 삭제·재삽입 primitive로
//!   구현해도 결과는 "부모·순서 변경"으로 보고됩니다.
//! - **되돌리기 journal**: 성공한 primitive마다 그 역 primitive(삭제한 레코드, 이전 payload·
//!   이름·stack)를 기록합니다. 전체 문서를 복제하지 않고, 바뀐 부분만 담습니다. 실패 시
//!   [`EditContext::rollback`]이 journal을 역순으로 재생해 변경 전 모델 상태로 되돌립니다
//!   (runtime 핸들은 새로 할당될 수 있으나 의미 상태 — 레코드·semantic snapshot — 는 같습니다).

use std::fmt;

use stellar_core_types::{ids::StableId, order_key::OrderKey};
use stellar_document::definitions::DocumentDefinition;
use stellar_document::layout_record::LayoutRecord;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::ModifierStack;
use stellar_document::records::{NodePayload, NodeRecord};

use crate::change::ChangeRecorder;
use crate::command::CommandError;

/// 역 primitive(journal 항목).
#[derive(Debug, Clone, PartialEq)]
enum UndoOp {
    /// 삽입을 되돌림: leaf 삭제.
    Remove(StableId),
    /// leaf 삭제를 되돌림: 레코드 재삽입 + stack 복원.
    Reinsert(Box<NodeRecord>, Option<ModifierStack>),
    /// payload 교체를 되돌림.
    Payload(StableId, Box<NodePayload>),
    /// 같은 부모 안의 순서 변경을 되돌립니다.
    ChildOrder(StableId, OrderKey),
    /// 이름 변경을 되돌림.
    Name(StableId, String),
    /// 표시·잠금 변경을 되돌림.
    Flags {
        id: StableId,
        visible: bool,
        locked: bool,
    },
    /// stack 교체를 되돌림(`None`이면 stack 제거).
    Stack(StableId, Option<ModifierStack>),
    Definition(Box<DocumentDefinition>),
    /// layout record 교체를 되돌림(`None`이면 record 제거, ADR-0086).
    Layout(StableId, Option<LayoutRecord>),
    /// 활성 기본값 세트 이름 변경을 되돌림(ADR-0086).
    StyleDefaults(String),
}

/// rollback 실패: journal 재생 중 모델이 역 primitive를 거부했습니다(내부 불일치 신호).
#[derive(Debug, Clone, PartialEq)]
pub struct RollbackError {
    /// 재생하지 못한 journal 항목 수.
    pub failed_ops: usize,
    /// 첫 실패 원인.
    pub first: CommandError,
}

impl fmt::Display for RollbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rollback 중 {}개 journal 항목 재생 실패: {}",
            self.failed_ops, self.first
        )
    }
}

impl std::error::Error for RollbackError {}

/// command 적용 중의 문서 변경 창구.
#[derive(Debug)]
pub struct EditContext<'a> {
    model: &'a mut DocumentModel,
    changes: ChangeRecorder,
    journal: Vec<UndoOp>,
    /// 테스트 fault injection: 이 수만큼 primitive가 성공한 뒤 다음 primitive가 실패합니다.
    #[cfg(test)]
    fail_after: Option<usize>,
    #[cfg(test)]
    primitive_count: usize,
}

impl<'a> EditContext<'a> {
    /// 모델 위에 context를 엽니다.
    pub fn new(model: &'a mut DocumentModel) -> EditContext<'a> {
        EditContext {
            model,
            changes: ChangeRecorder::new(),
            journal: Vec::new(),
            #[cfg(test)]
            fail_after: None,
            #[cfg(test)]
            primitive_count: 0,
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

    /// journal 항목 수(성공한 primitive 수).
    pub fn journal_len(&self) -> usize {
        self.journal.len()
    }

    /// 테스트 전용: `count`개 primitive가 성공한 뒤 다음 primitive를 실패시킵니다.
    #[cfg(test)]
    pub(crate) fn inject_fault_after(&mut self, count: usize) {
        self.fail_after = Some(count);
    }

    #[cfg(test)]
    fn before_primitive(&mut self) -> Result<(), CommandError> {
        if self.fail_after == Some(self.primitive_count) {
            return Err(CommandError::Model(
                stellar_document::model::ModelError::Inconsistent {
                    id: StableId::NIL,
                    reason: "primitive fault injection",
                },
            ));
        }
        self.primitive_count += 1;
        Ok(())
    }

    #[cfg(not(test))]
    fn before_primitive(&mut self) -> Result<(), CommandError> {
        Ok(())
    }

    /// node를 삽입합니다.
    pub fn insert_node(&mut self, record: NodeRecord) -> Result<(), CommandError> {
        self.before_primitive()?;
        let id = record.id;
        self.model.insert_node(record)?;
        self.journal.push(UndoOp::Remove(id));
        Ok(())
    }

    /// 자식 없는 node를 지우고 그 레코드와(있었다면) modifier stack을 반환합니다.
    pub fn remove_leaf(
        &mut self,
        id: StableId,
    ) -> Result<(NodeRecord, Option<ModifierStack>), CommandError> {
        self.before_primitive()?;
        let stack = self.model.modifier_stack(id).cloned();
        let record = self.model.remove_leaf(id)?;
        self.journal
            .push(UndoOp::Reinsert(Box::new(record.clone()), stack.clone()));
        Ok((record, stack))
    }

    /// payload를 교체합니다(종류 유지). 이전 payload를 반환합니다.
    pub fn set_payload(
        &mut self,
        id: StableId,
        payload: NodePayload,
    ) -> Result<NodePayload, CommandError> {
        self.before_primitive()?;
        let previous = self
            .model
            .payload(id)
            .cloned()
            .ok_or(CommandError::UnknownNode { id })?;
        self.model.set_payload(id, payload)?;
        self.journal
            .push(UndoOp::Payload(id, Box::new(previous.clone())));
        Ok(previous)
    }

    /// 표시 이름을 바꿉니다. 이전 이름을 반환합니다.
    pub fn set_display_name(&mut self, id: StableId, name: String) -> Result<String, CommandError> {
        self.before_primitive()?;
        let previous = self
            .model
            .runtime_id(id)
            .and_then(|handle| self.model.columns().get(handle))
            .map(|fields| fields.display_name)
            .ok_or(CommandError::UnknownNode { id })?;
        self.model.set_display_name(id, name)?;
        self.journal.push(UndoOp::Name(id, previous.clone()));
        Ok(previous)
    }

    /// node의 modifier stack을 교체합니다(빈 stack = 제거). 이전 stack을 반환합니다.
    pub fn set_modifier_stack(
        &mut self,
        stack: ModifierStack,
    ) -> Result<Option<ModifierStack>, CommandError> {
        self.before_primitive()?;
        let owner = stack.node;
        let previous = self.model.modifier_stack(owner).cloned();
        self.model.set_modifier_stack(stack)?;
        self.journal.push(UndoOp::Stack(owner, previous.clone()));
        Ok(previous)
    }

    /// node 레코드 전체(부모·순서·표시·잠금·이름·payload, 종류 포함)를 `record`로 바꾸고 이전
    /// 레코드를 반환합니다. 자손과 modifier stack은 그대로 유지합니다(composite primitive).
    ///
    /// 이름·표시·잠금과 같은 종류의 payload 변경은 기존 runtime 핸들에서 처리합니다.
    /// 부모·순서·종류가 바뀌면 전체 문서의 자식 인덱스를 만든 뒤 해당 subtree를 다시 넣습니다.
    /// 호출 command가 결과를 먼저 검증하며, 적용 중 오류가 나면 journal이 성공한 단계를 되돌립니다.
    pub fn replace_record(&mut self, record: NodeRecord) -> Result<NodeRecord, CommandError> {
        let id = record.id;
        let previous = self
            .model
            .record(id)
            .ok_or(CommandError::UnknownNode { id })?;
        if previous == record {
            return Ok(previous);
        }
        let structural = previous.parent != record.parent
            || previous.order_key != record.order_key
            || previous.kind() != record.kind();
        if !structural {
            if previous.visible != record.visible || previous.locked != record.locked {
                self.before_primitive()?;
                let (visible, locked) =
                    self.model
                        .replace_flags(id, record.visible, record.locked)?;
                self.journal.push(UndoOp::Flags {
                    id,
                    visible,
                    locked,
                });
            }
            if previous.display_name != record.display_name {
                self.set_display_name(id, record.display_name.clone())?;
            }
            if previous.payload != record.payload {
                self.set_payload(id, record.payload)?;
            }
            return Ok(previous);
        }
        let subtree = crate::tree::ChildIndex::build(self.model).subtree(id);
        let mut detached = Vec::with_capacity(subtree.len());
        for node in subtree.iter().rev() {
            detached.push(self.remove_leaf(*node)?);
        }
        detached.reverse();
        let mut stacks = Vec::new();
        for (index, (old, stack)) in detached.into_iter().enumerate() {
            if index == 0 {
                self.insert_node(record.clone())?;
            } else {
                self.insert_node(old)?;
            }
            if let Some(stack) = stack {
                stacks.push(stack);
            }
        }
        for stack in stacks {
            self.set_modifier_stack(stack)?;
        }
        Ok(previous)
    }

    /// 지금까지의 변경 기록을 꺼냅니다(journal은 버립니다).
    pub fn into_changes(self) -> ChangeRecorder {
        self.changes
    }

    /// 성공을 확정합니다. journal을 버리고 변경 기록을 반환합니다.
    pub fn commit(self) -> ChangeRecorder {
        self.changes
    }

    /// 노드를 다시 삽입하지 않고 같은 부모 안의 OrderKey 하나를 바꿉니다.
    pub(crate) fn replace_child_order(
        &mut self,
        node: StableId,
        order: OrderKey,
    ) -> Result<(), CommandError> {
        self.before_primitive()?;
        let previous = self.model.replace_child_order(node, order)?;
        self.journal.push(UndoOp::ChildOrder(node, previous));
        Ok(())
    }

    /// 문서·페이지·View 정의(Text style 표 포함)를 바꾸고 이전 정의를 반환합니다(ADR-0079).
    pub fn replace_definition(
        &mut self,
        definition: DocumentDefinition,
    ) -> Result<DocumentDefinition, CommandError> {
        self.before_primitive()?;
        let previous = self.model.replace_definition(definition);
        self.journal
            .push(UndoOp::Definition(Box::new(previous.clone())));
        Ok(previous)
    }

    /// node의 명시 layout record를 교체합니다(빈 record = 제거). 이전 record를 반환합니다(ADR-0086).
    pub fn set_layout_record(
        &mut self,
        record: LayoutRecord,
    ) -> Result<Option<LayoutRecord>, CommandError> {
        self.before_primitive()?;
        let owner = record.node;
        let previous = self.model.set_layout_record(record)?;
        self.journal.push(UndoOp::Layout(owner, previous.clone()));
        Ok(previous)
    }

    /// 활성 기본값 세트 이름을 바꾸고 이전 이름을 반환합니다(ADR-0086).
    pub fn set_style_defaults(&mut self, name: String) -> Result<String, CommandError> {
        self.before_primitive()?;
        let previous = self.model.set_style_defaults(name);
        self.journal.push(UndoOp::StyleDefaults(previous.clone()));
        Ok(previous)
    }

    /// journal을 역순으로 재생해 context를 연 시점의 의미 상태로 되돌립니다.
    ///
    /// 각 역 primitive는 해당 primitive가 성공했던 상태에서 재생되므로 실패하지 않아야
    /// 합니다. 실패하면(내부 불일치) 나머지도 계속 재생하고 실패 수와 첫 원인을 반환합니다.
    pub fn rollback(&mut self) -> Result<(), RollbackError> {
        let mut failed_ops = 0usize;
        let mut first: Option<CommandError> = None;
        while let Some(op) = self.journal.pop() {
            if let Err(error) = undo(self.model, op) {
                failed_ops += 1;
                first.get_or_insert(error);
            }
        }
        self.changes = ChangeRecorder::new();
        match first {
            None => Ok(()),
            Some(first) => Err(RollbackError { failed_ops, first }),
        }
    }
}

fn undo(model: &mut DocumentModel, op: UndoOp) -> Result<(), CommandError> {
    match op {
        UndoOp::Remove(id) => {
            model.remove_leaf(id)?;
        }
        UndoOp::Reinsert(record, stack) => {
            model.insert_node(*record)?;
            if let Some(stack) = stack {
                model.set_modifier_stack(stack)?;
            }
        }
        UndoOp::Payload(id, payload) => model.set_payload(id, *payload)?,
        UndoOp::ChildOrder(id, key) => {
            model.replace_child_order(id, key)?;
        }
        UndoOp::Name(id, name) => model.set_display_name(id, name)?,
        UndoOp::Flags {
            id,
            visible,
            locked,
        } => {
            model.replace_flags(id, visible, locked)?;
        }
        UndoOp::Stack(owner, previous) => {
            model.set_modifier_stack(previous.unwrap_or_else(|| ModifierStack::new(owner)))?;
        }
        UndoOp::Definition(definition) => {
            model.replace_definition(*definition);
        }
        UndoOp::Layout(owner, previous) => {
            model.set_layout_record(previous.unwrap_or_else(|| LayoutRecord::new(owner)))?;
        }
        UndoOp::StyleDefaults(name) => {
            model.set_style_defaults(name);
        }
    }
    Ok(())
}

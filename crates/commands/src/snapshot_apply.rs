//! 외부에서 바뀐 문서 원본을 command 하나로 반영합니다 (UX.4.1, ADR-0079).
//!
//! 다른 프로세스(`stellar apply`, 편집기, Git checkout)가 디스크의 문서를 바꾸면, 창은 새 원본을 다시 읽어
//! [`ApplyExternalSnapshot`] **하나**로 적용합니다. Undo 한 번이 외부 변경 전체를 되돌립니다.
//!
//! - 전체 snapshot을 담지 않습니다. 현재 문서와 새 원본의 **레코드 변경 집합**([`RecordChange`] 목록)만
//!   담습니다. 각 변경은 이전 값과 새 값을 함께 가지므로 역은 두 값을 바꾼 같은 command입니다(역집합이
//!   Undo).
//! - 레코드 단위는 문서 모델이 소유하는 것입니다: 문서 이름, 페이지, View, node, modifier stack, node의 명시
//!   layout record(ADR-0086), 문서 Text style resource 표, 문서 Component Context(ADR-0099, 한 단위). 레코드는
//!   StableId로만 짝지으며 이름·geometry 유사도는 쓰지 않습니다(semantic diff XIV.3.3과 같은 식별 규칙). 컴포넌트
//!   정의·token은 문서 모델 밖(project library)이라 이 command의 대상이 아닙니다(ADR-0079 결정 2).
//! - Context는 레코드 밖 거울이라 [`ApplyExternalSnapshot::between_with_context`]가 외부 원본의 영속 Context
//!   (`ContextSet::live`)를 따로 받습니다. 변경의 이전 값은 모델 거울 전체(삭제한 provider의 provision 포함)라 Undo가
//!   그 거울을 그대로 되살립니다(ADR-0114 결정 1).
//! - 검증: 각 변경의 이전 값이 현재 문서와 같아야 하고(낡은 변경 집합 거부), 적용 결과가 전체 validator
//!   (VI.10.1)에서 새 위반을 만들지 않아야 합니다.
//! - 출처: [`ApplyExternalSnapshot::ORIGIN`]([`CommandOrigin::System`])입니다. 외부 출처는 command 종류
//!   자체가 나타내고, Undo 메뉴 이름은 [`CommandKind::action_name`](crate::kind::CommandKind::action_name)
//!   ("외부 변경 반영")이 정합니다.
//!
//! 적용 순서: 지울 node와 부모·순서·종류가 바뀌는 node를 (현재 자손과 함께) 자손부터 떼어 내고, 문서 정의를
//! 한 번에 바꾼 뒤, 새 tree의 부모부터 다시 넣고, 제자리 변경(표시·잠금·이름·payload)과 modifier stack,
//! layout record, Context를 적용합니다.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_core_types::revision::DocumentRevision;
use stellar_document::context::set::ContextSet;
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::layout_record::LayoutRecord;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::ModifierStack;
use stellar_document::modifier_metadata::InvalidationSet;
use stellar_document::record_codec::{decode_node_record, encode_node_record};
use stellar_document::records::{DocumentRecords, NodePayload, NodeRecord};
use stellar_document::text_styles::DocumentTextStyles;

use crate::change::node_keys;
use crate::codec::{CommandCodecError, Reader, Writer};
use crate::command::{Command, CommandError};
use crate::edit::EditContext;
use crate::id::{CommandId, CommandOrigin};
use crate::page_view::ensure_valid_result;
use crate::transaction::CommandTransaction;
use crate::tree::ChildIndex;

/// 페이지의 문서 안 위치와 이름.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSlot {
    /// 표시 순 위치.
    pub index: u32,
    /// 표시 이름.
    pub name: String,
}

/// View의 소속 페이지·위치·이름·root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewSlot {
    /// 소속 페이지.
    pub page: StableId,
    /// 페이지 안 표시 순 위치.
    pub index: u32,
    /// 표시 이름.
    pub name: String,
    /// root Frame node.
    pub root_frame: StableId,
}

/// 레코드 하나의 변경(이전 값 → 새 값). `None`은 그 시점에 레코드가 없음을 뜻합니다.
///
/// 변경 집합은 외부 반영 한 번에 한 번 만들고 대부분 node 변경이라, 작은 variant를 위해 node 레코드를
/// box로 감싸지 않습니다.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum RecordChange {
    /// 문서 표시 이름.
    DocumentName {
        /// 이전 이름.
        before: String,
        /// 새 이름.
        after: String,
    },
    /// 페이지 추가·삭제·위치·이름 변경.
    Page {
        /// 페이지 id.
        id: StableId,
        /// 이전 값.
        before: Option<PageSlot>,
        /// 새 값.
        after: Option<PageSlot>,
    },
    /// View 추가·삭제·위치·이름·root 변경.
    View {
        /// View id.
        id: StableId,
        /// 이전 값.
        before: Option<ViewSlot>,
        /// 새 값.
        after: Option<ViewSlot>,
    },
    /// node 레코드 추가·삭제·교체.
    Node {
        /// node id.
        id: StableId,
        /// 이전 레코드.
        before: Option<NodeRecord>,
        /// 새 레코드.
        after: Option<NodeRecord>,
    },
    /// node의 modifier stack 교체(빈 stack은 `None`).
    ModifierStack {
        /// 소유 node.
        node: StableId,
        /// 이전 stack.
        before: Option<ModifierStack>,
        /// 새 stack.
        after: Option<ModifierStack>,
    },
    /// node의 명시 layout record 교체(빈 record는 `None`, ADR-0086).
    Layout {
        /// 소유 node.
        node: StableId,
        /// 이전 record.
        before: Option<LayoutRecord>,
        /// 새 record.
        after: Option<LayoutRecord>,
    },
    /// 문서 Text style resource 표 교체.
    TextStyles {
        /// 이전 표.
        before: Box<DocumentTextStyles>,
        /// 새 표.
        after: Box<DocumentTextStyles>,
    },
    /// 문서 Component Context 교체(ADR-0099·ADR-0114). 이전 값은 모델 거울 전체입니다.
    Context {
        /// 이전 Context.
        before: Box<ContextSet>,
        /// 새 Context.
        after: Box<ContextSet>,
    },
}

impl RecordChange {
    /// 이전 값과 새 값을 바꾼 변경(역).
    pub fn inverse(&self) -> RecordChange {
        match self.clone() {
            RecordChange::DocumentName { before, after } => RecordChange::DocumentName {
                before: after,
                after: before,
            },
            RecordChange::Page { id, before, after } => RecordChange::Page {
                id,
                before: after,
                after: before,
            },
            RecordChange::View { id, before, after } => RecordChange::View {
                id,
                before: after,
                after: before,
            },
            RecordChange::Node { id, before, after } => RecordChange::Node {
                id,
                before: after,
                after: before,
            },
            RecordChange::ModifierStack {
                node,
                before,
                after,
            } => RecordChange::ModifierStack {
                node,
                before: after,
                after: before,
            },
            RecordChange::Layout {
                node,
                before,
                after,
            } => RecordChange::Layout {
                node,
                before: after,
                after: before,
            },
            RecordChange::TextStyles { before, after } => RecordChange::TextStyles {
                before: after,
                after: before,
            },
            RecordChange::Context { before, after } => RecordChange::Context {
                before: after,
                after: before,
            },
        }
    }

    /// 변경 대상 key(같은 대상의 중복 변경 검사용).
    fn target(&self) -> (u8, StableId) {
        match self {
            RecordChange::DocumentName { .. } => (TAG_DOCUMENT_NAME, StableId::NIL),
            RecordChange::Page { id, .. } => (TAG_PAGE, *id),
            RecordChange::View { id, .. } => (TAG_VIEW, *id),
            RecordChange::Node { id, .. } => (TAG_NODE, *id),
            RecordChange::ModifierStack { node, .. } => (TAG_STACK, *node),
            RecordChange::Layout { node, .. } => (TAG_LAYOUT, *node),
            RecordChange::TextStyles { .. } => (TAG_TEXT_STYLES, StableId::NIL),
            RecordChange::Context { .. } => (TAG_CONTEXT, StableId::NIL),
        }
    }
}

// 변경 tag. 저장 계약이라 바꾸거나 재사용하지 않습니다.
const TAG_DOCUMENT_NAME: u8 = 1;
const TAG_PAGE: u8 = 2;
const TAG_VIEW: u8 = 3;
const TAG_NODE: u8 = 4;
const TAG_STACK: u8 = 5;
const TAG_TEXT_STYLES: u8 = 6;
/// command codec version 4(ADR-0086 개정).
const TAG_LAYOUT: u8 = 7;
/// command codec version 6(ADR-0114).
const TAG_CONTEXT: u8 = 8;

/// 외부에서 바뀐 문서 원본을 반영하는 command.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyExternalSnapshot {
    /// 레코드 변경 집합(대상마다 하나).
    pub changes: Vec<RecordChange>,
}

fn page_slots(definition: &DocumentDefinition) -> BTreeMap<StableId, PageSlot> {
    definition
        .pages
        .iter()
        .enumerate()
        .map(|(index, page)| {
            (
                page.id,
                PageSlot {
                    index: index as u32,
                    name: page.display_name().to_string(),
                },
            )
        })
        .collect()
}

fn view_slots(definition: &DocumentDefinition) -> BTreeMap<StableId, ViewSlot> {
    let mut out = BTreeMap::new();
    for page in &definition.pages {
        for (index, view) in page.views.iter().enumerate() {
            out.insert(
                view.id,
                ViewSlot {
                    page: page.id,
                    index: index as u32,
                    name: view.display_name().to_string(),
                    root_frame: view.root_frame,
                },
            );
        }
    }
    out
}

fn diff_maps<T: Clone + PartialEq>(
    before: &BTreeMap<StableId, T>,
    after: &BTreeMap<StableId, T>,
    mut push: impl FnMut(StableId, Option<T>, Option<T>),
) {
    let ids: BTreeSet<StableId> = before.keys().chain(after.keys()).copied().collect();
    for id in ids {
        let (old, new) = (before.get(&id), after.get(&id));
        if old != new {
            push(id, old.cloned(), new.cloned());
        }
    }
}

/// Context 교체가 바꾼 key·provision·requirement·Slot 공개 목록 항목마다 무효화를 보고합니다(key는 key 경로,
/// provision은 이전·새 provider의 값 경로, 목록 항목은 소유자의 key 경로).
fn report_context_changes(ctx: &mut EditContext<'_>, before: &ContextSet, after: &ContextSet) {
    use crate::context::{CONTEXT_INVALIDATION, CONTEXT_KEY_PATH};
    let keys: BTreeSet<StableId> = before.keys().chain(after.keys()).map(|k| k.id()).collect();
    for key in keys {
        if before.key(key) != after.key(key) {
            ctx.changes().property(
                key,
                PropertyPath::with_keys(key, vec![CONTEXT_KEY_PATH]),
                CONTEXT_INVALIDATION,
            );
        }
    }
    let provisions: BTreeSet<StableId> = before
        .provisions()
        .chain(after.provisions())
        .map(|p| p.id())
        .collect();
    for id in provisions {
        let (old, new) = (before.provision(id), after.provision(id));
        if old == new {
            continue;
        }
        for provision in old.into_iter().chain(new) {
            ctx.changes().property(
                provision.provider(),
                provision.value_path(),
                CONTEXT_INVALIDATION,
            );
        }
    }
    let members = |set: &ContextSet| -> BTreeSet<(StableId, StableId)> {
        set.all_requirements()
            .chain(set.all_slot_keys())
            .flat_map(|(owner, keys)| keys.iter().map(move |key| (owner, *key)))
            .collect()
    };
    for (owner, key) in members(before).symmetric_difference(&members(after)) {
        ctx.changes().property(
            *owner,
            PropertyPath::with_keys(*owner, vec![CONTEXT_KEY_PATH])
                .with_key_segment(&key.to_string()),
            CONTEXT_INVALIDATION,
        );
    }
}

fn stale() -> CommandError {
    CommandError::InvalidRequest {
        reason: "외부 변경의 이전 값이 현재 문서와 다릅니다",
    }
}

fn malformed_definition() -> CommandError {
    CommandError::InvalidRequest {
        reason: "외부 변경 결과의 페이지·View 위치가 연속되지 않습니다",
    }
}

impl ApplyExternalSnapshot {
    /// 이 command를 담는 transaction의 출처(ADR-0079).
    pub const ORIGIN: CommandOrigin = CommandOrigin::System;

    /// 현재 문서 `model`과 새 원본 레코드 `incoming`의 레코드 변경 집합을 계산합니다. 같은 문서여야 합니다.
    /// 바뀐 것이 없으면 빈 command입니다([`ApplyExternalSnapshot::is_empty`]).
    ///
    /// Context는 비교하지 않습니다(현재 Context 그대로). 외부 원본 전체를 반영하는 제품 경로는
    /// [`ApplyExternalSnapshot::between_with_context`]나 [`ApplyExternalSnapshot::between_models`]를 씁니다.
    pub fn between(
        model: &DocumentModel,
        incoming: &DocumentRecords,
    ) -> Result<ApplyExternalSnapshot, CommandError> {
        ApplyExternalSnapshot::between_records(model, incoming)
    }

    /// 현재 문서와 외부 원본 문서 모델(레코드 + 영속 Context)의 변경 집합입니다.
    pub fn between_models(
        model: &DocumentModel,
        incoming: &DocumentModel,
    ) -> Result<ApplyExternalSnapshot, CommandError> {
        ApplyExternalSnapshot::between_with_context(
            model,
            &incoming.to_records()?,
            &incoming.context().live(incoming),
        )
    }

    /// 레코드 변경 집합에 Context 변경을 더합니다. `context`는 외부 원본의 영속 Context(`ContextSet::live` 결과)이고,
    /// 현재 모델의 영속 Context와 다를 때만 변경 하나(이전 값 = 모델 거울 전체)를 더합니다(ADR-0114 결정 1).
    pub fn between_with_context(
        model: &DocumentModel,
        incoming: &DocumentRecords,
        context: &ContextSet,
    ) -> Result<ApplyExternalSnapshot, CommandError> {
        let mut command = ApplyExternalSnapshot::between_records(model, incoming)?;
        if model.context().live(model) != *context {
            command.changes.push(RecordChange::Context {
                before: Box::new(model.context().clone()),
                after: Box::new(context.clone()),
            });
        }
        Ok(command)
    }

    fn between_records(
        model: &DocumentModel,
        incoming: &DocumentRecords,
    ) -> Result<ApplyExternalSnapshot, CommandError> {
        let current = model.definition();
        let next = &incoming.definition;
        if current.id != next.id {
            return Err(CommandError::InvalidRequest {
                reason: "외부 원본이 다른 문서입니다",
            });
        }
        let mut changes = Vec::new();
        if current.display_name() != next.display_name() {
            changes.push(RecordChange::DocumentName {
                before: current.display_name().to_string(),
                after: next.display_name().to_string(),
            });
        }
        diff_maps(
            &page_slots(current),
            &page_slots(next),
            |id, before, after| changes.push(RecordChange::Page { id, before, after }),
        );
        diff_maps(
            &view_slots(current),
            &view_slots(next),
            |id, before, after| changes.push(RecordChange::View { id, before, after }),
        );
        let records = model.to_records()?;
        let by_id = |nodes: &[NodeRecord]| -> BTreeMap<StableId, NodeRecord> {
            nodes.iter().map(|n| (n.id, n.clone())).collect()
        };
        diff_maps(
            &by_id(&records.nodes),
            &by_id(&incoming.nodes),
            |id, before, after| changes.push(RecordChange::Node { id, before, after }),
        );
        let stacks = |stacks: &[ModifierStack]| -> BTreeMap<StableId, ModifierStack> {
            stacks
                .iter()
                .filter(|s| !s.entries().is_empty())
                .map(|s| (s.node, s.clone()))
                .collect()
        };
        diff_maps(
            &stacks(&records.modifier_stacks),
            &stacks(&incoming.modifier_stacks),
            |node, before, after| {
                changes.push(RecordChange::ModifierStack {
                    node,
                    before,
                    after,
                })
            },
        );
        let layouts = |records: &[LayoutRecord]| -> BTreeMap<StableId, LayoutRecord> {
            records
                .iter()
                .filter(|r| !r.is_empty())
                .map(|r| (r.node, r.clone()))
                .collect()
        };
        diff_maps(
            &layouts(&records.layout_records),
            &layouts(&incoming.layout_records),
            |node, before, after| {
                changes.push(RecordChange::Layout {
                    node,
                    before,
                    after,
                })
            },
        );
        if current.text_styles != next.text_styles {
            changes.push(RecordChange::TextStyles {
                before: Box::new(current.text_styles.clone()),
                after: Box::new(next.text_styles.clone()),
            });
        }
        Ok(ApplyExternalSnapshot { changes })
    }

    /// 바뀐 것이 없는지.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// 이 command 하나를 담은 transaction(출처 [`ApplyExternalSnapshot::ORIGIN`]).
    pub fn into_transaction(self, id: CommandId, base: DocumentRevision) -> CommandTransaction {
        CommandTransaction::new(id, Self::ORIGIN)
            .with_command(base, Command::ApplyExternalSnapshot(self))
    }

    fn definition_changed(&self) -> bool {
        self.changes.iter().any(|change| {
            matches!(
                change,
                RecordChange::DocumentName { .. }
                    | RecordChange::Page { .. }
                    | RecordChange::View { .. }
                    | RecordChange::TextStyles { .. }
            )
        })
    }

    /// 현재 정의에 정의 변경을 적용한 결과. 이전 값이 현재와 다르거나 위치가 연속되지 않으면 오류입니다.
    fn next_definition(
        &self,
        current: &DocumentDefinition,
    ) -> Result<DocumentDefinition, CommandError> {
        let mut pages = page_slots(current);
        let mut views = view_slots(current);
        let mut name = current.display_name().to_string();
        let mut text_styles = current.text_styles.clone();
        for change in &self.changes {
            match change {
                RecordChange::DocumentName { before, after } => {
                    if *before != name {
                        return Err(stale());
                    }
                    name = after.clone();
                }
                RecordChange::Page { id, before, after } => {
                    if pages.get(id) != before.as_ref() {
                        return Err(stale());
                    }
                    match after {
                        Some(slot) => pages.insert(*id, slot.clone()),
                        None => pages.remove(id),
                    };
                }
                RecordChange::View { id, before, after } => {
                    if views.get(id) != before.as_ref() {
                        return Err(stale());
                    }
                    match after {
                        Some(slot) => views.insert(*id, slot.clone()),
                        None => views.remove(id),
                    };
                }
                RecordChange::TextStyles { before, after } => {
                    if **before != text_styles {
                        return Err(stale());
                    }
                    text_styles = (**after).clone();
                }
                RecordChange::Node { .. }
                | RecordChange::ModifierStack { .. }
                | RecordChange::Layout { .. }
                | RecordChange::Context { .. } => {}
            }
        }
        let mut ordered: Vec<(u32, StableId, String)> = pages
            .into_iter()
            .map(|(id, slot)| (slot.index, id, slot.name))
            .collect();
        ordered.sort();
        let mut definition = DocumentDefinition::new(current.id, name);
        definition.text_styles = text_styles;
        for (position, (index, id, page_name)) in ordered.into_iter().enumerate() {
            if index as usize != position {
                return Err(malformed_definition());
            }
            definition
                .add_page(PageDefinition::new(id, page_name))
                .map_err(|_| malformed_definition())?;
        }
        let mut placed: Vec<(StableId, u32, StableId, ViewSlot)> = views
            .into_iter()
            .map(|(id, slot)| (slot.page, slot.index, id, slot))
            .collect();
        placed.sort_by_key(|a| (a.0, a.1, a.2));
        for (page, index, id, slot) in placed {
            let page = definition.page_mut(page).ok_or_else(malformed_definition)?;
            if index as usize != page.views.len() {
                return Err(malformed_definition());
            }
            page.add_view(ViewDefinition::new(id, slot.name, slot.root_frame))
                .map_err(|_| malformed_definition())?;
        }
        Ok(definition)
    }

    fn check_shape(&self) -> Result<(), CommandError> {
        if self.changes.is_empty() {
            return Err(CommandError::InvalidRequest {
                reason: "외부 변경 집합이 비어 있습니다",
            });
        }
        let mut targets = BTreeSet::new();
        for change in &self.changes {
            if !targets.insert(change.target()) {
                return Err(CommandError::InvalidRequest {
                    reason: "외부 변경 집합에 같은 대상이 두 번 있습니다",
                });
            }
            let consistent = match change {
                RecordChange::Node { id, before, after } => {
                    before.as_ref().is_none_or(|r| r.id == *id)
                        && after.as_ref().is_none_or(|r| r.id == *id)
                        && (before.is_some() || after.is_some())
                }
                RecordChange::ModifierStack {
                    node,
                    before,
                    after,
                } => {
                    let ok = |s: &Option<ModifierStack>| {
                        s.as_ref()
                            .is_none_or(|s| s.node == *node && !s.entries().is_empty())
                    };
                    ok(before) && ok(after) && (before.is_some() || after.is_some())
                }
                RecordChange::Layout {
                    node,
                    before,
                    after,
                } => {
                    let ok = |r: &Option<LayoutRecord>| {
                        r.as_ref().is_none_or(|r| r.node == *node && !r.is_empty())
                    };
                    ok(before) && ok(after) && (before.is_some() || after.is_some())
                }
                RecordChange::Page { before, after, .. } => before.is_some() || after.is_some(),
                RecordChange::View { before, after, .. } => before.is_some() || after.is_some(),
                RecordChange::DocumentName { .. }
                | RecordChange::TextStyles { .. }
                | RecordChange::Context { .. } => true,
            };
            if !consistent {
                return Err(CommandError::InvalidRequest {
                    reason: "외부 변경 항목의 형식이 맞지 않습니다",
                });
            }
        }
        Ok(())
    }

    /// 형식과 이전 값 일치만 확인하고 결과 정의를 돌려줍니다(전체 결과 검사 없음).
    fn check_preconditions(
        &self,
        model: &DocumentModel,
    ) -> Result<DocumentDefinition, CommandError> {
        self.check_shape()?;
        let definition = self.next_definition(model.definition())?;
        for change in &self.changes {
            let current = match change {
                RecordChange::Node { id, before, .. } => model.record(*id) == *before,
                RecordChange::ModifierStack { node, before, .. } => {
                    model.modifier_stack(*node) == before.as_ref()
                }
                RecordChange::Layout { node, before, .. } => {
                    model.layout_record(*node) == before.as_ref()
                }
                RecordChange::Context { before, .. } => model.context() == before.as_ref(),
                _ => true,
            };
            if !current {
                return Err(stale());
            }
        }
        Ok(definition)
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        let definition = self.check_preconditions(model)?;
        ensure_valid_result(model, |records| {
            records.definition = definition;
            let mut nodes: BTreeMap<StableId, NodeRecord> =
                records.nodes.drain(..).map(|n| (n.id, n)).collect();
            let mut stacks: BTreeMap<StableId, ModifierStack> = records
                .modifier_stacks
                .drain(..)
                .map(|s| (s.node, s))
                .collect();
            let mut layouts: BTreeMap<StableId, LayoutRecord> = records
                .layout_records
                .drain(..)
                .map(|r| (r.node, r))
                .collect();
            for change in &self.changes {
                match change {
                    RecordChange::Node { id, after, .. } => match after {
                        Some(record) => {
                            nodes.insert(*id, record.clone());
                        }
                        None => {
                            nodes.remove(id);
                        }
                    },
                    RecordChange::ModifierStack { node, after, .. } => match after {
                        Some(stack) => {
                            stacks.insert(*node, stack.clone());
                        }
                        None => {
                            stacks.remove(node);
                        }
                    },
                    RecordChange::Layout { node, after, .. } => match after {
                        Some(record) => {
                            layouts.insert(*node, record.clone());
                        }
                        None => {
                            layouts.remove(node);
                        }
                    },
                    _ => {}
                }
            }
            // 지운 node의 stack이 남으면 validator가 소유자 없는 stack으로 거부합니다.
            records.nodes = nodes.into_values().collect();
            records.modifier_stacks = stacks.into_values().collect();
            // 지운 node의 record가 남으면 validator가 소유자 없는 record로 거부합니다.
            records.layout_records = layouts.into_values().collect();
            Ok(())
        })
    }

    pub(crate) fn invert(&self) -> ApplyExternalSnapshot {
        ApplyExternalSnapshot {
            changes: self.changes.iter().map(RecordChange::inverse).collect(),
        }
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        // 실행기가 apply 전에 validate(전체 결과 검사 포함)를 부르므로 여기서는 이전 값만 다시 봅니다.
        self.check_preconditions(ctx.model())?;
        let mut node_after: BTreeMap<StableId, Option<&NodeRecord>> = BTreeMap::new();
        let mut stack_after: BTreeMap<StableId, Option<&ModifierStack>> = BTreeMap::new();
        let mut layout_after: BTreeMap<StableId, Option<&LayoutRecord>> = BTreeMap::new();
        for change in &self.changes {
            match change {
                RecordChange::Node { id, after, .. } => {
                    node_after.insert(*id, after.as_ref());
                }
                RecordChange::ModifierStack { node, after, .. } => {
                    stack_after.insert(*node, after.as_ref());
                }
                RecordChange::Layout { node, after, .. } => {
                    layout_after.insert(*node, after.as_ref());
                }
                _ => {}
            }
        }

        // 1. 떼어 낼 node: 지우거나 부모·순서·종류가 바뀌는 node와 그 현재 자손.
        let index = ChildIndex::build(ctx.model());
        let mut detach: BTreeSet<StableId> = BTreeSet::new();
        let mut order: Vec<StableId> = Vec::new();
        for (id, after) in &node_after {
            let Some(current) = ctx.model().record(*id) else {
                continue;
            };
            let structural = match after {
                None => true,
                Some(next) => {
                    next.parent != current.parent
                        || next.order_key != current.order_key
                        || next.kind() != current.kind()
                }
            };
            if structural && !detach.contains(id) {
                for node in index.subtree(*id) {
                    if detach.insert(node) {
                        order.push(node);
                    }
                }
            }
        }
        // 자손부터 떼어 냅니다(깊은 node 먼저). 깊이가 같으면 순서는 결과에 영향이 없습니다.
        let depth = |id: StableId| crate::tree::depth_of(ctx.model(), id).unwrap_or(0);
        let mut by_depth: Vec<(u32, StableId)> = order.iter().map(|id| (depth(*id), *id)).collect();
        by_depth.sort_by(|a, b| b.cmp(a));
        let mut detached: BTreeMap<StableId, (NodeRecord, Option<ModifierStack>)> = BTreeMap::new();
        for (_, id) in by_depth {
            let removed = ctx.remove_leaf(id)?;
            detached.insert(id, removed);
        }

        // 2. 문서 정의.
        if self.definition_changed() {
            let definition = self.next_definition(ctx.model().definition())?;
            ctx.replace_definition(definition)?;
            for change in &self.changes {
                let (id, key) = match change {
                    RecordChange::DocumentName { .. } => {
                        (ctx.model().document_id(), node_keys::DISPLAY_NAME)
                    }
                    RecordChange::Page { id, .. } | RecordChange::View { id, .. } => {
                        (*id, node_keys::ORDER_KEY)
                    }
                    // Text style 표 변경의 무효화는 끝에서 Text node마다 보고합니다.
                    _ => continue,
                };
                ctx.changes().property(
                    id,
                    PropertyPath::with_keys(id, vec![key]),
                    InvalidationSet::EMPTY,
                );
            }
        }

        // 3. 새 tree의 부모부터 넣습니다: 떼어 낸 node 중 남는 것 + 새 node.
        let mut pending: BTreeMap<StableId, NodeRecord> = BTreeMap::new();
        for (id, (record, _)) in &detached {
            match node_after.get(id) {
                Some(None) => {}
                Some(Some(next)) => {
                    pending.insert(*id, (*next).clone());
                }
                None => {
                    pending.insert(*id, record.clone());
                }
            }
        }
        for (id, after) in &node_after {
            if let Some(next) = after {
                if !ctx.model().contains(*id) && !detached.contains_key(id) {
                    pending.insert(*id, (*next).clone());
                }
            }
        }
        let mut children: BTreeMap<StableId, Vec<StableId>> = BTreeMap::new();
        let mut queue: VecDeque<StableId> = VecDeque::new();
        for (id, record) in &pending {
            match record.parent {
                Some(parent) if pending.contains_key(&parent) => {
                    children.entry(parent).or_default().push(*id)
                }
                _ => queue.push_back(*id),
            }
        }
        let mut inserted: Vec<StableId> = Vec::with_capacity(pending.len());
        let mut inserted_set: BTreeSet<StableId> = BTreeSet::new();
        while let Some(id) = queue.pop_front() {
            let record = pending
                .remove(&id)
                .ok_or(CommandError::UnknownNode { id })?;
            ctx.insert_node(record)?;
            inserted.push(id);
            inserted_set.insert(id);
            if let Some(next) = children.remove(&id) {
                queue.extend(next);
            }
        }
        if !pending.is_empty() {
            return Err(CommandError::InvalidRequest {
                reason: "외부 변경 결과의 node tree에 순환이 있습니다",
            });
        }
        for id in &inserted {
            match detached.get(id) {
                Some(_) => ctx.changes().structure(*id),
                None => ctx.changes().inserted(*id),
            }
            let stack = match stack_after.get(id) {
                Some(after) => after.cloned(),
                None => detached.get(id).and_then(|(_, stack)| stack.clone()),
            };
            if let Some(stack) = stack {
                ctx.set_modifier_stack(stack)?;
            }
        }
        for id in detached.keys() {
            if matches!(node_after.get(id), Some(None)) {
                ctx.changes().removed(*id);
            }
        }

        // 4. 제자리 변경(떼어 내지 않은 node).
        for (id, after) in &node_after {
            if let Some(next) = after {
                if !detached.contains_key(id) && !inserted_set.contains(id) {
                    ctx.replace_record((*next).clone())?;
                    ctx.changes().structure(*id);
                }
            }
        }

        // 5. 떼어 내지 않은 node의 modifier stack.
        for (node, after) in &stack_after {
            if detached.contains_key(node) || inserted_set.contains(node) {
                continue;
            }
            let stack = after.cloned().unwrap_or_else(|| ModifierStack::new(*node));
            ctx.set_modifier_stack(stack)?;
            ctx.changes().structure(*node);
        }

        // 6. layout record. 떼어 냈다 다시 넣은 node는 모델이 StableId로 들고 있던 record를 그대로 되찾습니다
        //    (ADR-0086 결정 5). 새로 넣은 node에 이 session에서 지운 같은 id의 record가 남아 있으면, 외부 원본에
        //    없는 값이므로 지웁니다. 변경 집합에 있는 record는 그 값으로 바꿉니다. 지운 node의 record는 모델에만
        //    남고 영속 레코드에는 나오지 않습니다.
        for id in &inserted {
            if !detached.contains_key(id)
                && !layout_after.contains_key(id)
                && ctx.model().layout_record(*id).is_some()
            {
                ctx.set_layout_record(LayoutRecord::new(*id))?;
            }
        }
        for (node, after) in &layout_after {
            if !ctx.model().contains(*node) {
                // validate가 소유자 없는 record를 거부하므로 여기 오는 변경의 새 값은 언제나 `None`입니다.
                continue;
            }
            let record = after.cloned().unwrap_or_else(|| LayoutRecord::new(*node));
            ctx.set_layout_record(record)?;
            ctx.changes().structure(*node);
        }

        // 7. 문서 Text style 표(글꼴 등록·문자·문단 style)가 바뀌면 모든 Text node의 layout·그리기가
        //    달라질 수 있으므로 Text node마다 구조 무효화를 보고합니다(외부 반영은 드문 동작이라 O(N) 허용).
        if self
            .changes
            .iter()
            .any(|change| matches!(change, RecordChange::TextStyles { .. }))
        {
            for id in crate::tree::node_ids(ctx.model()) {
                if matches!(ctx.model().payload(id), Some(NodePayload::Text(_))) {
                    ctx.changes().structure(id);
                }
            }
        }

        // 8. Context. 바뀐 key·provision·requirement·Slot 공개 목록마다 Context 편집 command와 같은 무효화를
        //    보고합니다.
        for change in &self.changes {
            if let RecordChange::Context { before, after } = change {
                ctx.replace_context((**after).clone())?;
                report_context_changes(ctx, before, after);
            }
        }
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        fn opt<T>(out: &mut Writer, value: &Option<T>, write: impl FnOnce(&mut Writer, &T)) {
            match value {
                None => out.u8(0),
                Some(value) => {
                    out.u8(1);
                    write(out, value);
                }
            }
        }
        let page = |out: &mut Writer, slot: &PageSlot| {
            out.u32(slot.index);
            out.str(&slot.name);
        };
        let view = |out: &mut Writer, slot: &ViewSlot| {
            out.id(slot.page);
            out.u32(slot.index);
            out.str(&slot.name);
            out.id(slot.root_frame);
        };
        let node = |out: &mut Writer, record: &NodeRecord| out.bytes(&encode_node_record(record));
        let stack = |out: &mut Writer, stack: &ModifierStack| out.bytes(&stack.encode());
        let layout = |out: &mut Writer, record: &LayoutRecord| out.bytes(&record.encode());
        out.len(self.changes.len());
        for change in &self.changes {
            match change {
                RecordChange::DocumentName { before, after } => {
                    out.u8(TAG_DOCUMENT_NAME);
                    out.str(before);
                    out.str(after);
                }
                RecordChange::Page { id, before, after } => {
                    out.u8(TAG_PAGE);
                    out.id(*id);
                    opt(out, before, page);
                    opt(out, after, page);
                }
                RecordChange::View { id, before, after } => {
                    out.u8(TAG_VIEW);
                    out.id(*id);
                    opt(out, before, view);
                    opt(out, after, view);
                }
                RecordChange::Node { id, before, after } => {
                    out.u8(TAG_NODE);
                    out.id(*id);
                    opt(out, before, node);
                    opt(out, after, node);
                }
                RecordChange::ModifierStack {
                    node: owner,
                    before,
                    after,
                } => {
                    out.u8(TAG_STACK);
                    out.id(*owner);
                    opt(out, before, stack);
                    opt(out, after, stack);
                }
                RecordChange::Layout {
                    node: owner,
                    before,
                    after,
                } => {
                    out.u8(TAG_LAYOUT);
                    out.id(*owner);
                    opt(out, before, layout);
                    opt(out, after, layout);
                }
                RecordChange::TextStyles { before, after } => {
                    out.u8(TAG_TEXT_STYLES);
                    out.bytes(&before.encode());
                    out.bytes(&after.encode());
                }
                RecordChange::Context { before, after } => {
                    out.u8(TAG_CONTEXT);
                    out.bytes(&before.encode());
                    out.bytes(&after.encode());
                }
            }
        }
    }

    pub(crate) fn decode_from(
        input: &mut Reader<'_>,
    ) -> Result<ApplyExternalSnapshot, CommandCodecError> {
        fn opt<T>(
            input: &mut Reader<'_>,
            read: impl FnOnce(&mut Reader<'_>) -> Result<T, CommandCodecError>,
        ) -> Result<Option<T>, CommandCodecError> {
            if input.bool()? {
                read(input).map(Some)
            } else {
                Ok(None)
            }
        }
        fn page(input: &mut Reader<'_>) -> Result<PageSlot, CommandCodecError> {
            Ok(PageSlot {
                index: input.u32()?,
                name: input.string()?,
            })
        }
        fn view(input: &mut Reader<'_>) -> Result<ViewSlot, CommandCodecError> {
            Ok(ViewSlot {
                page: input.id()?,
                index: input.u32()?,
                name: input.string()?,
                root_frame: input.id()?,
            })
        }
        fn node(input: &mut Reader<'_>) -> Result<NodeRecord, CommandCodecError> {
            decode_node_record(input.bytes()?).map_err(|_| CommandCodecError::Malformed {
                reason: "잘못된 node 레코드",
            })
        }
        fn stack(input: &mut Reader<'_>) -> Result<ModifierStack, CommandCodecError> {
            ModifierStack::decode(input.bytes()?).map_err(|_| CommandCodecError::Malformed {
                reason: "잘못된 modifier stack",
            })
        }
        fn layout(input: &mut Reader<'_>) -> Result<LayoutRecord, CommandCodecError> {
            LayoutRecord::decode(input.bytes()?).map_err(|_| CommandCodecError::Malformed {
                reason: "잘못된 layout record",
            })
        }
        fn styles(input: &mut Reader<'_>) -> Result<Box<DocumentTextStyles>, CommandCodecError> {
            DocumentTextStyles::decode(input.bytes()?)
                .map(Box::new)
                .map_err(|_| CommandCodecError::Malformed {
                    reason: "잘못된 Text style 표",
                })
        }
        fn context(input: &mut Reader<'_>) -> Result<Box<ContextSet>, CommandCodecError> {
            ContextSet::decode(input.bytes()?)
                .map(Box::new)
                .map_err(|_| CommandCodecError::Malformed {
                    reason: "잘못된 Context",
                })
        }
        let count = input.len()?;
        let mut changes = Vec::with_capacity(count);
        for _ in 0..count {
            changes.push(match input.u8()? {
                TAG_DOCUMENT_NAME => RecordChange::DocumentName {
                    before: input.string()?,
                    after: input.string()?,
                },
                TAG_PAGE => RecordChange::Page {
                    id: input.id()?,
                    before: opt(input, page)?,
                    after: opt(input, page)?,
                },
                TAG_VIEW => RecordChange::View {
                    id: input.id()?,
                    before: opt(input, view)?,
                    after: opt(input, view)?,
                },
                TAG_NODE => RecordChange::Node {
                    id: input.id()?,
                    before: opt(input, node)?,
                    after: opt(input, node)?,
                },
                TAG_STACK => RecordChange::ModifierStack {
                    node: input.id()?,
                    before: opt(input, stack)?,
                    after: opt(input, stack)?,
                },
                TAG_LAYOUT => RecordChange::Layout {
                    node: input.id()?,
                    before: opt(input, layout)?,
                    after: opt(input, layout)?,
                },
                TAG_TEXT_STYLES => RecordChange::TextStyles {
                    before: styles(input)?,
                    after: styles(input)?,
                },
                TAG_CONTEXT => RecordChange::Context {
                    before: context(input)?,
                    after: context(input)?,
                },
                _ => {
                    return Err(CommandCodecError::Malformed {
                        reason: "알 수 없는 외부 변경 tag",
                    })
                }
            });
        }
        Ok(ApplyExternalSnapshot { changes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{decode_command, encode_command};
    use crate::command::test_fixtures::{definition, frame, sid, small_model};
    use crate::id::CommandId;
    use crate::state::DocumentState;
    use stellar_core_types::order_key::OrderKey;
    use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierKind, ModifierPayload};
    use stellar_document::test_support::random_document::{random_valid_document, SplitMix64};
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
    use stellar_document::validate::validate_document;

    fn snapshot(model: &DocumentModel) -> String {
        model_snapshot_json(model).unwrap()
    }

    /// `display` 하나를 명시한 Frame layout record.
    fn layout(node: u64, display: &str) -> LayoutRecord {
        use stellar_document::layout_record::LayoutLonghand;
        let mut record = LayoutRecord::new(sid(node));
        record
            .set(
                LayoutLonghand::Display,
                LayoutLonghand::Display.parse(display).unwrap(),
            )
            .unwrap();
        record
    }

    fn layouts(model: &DocumentModel) -> Vec<LayoutRecord> {
        model.to_records().unwrap().layout_records
    }

    #[test]
    fn external_layout_record_changes_are_applied_and_undone() {
        let mut current = small_model();
        current.set_layout_record(layout(11, "flex")).unwrap();
        current.set_layout_record(layout(12, "block")).unwrap();
        let mut state = DocumentState::new(current);
        let original = layouts(state.model());
        // 외부: 11 값 변경, 12 record 삭제, 13을 root 아래로 옮기고 record 추가, 새 node 20과 그 record.
        let mut incoming = state.model().to_records().unwrap();
        incoming.layout_records = vec![layout(11, "grid"), layout(13, "flex"), layout(20, "grid")];
        for node in &mut incoming.nodes {
            if node.id == sid(13) {
                node.parent = Some(sid(10));
                node.order_key = OrderKey::from_bytes(vec![0x30]).unwrap();
            }
        }
        incoming.nodes.push(frame(20, Some(10), 0x40));
        incoming.sort_canonical();
        let command = ApplyExternalSnapshot::between(state.model(), &incoming).unwrap();
        let layout_changes = command
            .changes
            .iter()
            .filter(|c| matches!(c, RecordChange::Layout { .. }))
            .count();
        assert_eq!(layout_changes, 4);
        let transaction =
            command.into_transaction(CommandId::from_stable_id(sid(901)), state.revision());
        let applied = state.execute_transaction(&transaction).unwrap();
        assert_eq!(layouts(state.model()), incoming.layout_records);
        assert_eq!(state.model().to_records().unwrap(), incoming);
        state.execute_transaction(&applied.inverse).unwrap();
        assert_eq!(layouts(state.model()), original);
    }

    #[test]
    fn external_layout_changes_check_previous_values_and_owners() {
        let mut current = small_model();
        current.set_layout_record(layout(11, "flex")).unwrap();
        let mut state = DocumentState::new(current);
        // 이전 값이 다르면 낡은 변경입니다.
        let stale_change = ApplyExternalSnapshot {
            changes: vec![RecordChange::Layout {
                node: sid(11),
                before: Some(layout(11, "grid")),
                after: None,
            }],
        };
        assert_eq!(stale_change.validate(state.model()), Err(stale()));
        // 소유 node가 없는 record는 결과 검사가 거부합니다.
        let orphan = ApplyExternalSnapshot {
            changes: vec![RecordChange::Layout {
                node: sid(77),
                before: None,
                after: Some(layout(77, "flex")),
            }],
        };
        assert!(matches!(
            orphan.validate(state.model()),
            Err(CommandError::InvalidResult(_))
        ));
        // 소유 node와 다른 record, 빈 record는 형식 오류입니다.
        for bad in [
            RecordChange::Layout {
                node: sid(11),
                before: Some(layout(11, "flex")),
                after: Some(layout(12, "flex")),
            },
            RecordChange::Layout {
                node: sid(11),
                before: Some(layout(11, "flex")),
                after: Some(LayoutRecord::new(sid(11))),
            },
        ] {
            assert!(ApplyExternalSnapshot { changes: vec![bad] }
                .validate(state.model())
                .is_err());
        }
        // 이 session에서 지운 node와 같은 id를 외부가 record 없이 다시 만들면 남은 record가 보이지 않습니다.
        state
            .execute(&crate::request::CommandRequest::at(
                &state,
                Command::DeleteSubtree(crate::node_ops::DeleteSubtree { root: sid(11) }),
            ))
            .unwrap();
        let mut incoming = state.model().to_records().unwrap();
        incoming.nodes.push(frame(11, Some(10), 0x10));
        incoming.sort_canonical();
        let command = ApplyExternalSnapshot::between(state.model(), &incoming).unwrap();
        state
            .execute(&crate::request::CommandRequest::at(
                &state,
                Command::ApplyExternalSnapshot(command),
            ))
            .unwrap();
        assert!(state.model().layout_record(sid(11)).is_none());
        assert_eq!(state.model().to_records().unwrap(), incoming);
    }

    /// 다른 프로세스가 쓴 새 원본: 이름·삭제·이동·추가·modifier·페이지/View 변경을 섞습니다.
    fn external_version() -> DocumentRecords {
        let mut records = small_model().to_records().unwrap();
        // 페이지·View 이름 변경, 새 View.
        let mut def = definition();
        def.rename("외부 문서".into());
        def.pages[0].rename("페이지".into());
        def.pages[0].views[0].rename("홈".into());
        def.pages[0]
            .add_view(ViewDefinition::new(sid(30), "새 View".into(), sid(31)))
            .unwrap();
        records.definition = def;
        // 12 삭제, 13을 11 아래에서 root 아래로 이동, 11 이름 변경, 새 View root 31과 자식 32.
        records.nodes.retain(|n| n.id != sid(12));
        for node in &mut records.nodes {
            if node.id == sid(13) {
                node.parent = Some(sid(10));
                node.order_key = OrderKey::from_bytes(vec![0x30]).unwrap();
            }
            if node.id == sid(11) {
                node.display_name = "바뀐 이름".into();
                node.visible = false;
            }
        }
        records.nodes.push(frame(31, None, 0x10));
        records.nodes.push(frame(32, Some(31), 0x10));
        let mut stack = ModifierStack::new(sid(11));
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(sid(40)),
                order: OrderKey::between(None, None),
                kind: ModifierKind::Opacity,
                enabled: true,
                payload: ModifierPayload::default_for(ModifierKind::Opacity),
            })
            .unwrap();
        records.modifier_stacks.push(stack);
        records.sort_canonical();
        records
    }

    fn reload(records: &DocumentRecords) -> DocumentModel {
        DocumentModel::from_records(records.clone()).unwrap().0
    }

    #[test]
    fn external_change_is_one_undoable_command() {
        let incoming = external_version();
        validate_document(&incoming).unwrap();
        let mut state = DocumentState::new(small_model());
        let original = snapshot(state.model());
        let command = ApplyExternalSnapshot::between(state.model(), &incoming).unwrap();
        assert!(!command.is_empty());
        // 전체 snapshot이 아니라 바뀐 레코드만 담습니다(10·13 이외의 node 10은 그대로).
        assert!(!command
            .changes
            .iter()
            .any(|c| matches!(c, RecordChange::Node { id, .. } if *id == sid(10))));
        let transaction = command
            .clone()
            .into_transaction(CommandId::from_stable_id(sid(900)), state.revision());
        assert_eq!(transaction.origin, ApplyExternalSnapshot::ORIGIN);
        let applied = state.execute_transaction(&transaction).unwrap();
        assert_eq!(snapshot(state.model()), snapshot(&reload(&incoming)));
        let changes = &applied.changes;
        assert!(changes.removed_nodes().contains(&sid(12)));
        assert!(changes.inserted_nodes().contains(&sid(31)));
        // Undo 한 번(역 transaction)으로 원래 문서.
        state.execute_transaction(&applied.inverse).unwrap();
        assert_eq!(snapshot(state.model()), original);
        // Redo.
        let again = transaction.rebased(state.revision());
        state.execute_transaction(&again).unwrap();
        assert_eq!(snapshot(state.model()), snapshot(&reload(&incoming)));
    }

    #[test]
    fn stale_or_invalid_change_sets_are_rejected_without_change() {
        let incoming = external_version();
        let mut state = DocumentState::new(small_model());
        let command = ApplyExternalSnapshot::between(state.model(), &incoming).unwrap();
        // 같은 node를 먼저 바꾸면 이전 값이 달라 낡은 변경입니다.
        let rename = Command::rename_node(sid(11), "앱에서 바꿈");
        state
            .execute(&crate::request::CommandRequest::at(&state, rename))
            .unwrap();
        let before = snapshot(state.model());
        let request =
            crate::request::CommandRequest::at(&state, Command::ApplyExternalSnapshot(command));
        assert_eq!(state.execute(&request).unwrap_err(), stale());
        assert_eq!(snapshot(state.model()), before);
        // 빈 변경, 결과가 무효(부모 없는 node).
        let state_model = state.model();
        assert!(ApplyExternalSnapshot { changes: vec![] }
            .validate(state_model)
            .is_err());
        let orphan = ApplyExternalSnapshot {
            changes: vec![RecordChange::Node {
                id: sid(77),
                before: None,
                after: Some(frame(77, Some(76), 0x10)),
            }],
        };
        assert!(matches!(
            orphan.validate(state_model),
            Err(CommandError::InvalidResult(_))
        ));
        // 위치가 비는 페이지 배치.
        let gap = ApplyExternalSnapshot {
            changes: vec![RecordChange::Page {
                id: sid(78),
                before: None,
                after: Some(PageSlot {
                    index: 5,
                    name: String::new(),
                }),
            }],
        };
        assert_eq!(gap.validate(state_model), Err(malformed_definition()));
    }

    #[test]
    fn codec_round_trips_every_change_tag() {
        let mut state = DocumentState::new(small_model());
        let mut incoming = external_version();
        incoming.definition.text_styles = DocumentTextStyles::initial(sid(90));
        incoming.layout_records.push(layout(11, "grid"));
        let command = ApplyExternalSnapshot::between_with_context(
            state.model(),
            &incoming,
            &context_with(&[(500, "theme", Some(13))]),
        )
        .unwrap();
        let tags: BTreeSet<u8> = command.changes.iter().map(|c| c.target().0).collect();
        assert_eq!(tags, (1..=8).collect());
        let command = Command::ApplyExternalSnapshot(command);
        let bytes = encode_command(&command);
        assert_eq!(&bytes[..2], &0x0070u16.to_le_bytes());
        assert_eq!(decode_command(&bytes).unwrap(), command);
        for cut in 0..bytes.len() {
            assert!(decode_command(&bytes[..cut]).is_err(), "절단 {cut}");
        }
        let request = crate::request::CommandRequest::at(&state, command);
        let outcome = state.execute(&request).unwrap();
        assert_eq!(snapshot(state.model()), snapshot(&reload(&incoming)));
        let Command::ApplyExternalSnapshot(inverse) = &outcome.inverse else {
            panic!("역도 외부 변경 반영");
        };
        assert_eq!(inverse.changes.len(), request_len(&request));
    }

    #[test]
    fn text_style_table_change_invalidates_text_nodes() {
        // Text style 표만 바뀐 외부 원본도 Text node의 다시 그리기(구조 무효화)를 보고합니다.
        use stellar_document::records::NodePayload;
        use stellar_document::text::TextPayload;
        let mut model = small_model();
        let mut text = frame(20, Some(10), 0x70);
        text.payload = NodePayload::Text(TextPayload::empty(0));
        model.insert_node(text).unwrap();
        let mut state = DocumentState::new(model);
        let mut incoming = state.model().to_records().unwrap();
        incoming.definition.text_styles = DocumentTextStyles::initial(sid(90));
        let command = ApplyExternalSnapshot::between(state.model(), &incoming).unwrap();
        assert_eq!(command.changes.len(), 1);
        let request =
            crate::request::CommandRequest::at(&state, Command::ApplyExternalSnapshot(command));
        let outcome = state.execute(&request).unwrap();
        assert!(outcome.changes.changed_nodes().contains(&sid(20)));
        assert!(
            !outcome.changes.changed_nodes().contains(&sid(11)),
            "Text 아닌 node는 그대로"
        );
    }

    /// key(번호, 이름)와 그 key의 View provision(provider 번호, 없으면 provision 없음)을 담은 Context.
    fn context_with(items: &[(u64, &str, Option<u64>)]) -> ContextSet {
        use stellar_core_types::typed_value::TypedValue;
        use stellar_core_types::value::FiniteF32;
        use stellar_document::context::key::{ContextKey, ContextScope};
        use stellar_document::context::provision::{ContextOwner, ContextProvision};
        use stellar_document::expressions::ast::{ExprNode, ExprNodeId, ExpressionAst};
        use stellar_document::expressions::types::{ExpressionType, TypeSpec};
        let number = |seed: u64, value: f32| {
            let id = ExprNodeId::new(sid(seed));
            ExpressionAst::new(
                id,
                vec![ExprNode::literal(
                    id,
                    TypedValue::Number(FiniteF32::new(value).unwrap()),
                )],
            )
            .unwrap()
        };
        let mut set = ContextSet::new();
        for (key, name, provider) in items {
            set.insert_key(
                ContextKey::new(
                    sid(*key),
                    (*name).to_owned(),
                    ContextScope::Document(sid(1)),
                    TypeSpec::required(ExpressionType::Number),
                    number(key + 1, 1.0),
                )
                .unwrap(),
            )
            .unwrap();
            if let Some(provider) = provider {
                set.insert_provision(
                    ContextProvision::new(
                        sid(key + 2),
                        ContextOwner::View(sid(3)),
                        sid(*provider),
                        sid(*key),
                        number(key + 3, 2.0),
                    )
                    .unwrap(),
                )
                .unwrap();
            }
        }
        set
    }

    #[test]
    fn external_context_change_is_applied_and_undone_with_the_whole_mirror() {
        // 밖에서 바꾼 Context 원본(ADR-0114 결정 1): 열린 창에 반영되고 Undo 한 번으로 거울 전체가 돌아옵니다.
        let mut model = small_model();
        model.replace_context(context_with(&[
            (500, "theme", Some(13)),
            (510, "size", Some(12)),
        ]));
        let mut state = DocumentState::new(model);
        // 창에서 12를 지우면 그 provision은 거울에만 남습니다(삭제 undo용).
        state
            .execute(&crate::request::CommandRequest::at(
                &state,
                Command::DeleteSubtree(crate::node_ops::DeleteSubtree { root: sid(12) }),
            ))
            .unwrap();
        let mirror = state.model().context().clone();
        assert!(mirror.provision(sid(512)).is_some(), "거울에는 남음");
        assert!(state
            .model()
            .context()
            .live(state.model())
            .provision(sid(512))
            .is_none());
        let incoming_records = state.model().to_records().unwrap();
        // 같은 영속 Context면 변경이 없습니다(거울에만 있는 provision은 비교하지 않음).
        let same = state.model().context().live(state.model());
        assert!(ApplyExternalSnapshot::between_with_context(
            state.model(),
            &incoming_records,
            &same
        )
        .unwrap()
        .is_empty());
        // 외부: theme key 이름 변경과 provision 13 삭제, size key 삭제, 새 key gap과 provision 11.
        let incoming = context_with(&[(500, "brand", None), (520, "gap", Some(11))]);
        let command = ApplyExternalSnapshot::between_with_context(
            state.model(),
            &incoming_records,
            &incoming,
        )
        .unwrap();
        assert_eq!(command.changes.len(), 1);
        let transaction =
            command.into_transaction(CommandId::from_stable_id(sid(902)), state.revision());
        let applied = state.execute_transaction(&transaction).unwrap();
        assert_eq!(state.model().context(), &incoming);
        let changed = applied.changes.changed_nodes();
        for object in [sid(500), sid(510), sid(520), sid(13), sid(11)] {
            assert!(changed.contains(&object), "{object} 무효화 보고");
        }
        state.execute_transaction(&applied.inverse).unwrap();
        assert_eq!(
            state.model().context(),
            &mirror,
            "Undo가 거울 전체를 되살림"
        );
    }

    #[test]
    fn external_context_change_checks_the_previous_mirror() {
        let mut model = small_model();
        model.replace_context(context_with(&[(500, "theme", Some(13))]));
        let state = DocumentState::new(model);
        let stale_change = ApplyExternalSnapshot {
            changes: vec![RecordChange::Context {
                before: Box::new(ContextSet::new()),
                after: Box::new(context_with(&[(530, "x", None)])),
            }],
        };
        assert_eq!(stale_change.validate(state.model()), Err(stale()));
        // 모델 둘 사이 계산(레코드 + 영속 Context)도 같은 변경을 냅니다.
        let mut other = small_model();
        other.replace_context(context_with(&[(530, "x", None)]));
        let command = ApplyExternalSnapshot::between_models(state.model(), &other).unwrap();
        assert!(matches!(
            command.changes.as_slice(),
            [RecordChange::Context { after, .. }] if **after == context_with(&[(530, "x", None)])
        ));
    }

    fn request_len(request: &crate::request::CommandRequest) -> usize {
        match &request.command {
            Command::ApplyExternalSnapshot(command) => command.changes.len(),
            _ => 0,
        }
    }

    #[test]
    fn random_external_versions_round_trip() {
        // 무작위 문서 두 개 사이는 다른 문서라 거부하고, 한 문서에 무작위 command를 적용한 결과를
        // 외부 원본으로 삼아 apply → undo → redo 왕복을 확인합니다.
        let mut applied = 0;
        for seed in 0..60 {
            let base = random_valid_document(seed);
            let mut edited = DocumentState::new(random_valid_document(seed));
            let mut rng = SplitMix64::new(seed ^ 0xE5);
            let mut ids = crate::test_support::FreshIds::new(seed);
            for _ in 0..6 {
                if let Some(request) =
                    crate::test_support::random_valid_request(&edited, &mut rng, &mut ids, 16)
                {
                    let _ = edited.execute(&request);
                }
            }
            let incoming = edited.model().to_records().unwrap();
            let mut state = DocumentState::new(base);
            let original = snapshot(state.model());
            let command = ApplyExternalSnapshot::between(state.model(), &incoming).unwrap();
            if command.is_empty() {
                continue;
            }
            let request =
                crate::request::CommandRequest::at(&state, Command::ApplyExternalSnapshot(command));
            let outcome = state
                .execute(&request)
                .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
            assert_eq!(
                snapshot(state.model()),
                snapshot(edited.model()),
                "seed {seed}"
            );
            let undo = crate::request::CommandRequest::at(&state, outcome.inverse);
            state.execute(&undo).unwrap();
            assert_eq!(snapshot(state.model()), original, "seed {seed} undo");
            applied += 1;
        }
        assert!(applied > 40, "적용 {applied}");
    }
}

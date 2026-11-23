//! Page·View 추가·삭제·이름 변경 command (UX.12.1, ADR-0079).
//!
//! 문서 정의(VI.1)의 페이지·View 목록을 바꾸는 편집도 다른 문서 편집과 같은 command 경로를 지납니다.
//! 추가 하나가 history 항목 하나이며 Undo 한 번으로 되돌아갑니다.
//!
//! - [`AddPage`]: 페이지 하나와 그 View들(각 View의 root Frame과 subtree)을 표시 순 위치에 넣습니다.
//!   역은 [`RemovePage`]입니다.
//! - [`AddView`]: 기존 페이지의 표시 순 위치에 View 하나(root Frame과 subtree)를 넣습니다. 역은
//!   [`RemoveView`]입니다.
//! - [`RemovePage`]·[`RemoveView`]: 대상과 그 안의 node를 모두 지웁니다. 남은 node가 지울 node를
//!   참조하면 거부합니다(VI.2.5, [`CommandError::ExternalReference`]). 역은 지운 내용을 그대로 담은
//!   [`AddPage`]·[`AddView`]입니다. 문서 모델은 View 없는 페이지와 페이지 없는 문서를 허용하므로(VI.1)
//!   command도 막지 않습니다. 모든 command의 역이 항상 적용 가능해야 하기 때문입니다. "마지막 View는
//!   지우지 않음" 같은 편집기 규칙은 UI가 정합니다.
//! - [`RenamePage`]·[`RenameView`]: 표시 이름만 바꿉니다. id·root·참조는 그대로입니다(VI.1.3).
//!
//! View 하나의 원본 파일 단위(ADR-0059)와 맞도록 View 추가·삭제는 root Frame subtree를 통째로
//! 다룹니다. 추가 command의 결과 문서는 전체 validator(VI.10.1)로 검사해 **새로 생기는 위반**이 있으면
//! [`CommandError::InvalidResult`]로 거부합니다. 정의 편집은 드문 사용자 동작이라 O(N) 검사를 허용합니다.
//!
//! 바뀐 정의 객체는 change set에 그 객체 StableId를 root로 한 property path(`DISPLAY_NAME`·
//! `PARENT`·`ORDER_KEY` key)로 보고합니다. 추가·삭제한 node는 inserted·removed로 보고합니다.

use std::collections::BTreeSet;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::frame::FramePayload;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::ModifierStack;
use stellar_document::modifier_metadata::InvalidationSet;
use stellar_document::record_codec::{decode_node_record, encode_node_record};
use stellar_document::records::{DocumentRecords, NodePayload, NodeRecord};
use stellar_document::validate::{validate_document, DocumentValidationError, DocumentViolation};

use crate::change::node_keys;
use crate::codec::{CommandCodecError, Reader, Writer};
use crate::command::CommandError;
use crate::edit::EditContext;
use crate::tree::ChildIndex;

/// View 하나와 그 안의 내용: 정의, root Frame부터 pre-order인 node 레코드, modifier stack.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewContent {
    /// View 정의(`root_frame`이 `nodes[0].id`).
    pub view: ViewDefinition,
    /// root Frame부터 pre-order. 부모가 앞선 레코드여야 합니다.
    pub nodes: Vec<NodeRecord>,
    /// 이 View node들의 modifier stack(빈 stack은 넣지 않습니다).
    pub modifier_stacks: Vec<ModifierStack>,
}

impl ViewContent {
    /// root Frame 하나만 가진 빈 View.
    pub fn empty(view: StableId, name: impl Into<String>, root: StableId) -> ViewContent {
        ViewContent {
            view: ViewDefinition::new(view, name.into(), root),
            nodes: vec![view_root_record(root)],
            modifier_stacks: Vec::new(),
        }
    }

    /// 모델의 View 하나를 내용째 읽습니다.
    fn capture(model: &DocumentModel, view: &ViewDefinition) -> Result<ViewContent, CommandError> {
        let index = ChildIndex::build(model);
        Self::capture_with(model, &index, view)
    }

    fn capture_with(
        model: &DocumentModel,
        index: &ChildIndex,
        view: &ViewDefinition,
    ) -> Result<ViewContent, CommandError> {
        let mut nodes = Vec::new();
        let mut modifier_stacks = Vec::new();
        if model.contains(view.root_frame) {
            for id in index.subtree(view.root_frame) {
                nodes.push(model.record(id).ok_or(CommandError::UnknownNode { id })?);
                if let Some(stack) = model.modifier_stack(id) {
                    modifier_stacks.push(stack.clone());
                }
            }
        }
        Ok(ViewContent {
            view: view.clone(),
            nodes,
            modifier_stacks,
        })
    }

    fn check_shape(&self) -> Result<(), CommandError> {
        let Some(root) = self.nodes.first() else {
            return Err(CommandError::InvalidRequest {
                reason: "View에 root Frame 레코드가 없습니다",
            });
        };
        if root.id != self.view.root_frame || root.parent.is_some() {
            return Err(CommandError::InvalidRequest {
                reason: "View 내용의 첫 레코드는 부모 없는 root Frame이어야 합니다",
            });
        }
        if !matches!(root.payload, NodePayload::Frame(_)) {
            return Err(CommandError::RootKind {
                node: root.id,
                found: root.kind(),
            });
        }
        let mut seen = BTreeSet::new();
        for record in &self.nodes {
            if record.parent.is_some_and(|parent| !seen.contains(&parent)) {
                return Err(CommandError::InvalidParent {
                    node: record.id,
                    parent: record.parent,
                });
            }
            if !seen.insert(record.id) {
                return Err(CommandError::DuplicateId { id: record.id });
            }
        }
        let mut owners = BTreeSet::new();
        for stack in &self.modifier_stacks {
            if !seen.contains(&stack.node) || !owners.insert(stack.node) {
                return Err(CommandError::InvalidRequest {
                    reason: "modifier stack 소유 node가 View 내용에 없거나 중복입니다",
                });
            }
        }
        Ok(())
    }

    fn insert_into(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        for record in &self.nodes {
            ctx.insert_node(record.clone())?;
            ctx.changes().inserted(record.id);
        }
        for stack in &self.modifier_stacks {
            if !stack.entries().is_empty() {
                ctx.set_modifier_stack(stack.clone())?;
            }
        }
        Ok(())
    }

    fn remove_from(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        for record in self.nodes.iter().rev() {
            ctx.remove_leaf(record.id)?;
            ctx.changes().removed(record.id);
        }
        Ok(())
    }

    fn encode_into(&self, out: &mut Writer) {
        encode_view(out, &self.view);
        out.len(self.nodes.len());
        for record in &self.nodes {
            out.bytes(&encode_node_record(record));
        }
        out.len(self.modifier_stacks.len());
        for stack in &self.modifier_stacks {
            out.bytes(&stack.encode());
        }
    }

    fn decode_from(input: &mut Reader<'_>) -> Result<ViewContent, CommandCodecError> {
        let view = decode_view(input)?;
        let count = input.len()?;
        let mut nodes = Vec::with_capacity(count);
        for _ in 0..count {
            nodes.push(decode_node_record(input.bytes()?).map_err(|_| {
                CommandCodecError::Malformed {
                    reason: "잘못된 node 레코드",
                }
            })?);
        }
        let count = input.len()?;
        let mut modifier_stacks = Vec::with_capacity(count);
        for _ in 0..count {
            modifier_stacks.push(ModifierStack::decode(input.bytes()?).map_err(|_| {
                CommandCodecError::Malformed {
                    reason: "잘못된 modifier stack",
                }
            })?);
        }
        Ok(ViewContent {
            view,
            nodes,
            modifier_stacks,
        })
    }

    fn add_to_records(&self, records: &mut DocumentRecords) {
        records.nodes.extend(self.nodes.iter().cloned());
        records.modifier_stacks.extend(
            self.modifier_stacks
                .iter()
                .filter(|stack| !stack.entries().is_empty())
                .cloned(),
        );
    }
}

/// 새 View의 root Frame 레코드(기본 Frame payload, 표시·잠금 해제).
pub fn view_root_record(id: StableId) -> NodeRecord {
    NodeRecord {
        id,
        parent: None,
        order_key: OrderKey::between(None, None),
        visible: true,
        locked: false,
        display_name: String::new(),
        payload: NodePayload::Frame(FramePayload::default()),
    }
}

fn encode_view(out: &mut Writer, view: &ViewDefinition) {
    out.id(view.id);
    out.str(view.display_name());
    out.id(view.root_frame);
}

fn decode_view(input: &mut Reader<'_>) -> Result<ViewDefinition, CommandCodecError> {
    let id = input.id()?;
    let name = input.string()?;
    let root = input.id()?;
    Ok(ViewDefinition::new(id, name, root))
}

fn index_of(index: u32) -> usize {
    index as usize
}

fn definition_error(error: stellar_document::definitions::DefinitionError) -> CommandError {
    use stellar_document::definitions::DefinitionError;
    match error {
        DefinitionError::DuplicateId { id } => CommandError::DuplicateId { id },
        DefinitionError::NotFound { id } => CommandError::UnknownNode { id },
        DefinitionError::IndexOutOfRange { .. } => CommandError::InvalidRequest {
            reason: "표시 순 위치가 목록 길이보다 큽니다",
        },
        _ => CommandError::InvalidRequest {
            reason: "문서 정의 변경이 유효하지 않습니다",
        },
    }
}

/// 현재 문서 레코드에 `edit`를 적용한 결과가 전체 validator(VI.10.1)에서 **새 위반**을 만들지 않는지
/// 확인합니다. 문서를 바꾸지 않습니다. 이미 있던 위반은 이 command의 책임이 아니므로 결과에 위반이 있을
/// 때만 변경 전 문서를 다시 검사해 새로 생긴 위반만 보고합니다.
pub(crate) fn ensure_valid_result(
    model: &DocumentModel,
    edit: impl FnOnce(&mut DocumentRecords) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    let original = model.to_records()?;
    let mut records = original.clone();
    edit(&mut records)?;
    let Err(after) = validate_document(&records) else {
        return Ok(());
    };
    let existing = validate_document(&original)
        .err()
        .map(|error| error.violations)
        .unwrap_or_default();
    let introduced: Vec<DocumentViolation> = after
        .violations
        .into_iter()
        .filter(|violation| !existing.contains(violation))
        .collect();
    if introduced.is_empty() {
        Ok(())
    } else {
        Err(CommandError::InvalidResult(Box::new(
            DocumentValidationError {
                violations: introduced,
            },
        )))
    }
}

/// `members` 밖 node가 `members` 안 node를 참조하면 거부합니다(VI.2.5).
fn reject_external_references(
    model: &DocumentModel,
    members: &BTreeSet<StableId>,
) -> Result<(), CommandError> {
    let references = model.references();
    for target in members {
        if let Some(from) = references
            .inbound(*target)
            .into_iter()
            .find(|from| !members.contains(from))
        {
            return Err(CommandError::ExternalReference {
                target: *target,
                from,
            });
        }
    }
    Ok(())
}

fn report_definition(
    ctx: &mut EditContext<'_>,
    id: StableId,
    keys: &[stellar_core_types::property::PropertyKey],
) {
    for key in keys {
        ctx.changes().property(
            id,
            stellar_core_types::property::PropertyPath::with_keys(id, vec![*key]),
            InvalidationSet::EMPTY,
        );
    }
}

fn with_definition(
    ctx: &mut EditContext<'_>,
    change: impl FnOnce(&mut DocumentDefinition) -> Result<(), CommandError>,
) -> Result<(), CommandError> {
    let mut definition = ctx.model().definition().clone();
    change(&mut definition)?;
    ctx.replace_definition(definition)?;
    Ok(())
}

// --- AddPage / RemovePage ---

/// 페이지 하나와 그 View들을 표시 순 위치에 넣습니다.
#[derive(Debug, Clone, PartialEq)]
pub struct AddPage {
    /// 문서 안 표시 순 위치(0 = 맨 앞, 페이지 수 = 맨 뒤).
    pub index: u32,
    /// 새 페이지 id.
    pub page: StableId,
    /// 표시 이름.
    pub name: String,
    /// 페이지의 View들(표시 순).
    pub views: Vec<ViewContent>,
}

impl AddPage {
    /// 빈 View 하나를 가진 새 페이지(`stellar` UI의 "새 Page" 동작).
    pub fn with_empty_view(
        index: u32,
        page: StableId,
        name: impl Into<String>,
        view: StableId,
        view_name: impl Into<String>,
        root: StableId,
    ) -> AddPage {
        AddPage {
            index,
            page,
            name: name.into(),
            views: vec![ViewContent::empty(view, view_name, root)],
        }
    }

    fn page_definition(&self) -> PageDefinition {
        let mut page = PageDefinition::new(self.page, self.name.clone());
        page.views = self
            .views
            .iter()
            .map(|content| content.view.clone())
            .collect();
        page
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        if self.page == StableId::NIL {
            return Err(CommandError::NilId);
        }
        if model.contains_persistent_id(self.page) {
            return Err(CommandError::DuplicateId { id: self.page });
        }
        if index_of(self.index) > model.definition().pages.len() {
            return Err(CommandError::InvalidRequest {
                reason: "표시 순 위치가 목록 길이보다 큽니다",
            });
        }
        for content in &self.views {
            content.check_shape()?;
        }
        ensure_valid_result(model, |records| {
            records
                .definition
                .insert_page(index_of(self.index), self.page_definition())
                .map_err(definition_error)?;
            for content in &self.views {
                content.add_to_records(records);
            }
            Ok(())
        })
    }

    pub(crate) fn invert(&self) -> RemovePage {
        RemovePage { page: self.page }
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let page = self.page_definition();
        with_definition(ctx, |definition| {
            definition
                .insert_page(index_of(self.index), page)
                .map_err(definition_error)
        })?;
        report_definition(
            ctx,
            self.page,
            &[node_keys::DISPLAY_NAME, node_keys::ORDER_KEY],
        );
        for content in &self.views {
            content.insert_into(ctx)?;
            report_definition(
                ctx,
                content.view.id,
                &[node_keys::DISPLAY_NAME, node_keys::PARENT],
            );
        }
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.u32(self.index);
        out.id(self.page);
        out.str(&self.name);
        out.len(self.views.len());
        for content in &self.views {
            content.encode_into(out);
        }
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<AddPage, CommandCodecError> {
        let index = input.u32()?;
        let page = input.id()?;
        let name = input.string()?;
        let count = input.len()?;
        let mut views = Vec::with_capacity(count);
        for _ in 0..count {
            views.push(ViewContent::decode_from(input)?);
        }
        Ok(AddPage {
            index,
            page,
            name,
            views,
        })
    }
}

/// 페이지와 그 안의 View·node를 모두 지웁니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovePage {
    /// 지울 페이지.
    pub page: StableId,
}

impl RemovePage {
    fn located<'a>(
        &self,
        model: &'a DocumentModel,
    ) -> Result<(usize, &'a PageDefinition), CommandError> {
        model
            .definition()
            .pages
            .iter()
            .enumerate()
            .find(|(_, page)| page.id == self.page)
            .ok_or(CommandError::UnknownNode { id: self.page })
    }

    fn contents(&self, model: &DocumentModel) -> Result<(usize, AddPage), CommandError> {
        let (index, page) = self.located(model)?;
        let tree = ChildIndex::build(model);
        let views = page
            .views
            .iter()
            .map(|view| ViewContent::capture_with(model, &tree, view))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((
            index,
            AddPage {
                index: u32::try_from(index).map_err(|_| CommandError::InvalidRequest {
                    reason: "페이지 위치가 너무 큽니다",
                })?,
                page: page.id,
                name: page.display_name().to_string(),
                views,
            },
        ))
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        let (_, contents) = self.contents(model)?;
        let members: BTreeSet<StableId> = contents
            .views
            .iter()
            .flat_map(|content| content.nodes.iter().map(|record| record.id))
            .collect();
        reject_external_references(model, &members)
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<AddPage, CommandError> {
        self.validate(model)?;
        Ok(self.contents(model)?.1)
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let (_, contents) = self.contents(ctx.model())?;
        for content in contents.views.iter().rev() {
            content.remove_from(ctx)?;
        }
        with_definition(ctx, |definition| {
            definition
                .remove_page(self.page)
                .map(|_| ())
                .map_err(definition_error)
        })?;
        report_definition(ctx, self.page, &[node_keys::ORDER_KEY]);
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.page);
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<RemovePage, CommandCodecError> {
        Ok(RemovePage { page: input.id()? })
    }
}

/// 페이지 표시 이름을 바꿉니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamePage {
    /// 대상 페이지.
    pub page: StableId,
    /// 새 표시 이름.
    pub name: String,
}

impl RenamePage {
    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        model
            .definition()
            .page(self.page)
            .map(|_| ())
            .ok_or(CommandError::UnknownNode { id: self.page })
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<RenamePage, CommandError> {
        let page = model
            .definition()
            .page(self.page)
            .ok_or(CommandError::UnknownNode { id: self.page })?;
        Ok(RenamePage {
            page: self.page,
            name: page.display_name().to_string(),
        })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        with_definition(ctx, |definition| {
            definition
                .page_mut(self.page)
                .ok_or(CommandError::UnknownNode { id: self.page })?
                .rename(self.name.clone());
            Ok(())
        })?;
        report_definition(ctx, self.page, &[node_keys::DISPLAY_NAME]);
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.page);
        out.str(&self.name);
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<RenamePage, CommandCodecError> {
        Ok(RenamePage {
            page: input.id()?,
            name: input.string()?,
        })
    }
}

// --- AddView / RemoveView / RenameView ---

/// 기존 페이지의 표시 순 위치에 View 하나를 넣습니다.
#[derive(Debug, Clone, PartialEq)]
pub struct AddView {
    /// 대상 페이지.
    pub page: StableId,
    /// 페이지 안 표시 순 위치(0 = 맨 앞, View 수 = 맨 뒤).
    pub index: u32,
    /// View와 그 내용.
    pub content: ViewContent,
}

impl AddView {
    /// root Frame 하나만 가진 빈 View를 넣습니다(`stellar` UI의 "새 View" 동작).
    pub fn empty(
        page: StableId,
        index: u32,
        view: StableId,
        name: impl Into<String>,
        root: StableId,
    ) -> AddView {
        AddView {
            page,
            index,
            content: ViewContent::empty(view, name, root),
        }
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        let view = self.content.view.id;
        if view == StableId::NIL {
            return Err(CommandError::NilId);
        }
        if model.contains_persistent_id(view) {
            return Err(CommandError::DuplicateId { id: view });
        }
        let page = model
            .definition()
            .page(self.page)
            .ok_or(CommandError::UnknownNode { id: self.page })?;
        if index_of(self.index) > page.views.len() {
            return Err(CommandError::InvalidRequest {
                reason: "표시 순 위치가 목록 길이보다 큽니다",
            });
        }
        self.content.check_shape()?;
        ensure_valid_result(model, |records| {
            records
                .definition
                .page_mut(self.page)
                .ok_or(CommandError::UnknownNode { id: self.page })?
                .insert_view(index_of(self.index), self.content.view.clone())
                .map_err(definition_error)?;
            self.content.add_to_records(records);
            Ok(())
        })
    }

    pub(crate) fn invert(&self) -> RemoveView {
        RemoveView {
            view: self.content.view.id,
        }
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        with_definition(ctx, |definition| {
            definition
                .page_mut(self.page)
                .ok_or(CommandError::UnknownNode { id: self.page })?
                .insert_view(index_of(self.index), self.content.view.clone())
                .map_err(definition_error)
        })?;
        self.content.insert_into(ctx)?;
        report_definition(
            ctx,
            self.content.view.id,
            &[
                node_keys::DISPLAY_NAME,
                node_keys::PARENT,
                node_keys::ORDER_KEY,
            ],
        );
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.page);
        out.u32(self.index);
        self.content.encode_into(out);
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<AddView, CommandCodecError> {
        Ok(AddView {
            page: input.id()?,
            index: input.u32()?,
            content: ViewContent::decode_from(input)?,
        })
    }
}

/// View와 그 안의 node를 모두 지웁니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveView {
    /// 지울 View.
    pub view: StableId,
}

impl RemoveView {
    fn located(&self, model: &DocumentModel) -> Result<(usize, usize), CommandError> {
        model
            .definition()
            .view_position(self.view)
            .ok_or(CommandError::UnknownNode { id: self.view })
    }

    fn contents(&self, model: &DocumentModel) -> Result<AddView, CommandError> {
        let (page_index, view_index) = self.located(model)?;
        let page = &model.definition().pages[page_index];
        Ok(AddView {
            page: page.id,
            index: u32::try_from(view_index).map_err(|_| CommandError::InvalidRequest {
                reason: "View 위치가 너무 큽니다",
            })?,
            content: ViewContent::capture(model, &page.views[view_index])?,
        })
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        let contents = self.contents(model)?;
        let members: BTreeSet<StableId> = contents
            .content
            .nodes
            .iter()
            .map(|record| record.id)
            .collect();
        reject_external_references(model, &members)
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<AddView, CommandError> {
        self.validate(model)?;
        self.contents(model)
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        let contents = self.contents(ctx.model())?;
        contents.content.remove_from(ctx)?;
        with_definition(ctx, |definition| {
            definition
                .page_mut(contents.page)
                .ok_or(CommandError::UnknownNode { id: contents.page })?
                .remove_view(self.view)
                .map(|_| ())
                .map_err(definition_error)
        })?;
        report_definition(ctx, contents.page, &[node_keys::ORDER_KEY]);
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.view);
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<RemoveView, CommandCodecError> {
        Ok(RemoveView { view: input.id()? })
    }
}

/// View 표시 이름을 바꿉니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameView {
    /// 대상 View.
    pub view: StableId,
    /// 새 표시 이름.
    pub name: String,
}

impl RenameView {
    fn current<'a>(&self, model: &'a DocumentModel) -> Result<&'a ViewDefinition, CommandError> {
        let (page, view) = model
            .definition()
            .view_position(self.view)
            .ok_or(CommandError::UnknownNode { id: self.view })?;
        Ok(&model.definition().pages[page].views[view])
    }

    pub(crate) fn validate(&self, model: &DocumentModel) -> Result<(), CommandError> {
        self.current(model).map(|_| ())
    }

    pub(crate) fn invert(&self, model: &DocumentModel) -> Result<RenameView, CommandError> {
        Ok(RenameView {
            view: self.view,
            name: self.current(model)?.display_name().to_string(),
        })
    }

    pub(crate) fn apply(&self, ctx: &mut EditContext<'_>) -> Result<(), CommandError> {
        with_definition(ctx, |definition| {
            definition
                .view_mut(self.view)
                .ok_or(CommandError::UnknownNode { id: self.view })?
                .rename(self.name.clone());
            Ok(())
        })?;
        report_definition(ctx, self.view, &[node_keys::DISPLAY_NAME]);
        Ok(())
    }

    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.id(self.view);
        out.str(&self.name);
    }

    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<RenameView, CommandCodecError> {
        Ok(RenameView {
            view: input.id()?,
            name: input.string()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{decode_command, encode_command};
    use crate::command::test_fixtures::{frame, sid, small_model};
    use crate::command::Command;
    use crate::request::CommandRequest;
    use crate::state::DocumentState;
    use stellar_document::test_support::semantic_snapshot::model_snapshot_json;

    fn snapshot(state: &DocumentState) -> String {
        model_snapshot_json(state.model()).unwrap()
    }

    fn run(state: &mut DocumentState, command: Command) -> Result<Command, CommandError> {
        let request = CommandRequest::at(state, command);
        state.execute(&request).map(|outcome| outcome.inverse)
    }

    /// apply → undo → redo 뒤 semantic snapshot이 각 단계 기대값과 같은지 확인합니다.
    fn round_trip(state: &mut DocumentState, command: Command) -> String {
        let before = snapshot(state);
        let inverse = run(state, command.clone()).unwrap();
        let after = snapshot(state);
        validate_document(&state.model().to_records().unwrap()).unwrap();
        let redo = run(state, inverse).unwrap();
        assert_eq!(snapshot(state), before, "undo 뒤 원래 문서");
        assert_eq!(redo, command, "역의 역은 원래 command");
        run(state, redo).unwrap();
        assert_eq!(snapshot(state), after, "redo 뒤 적용 결과");
        after
    }

    fn codec_round_trip(command: &Command) {
        let bytes = encode_command(command);
        assert_eq!(&decode_command(&bytes).unwrap(), command);
        assert_eq!(encode_command(&decode_command(&bytes).unwrap()), bytes);
        for cut in 0..bytes.len() {
            assert!(decode_command(&bytes[..cut]).is_err(), "절단 {cut}");
        }
    }

    #[test]
    fn add_view_and_page_are_single_undoable_commands() {
        let mut state = DocumentState::new(small_model());
        let add_view = Command::AddView(AddView::empty(sid(2), 1, sid(40), "두 번째", sid(41)));
        round_trip(&mut state, add_view.clone());
        let definition = state.model().definition();
        assert_eq!(definition.pages[0].views.len(), 2);
        assert_eq!(definition.pages[0].views[1].display_name(), "두 번째");
        assert_eq!(definition.pages[0].views[1].root_frame, sid(41));
        assert!(state.model().contains(sid(41)));

        let add_page = Command::AddPage(AddPage::with_empty_view(
            0,
            sid(50),
            "앞 페이지",
            sid(51),
            "View",
            sid(52),
        ));
        round_trip(&mut state, add_page);
        let definition = state.model().definition();
        assert_eq!(definition.pages.len(), 2);
        assert_eq!(definition.pages[0].id, sid(50));
        assert_eq!(definition.pages[0].display_name(), "앞 페이지");
        assert_eq!(definition.pages[1].id, sid(2));
        codec_round_trip(&add_view);
    }

    #[test]
    fn rename_page_and_view_keep_ids_and_round_trip() {
        let mut state = DocumentState::new(small_model());
        let rename_page = Command::RenamePage(RenamePage {
            page: sid(2),
            name: "표지 🎨".into(),
        });
        round_trip(&mut state, rename_page.clone());
        assert_eq!(
            state.model().definition().pages[0].display_name(),
            "표지 🎨"
        );
        let rename_view = Command::RenameView(RenameView {
            view: sid(3),
            name: "홈".into(),
        });
        round_trip(&mut state, rename_view.clone());
        let view = &state.model().definition().pages[0].views[0];
        assert_eq!(
            (view.id, view.display_name(), view.root_frame),
            (sid(3), "홈", sid(10))
        );
        codec_round_trip(&rename_page);
        codec_round_trip(&rename_view);
        assert_eq!(
            run(
                &mut state,
                Command::RenameView(RenameView {
                    view: sid(99),
                    name: String::new()
                })
            ),
            Err(CommandError::UnknownNode { id: sid(99) })
        );
    }

    #[test]
    fn remove_view_and_page_restore_whole_subtrees_on_undo() {
        let mut state = DocumentState::new(small_model());
        // 페이지의 유일한 View도 지울 수 있고, 역은 내용째 되돌립니다.
        let original = snapshot(&state);
        let inverse = run(&mut state, Command::RemoveView(RemoveView { view: sid(3) })).unwrap();
        assert!(state.model().definition().pages[0].views.is_empty());
        assert_eq!(state.model().node_count(), 0);
        run(&mut state, inverse).unwrap();
        assert_eq!(snapshot(&state), original);
        run(
            &mut state,
            Command::AddPage(AddPage::with_empty_view(
                1,
                sid(60),
                "둘",
                sid(61),
                "",
                sid(62),
            )),
        )
        .unwrap();
        let before = snapshot(&state);
        // 페이지 1의 View(root 10, 자손 11·12·13)를 내용째 지웠다가 되돌립니다.
        let inverse = run(&mut state, Command::RemovePage(RemovePage { page: sid(2) })).unwrap();
        assert!(!state.model().contains(sid(13)));
        let Command::AddPage(restore) = &inverse else {
            panic!("역은 AddPage");
        };
        assert_eq!(restore.index, 0);
        assert_eq!(restore.views[0].nodes.len(), 4);
        codec_round_trip(&inverse);
        run(&mut state, inverse).unwrap();
        assert_eq!(snapshot(&state), before);
    }

    #[test]
    fn add_rejects_bad_content_without_changing_the_document() {
        let mut state = DocumentState::new(small_model());
        let before = snapshot(&state);
        // 이미 쓰는 id, 없는 페이지, 범위 밖 위치, root가 아닌 첫 레코드.
        for command in [
            Command::AddView(AddView::empty(sid(2), 0, sid(3), "", sid(70))),
            Command::AddView(AddView::empty(sid(2), 0, sid(70), "", sid(11))),
            Command::AddView(AddView::empty(sid(99), 0, sid(70), "", sid(71))),
            Command::AddView(AddView::empty(sid(2), 5, sid(70), "", sid(71))),
            Command::AddView(AddView {
                page: sid(2),
                index: 0,
                content: ViewContent {
                    view: ViewDefinition::new(sid(70), String::new(), sid(71)),
                    nodes: vec![frame(71, Some(10), 0x40)],
                    modifier_stacks: Vec::new(),
                },
            }),
            Command::AddPage(AddPage {
                index: 3,
                page: sid(70),
                name: String::new(),
                views: Vec::new(),
            }),
            Command::AddPage(AddPage {
                index: 0,
                page: sid(3),
                name: String::new(),
                views: Vec::new(),
            }),
            Command::RemoveView(RemoveView { view: sid(99) }),
            Command::RemovePage(RemovePage { page: sid(3) }),
        ] {
            assert!(run(&mut state, command.clone()).is_err(), "{command:?}");
        }
        assert_eq!(snapshot(&state), before);
    }
}

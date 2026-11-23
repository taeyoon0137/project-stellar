//! 캔버스 Text 편집기 코어 (XII.10.33, ADR-0118 결정 1·2·3).
//!
//! - **만들기.** Text 도구 클릭은 자동 폭(`width: max-content`), 끌기는 고정 폭(`width: <끈 너비>px`) Text 자리를
//!   정합니다([`plan_text_tool`]). 새 Text는 첫 확정 입력 때 `InsertNode` 하나로 만들고(폭 layout record 포함), 입력
//!   없이 편집을 끝내면 아무것도 기록하지 않습니다(history 0건).
//! - **편집 모드.** 선택 엔진의 내용 편집 요청(더블클릭·Return, `SessionOutput::EditContent`)으로
//!   [`CanvasTextEditor::begin_existing`]을 부릅니다. Escape는 조합이 있으면 조합만 버리고, 없으면 편집을 끝냅니다.
//!   바깥 클릭은 조합을 확정하고 끝냅니다. 끝나면 Text node 하나가 선택됩니다. 이 편집에서 만든 Text의 본문을 모두
//!   지우고 끝내면 그 빈 Text를 지우는 항목 하나를 기록합니다(ADR-0129 결정 1).
//! - **기록 host.** 편집기는 무엇을 기록할지만 정하고, 기록은 [`TextEditHost`]가 합니다(코어 경로 [`HistoryHost`],
//!   앱 창 session은 `app::text_input`). 앱 Text 입력 경로도 이 편집기 하나를 씁니다(ADR-0129 결정 2).
//! - **입력.** 본문·선택·조합은 공통 text buffer(`stellar_text::edit`)가 갖습니다. 조합 중 문자열은 [`TextPreview`]
//!   (transient, scene이 공통 text 엔진으로 그림)로만 보이고, 확정 입력만 `replace-text` command(XII.10.32)와
//!   입력 종류별 grouping key(`GroupingKey::text_edit`)로 기록합니다.
//! - **typography.** 편집 중 문자 style 변경은 CommandTransaction 하나입니다. 조합이 있으면 조합 확정과 그 범위
//!   style을, 선택이 있으면 선택 범위 style을 한 transaction으로 적용합니다. caret뿐이면 다음 확정 입력의
//!   transaction에 같이 넣습니다.
//! - **폭.** 자동 폭인지는 명시 `width`가 길이·백분율이 아닌지로 정합니다(ADR-0082 기본값 `auto` 포함). 상자 변을
//!   끌면 [`edge_width_command`]가 `width: <px>`로 바꿉니다(자동 → 고정).

use stellar_commands::command::Command;
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::lock::{edit_permission, EditIntent};
use stellar_commands::node_ops::{DeleteSubtree, InsertNode};
use stellar_commands::selection::{Selection, SetSelection};
use stellar_commands::state::DocumentState;
use stellar_commands::style_defaults::SetLayoutProperty;
use stellar_commands::text_edit::{ReplaceText, SetTextRangeStyle};
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::clock::MonotonicClock;
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::value::{Transform2D, Vec2};
use stellar_document::layout_record::{LayoutLonghand, LayoutRecord, LayoutValue};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{
    ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::text::{edit_text, TextPayload, TextRange};
use stellar_history::entry::{GroupingKey, HistoryError, HistorySequence, TextInputKind};
use stellar_history::window::WindowHistory;
use stellar_text::edit::{EditKind, EditSelection, ScalarRange, TextEdit, TextEditBuffer};

use super::TextPreview;
use crate::canvas::tool_units::DRAG_THRESHOLD_PT;

/// 자동 폭 Text의 `width` 값(내용 폭으로 늘어나고 줄바꿈하지 않음).
pub const AUTO_WIDTH: LayoutValue = LayoutValue::Keyword("max-content");

/// 새 Text의 폭.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TextWidth {
    /// 자동 폭(`width: max-content`).
    Auto,
    /// 고정 폭(`width: <px>`).
    Fixed(f32),
}

impl TextWidth {
    /// 저장할 `width` 정규 값.
    pub fn value(self) -> LayoutValue {
        match self {
            TextWidth::Auto => AUTO_WIDTH,
            TextWidth::Fixed(px) => LayoutValue::Length(px),
        }
    }
}

/// Text 도구 한 번의 결과: 새 Text를 둘 부모, 부모 local 원점, 폭.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextPlacement {
    /// 자유 배치 부모.
    pub parent: StableId,
    /// 부모 local 원점(상자 왼쪽 위).
    pub origin: Vec2,
    /// 폭.
    pub width: TextWidth,
}

/// Text 도구 누름·뗌으로 자리를 정합니다. 화면 이동이 끌기 시작 거리([`DRAG_THRESHOLD_PT`]) 안이면 클릭(누른
/// 점, 자동 폭)이고, 넘으면 끈 상자의 왼쪽 위와 가로 길이(고정 폭, 최소 1px)입니다.
pub fn plan_text_tool(
    parent: StableId,
    press_view: Vec2,
    release_view: Vec2,
    press_local: Vec2,
    release_local: Vec2,
) -> Option<TextPlacement> {
    let distance = (release_view.x() - press_view.x()).hypot(release_view.y() - press_view.y());
    if distance <= DRAG_THRESHOLD_PT {
        return Some(TextPlacement {
            parent,
            origin: press_local,
            width: TextWidth::Auto,
        });
    }
    let origin = Vec2::new(
        press_local.x().min(release_local.x()),
        press_local.y().min(release_local.y()),
    )
    .ok()?;
    let width = (release_local.x() - press_local.x()).abs().max(1.0);
    Some(TextPlacement {
        parent,
        origin,
        width: TextWidth::Fixed(width),
    })
}

/// Text 하나의 폭 방식.
#[derive(Debug, Clone, PartialEq)]
pub enum TextWidthMode {
    /// 명시 `width`가 없거나 keyword(`auto`·`max-content`·`min-content`·`fit-content`).
    Auto,
    /// 명시 `width: <length>`.
    Fixed(f32),
    /// 명시 `width: <percentage>`(부모 폭 기준, 끌기로는 px로 바뀝니다).
    Percentage(f32),
}

/// Text의 폭 방식. Text가 아니면 `None`입니다.
pub fn text_width_mode(model: &DocumentModel, node: StableId) -> Option<TextWidthMode> {
    let Some(NodePayload::Text(_)) = model.payload(node) else {
        return None;
    };
    Some(
        match model
            .layout_record(node)
            .and_then(|r| r.get(LayoutLonghand::Width))
        {
            Some(LayoutValue::Length(px)) => TextWidthMode::Fixed(*px),
            Some(LayoutValue::Percentage(p)) => TextWidthMode::Percentage(*p),
            _ => TextWidthMode::Auto,
        },
    )
}

/// Text 상자 왼쪽·오른쪽 변 끌기의 결과 command: `width: <px>`(자동 폭이면 고정 폭으로 바뀜). 끄는 동안 preview로,
/// 놓을 때 한 transaction으로 씁니다. 높이는 내용으로 정해지므로 바꾸지 않습니다.
pub fn edge_width_command(
    model: &DocumentModel,
    node: StableId,
    width: f32,
) -> Result<Command, TextEditError> {
    text_width_mode(model, node).ok_or(TextEditError::NotText)?;
    edit_permission(model, node, EditIntent::Edit).map_err(|_| TextEditError::NotEditable)?;
    if !width.is_finite() {
        return Err(TextEditError::InvalidWidth);
    }
    Ok(Command::SetLayoutProperty(SetLayoutProperty {
        node,
        longhand: LayoutLonghand::Width,
        value: LayoutValue::Length(width.max(1.0)),
    }))
}

/// 편집 오류.
#[derive(Debug, Clone, PartialEq)]
pub enum TextEditError {
    /// 대상이 Text가 아닙니다.
    NotText,
    /// 잠긴 Text이거나 잠긴 부모 아래입니다.
    NotEditable,
    /// 편집 중 문서가 바뀌어 대상이 사라졌습니다(편집을 끝내야 함).
    Stale,
    /// 폭이 유한 값이 아닙니다.
    InvalidWidth,
    /// 새 Text가 쓸 기본 Text style(문단 0)이 문서에 없습니다.
    NoDefaultStyle,
    /// history 기록 오류.
    History(HistoryError),
}

impl From<HistoryError> for TextEditError {
    fn from(error: HistoryError) -> TextEditError {
        TextEditError::History(error)
    }
}

impl std::fmt::Display for TextEditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TextEditError::NotText => f.write_str("Text node가 아닙니다"),
            TextEditError::NotEditable => f.write_str("잠긴 Text는 편집할 수 없습니다"),
            TextEditError::Stale => f.write_str("편집 중 Text가 문서에서 사라졌습니다"),
            TextEditError::InvalidWidth => f.write_str("Text 폭이 유한 값이 아닙니다"),
            TextEditError::NoDefaultStyle => {
                f.write_str("문서에 새 Text가 쓸 기본 Text style이 없습니다")
            }
            TextEditError::History(error) => write!(f, "history: {error}"),
        }
    }
}

impl std::error::Error for TextEditError {}

/// 편집기가 확정 입력을 기록하는 곳: 작업 문서와 그 창의 history·선택(ADR-0129 결정 2).
///
/// 편집 규칙(무엇을 어떤 command·grouping key로 기록할지)은 [`CanvasTextEditor`] 하나가 정하고, host는 기록만
/// 합니다. 코어 시험·CLI처럼 history를 직접 쥔 경로는 [`HistoryHost`]를, 앱 창은 창 session(Undo 메뉴 이름표 포함)을
/// 씁니다.
pub trait TextEditHost {
    /// 작업 문서. 없으면 편집 대상이 사라진 것으로 봅니다.
    fn document(&self) -> Option<&DocumentState>;
    /// 지금 창 선택.
    fn selected(&self) -> &[StableId];
    /// 확정 입력 하나(연속 입력 묶기).
    fn record_input<C: MonotonicClock>(
        &mut self,
        transaction: &CommandTransaction,
        key: GroupingKey,
        clock: &C,
    ) -> Result<(), TextEditError>;
    /// 새 Text의 첫 확정 입력: 입력 group의 첫 항목으로 기록하고 그 Text 하나를 선택합니다.
    fn record_creation<C: MonotonicClock>(
        &mut self,
        transaction: &CommandTransaction,
        key: GroupingKey,
        node: StableId,
        clock: &C,
    ) -> Result<(), TextEditError>;
    /// 편집 중 typography 변경(앞 입력과 묶지 않는 항목 하나).
    fn record_style(&mut self, transaction: &CommandTransaction) -> Result<(), TextEditError>;
    /// 빈 Text 정리(삭제 항목 하나, 결정 1).
    fn record_removal(&mut self, transaction: &CommandTransaction) -> Result<(), TextEditError>;
    /// node 하나를 선택합니다. 이미 그 선택이면 기록하지 않고 `None`입니다.
    fn select_only(&mut self, node: StableId) -> Result<Option<HistorySequence>, TextEditError>;
}

/// history를 직접 쥔 기록 host(창 session 밖의 코어 경로).
pub struct HistoryHost<'a> {
    history: &'a mut WindowHistory,
    doc: &'a mut DocumentState,
}

impl<'a> HistoryHost<'a> {
    /// 창 history와 작업 문서를 묶습니다.
    pub fn new(history: &'a mut WindowHistory, doc: &'a mut DocumentState) -> HistoryHost<'a> {
        HistoryHost { history, doc }
    }
}

impl TextEditHost for HistoryHost<'_> {
    fn document(&self) -> Option<&DocumentState> {
        Some(self.doc)
    }

    fn selected(&self) -> &[StableId] {
        self.history.view().selection().ids()
    }

    fn record_input<C: MonotonicClock>(
        &mut self,
        transaction: &CommandTransaction,
        key: GroupingKey,
        clock: &C,
    ) -> Result<(), TextEditError> {
        self.history
            .record_grouped(self.doc, transaction, key, clock)?;
        Ok(())
    }

    fn record_creation<C: MonotonicClock>(
        &mut self,
        transaction: &CommandTransaction,
        key: GroupingKey,
        node: StableId,
        clock: &C,
    ) -> Result<(), TextEditError> {
        let select = SetSelection::new(Selection::new([node], Some(node)));
        self.history
            .record_grouped_and_select(self.doc, transaction, key, &select, clock)?;
        Ok(())
    }

    fn record_style(&mut self, transaction: &CommandTransaction) -> Result<(), TextEditError> {
        self.history.record_document(self.doc, transaction, None)?;
        Ok(())
    }

    fn record_removal(&mut self, transaction: &CommandTransaction) -> Result<(), TextEditError> {
        self.history.record_document(self.doc, transaction, None)?;
        Ok(())
    }

    fn select_only(&mut self, node: StableId) -> Result<Option<HistorySequence>, TextEditError> {
        Ok(self.history.record_selection(
            self.doc,
            &SetSelection::new(Selection::new([node], Some(node))),
        )?)
    }
}

fn host_doc<H: TextEditHost>(host: &H) -> Result<&DocumentState, TextEditError> {
    host.document().ok_or(TextEditError::Stale)
}

/// 편집 대상.
#[derive(Debug, Clone)]
enum Target {
    /// 문서에 있는 Text(처음부터 있었거나 이 편집에서 만듦).
    Node {
        /// 이 편집에서 만들었는지.
        created: bool,
    },
    /// 첫 확정 입력 때 만들 새 Text.
    Pending {
        parent: StableId,
        stack: ModifierStack,
        width: TextWidth,
    },
}

/// Escape 한 번의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeStep {
    /// 조합만 버림(편집은 계속).
    CompositionCancelled,
    /// 편집을 끝내야 함([`CanvasTextEditor::finish`]).
    End,
}

/// 편집을 끝낸 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditEnd {
    /// 입력 없이 끝난 새 Text: 아무것도 만들지 않고 기록하지 않음.
    CreationCancelled,
    /// 이 편집에서 만든 Text의 본문을 모두 지우고 끝냄: 빈 Text를 지우는 항목 하나를 기록함(ADR-0129 결정 1).
    Removed {
        /// 지운 Text node.
        node: StableId,
    },
    /// Text node 하나를 선택함. 선택이 바뀌었으면 그 선택 기록.
    Selected {
        /// Text node.
        node: StableId,
        /// 선택 항목(이미 그 Text 하나만 선택돼 있었으면 `None`).
        selection: Option<HistorySequence>,
    },
}

/// typography 변경의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleOutcome {
    /// 범위 style을 transaction 하나로 기록함.
    Recorded,
    /// caret뿐이라 다음 확정 입력에 같이 넣음.
    Pending,
}

/// caret 이동 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaretMove {
    /// 앞 grapheme.
    Backward,
    /// 뒤 grapheme.
    Forward,
    /// 앞 단어 시작.
    WordBackward,
    /// 뒤 단어 끝.
    WordForward,
    /// 문단 시작.
    ParagraphStart,
    /// 문단 끝.
    ParagraphEnd,
    /// 전체 선택.
    SelectAll,
}

/// 창 하나의 진행 중 Text 편집. 저장·history에는 확정 입력만 남습니다.
#[derive(Debug, Clone)]
pub struct CanvasTextEditor {
    node: StableId,
    target: Target,
    buffer: TextEditBuffer,
    revision: DocumentRevision,
    /// caret 위치에 다음 입력이 쓸 문자 style(`Some(None)`은 기본 style).
    typing_style: Option<Option<u32>>,
    token: u64,
    sequence: u64,
}

fn input_kind(kind: EditKind) -> TextInputKind {
    match kind {
        EditKind::Insert => TextInputKind::Insert,
        EditKind::DeleteBackward => TextInputKind::DeleteBackward,
        EditKind::DeleteForward => TextInputKind::DeleteForward,
        EditKind::Replace => TextInputKind::Replace,
    }
}

fn text_range(range: ScalarRange) -> TextRange {
    TextRange::new(range.start, range.end).expect("buffer 범위는 정렬돼 있음")
}

fn text_payload(model: &DocumentModel, node: StableId) -> Option<&TextPayload> {
    match model.payload(node) {
        Some(NodePayload::Text(text)) => Some(text),
        _ => None,
    }
}

/// 새 Text를 둘 수 있는 부모: 문서에 있고 잠기지 않은 Frame.
fn editable_parent(model: &DocumentModel, parent: StableId) -> Result<(), TextEditError> {
    let Some(NodePayload::Frame(_)) = model.payload(parent) else {
        return Err(TextEditError::Stale);
    };
    edit_permission(model, parent, EditIntent::Edit).map_err(|_| TextEditError::NotEditable)
}

impl CanvasTextEditor {
    /// 문서에 있는 Text의 편집을 시작합니다. `selection`이 없으면 caret은 본문 끝입니다. `token`은 조합 preview의
    /// scene 재사용 key입니다(창 안에서 편집마다 다른 값).
    pub fn begin_existing(
        doc: &DocumentState,
        node: StableId,
        selection: Option<EditSelection>,
        token: u64,
    ) -> Result<CanvasTextEditor, TextEditError> {
        let model = doc.model();
        let payload = text_payload(model, node).ok_or(TextEditError::NotText)?;
        edit_permission(model, node, EditIntent::Edit).map_err(|_| TextEditError::NotEditable)?;
        let mut buffer = TextEditBuffer::new(payload.content());
        if let Some(selection) = selection {
            buffer.set_selection(selection);
        }
        Ok(CanvasTextEditor {
            node,
            target: Target::Node { created: false },
            buffer,
            revision: doc.revision(),
            typing_style: None,
            token,
            sequence: 0,
        })
    }

    /// Text 도구 자리에서 새 Text 편집을 시작합니다. 문서는 바뀌지 않습니다. `node`·`modifier`는 첫 확정 때
    /// 만들 Text와 원점 Transform modifier의 영속 ID입니다.
    pub fn begin_new(
        doc: &DocumentState,
        placement: TextPlacement,
        node: StableId,
        modifier: StableId,
        token: u64,
    ) -> Result<CanvasTextEditor, TextEditError> {
        let model = doc.model();
        editable_parent(model, placement.parent)?;
        // 해석할 수 없는 기본 style로 Text를 만들면 보이지 않는 node가 생깁니다(ADR-0067 §5).
        if model
            .definition()
            .text_styles
            .resolve(0, None)
            .is_none_or(|s| s.typography.record().is_err())
        {
            return Err(TextEditError::NoDefaultStyle);
        }
        if let TextWidth::Fixed(px) = placement.width {
            if !px.is_finite() || px <= 0.0 {
                return Err(TextEditError::InvalidWidth);
            }
        }
        let transform =
            Transform2D::new(1., 0., 0., 1., placement.origin.x(), placement.origin.y())
                .map_err(|_| TextEditError::InvalidWidth)?;
        let mut stack = ModifierStack::new(node);
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(modifier),
                order: OrderKey::between(None, None),
                kind: ModifierKind::Transform,
                enabled: true,
                payload: ModifierPayload::Transform(transform),
            })
            .map_err(|_| TextEditError::Stale)?;
        Ok(CanvasTextEditor {
            node,
            target: Target::Pending {
                parent: placement.parent,
                stack,
                width: placement.width,
            },
            buffer: TextEditBuffer::new(""),
            revision: doc.revision(),
            typing_style: None,
            token,
            sequence: 0,
        })
    }

    /// 편집 대상 node(새 Text는 만들 ID).
    pub fn node(&self) -> StableId {
        self.node
    }

    /// 문서에 Text가 있는지(새 Text는 첫 확정 뒤).
    pub fn exists(&self) -> bool {
        matches!(self.target, Target::Node { .. })
    }

    /// 본문·선택·조합.
    pub fn buffer(&self) -> &TextEditBuffer {
        &self.buffer
    }

    /// 다른 경로(undo·다른 창)로 문서가 바뀌었으면 본문을 다시 읽습니다. 대상이 사라졌으면 [`TextEditError::Stale`],
    /// 잠겼으면 [`TextEditError::NotEditable`]입니다.
    fn sync(&mut self, doc: &DocumentState) -> Result<(), TextEditError> {
        if doc.revision() == self.revision {
            return Ok(());
        }
        self.revision = doc.revision();
        match self.target {
            Target::Node { .. } => {
                let payload = text_payload(doc.model(), self.node).ok_or(TextEditError::Stale)?;
                // 편집 중 다른 경로로 잠기면 더 쓰지 않습니다(ADR-0118 결정 2.8).
                edit_permission(doc.model(), self.node, EditIntent::Edit)
                    .map_err(|_| TextEditError::NotEditable)?;
                if payload.content() != self.buffer.text() {
                    self.buffer.reset_text(payload.content());
                }
                Ok(())
            }
            Target::Pending { parent, .. } => editable_parent(doc.model(), parent),
        }
    }

    fn style_command(&self, range: ScalarRange, style: Option<u32>) -> Option<Command> {
        (!range.is_empty()).then(|| {
            Command::SetTextRangeStyle(SetTextRangeStyle {
                node: self.node,
                range: text_range(range),
                style,
            })
        })
    }

    /// 확정 입력 하나를 기록합니다. 새 Text는 첫 확정 때 만들고 선택합니다(이어지는 입력과 같은 group).
    fn record<H: TextEditHost, C: MonotonicClock>(
        &mut self,
        host: &mut H,
        edit: TextEdit,
        id: CommandId,
        clock: &C,
    ) -> Result<bool, TextEditError> {
        let replace = ReplaceText {
            node: self.node,
            range: text_range(edit.range),
            replacement: edit.replacement.clone(),
        };
        let key = GroupingKey::text_edit(&replace, input_kind(edit.kind));
        let inserted = ScalarRange {
            start: edit.range.start,
            end: edit.range.start + stellar_document::text::scalar_len(&edit.replacement),
        };
        let style = self
            .typing_style
            .and_then(|style| self.style_command(inserted, style));
        let revision = host_doc(host)?.revision();
        let mut tx = CommandTransaction::new(id, CommandOrigin::User);
        match &self.target {
            Target::Node { .. } => {
                tx = tx.with_command(revision, Command::ReplaceText(replace));
                if let Some(style) = style {
                    tx = tx.with_command(revision, style);
                }
                host.record_input(&tx, key, clock)?;
            }
            Target::Pending {
                parent,
                stack,
                width,
            } => {
                if edit.replacement.is_empty() {
                    return Ok(false);
                }
                let model = host_doc(host)?.model();
                editable_parent(model, *parent)?;
                let payload = edit_text(
                    &TextPayload::empty(0),
                    replace.range,
                    &edit.replacement,
                    &[],
                )
                .map_err(|_| TextEditError::Stale)?;
                let mut layout = LayoutRecord::new(self.node);
                layout
                    .set(LayoutLonghand::Width, width.value())
                    .map_err(|_| TextEditError::InvalidWidth)?;
                let insert = InsertNode {
                    nodes: vec![NodeRecord {
                        id: self.node,
                        parent: Some(*parent),
                        order_key: model.next_child_order_key(*parent),
                        visible: true,
                        locked: false,
                        display_name: "Text".into(),
                        payload: NodePayload::Text(payload),
                    }],
                    modifier_stacks: vec![stack.clone()],
                    layout_records: vec![layout],
                };
                tx = tx.with_command(revision, Command::InsertNode(insert));
                if let Some(style) = style {
                    tx = tx.with_command(revision, style);
                }
                host.record_creation(&tx, key, self.node, clock)?;
                self.target = Target::Node { created: true };
            }
        }
        self.typing_style = None;
        self.revision = host_doc(host)?.revision();
        Ok(true)
    }

    /// buffer 편집 하나를 적용하고 확정 입력이면 기록합니다. 기록이 실패하면(잠금, 표에 없는 style 번호 등) buffer를
    /// 편집 전으로 되돌려 문서 본문과 어긋나지 않게 하고, 실패한 transaction에 들어 있던 다음 입력 style도 버립니다.
    fn confirmed<H: TextEditHost, C: MonotonicClock>(
        &mut self,
        host: &mut H,
        edit: impl FnOnce(&mut TextEditBuffer) -> Option<TextEdit>,
        id: CommandId,
        clock: &C,
    ) -> Result<bool, TextEditError> {
        let before = self.buffer.clone();
        let Some(edit) = edit(&mut self.buffer) else {
            return Ok(false);
        };
        let recorded = self.record(host, edit, id, clock);
        if recorded.is_err() {
            self.buffer = before;
            self.typing_style = None;
        }
        recorded
    }

    /// 확정 입력(타이핑·붙여넣기·`insertText`). 조합 중이면 조합을 이 문자열로 확정합니다. 기록했으면 `true`.
    pub fn insert<H: TextEditHost, C: MonotonicClock>(
        &mut self,
        host: &mut H,
        text: &str,
        id: CommandId,
        clock: &C,
    ) -> Result<bool, TextEditError> {
        self.sync(host_doc(host)?)?;
        self.confirmed(host, |b| b.insert(text), id, clock)
    }

    /// Backspace.
    pub fn delete_backward<H: TextEditHost, C: MonotonicClock>(
        &mut self,
        host: &mut H,
        id: CommandId,
        clock: &C,
    ) -> Result<bool, TextEditError> {
        self.sync(host_doc(host)?)?;
        self.confirmed(host, TextEditBuffer::delete_backward, id, clock)
    }

    /// Delete.
    pub fn delete_forward<H: TextEditHost, C: MonotonicClock>(
        &mut self,
        host: &mut H,
        id: CommandId,
        clock: &C,
    ) -> Result<bool, TextEditError> {
        self.sync(host_doc(host)?)?;
        self.confirmed(host, TextEditBuffer::delete_forward, id, clock)
    }

    /// 조합 문자열 갱신(`setMarkedText`). 문서·history는 바뀌지 않고 [`Self::preview`]만 바뀝니다.
    pub fn set_marked(
        &mut self,
        doc: &DocumentState,
        marked: &str,
        selected: ScalarRange,
    ) -> Result<(), TextEditError> {
        self.sync(doc)?;
        self.buffer.set_marked(marked, selected);
        self.sequence += 1;
        Ok(())
    }

    /// 조합을 지금 문자열로 확정합니다(`unmarkText`).
    pub fn commit_composition<H: TextEditHost, C: MonotonicClock>(
        &mut self,
        host: &mut H,
        id: CommandId,
        clock: &C,
    ) -> Result<bool, TextEditError> {
        self.sync(host_doc(host)?)?;
        self.sequence += 1;
        self.confirmed(host, TextEditBuffer::commit_composition, id, clock)
    }

    /// caret·선택 이동(기록 없음). 조합 중이면 입력기 몫이라 `false`입니다.
    pub fn move_caret(
        &mut self,
        doc: &DocumentState,
        motion: CaretMove,
        extend: bool,
    ) -> Result<bool, TextEditError> {
        self.sync(doc)?;
        let b = &mut self.buffer;
        let moved = match motion {
            CaretMove::Backward => b.move_backward(extend),
            CaretMove::Forward => b.move_forward(extend),
            CaretMove::WordBackward => b.move_word_backward(extend),
            CaretMove::WordForward => b.move_word_forward(extend),
            CaretMove::ParagraphStart => b.move_paragraph_start(extend),
            CaretMove::ParagraphEnd => b.move_paragraph_end(extend),
            CaretMove::SelectAll => b.select_all(),
        };
        if moved {
            self.typing_style = None;
        }
        Ok(moved)
    }

    /// pointer로 caret·선택을 놓습니다(본문 hit 결과, 기록 없음).
    pub fn set_selection(
        &mut self,
        doc: &DocumentState,
        selection: EditSelection,
    ) -> Result<bool, TextEditError> {
        self.sync(doc)?;
        let changed = self.buffer.selection() != selection && self.buffer.set_selection(selection);
        if changed {
            self.typing_style = None;
        }
        Ok(changed)
    }

    /// 편집 중 문자 style(`DocumentTextStyles::runs` 번호, `None`은 기본 style) 변경. 조합이 있으면 조합 확정과 그
    /// 범위 style을, 선택이 있으면 선택 범위 style을 CommandTransaction 하나로 기록합니다(history 1건, 앞 입력과
    /// 묶지 않음). caret뿐이면 다음 확정 입력의 transaction에 넣습니다.
    pub fn apply_style<H: TextEditHost>(
        &mut self,
        host: &mut H,
        style: Option<u32>,
        id: CommandId,
    ) -> Result<StyleOutcome, TextEditError> {
        self.sync(host_doc(host)?)?;
        let composing = self.buffer.composition().is_some();
        let selected = self.buffer.selection().range();
        if !composing && (selected.is_empty() || !self.exists()) {
            self.typing_style = Some(style);
            return Ok(StyleOutcome::Pending);
        }
        if composing && !self.exists() {
            // 새 Text의 조합: 첫 확정(생성)에 style을 같이 넣습니다.
            self.typing_style = Some(style);
            return Ok(StyleOutcome::Pending);
        }
        let before = self.buffer.clone();
        let revision = host_doc(host)?.revision();
        let mut tx = CommandTransaction::new(id, CommandOrigin::User);
        let range = if let Some(composition) = self.buffer.composition() {
            let start = composition.replaced.start;
            let inserted = ScalarRange {
                start,
                end: start + stellar_document::text::scalar_len(&composition.marked),
            };
            // 조합 문자열이 바꿀 범위의 본문과 같으면 교체 command 없이 style만 씁니다.
            if let Some(edit) = self.buffer.commit_composition() {
                tx = tx.with_command(
                    revision,
                    Command::ReplaceText(ReplaceText {
                        node: self.node,
                        range: text_range(edit.range),
                        replacement: edit.replacement,
                    }),
                );
            }
            inserted
        } else {
            selected
        };
        let Some(command) = self.style_command(range, style) else {
            self.buffer = before;
            self.typing_style = Some(style);
            return Ok(StyleOutcome::Pending);
        };
        tx = tx.with_command(revision, command);
        if let Err(error) = host.record_style(&tx) {
            self.buffer = before;
            return Err(error);
        }
        self.sequence += 1;
        self.revision = host_doc(host)?.revision();
        Ok(StyleOutcome::Recorded)
    }

    /// 조합 중 본문 미리보기. 조합이 없으면 `None`(문서 본문이 곧 표시 본문). scene이 공통 text 엔진으로 그리고,
    /// 새 Text는 부모의 마지막 자식 자리에 hit 대상 없이 그립니다.
    pub fn preview(&self, doc: &DocumentState) -> Option<TextPreview> {
        let composition = self.buffer.composition()?;
        let (base, new_in) = match &self.target {
            Target::Node { .. } => (text_payload(doc.model(), self.node)?.clone(), None),
            Target::Pending { parent, stack, .. } => {
                (TextPayload::empty(0), Some((*parent, stack.clone())))
            }
        };
        let payload = edit_text(
            &base,
            text_range(composition.replaced),
            &composition.marked,
            &[],
        )
        .ok()?;
        Some(TextPreview {
            key: (self.token, self.sequence),
            node: self.node,
            payload,
            new_in,
        })
    }

    /// Escape: 조합이 있으면 조합만 버리고, 없으면 끝낼 차례입니다.
    pub fn escape(&mut self) -> EscapeStep {
        if self.buffer.cancel_composition() {
            self.sequence += 1;
            EscapeStep::CompositionCancelled
        } else {
            EscapeStep::End
        }
    }

    /// 편집을 끝냅니다(Escape 뒤, 바깥 클릭, 도구 전환). 남은 조합은 확정합니다. 입력 없이 끝난 새 Text는
    /// 아무것도 기록하지 않습니다. 이 편집에서 만든 Text의 본문이 비었으면 그 Text를 지우는 `delete-subtree` 항목
    /// 하나를 기록합니다(ADR-0129 결정 1). 그 밖에는 Text node 하나를 선택합니다.
    ///
    /// 조합 확정과 빈 Text 정리는 함께 일어나지 않습니다(확정한 조합 문자열은 비어 있지 않음). 그래서 `id`는 둘 중
    /// 하나에만 쓰입니다.
    pub fn finish<H: TextEditHost, C: MonotonicClock>(
        mut self,
        host: &mut H,
        id: CommandId,
        clock: &C,
    ) -> Result<EditEnd, TextEditError> {
        if self.buffer.composition().is_some() {
            self.commit_composition(host, id, clock)?;
        } else {
            self.sync(host_doc(host)?)?;
        }
        if !self.exists() {
            return Ok(EditEnd::CreationCancelled);
        }
        let doc = host_doc(host)?;
        let payload = text_payload(doc.model(), self.node).ok_or(TextEditError::Stale)?;
        if self.created() && payload.content().is_empty() {
            let tx = CommandTransaction::new(id, CommandOrigin::User).with_command(
                doc.revision(),
                Command::DeleteSubtree(DeleteSubtree { root: self.node }),
            );
            host.record_removal(&tx)?;
            return Ok(EditEnd::Removed { node: self.node });
        }
        let selection = if host.selected() == [self.node] {
            None
        } else {
            host.select_only(self.node)?
        };
        Ok(EditEnd::Selected {
            node: self.node,
            selection,
        })
    }

    /// 이 편집에서 Text를 만들었는지.
    pub fn created(&self) -> bool {
        matches!(self.target, Target::Node { created: true })
    }
}

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;

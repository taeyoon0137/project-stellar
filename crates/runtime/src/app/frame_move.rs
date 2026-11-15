//! 활성 창의 Frame 속성 편집과 캔버스 편집 경로가 함께 쓰는 창·잠금·ID 도우미입니다. 이동은
//! `canvas_gesture`(ADR-0071)가 합니다.

use super::*;
use stellar_commands::command::Command;
use stellar_commands::document::{
    frame::{keys, Clipping},
    records::NodePayload,
};
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::property::{PropertyValue, SetProperty};
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::color::Color;
use stellar_core_types::property::PropertyKey;
use stellar_core_types::value::{FiniteF32, Size2};
use stellar_document::model::DocumentModel;

#[derive(Clone, Copy)]
enum FrameEdit {
    /// Inspector 속성 하나(선택 history sequence, CSS longhand key, 값).
    Property(u64, PropertyKey, FrameValue),
}

/// Inspector가 Frame에 쓰는 값. 길이 key(width·height·모서리)는 `Length`, 배경은 `Background`입니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameValue {
    /// 비음수 논리 픽셀 길이.
    Length(FiniteF32),
    /// 단색 배경. `None`은 배경 제거입니다.
    Background(Option<Color>),
}

impl FrameValue {
    /// Inspector가 길이로 편집하는 Frame longhand(width·height·모서리 TL·TR·BR·BL).
    pub const LENGTH_KEYS: [PropertyKey; 6] = [
        keys::WIDTH,
        keys::HEIGHT,
        keys::BORDER_TOP_LEFT_RADIUS,
        keys::BORDER_TOP_RIGHT_RADIUS,
        keys::BORDER_BOTTOM_RIGHT_RADIUS,
        keys::BORDER_BOTTOM_LEFT_RADIUS,
    ];
    /// Inspector가 색으로 편집하는 Frame 배경 key.
    pub const BACKGROUND_KEY: PropertyKey = keys::BACKGROUND;
}

/// 확정된 단일 선택의 Inspector 값. core가 문서에서 읽습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedNode {
    /// 선택 node.
    pub node: StableId,
    /// NodeKind 저장 tag.
    pub kind: u16,
    /// ShapeKind 저장 tag. Shape가 아니면 0.
    pub shape: u8,
    /// Frame이면 Inspector 값.
    pub frame: Option<SelectedFrame>,
}

/// 선택 Frame의 문서 값과 편집 가능 여부. 편집 경로가 받는 조건(`editable_frame`)과 같습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedFrame {
    /// 고정 크기.
    pub size: Size2,
    /// 단색 배경.
    pub background: Option<Color>,
    /// bounds 정규화 전 원본 모서리 반경.
    pub corners: stellar_document::frame::CornerRadius,
    /// 활성 View에서 Inspector 편집을 받을 수 있는지(표시·잠금·자유 배치·clip 조건).
    pub editable: bool,
}

/// 단일 선택 node의 Inspector snapshot을 만듭니다. 조상 수에 비례합니다.
pub(super) fn selected_node(
    model: &DocumentModel,
    view: Option<StableId>,
    node: StableId,
) -> Option<SelectedNode> {
    let payload = model.payload(node)?;
    let frame = match payload {
        NodePayload::Frame(frame) => Some(SelectedFrame {
            size: frame.size,
            background: frame.background,
            corners: frame.corner_radius,
            editable: view.is_some_and(|view| editable_frame(model, view, node).is_some()),
        }),
        _ => None,
    };
    Some(SelectedNode {
        node,
        kind: payload.kind().as_u16(),
        shape: match payload {
            NodePayload::Shape(shape) => shape.kind().tag(),
            _ => 0,
        },
        frame,
    })
}

impl AppRuntime {
    /// 선택 Frame의 Inspector 속성 하나(width·height·모서리 longhand·background)를 Undo 가능한 편집으로 바꿉니다.
    /// 표시한 선택 대상의 history sequence·View·revision과 편집 가능 여부는 document thread에서 다시 확인합니다.
    pub fn set_canvas_frame_property(
        &self,
        target: SelectionEditTarget,
        node: StableId,
        key: PropertyKey,
        value: FrameValue,
    ) -> Result<RequestId, RequestError> {
        self.edit_canvas_frame(
            target.session,
            target.view,
            target.revision,
            node,
            FrameEdit::Property(target.history_sequence, key, value),
        )
    }

    fn edit_canvas_frame(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        node: StableId,
        edit: FrameEdit,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let window = canvas_window(ctx, session)?;
            edit_selected(window, request, view, revision, node, edit)?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }
}

/// 캔버스·Inspector·Layers 편집 대상 창을 찾습니다.
///
/// 문서 편집과 저장은 프로젝트 신뢰와 무관하게 허용합니다(사용자 결정 D1, 아키텍처 §24.3). 제한 모드는
/// Git hook·filter, 내장 터미널, MCP·에이전트, 외부 명령처럼 코드를 실행하는 경로에만 적용합니다.
pub(super) fn canvas_window(
    ctx: &mut DocumentContext,
    session: WindowSessionHandle,
) -> Result<&mut ProjectWindowSession, Failure> {
    window_mut(ctx, session)
}

fn edit_selected(
    window: &mut ProjectWindowSession,
    request: RequestId,
    view: StableId,
    revision: DocumentRevision,
    node: StableId,
    edit: FrameEdit,
) -> Result<(), Failure> {
    let stale = || {
        Failure::new(
            RequestFailure::UnknownView,
            "Frame의 선택·뷰·편집 상태가 변경되었습니다",
        )
    };
    if window.revision() != revision
        || window.tabs().active_view() != Some(view)
        || window.view().selection().ids() != [node]
    {
        return Err(stale());
    }
    let model = window.document().ok_or_else(stale)?.model();
    let command_id = command_id(window.window(), request);
    let transaction = CommandTransaction::new(command_id, CommandOrigin::User);
    let command = {
        let target = editable_frame(model, view, node).ok_or_else(stale)?;
        let FrameEdit::Property(sequence, key, value) = edit;
        if window.history_sequence() != sequence {
            return Err(stale());
        }
        let (tl, tr, br, bl) = target.corner_radius.components();
        let current = match (key, value) {
            (keys::BACKGROUND, FrameValue::Background(color)) => {
                if target.background == color {
                    return Ok(());
                }
                None
            }
            (keys::WIDTH, FrameValue::Length(_)) => Some(target.size.width()),
            (keys::HEIGHT, FrameValue::Length(_)) => Some(target.size.height()),
            (_, FrameValue::Length(_)) => {
                let index = keys::BORDER_RADIUS_LONGHANDS
                    .iter()
                    .position(|&k| k == key)
                    .ok_or_else(stale)?;
                Some([tl, tr, br, bl][index])
            }
            _ => return Err(stale()),
        };
        let (key, value) = match value {
            FrameValue::Length(length) => {
                if length.get() < 0.0 {
                    return Err(stale());
                }
                if current == Some(length.get()) {
                    return Ok(());
                }
                (key, PropertyValue::NonNegative(length))
            }
            FrameValue::Background(color) => (key, PropertyValue::OptionalColor(color)),
        };
        Command::SetProperty(SetProperty { node, key, value })
    };
    let transaction = transaction.with_command(revision, command);
    window.apply(&transaction).map_err(view_failure)?;
    Ok(())
}

pub(super) fn editable_frame(
    model: &stellar_document::model::DocumentModel,
    view: StableId,
    node: StableId,
) -> Option<&stellar_document::frame::FramePayload> {
    let Some(NodePayload::Frame(target)) = model.payload(node) else {
        return None;
    };
    editable_ancestry(model, view, node, false)?;
    Some(target)
}

pub(super) fn editable_ancestry(
    model: &DocumentModel,
    view: StableId,
    node: StableId,
    modifier_target: bool,
) -> Option<()> {
    let root = model
        .definition()
        .pages
        .iter()
        .flat_map(|page| &page.views)
        .find(|v| v.id == view)?
        .root_frame;
    let mut current = node;
    loop {
        let fields = model
            .runtime_id(current)
            .and_then(|id| model.columns().get(id))?;
        if !fields.visible
            || fields.locked
            || (!(modifier_target && current == node) && model.modifier_stack(current).is_some())
        {
            return None;
        }
        match model.payload(current) {
            Some(NodePayload::Frame(frame))
                if frame.layout_mode.is_none() && !matches!(frame.clipping, Clipping::Path(_)) => {}
            Some(NodePayload::Fragment(_)) => {}
            Some(
                NodePayload::Shape(_)
                | NodePayload::ComponentInstance(_)
                | NodePayload::Text(_)
                | NodePayload::Image(_),
            ) if modifier_target && current == node => {}
            _ => return None,
        }
        if current == root {
            break;
        }
        current = fields.parent.and_then(|id| model.stable_id(id))?;
    }
    Some(())
}

/// 새 node·modifier의 영속 ID(UUIDv7, OS 난수)를 만듭니다. 요청 순번이나 command ID에서 파생하지 않습니다.
pub(super) fn fresh_ids<const N: usize>() -> Result<[StableId; N], Failure> {
    stellar_project::new_ids::new_ids().map_err(|error| {
        Failure::new(
            RequestFailure::UnknownView,
            format!("새 식별자를 만들지 못했습니다: {error}"),
        )
    })
}

/// 복제·붙여넣기 계획 API는 ID 생성기를 받습니다. seed를 OS 난수로 채워 요청마다 독립 ID를 만듭니다.
pub(super) fn fresh_generator() -> Result<stellar_core_types::ids::StableIdGenerator, Failure> {
    let [seed] = fresh_ids()?;
    let bytes = seed.to_bytes();
    Ok(stellar_core_types::ids::StableIdGenerator::new(
        u64::from_le_bytes(bytes[8..].try_into().expect("eight bytes")),
    ))
}

/// 요청 순번은 runtime 안에서만 유일하므로 창의 영속 ID와 함께 식별합니다.
pub(super) fn command_id(window: WindowSessionId, request: RequestId) -> CommandId {
    let mut key = [0; 24];
    key[..16].copy_from_slice(&window.stable_id().to_bytes());
    key[16..].copy_from_slice(&request.0.to_le_bytes());
    let digest = stellar_storage::hash::ContentHash::compute(&key);
    let mut raw = [0; 16];
    raw.copy_from_slice(&digest.digest()[..16]);
    CommandId::from_bytes(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_window_tests::{next_event, open_project, open_window};
    use crate::window_session::test_fixtures::{
        identity, sample_document, saved_project_with, sid, window,
    };
    use stellar_commands::selection::Selection;

    #[test]
    fn returning_to_the_same_selected_frame_rejects_the_old_corner_edit() {
        let project = saved_project_with("corner-return-selection", &[&sample_document()]);
        let mut session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        let (a, b) = (sid(11), sid(10));
        session.select(Selection::new([a], None)).unwrap();
        let revision = session.revision();
        let original_sequence = session.history_sequence();
        session.select(Selection::new([b], None)).unwrap();
        session.select(Selection::new([a], None)).unwrap();
        assert_eq!(session.revision(), revision);
        assert_ne!(session.history_sequence(), original_sequence);
        let length = stellar_core_types::value::FiniteF32::new(128.).unwrap();
        let stale = edit_selected(
            &mut session,
            RequestId(9),
            sid(3),
            revision,
            a,
            FrameEdit::Property(
                original_sequence,
                keys::BORDER_TOP_RIGHT_RADIUS,
                FrameValue::Length(length),
            ),
        );
        assert!(
            stale.is_err(),
            "old edit after A -> B -> A: captured seq={original_sequence}, resulting rev={:?}",
            session.revision()
        );
        // 크기·배경도 같은 history sequence 검사를 거칩니다.
        for (key, value) in [
            (keys::WIDTH, FrameValue::Length(length)),
            (keys::HEIGHT, FrameValue::Length(length)),
            (keys::BACKGROUND, FrameValue::Background(None)),
        ] {
            assert!(edit_selected(
                &mut session,
                RequestId(9),
                sid(3),
                revision,
                a,
                FrameEdit::Property(original_sequence, key, value),
            )
            .is_err());
        }
        assert_eq!(session.revision(), revision);
        let current = session.history_sequence();
        assert!(edit_selected(
            &mut session,
            RequestId(10),
            sid(3),
            revision,
            a,
            FrameEdit::Property(
                current,
                keys::BORDER_TOP_RIGHT_RADIUS,
                FrameValue::Length(length),
            ),
        )
        .is_ok());
        assert_eq!(session.revision().0, revision.0 + 1);
    }

    #[test]
    fn corner_edits_reject_stale_selection_revision_and_invalid_key() {
        let model = sample_document();
        let project = saved_project_with("corner-edit-target", &[&model]);
        let mut session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        let revision = session.revision();
        let key = keys::BORDER_BOTTOM_RIGHT_RADIUS;
        let length = stellar_core_types::value::FiniteF32::new(128.).unwrap();
        session.select(Selection::new([sid(11)], None)).unwrap();
        let sequence = session.history_sequence();
        for (view, node, rev, property) in [
            (sid(3), sid(10), revision, key),
            (sid(4), sid(11), revision, key),
            (sid(3), sid(11), DocumentRevision(revision.0 + 1), key),
            (sid(3), sid(11), revision, keys::SIZE),
            (sid(3), sid(11), revision, keys::TRANSFORM),
        ] {
            assert!(edit_selected(
                &mut session,
                RequestId(1),
                view,
                rev,
                node,
                FrameEdit::Property(sequence, property, FrameValue::Length(length))
            )
            .is_err());
            assert_eq!(session.revision(), revision);
        }
        // 값 종류가 key와 맞지 않거나 길이가 음수이면 거부합니다.
        for (key, value) in [
            (keys::BACKGROUND, FrameValue::Length(length)),
            (keys::WIDTH, FrameValue::Background(None)),
            (
                keys::WIDTH,
                FrameValue::Length(FiniteF32::new(-1.0).unwrap()),
            ),
        ] {
            assert!(edit_selected(
                &mut session,
                RequestId(2),
                sid(3),
                revision,
                sid(11),
                FrameEdit::Property(sequence, key, value)
            )
            .is_err());
            assert_eq!(session.revision(), revision);
        }
    }

    #[test]
    fn selected_node_snapshot_reads_document_values_and_edit_conditions() {
        let mut model = sample_document();
        let Some(NodePayload::Frame(mut frame)) = model.payload(sid(11)).cloned() else {
            unreachable!()
        };
        frame.size = Size2::new(31.0, 17.0).unwrap();
        frame.background = None;
        model
            .set_payload(sid(11), NodePayload::Frame(frame))
            .unwrap();
        let selected = selected_node(&model, Some(sid(3)), sid(11)).unwrap();
        assert_eq!(
            (selected.node, selected.kind, selected.shape),
            (sid(11), 1, 0)
        );
        let values = selected.frame.unwrap();
        assert_eq!(values.size, Size2::new(31.0, 17.0).unwrap());
        assert_eq!(values.background, None);
        assert!(values.editable);
        // 편집 경로가 거부하는 조건(다른 View, 활성 View 없음, 잠긴 조상)이면 편집 불가로 표시합니다.
        for view in [Some(sid(4)), None] {
            assert!(
                !selected_node(&model, view, sid(11))
                    .unwrap()
                    .frame
                    .unwrap()
                    .editable
            );
        }
        model.replace_flags(sid(10), true, true).unwrap();
        assert!(
            !selected_node(&model, Some(sid(3)), sid(11))
                .unwrap()
                .frame
                .unwrap()
                .editable
        );
        assert!(selected_node(&model, Some(sid(3)), sid(999)).is_none());
    }

    #[test]
    fn restricted_mode_window_edits_frames_without_project_trust() {
        // 제한 모드는 코드 실행 경로만 막고 문서 편집은 막지 않습니다(사용자 결정 D1).
        let project = saved_project_with("canvas-move-restricted", &[&sample_document()]);
        let app = AppRuntime::start().unwrap();
        let owner = open_project(&app, project.path());
        assert_ne!(
            app.state().project(owner).unwrap().trust,
            TrustStatus::Trusted
        );
        let (session, _, _) = open_window(&app, owner);
        app.set_selection(session, vec![sid(11)]).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        let state = app.state();
        let window = state.window(session).unwrap();
        let rev = window.revision;
        let resized = app
            .set_canvas_frame_property(
                SelectionEditTarget {
                    session,
                    view: sid(3),
                    revision: rev,
                    history_sequence: window.history_sequence,
                },
                sid(11),
                keys::WIDTH,
                FrameValue::Length(FiniteF32::new(10.0).unwrap()),
            )
            .unwrap();
        assert!(
            matches!(next_event(&app), RuntimeEvent::WindowViewChanged { request, .. } if request == resized)
        );
        let summary = app.state();
        let summary = summary.window(session).unwrap();
        assert_eq!(summary.revision.0, rev.0 + 1);
        assert!(summary.has_unsaved_changes);
    }
}

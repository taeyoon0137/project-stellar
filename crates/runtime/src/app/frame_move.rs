//! 활성 창의 Frame 편집, Rectangle 및 배치한 인스턴스 이동을 한 transaction으로 적용합니다.

use super::*;
use stellar_commands::command::Command;
use stellar_commands::document::{
    frame::{keys, Clipping},
    records::NodePayload,
};
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::modifier_ops::{InsertModifier, SetModifierProperty};
use stellar_commands::property::{PropertyValue, SetProperty};
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::color::Color;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::PropertyKey;
use stellar_core_types::value::{FiniteF32, Size2, Transform2D, Vec2};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierKind, ModifierPayload};
use stellar_document::modifier_metadata::keys::TRANSFORM_MATRIX;

#[derive(Clone, Copy)]
enum FrameEdit {
    Move(Vec2),
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
    /// 표시한 View와 revision에서 선택한 Frame을 부모 좌표계의 delta만큼 이동합니다.
    /// 활성 View, 선택 및 조상 잠금은 document thread에서 다시 확인합니다.
    pub fn move_canvas_frame(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        revision: DocumentRevision,
        node: StableId,
        delta: Vec2,
    ) -> Result<RequestId, RequestError> {
        self.edit_canvas_frame(session, view, revision, node, FrameEdit::Move(delta))
    }

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
    let command = match (edit, model.payload(node)) {
        (
            FrameEdit::Move(delta),
            Some(payload @ (NodePayload::Shape(_) | NodePayload::ComponentInstance(_))),
        ) => {
            if delta == Vec2::ZERO {
                return Ok(());
            }
            editable_ancestry(model, view, node, true).ok_or_else(stale)?;
            let stack = model.modifier_stack(node);
            let last_transform = match payload {
                NodePayload::ComponentInstance(instance) => {
                    // Place가 기록한 배치만 수정합니다. 별도 변환·override에는 손대지 않습니다.
                    let [entry] = stack.ok_or_else(stale)?.entries() else {
                        return Err(stale());
                    };
                    let ModifierPayload::Transform(matrix) = entry.payload else {
                        return Err(stale());
                    };
                    let (a, b, c, d, _, _) = matrix.components();
                    if !instance.has_no_overrides()
                        || !entry.enabled
                        || (a, b, c, d) != (1., 0., 0., 1.)
                    {
                        return Err(stale());
                    }
                    Some((entry.id, matrix))
                }
                _ => {
                    crate::canvas::shape::shape_style(stack)
                        .ok_or_else(stale)?
                        .last_transform
                }
            };
            if let Some((modifier, last)) = last_transform {
                let next = translated(last, delta).ok_or_else(stale)?;
                if next == last {
                    return Ok(());
                }
                Command::SetModifierProperty(SetModifierProperty {
                    node,
                    modifier,
                    key: TRANSFORM_MATRIX,
                    value: PropertyValue::Transform(next),
                })
            } else {
                let transform = translated(Transform2D::identity(), delta).ok_or_else(stale)?;
                let order = OrderKey::between(
                    None,
                    stack.and_then(|s| s.entries().first()).map(|e| &e.order),
                );
                let [id] = fresh_ids()?;
                Command::InsertModifier(InsertModifier {
                    node,
                    entry: ModifierEntry {
                        id: ModifierId::new(id),
                        order,
                        kind: ModifierKind::Transform,
                        enabled: true,
                        payload: ModifierPayload::Transform(transform),
                    },
                })
            }
        }
        _ => {
            let target = editable_frame(model, view, node).ok_or_else(stale)?;
            let (key, value) = match edit {
                FrameEdit::Move(delta) => {
                    let next = translated(target.transform, delta).ok_or_else(stale)?;
                    if next == target.transform {
                        return Ok(());
                    }
                    (keys::TRANSFORM, PropertyValue::Transform(next))
                }
                FrameEdit::Property(sequence, key, value) => {
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
                    match value {
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
                    }
                }
            };
            Command::SetProperty(SetProperty { node, key, value })
        }
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

fn translated(current: Transform2D, delta: Vec2) -> Option<Transform2D> {
    let (a, b, c, d, x, y) = current.components();
    Transform2D::new(a, b, c, d, x + delta.x(), y + delta.y()).ok()
}

fn editable_ancestry(
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
            Some(NodePayload::Shape(_) | NodePayload::ComponentInstance(_))
                if modifier_target && current == node => {}
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

    fn transform_of(window: &ProjectWindowSession, node: StableId) -> Transform2D {
        match window.document().unwrap().model().payload(node).unwrap() {
            NodePayload::Frame(frame) => frame.transform,
            _ => unreachable!(),
        }
    }

    #[test]
    fn move_uses_visible_revision_selection_and_persists_one_transform() {
        let mut model = sample_document();
        let node = sid(11);
        let Some(NodePayload::Frame(mut frame)) = model.payload(node).cloned() else {
            unreachable!()
        };
        let original = Transform2D::new(0.5, 0.25, -0.25, 0.5, 12.5, -6.0).unwrap();
        frame.transform = original;
        model.set_payload(node, NodePayload::Frame(frame)).unwrap();
        let project = saved_project_with("canvas-move", &[&model]);
        let mut session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        let delta = Vec2::new(16.0, -3.5).unwrap();
        let revision = session.revision();
        assert!(edit_selected(
            &mut session,
            RequestId(1),
            sid(3),
            revision,
            node,
            FrameEdit::Move(delta)
        )
        .is_err());
        session.select(Selection::new([node], None)).unwrap();
        assert!(edit_selected(
            &mut session,
            RequestId(2),
            sid(4),
            revision,
            node,
            FrameEdit::Move(delta)
        )
        .is_err());
        assert!(edit_selected(
            &mut session,
            RequestId(3),
            sid(3),
            revision,
            node,
            FrameEdit::Move(delta)
        )
        .is_ok());
        let translated = Transform2D::new(0.5, 0.25, -0.25, 0.5, 28.5, -9.5).unwrap();
        assert_eq!(transform_of(&session, node), translated);
        assert!(edit_selected(
            &mut session,
            RequestId(4),
            sid(3),
            revision,
            node,
            FrameEdit::Move(delta)
        )
        .is_err());
        assert!(session.undo().unwrap());
        assert_eq!(transform_of(&session, node), original);
        assert_eq!(session.view().selection().ids(), [node]);
        assert!(session.redo().unwrap());
        session.save_now().unwrap();
        let reopened = ProjectWindowSession::load(identity(project.path()), window(2)).unwrap();
        assert_eq!(transform_of(&reopened, node), translated);
        assert_eq!(reopened.generation_number(), 2);
    }

    #[test]
    fn moving_a_placed_instance_only_edits_its_single_active_translation() {
        use stellar_core_types::reference::ComponentRef;
        use stellar_document::component::instance::ComponentInstancePayload;
        use stellar_document::modifier::ModifierStack;
        let mut model = sample_document();
        let node = sid(30);
        let mut record = model.record(sid(11)).unwrap();
        record.id = node;
        record.parent = Some(sid(11));
        record.payload = NodePayload::ComponentInstance(ComponentInstancePayload::new(
            ComponentRef::new(sid(90)),
        ));
        model.insert_node(record).unwrap();
        let modifier = ModifierId::new(sid(31));
        let mut placement = ModifierStack::new(node);
        placement
            .push_back(modifier, ModifierKind::Transform)
            .unwrap();
        placement
            .set_payload(
                modifier,
                ModifierPayload::Transform(Transform2D::new(1., 0., 0., 1., 15., -3.).unwrap()),
            )
            .unwrap();
        for case in [
            "placed",
            "no placement",
            "disabled",
            "scaled",
            "additional",
            "locked parent",
        ] {
            let (mut scene, _) = DocumentModel::from_records(model.to_records().unwrap()).unwrap();
            let mut stack = placement.clone();
            match case {
                "disabled" => {
                    stack.set_enabled(modifier, false).unwrap();
                }
                "scaled" => {
                    stack
                        .set_payload(
                            modifier,
                            ModifierPayload::Transform(
                                Transform2D::new(2., 0., 0., 1., 15., -3.).unwrap(),
                            ),
                        )
                        .unwrap();
                }
                "additional" => {
                    stack
                        .push_back(ModifierId::new(sid(32)), ModifierKind::Transform)
                        .unwrap();
                }
                "locked parent" => {
                    scene.replace_flags(sid(11), true, true).unwrap();
                }
                _ => {}
            }
            if case != "no placement" {
                scene.set_modifier_stack(stack).unwrap();
            }
            let directory = saved_project_with("instance-move", &[&scene]);
            let mut session =
                ProjectWindowSession::load(identity(directory.path()), window(1)).unwrap();
            session.select(Selection::new([node], None)).unwrap();
            let revision = session.revision();
            let edit = FrameEdit::Move(Vec2::new(7., -4.).unwrap());
            let moved = edit_selected(&mut session, RequestId(1), sid(3), revision, node, edit);
            if case != "placed" {
                assert!(moved.is_err(), "{case}");
                assert_eq!(session.revision(), revision);
                assert_eq!(
                    session.document().unwrap().model().to_records().unwrap(),
                    scene.to_records().unwrap()
                );
                continue;
            }
            assert!(moved.is_ok());
            assert!(
                edit_selected(&mut session, RequestId(2), sid(3), revision, node, edit).is_err()
            );
            let mut expected = placement.clone();
            expected
                .set_payload(
                    modifier,
                    ModifierPayload::Transform(Transform2D::new(1., 0., 0., 1., 22., -7.).unwrap()),
                )
                .unwrap();
            assert_eq!(
                session.document().unwrap().model().modifier_stack(node),
                Some(&expected)
            );
            assert!(session.undo().unwrap());
            assert_eq!(
                session.document().unwrap().model().to_records().unwrap(),
                scene.to_records().unwrap()
            );
        }
    }

    #[test]
    fn rectangle_translation_preserves_geometry_and_other_modifier_entries() {
        use stellar_document::frame::CornerRadius;
        use stellar_document::modifier::ModifierStack;
        use stellar_document::records::NodeRecord;
        use stellar_document::shape::{RectangleParams, ShapeGeometry};
        let delta = Vec2::new(7.0, -3.0).unwrap();
        let before = Transform2D::new(0.0, 2.0, -0.5, 0.0, -11.0, 8.0).unwrap();
        for has_transform in [false, true] {
            let mut model = sample_document();
            let parent = sid(11);
            let id = sid(30);
            let order_key = model.next_child_order_key(parent);
            let payload = NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
                size: Size2::new(40., 12.).unwrap(),
                corner_radius: CornerRadius::ZERO,
            }));
            model
                .insert_node(NodeRecord {
                    id,
                    parent: Some(parent),
                    order_key,
                    visible: true,
                    locked: false,
                    display_name: "Rectangle".into(),
                    payload: payload.clone(),
                })
                .unwrap();
            let mut stack = ModifierStack::new(id);
            let first = ModifierId::new(sid(31));
            let last = ModifierId::new(sid(32));
            if has_transform {
                stack.push_back(first, ModifierKind::Transform).unwrap();
                stack
                    .set_payload(
                        first,
                        ModifierPayload::Transform(
                            Transform2D::new(2., 0., 0., 1., 3., 4.).unwrap(),
                        ),
                    )
                    .unwrap();
                stack.push_back(last, ModifierKind::Transform).unwrap();
                stack
                    .set_payload(last, ModifierPayload::Transform(before))
                    .unwrap();
            }
            stack
                .push_back(ModifierId::new(sid(33)), ModifierKind::Fill)
                .unwrap();
            let disabled = ModifierId::new(sid(34));
            stack.push_back(disabled, ModifierKind::Transform).unwrap();
            stack.set_enabled(disabled, false).unwrap();
            model.set_modifier_stack(stack.clone()).unwrap();
            let dir = saved_project_with("rectangle-move-stack", &[&model]);
            let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
            session.select(Selection::new([id], None)).unwrap();
            let revision = session.revision();
            assert!(edit_selected(
                &mut session,
                RequestId(11),
                sid(3),
                revision,
                id,
                FrameEdit::Move(delta)
            )
            .is_ok());
            let changed = session
                .document()
                .unwrap()
                .model()
                .modifier_stack(id)
                .unwrap()
                .clone();
            let mut expected = stack.clone();
            if has_transform {
                expected
                    .set_payload(
                        last,
                        ModifierPayload::Transform(translated(before, delta).unwrap()),
                    )
                    .unwrap();
            } else {
                let inserted = &changed.entries()[0];
                // 새 Transform modifier는 command ID에서 파생하지 않은 독립 UUIDv7입니다.
                let fresh = inserted.id.stable_id();
                assert_eq!((fresh.version(), fresh.variant()), (7, 2));
                assert_ne!(
                    fresh.to_bytes(),
                    command_id(session.window(), RequestId(11)).to_bytes()
                );
                assert_eq!(
                    inserted.payload,
                    ModifierPayload::Transform(translated(Transform2D::identity(), delta).unwrap())
                );
                expected.insert(inserted.clone()).unwrap();
            }
            assert_eq!(changed, expected);
            assert_eq!(session.view().selection().ids(), [id]);
            assert!(session.undo().unwrap());
            assert_eq!(
                session.document().unwrap().model().modifier_stack(id),
                Some(&stack)
            );
            assert!(session.redo().unwrap());
            session.save_now().unwrap();
            let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
            assert_eq!(
                reopened.document().unwrap().model().payload(id),
                Some(&payload)
            );
            assert_eq!(
                reopened.document().unwrap().model().modifier_stack(id),
                Some(&expected)
            );
            model.replace_flags(parent, true, true).unwrap();
            let locked = saved_project_with("rectangle-move-locked", &[&model]);
            let mut session =
                ProjectWindowSession::load(identity(locked.path()), window(1)).unwrap();
            session.select(Selection::new([id], None)).unwrap();
            let revision = session.revision();
            assert!(edit_selected(
                &mut session,
                RequestId(12),
                sid(3),
                revision,
                id,
                FrameEdit::Move(delta)
            )
            .is_err());
            assert!(!session.has_unsaved_changes());
        }
    }

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
    fn locked_ancestor_prevents_moving_a_selected_child() {
        let mut model = sample_document();
        model.replace_flags(sid(10), true, true).unwrap();
        let project = saved_project_with("canvas-move-locked", &[&model]);
        let mut session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        let revision = session.revision();
        session.select(Selection::new([sid(11)], None)).unwrap();
        let delta = Vec2::new(12.0, -8.5).unwrap();
        let expected_selection = session.history_sequence();
        assert!(edit_selected(
            &mut session,
            RequestId(1),
            sid(3),
            revision,
            sid(11),
            FrameEdit::Move(delta)
        )
        .is_err());
        assert!(edit_selected(
            &mut session,
            RequestId(2),
            sid(3),
            revision,
            sid(11),
            FrameEdit::Property(
                expected_selection,
                keys::WIDTH,
                FrameValue::Length(FiniteF32::new(45.0).unwrap())
            )
        )
        .is_err());
        assert!(edit_selected(
            &mut session,
            RequestId(3),
            sid(3),
            revision,
            sid(11),
            FrameEdit::Property(
                expected_selection,
                keys::BACKGROUND,
                FrameValue::Background(None)
            )
        )
        .is_err());
        assert!(edit_selected(
            &mut session,
            RequestId(4),
            sid(3),
            revision,
            sid(11),
            FrameEdit::Property(
                expected_selection,
                keys::BORDER_TOP_LEFT_RADIUS,
                FrameValue::Length(FiniteF32::ZERO)
            )
        )
        .is_err());
        assert_eq!(session.revision(), revision);
        assert!(!session.has_unsaved_changes());
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
        let rev = app.state().window(session).unwrap().revision;
        let moved = app
            .move_canvas_frame(
                session,
                sid(3),
                rev,
                sid(11),
                Vec2::new(20.0, 10.0).unwrap(),
            )
            .unwrap();
        assert!(
            matches!(next_event(&app), RuntimeEvent::WindowViewChanged { request, .. } if request == moved)
        );
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

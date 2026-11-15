//! 캔버스 직접 조작(ADR-0071).
//!
//! 셸은 그린 행·화면 결과(포인터 아래 행, world delta, local 상자)만 보내고, 카메라·선택·편집의 의미와 command
//! 구성은 여기서 정합니다. 카메라 조작은 문서·history를 바꾸지 않습니다.

use super::frame_move::editable_ancestry;
use super::*;
use crate::canvas::CanvasScene;
use stellar_commands::selection::Selection;
use stellar_core_types::value::{Rect, Transform2D, Vec2};
use stellar_document::frame::CornerRadius;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierId, ModifierPayload};
use stellar_document::records::NodePayload;
use stellar_document::shape::ShapeGeometry;
use stellar_scene::bounds::Aabb;

/// 셸이 그린 행 위에서 고른 선택 의도(UX 명세 5.3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanvasPick {
    /// 포인터 아래 가장 위 행(`leaf`). `None`은 모든 Frame 밖입니다.
    Point {
        /// 셸이 그린 가장 위 행의 node.
        leaf: Option<StableId>,
        /// Shift: 선택에 넣고 빼기.
        toggle: bool,
        /// ⌘: 편집 범위와 관계없이 그 행을 바로 고릅니다.
        deep: bool,
        /// 더블클릭: 선택한 node 안으로 한 단계 들어갑니다.
        enter: bool,
    },
    /// world 영역. root의 직계 자식 가운데 겹치는(contain이면 완전히 포함된) node를 고릅니다.
    Area {
        /// 영역 한 꼭짓점(world).
        from: Vec2,
        /// 맞은편 꼭짓점(world).
        to: Vec2,
        /// Option: 완전히 포함된 것만.
        contain: bool,
        /// Shift: 지금 선택에 더합니다.
        add: bool,
    },
}

/// 캔버스가 표시하는 선택 노드 종류.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasNodeKind {
    /// 자유 배치 Frame.
    Frame,
    /// Rectangle Shape.
    Rectangle,
    /// Ellipse Shape.
    Ellipse,
    /// Line Shape.
    Line,
    /// Polygon Shape.
    Polygon,
    /// Text.
    Text,
    /// Image.
    Image,
    /// 배치한 ComponentInstance.
    Instance,
}

/// 반경 핸들 방식.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RadiusHandles {
    /// `border-*-radius` 네 longhand(Option으로 한 모서리).
    PerCorner,
    /// 값 하나(Polygon).
    Uniform,
}

/// 선택 표시와 핸들에 쓰는 노드 하나의 상자(코어가 정한 편집 가능 범위 포함).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanvasSelectionItem {
    /// 문서 node.
    pub id: StableId,
    /// 노드 종류.
    pub kind: CanvasNodeKind,
    /// local → world.
    pub world: Transform2D,
    /// local 상자(Frame·Rectangle·Text·Image는 원점 0, Ellipse·Polygon은 bounds, Line은 두 끝점 상자).
    pub rect: Rect,
    /// 저장된 모서리 반경(Frame·Rectangle은 네 모서리, Polygon은 같은 값 넷).
    pub radii: CornerRadius,
    /// 이동할 수 있는지(잠금·숨김·흐름 부모가 없음).
    pub movable: bool,
    /// 크기 핸들로 `width`·`height`를 바꿀 수 있는지.
    pub resizable: bool,
    /// 반경 핸들이 있는지와 그 방식.
    pub radius: Option<RadiusHandles>,
    /// 가로·세로가 고정 크기인지(아니면 크기 라벨에 `fit-content`).
    pub fixed: [bool; 2],
    /// Line의 시작·끝점(local). 끝점 핸들 자리입니다.
    pub line: Option<[Vec2; 2]>,
}

/// 캔버스 제스처가 만드는 문서 편집. 한 요청이 history 항목 하나입니다(UX 명세 5.7).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CanvasEdit {
    /// 선택한 node 전부(다른 선택의 자손은 빼고)를 옮깁니다.
    Move {
        /// 드래그를 시작한 행. 선택이 이 행을 덮지 않으면(Shift로 빠졌으면) 옮기지 않습니다. 키 이동은 `None`.
        anchor: Option<StableId>,
        /// 이동량. `parent_space`가 false이면 world, true이면 각 node의 부모 공간(CSS `left`·`top` px)입니다.
        delta: Vec2,
        /// 화살표 키 이동처럼 부모 공간 단위인지.
        parent_space: bool,
        /// Option 드래그: 복제본을 원본 바로 위에 만들어 옮기고 복제본을 선택합니다.
        duplicate: bool,
    },
    /// 하나뿐인 선택 node의 상자를 지금 local 좌표의 `rect`로 바꿉니다(크기 핸들). `width`·`height`는 고정 크기입니다.
    Resize {
        /// 대상(선택 전체와 같아야 함).
        node: StableId,
        /// 새 상자(지금 local 좌표, 크기 > 0).
        rect: Rect,
    },
    /// 하나뿐인 선택 Line의 끝점 하나를 지금 local 좌표 `point`로 옮깁니다.
    LineEnd {
        /// 대상 Line.
        node: StableId,
        /// false면 시작점, true면 끝점.
        end: bool,
        /// 새 끝점(local).
        point: Vec2,
    },
    /// 하나뿐인 선택 node의 모서리 반경(반경 핸들). Frame·Rectangle은 `border-*-radius` longhand, Polygon은 값 하나입니다.
    Radius {
        /// 대상.
        node: StableId,
        /// 한 모서리만(0 tl, 1 tr, 2 br, 3 bl, Option). `None`이면 네 모서리 모두입니다.
        corner: Option<usize>,
        /// 원본 반경(px, ≥ 0). 상자보다 큰 값은 평가할 때 줄어듭니다.
        radius: stellar_core_types::value::FiniteF32,
    },
}

/// 창 entry와 지금 표시 목록(Text 조합 미리보기 포함, 저장본 그대로인 창끼리는 공유 목록)을 함께 얻습니다.
fn window_scene(
    sessions: &mut AppSessions,
    session: WindowSessionHandle,
) -> Result<(&mut WindowEntry, Option<Arc<crate::canvas::CanvasScene>>), Failure> {
    let entry = sessions
        .windows
        .get_mut(&session)
        .ok_or_else(|| unknown_window(session))?;
    let preview = entry
        .text
        .as_ref()
        .and_then(|c| c.preview_at(entry.session.revision()));
    let scene = entry
        .canvas
        .current(&entry.session, preview, &mut sessions.shared_scenes);
    Ok((entry, scene))
}

impl AppRuntime {
    /// 셸이 그린 행에서 고른 선택을 코어 규칙(직계 자식, Shift 넣고 빼기, ⌘ 깊은 선택, 영역)으로 정해 기록합니다.
    pub fn canvas_pick(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        pick: CanvasPick,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let (entry, scene) = window_scene(ctx.app_sessions_mut(), session)?;
            let window = &mut entry.session;
            let selection = resolve_pick(window, scene.as_deref(), view, pick)?;
            window.select(selection).map_err(view_failure)?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// 캔버스 제스처 하나를 문서 편집 하나(Undo 한 번)로 기록합니다.
    pub fn canvas_edit(
        &self,
        session: WindowSessionHandle,
        view: StableId,
        edit: CanvasEdit,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let (entry, scene) = window_scene(ctx.app_sessions_mut(), session)?;
            let scene = scene.ok_or_else(stale_view)?;
            apply_edit(&mut entry.session, &scene, request, view, edit)?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }

    /// 화면 중심에서 `anchor` point인 점을 고정하고 배율을 `factor`배 합니다(핀치, ⌘ 스크롤).
    ///
    /// 배율 상하한은 viewport가 정합니다. 문서·history·저장 상태는 바뀌지 않습니다.
    pub fn zoom_viewport_by(
        &self,
        session: WindowSessionHandle,
        anchor: [f64; 2],
        factor: f64,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            change_view_on_thread(ctx, request, session, |window| {
                window.zoom(factor, anchor[0], anchor[1]).map(|_| ())
            })
        })
    }
}

fn apply_edit(
    window: &mut ProjectWindowSession,
    scene: &CanvasScene,
    request: RequestId,
    view: StableId,
    edit: CanvasEdit,
) -> Result<(), Failure> {
    if window.tabs().active_view() != Some(view) {
        return Err(stale_view());
    }
    match edit {
        CanvasEdit::Move {
            anchor,
            delta,
            parent_space,
            duplicate,
        } => move_selection(
            window,
            scene,
            request,
            view,
            anchor,
            delta,
            parent_space,
            duplicate,
        ),
        CanvasEdit::Resize { node, rect } => resize_node(window, request, view, node, rect),
        CanvasEdit::LineEnd { node, end, point } => {
            move_line_end(window, request, view, node, end, point)
        }
        CanvasEdit::Radius {
            node,
            corner,
            radius,
        } => set_radius(window, request, view, node, corner, radius),
    }
}

/// 반경 핸들 한 번(Undo 한 번).
fn set_radius(
    window: &mut ProjectWindowSession,
    request: RequestId,
    view: StableId,
    node: StableId,
    corner: Option<usize>,
    radius: stellar_core_types::value::FiniteF32,
) -> Result<(), Failure> {
    use stellar_commands::command::Command;
    use stellar_commands::document::frame::keys;
    use stellar_commands::property::{PropertyValue, SetProperty};
    use stellar_core_types::value::Angle;
    use stellar_document::shape::{PolygonParams, RectangleParams};
    if radius.get() < 0.0 || corner.is_some_and(|c| c > 3) {
        return Err(stale_view());
    }
    let (model, payload) = sole_editable(window, view, node)?;
    let corners = |current: CornerRadius| {
        let (tl, tr, br, bl) = current.components();
        let mut values = [tl, tr, br, bl];
        for (index, value) in values.iter_mut().enumerate() {
            if corner.is_none_or(|c| c == index) {
                *value = radius.get();
            }
        }
        values
    };
    let commands = match payload {
        NodePayload::Frame(frame) => {
            let (tl, tr, br, bl) = frame.corner_radius.components();
            let before = [tl, tr, br, bl];
            corners(frame.corner_radius)
                .into_iter()
                .zip(keys::BORDER_RADIUS_LONGHANDS)
                .zip(before)
                .filter(|((next, _), old)| next != old)
                .map(|((next, key), _)| {
                    Command::SetProperty(SetProperty {
                        node,
                        key,
                        value: PropertyValue::NonNegative(
                            stellar_core_types::value::FiniteF32::new(next).expect("유한 반경"),
                        ),
                    })
                })
                .collect()
        }
        NodePayload::Shape(ShapeGeometry::Rectangle(rect)) => {
            let [tl, tr, br, bl] = corners(rect.corner_radius);
            let next = CornerRadius::new(tl, tr, br, bl).map_err(|_| stale_view())?;
            if next == rect.corner_radius {
                return Ok(());
            }
            vec![replace_payload(
                model,
                node,
                NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
                    size: rect.size,
                    corner_radius: next,
                })),
            )?]
        }
        NodePayload::Shape(ShapeGeometry::Polygon(polygon)) => {
            if polygon.corner_radius() == radius.get() {
                return Ok(());
            }
            let rotation = Angle::new(polygon.rotation_degrees()).map_err(|_| stale_view())?;
            let next =
                PolygonParams::new(polygon.bounds(), polygon.sides(), rotation, radius.get())
                    .map_err(|_| stale_view())?;
            vec![replace_payload(
                model,
                node,
                NodePayload::Shape(ShapeGeometry::Polygon(next)),
            )?]
        }
        _ => return Err(stale_view()),
    };
    apply_commands(window, request, commands)
}

/// 하나뿐인 선택이 `node`이고 편집할 수 있는지 확인합니다.
fn sole_editable(
    window: &ProjectWindowSession,
    view: StableId,
    node: StableId,
) -> Result<(&DocumentModel, NodePayload), Failure> {
    if window.view().selection().ids() != [node] {
        return Err(stale_view());
    }
    let model = window.document().ok_or_else(stale_view)?.model();
    let payload = model.payload(node).ok_or_else(stale_view)?.clone();
    let frame = matches!(payload, NodePayload::Frame(_));
    editable_ancestry(model, view, node, !frame).ok_or_else(stale_view)?;
    Ok((model, payload))
}

/// local 원점 이동을 transform에 반영합니다(`T ∘ translate(origin)`).
fn shifted(transform: Transform2D, origin: Vec2) -> Option<Transform2D> {
    let (a, b, c, d, x, y) = transform.components();
    Transform2D::new(
        a,
        b,
        c,
        d,
        x + a * origin.x() + c * origin.y(),
        y + b * origin.x() + d * origin.y(),
    )
    .ok()
}

/// payload만 바꾸는 command(형제 순서·modifier는 그대로). 역 command도 같은 종류입니다.
fn replace_payload(
    model: &DocumentModel,
    node: StableId,
    payload: NodePayload,
) -> Result<stellar_commands::command::Command, Failure> {
    let mut record = model.record(node).ok_or_else(stale_view)?;
    record.payload = payload;
    Ok(stellar_commands::command::Command::RestoreNodeRecords(
        stellar_commands::convert::RestoreNodeRecords {
            records: vec![record],
        },
    ))
}

fn apply_commands(
    window: &mut ProjectWindowSession,
    request: RequestId,
    commands: Vec<stellar_commands::command::Command>,
) -> Result<(), Failure> {
    use stellar_commands::id::CommandOrigin;
    use stellar_commands::transaction::CommandTransaction;
    if commands.is_empty() {
        return Ok(());
    }
    let revision = window.revision();
    let id = super::frame_move::command_id(window.window(), request);
    let transaction = commands
        .into_iter()
        .fold(CommandTransaction::new(id, CommandOrigin::User), |tx, c| {
            tx.with_command(revision, c)
        });
    window.apply(&transaction).map_err(view_failure)?;
    Ok(())
}

/// 크기 핸들 한 번. Frame은 `width`·`height`(+원점이 바뀌면 `transform`), Shape·Image는 geometry와 Transform입니다.
fn resize_node(
    window: &mut ProjectWindowSession,
    request: RequestId,
    view: StableId,
    node: StableId,
    rect: Rect,
) -> Result<(), Failure> {
    use stellar_commands::command::Command;
    use stellar_commands::document::frame::keys;
    use stellar_commands::modifier_ops::{InsertModifier, SetModifierProperty};
    use stellar_commands::property::{PropertyValue, SetProperty};
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::value::Angle;
    use stellar_document::modifier::{ModifierEntry, ModifierKind};
    use stellar_document::modifier_metadata::keys::TRANSFORM_MATRIX;
    use stellar_document::shape::{EllipseParams, PolygonParams, RectangleParams};
    let (model, payload) = sole_editable(window, view, node)?;
    let size = rect.size();
    if size.width() <= 0.0 || size.height() <= 0.0 {
        return Err(stale_view());
    }
    let frame = matches!(payload, NodePayload::Frame(_));
    let (modifier, transform) = resize_transform(model, node, frame).ok_or_else(stale_view)?;
    let origin = rect.origin();
    let mut commands = Vec::new();
    let next = match payload {
        NodePayload::Frame(current) => {
            for command in SetProperty::frame_dimensions(node, size) {
                let old = if command.key == keys::WIDTH {
                    current.size.width()
                } else {
                    current.size.height()
                };
                if matches!(command.value, PropertyValue::NonNegative(v) if v.get() != old) {
                    commands.push(Command::SetProperty(command));
                }
            }
            if origin != Vec2::ZERO {
                commands.push(Command::SetProperty(SetProperty {
                    node,
                    key: keys::TRANSFORM,
                    value: PropertyValue::Transform(
                        shifted(transform, origin).ok_or_else(stale_view)?,
                    ),
                }));
            }
            return apply_commands(window, request, commands);
        }
        NodePayload::Shape(ShapeGeometry::Rectangle(current)) => {
            NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
                size,
                corner_radius: current.corner_radius,
            }))
        }
        // Ellipse·Polygon은 local bounds에 원점이 있어 transform을 바꾸지 않습니다.
        NodePayload::Shape(ShapeGeometry::Ellipse(_)) => NodePayload::Shape(
            ShapeGeometry::Ellipse(EllipseParams::new(rect).map_err(|_| stale_view())?),
        ),
        NodePayload::Shape(ShapeGeometry::Polygon(current)) => {
            let rotation = Angle::new(current.rotation_degrees()).map_err(|_| stale_view())?;
            NodePayload::Shape(ShapeGeometry::Polygon(
                PolygonParams::new(rect, current.sides(), rotation, current.corner_radius())
                    .map_err(|_| stale_view())?,
            ))
        }
        NodePayload::Shape(ShapeGeometry::Line(current)) => {
            // 끝점을 이전 상자에서 새 상자로 같은 비율로 옮깁니다(길이 0인 축은 그대로 평행 이동).
            let min_x = current.start.x().min(current.end.x());
            let min_y = current.start.y().min(current.end.y());
            let old_w = (current.end.x() - current.start.x()).abs();
            let old_h = (current.end.y() - current.start.y()).abs();
            let map = |p: Vec2| {
                let sx = if old_w > 0.0 {
                    size.width() / old_w
                } else {
                    1.0
                };
                let sy = if old_h > 0.0 {
                    size.height() / old_h
                } else {
                    1.0
                };
                Vec2::new(
                    origin.x() + (p.x() - min_x) * sx,
                    origin.y() + (p.y() - min_y) * sy,
                )
                .map_err(|_| stale_view())
            };
            let mut line = current;
            line.start = map(current.start)?;
            line.end = map(current.end)?;
            NodePayload::Shape(ShapeGeometry::Line(line))
        }
        NodePayload::Image(mut image) => {
            image.size = size;
            NodePayload::Image(image)
        }
        _ => return Err(stale_view()),
    };
    let moves_origin = matches!(
        next,
        NodePayload::Shape(ShapeGeometry::Rectangle(_)) | NodePayload::Image(_)
    );
    if model.payload(node) != Some(&next) {
        commands.push(replace_payload(model, node, next)?);
    }
    if moves_origin && origin != Vec2::ZERO {
        let next = shifted(transform, origin).ok_or_else(stale_view)?;
        commands.push(match modifier {
            Some(modifier) => Command::SetModifierProperty(SetModifierProperty {
                node,
                modifier,
                key: TRANSFORM_MATRIX,
                value: PropertyValue::Transform(next),
            }),
            None => {
                let stack = model.modifier_stack(node);
                let order = OrderKey::between(
                    None,
                    stack.and_then(|s| s.entries().first()).map(|e| &e.order),
                );
                let id = super::frame_move::command_id(window.window(), request);
                Command::InsertModifier(InsertModifier {
                    node,
                    entry: ModifierEntry {
                        id: ModifierId::new(derived_id(id, node)),
                        order,
                        kind: ModifierKind::Transform,
                        enabled: true,
                        payload: ModifierPayload::Transform(next),
                    },
                })
            }
        });
    }
    apply_commands(window, request, commands)
}

/// Line 끝점 핸들 한 번.
fn move_line_end(
    window: &mut ProjectWindowSession,
    request: RequestId,
    view: StableId,
    node: StableId,
    end: bool,
    point: Vec2,
) -> Result<(), Failure> {
    let (model, payload) = sole_editable(window, view, node)?;
    let NodePayload::Shape(ShapeGeometry::Line(current)) = payload else {
        return Err(stale_view());
    };
    let mut line = current;
    if end {
        line.end = point;
    } else {
        line.start = point;
    }
    if line == current {
        return Ok(());
    }
    let command = replace_payload(model, node, NodePayload::Shape(ShapeGeometry::Line(line)))?;
    apply_commands(window, request, vec![command])
}

/// 선택에서 다른 선택의 자손을 뺀 node(선택 순서).
fn selection_roots(
    model: &DocumentModel,
    root: StableId,
    selected: &[StableId],
) -> Option<Vec<StableId>> {
    let mut roots = Vec::with_capacity(selected.len());
    for &node in selected {
        let chain = ancestry(model, node, root)?;
        if !chain[1..].iter().any(|a| selected.contains(a)) {
            roots.push(node);
        }
    }
    Some(roots)
}

/// 문서 node별 첫 캔버스 행 번호.
fn row_index(scene: &CanvasScene) -> std::collections::BTreeMap<StableId, usize> {
    let mut rows = std::collections::BTreeMap::new();
    for (index, row) in scene.frames.iter().enumerate() {
        rows.entry(row.id).or_insert(index);
    }
    rows
}

/// world 벡터를 `node` 부모 공간 벡터로 바꿉니다(부모 행 world의 선형부 역변환). root는 world 그대로입니다.
fn parent_vector(
    scene: &CanvasScene,
    rows: &std::collections::BTreeMap<StableId, usize>,
    node: StableId,
    vector: Vec2,
) -> Option<Vec2> {
    let row = &scene.frames[*rows.get(&node)?];
    let parent = row
        .parent
        .map_or(Transform2D::identity(), |p| scene.frames[p as usize].world);
    let inverse = parent.inverse()?;
    let (a, b, c, d, _, _) = inverse.components();
    Vec2::new(
        a * vector.x() + c * vector.y(),
        b * vector.x() + d * vector.y(),
    )
    .ok()
}

fn translated(current: Transform2D, delta: Vec2) -> Option<Transform2D> {
    let (a, b, c, d, x, y) = current.components();
    Transform2D::new(a, b, c, d, x + delta.x(), y + delta.y()).ok()
}

/// 같은 요청 안에서 node마다 다른 영속 id(새 Transform modifier).
fn derived_id(command: stellar_commands::id::CommandId, node: StableId) -> StableId {
    let mut key = [0; 32];
    key[..16].copy_from_slice(&command.to_bytes());
    key[16..].copy_from_slice(&node.to_bytes());
    let digest = stellar_storage::hash::ContentHash::compute(&key);
    let mut raw = [0; 16];
    raw.copy_from_slice(&digest.digest()[..16]);
    StableId::from_bytes(raw)
}

/// `source`를 `delta`(부모 공간)만큼 옮기는 command를 `target`(복제본이면 remap 적용)에 대해 만듭니다.
/// Frame은 `transform`, 그 밖의 node는 마지막 Transform modifier(없으면 맨 앞에 새로)를 바꿉니다.
fn move_command(
    model: &DocumentModel,
    source: StableId,
    target: StableId,
    delta: Vec2,
    remap: Option<&std::collections::BTreeMap<StableId, StableId>>,
    new_modifier: StableId,
) -> Result<Option<stellar_commands::command::Command>, Failure> {
    use stellar_commands::command::Command;
    use stellar_commands::document::frame::keys;
    use stellar_commands::modifier_ops::{InsertModifier, SetModifierProperty};
    use stellar_commands::property::{PropertyValue, SetProperty};
    use stellar_core_types::order_key::OrderKey;
    use stellar_document::modifier::{ModifierEntry, ModifierKind};
    use stellar_document::modifier_metadata::keys::TRANSFORM_MATRIX;
    let modifier_of = |id: ModifierId| -> Result<ModifierId, Failure> {
        match remap {
            None => Ok(id),
            Some(map) => map
                .get(&id.stable_id())
                .map(|&copy| ModifierId::new(copy))
                .ok_or_else(stale_view),
        }
    };
    let stack = model.modifier_stack(source);
    let last = match model.payload(source).ok_or_else(stale_view)? {
        NodePayload::Frame(frame) => {
            let next = translated(frame.transform, delta).ok_or_else(stale_view)?;
            if next == frame.transform {
                return Ok(None);
            }
            return Ok(Some(Command::SetProperty(SetProperty {
                node: target,
                key: keys::TRANSFORM,
                value: PropertyValue::Transform(next),
            })));
        }
        NodePayload::ComponentInstance(instance) => {
            // 배치가 기록한 이동 하나만 바꿉니다. 별도 변환·override에는 손대지 않습니다.
            let [entry] = stack.ok_or_else(stale_view)?.entries() else {
                return Err(stale_view());
            };
            let ModifierPayload::Transform(matrix) = entry.payload else {
                return Err(stale_view());
            };
            let (a, b, c, d, _, _) = matrix.components();
            if !instance.has_no_overrides() || !entry.enabled || (a, b, c, d) != (1., 0., 0., 1.) {
                return Err(stale_view());
            }
            Some((entry.id, matrix))
        }
        NodePayload::Shape(_) | NodePayload::Text(_) | NodePayload::Image(_) => {
            crate::canvas::shape::shape_style(stack)
                .ok_or_else(stale_view)?
                .last_transform
        }
        _ => return Err(stale_view()),
    };
    Ok(Some(match last {
        Some((modifier, current)) => {
            let next = translated(current, delta).ok_or_else(stale_view)?;
            if next == current {
                return Ok(None);
            }
            Command::SetModifierProperty(SetModifierProperty {
                node: target,
                modifier: modifier_of(modifier)?,
                key: TRANSFORM_MATRIX,
                value: PropertyValue::Transform(next),
            })
        }
        None => {
            let order = OrderKey::between(
                None,
                stack.and_then(|s| s.entries().first()).map(|e| &e.order),
            );
            Command::InsertModifier(InsertModifier {
                node: target,
                entry: ModifierEntry {
                    id: ModifierId::new(new_modifier),
                    order,
                    kind: ModifierKind::Transform,
                    enabled: true,
                    payload: ModifierPayload::Transform(
                        translated(Transform2D::identity(), delta).ok_or_else(stale_view)?,
                    ),
                },
            })
        }
    }))
}

/// 형제 순서에서 `node` 바로 위(다음 형제 앞) 자리.
fn order_above(
    model: &DocumentModel,
    parent: StableId,
    node: StableId,
) -> Option<stellar_core_types::order_key::OrderKey> {
    let siblings = model.children_in_order(parent);
    let index = siblings.iter().position(|&s| s == node)?;
    let own = model.record(node)?.order_key;
    let next = siblings
        .get(index + 1)
        .and_then(|&n| model.record(n))
        .map(|r| r.order_key);
    Some(stellar_core_types::order_key::OrderKey::between(
        Some(&own),
        next.as_ref(),
    ))
}

#[allow(clippy::too_many_arguments)]
fn move_selection(
    window: &mut ProjectWindowSession,
    scene: &CanvasScene,
    request: RequestId,
    view: StableId,
    anchor: Option<StableId>,
    delta: Vec2,
    parent_space: bool,
    duplicate: bool,
) -> Result<(), Failure> {
    use stellar_commands::command::Command;
    use stellar_commands::duplicate::DuplicateSubtree;
    use stellar_commands::id::CommandOrigin;
    use stellar_commands::transaction::CommandTransaction;
    use stellar_core_types::{clock::ManualClock, ids::StableIdGenerator};
    let model = window.document().ok_or_else(stale_view)?.model();
    let root = view_root(model, view).ok_or_else(stale_view)?;
    let selected = window.view().selection().ids().to_vec();
    let roots = selection_roots(model, root, &selected).ok_or_else(stale_view)?;
    if roots.is_empty() {
        return Err(stale_view());
    }
    if let Some(anchor) = anchor {
        // 드래그를 시작한 행을 지금 선택이 덮어야 합니다(Shift로 뺀 node를 끌면 옮기지 않음).
        let chain = ancestry(model, anchor, root).ok_or_else(stale_view)?;
        if !chain.iter().any(|n| selected.contains(n)) {
            return Err(stale_view());
        }
    }
    if delta == Vec2::ZERO && !duplicate {
        return Ok(());
    }
    let rows = row_index(scene);
    let id = super::frame_move::command_id(window.window(), request);
    let mut generator = StableIdGenerator::new(u64::from_le_bytes(
        id.to_bytes()[..8].try_into().expect("eight bytes"),
    ));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_nanos()).ok())
        .ok_or_else(stale_view)?;
    let clock = ManualClock::new(nanos);
    let revision = window.revision();
    let mut transaction = CommandTransaction::new(id, CommandOrigin::User);
    let mut commands = 0;
    let mut copies = Vec::new();
    for &node in &roots {
        let frame = matches!(model.payload(node), Some(NodePayload::Frame(_)));
        editable_ancestry(model, view, node, !frame).ok_or_else(stale_view)?;
        let local = if parent_space {
            delta
        } else {
            parent_vector(scene, &rows, node, delta).ok_or_else(stale_view)?
        };
        let (target, remap) = if duplicate {
            if node == root {
                return Err(stale_view());
            }
            let parent = parent_of(model, node).ok_or_else(stale_view)?;
            let order = order_above(model, parent, node).ok_or_else(stale_view)?;
            let plan = DuplicateSubtree::plan(model, node, parent, order, &mut generator, &clock)
                .map_err(|_| stale_view())?;
            let copy = plan.copy_root().ok_or_else(stale_view)?;
            let remap: std::collections::BTreeMap<_, _> = plan.remap.iter().copied().collect();
            transaction = transaction.with_command(revision, Command::DuplicateSubtree(plan));
            commands += 1;
            copies.push(copy);
            (copy, Some(remap))
        } else {
            (node, None)
        };
        if let Some(command) = move_command(
            model,
            node,
            target,
            local,
            remap.as_ref(),
            derived_id(id, target),
        )? {
            transaction = transaction.with_command(revision, command);
            commands += 1;
        }
    }
    if commands == 0 {
        return Ok(());
    }
    if duplicate {
        let primary = copies.last().copied();
        window.apply_and_select(&transaction, Selection::new(copies, primary))
    } else {
        window.apply(&transaction)
    }
    .map_err(view_failure)?;
    Ok(())
}

fn stale_view() -> Failure {
    Failure::new(
        RequestFailure::UnknownView,
        "캔버스의 View·선택·편집 상태가 변경되었습니다",
    )
}

pub(super) fn view_root(model: &DocumentModel, view: StableId) -> Option<StableId> {
    model
        .definition()
        .pages
        .iter()
        .flat_map(|page| &page.views)
        .find(|v| v.id == view)
        .map(|v| v.root_frame)
}

pub(super) fn parent_of(model: &DocumentModel, node: StableId) -> Option<StableId> {
    let fields = model
        .runtime_id(node)
        .and_then(|id| model.columns().get(id))?;
    fields.parent.and_then(|id| model.stable_id(id))
}

/// `[node, 부모, …, root]`. root 아래가 아니면 `None`입니다.
fn ancestry(model: &DocumentModel, node: StableId, root: StableId) -> Option<Vec<StableId>> {
    let mut chain = vec![node];
    let mut current = node;
    while current != root {
        current = parent_of(model, current)?;
        chain.push(current);
    }
    Some(chain)
}

/// 클릭이 고를 node. 지금 선택과 같은 깊이(선택의 부모가 조상이면 그 자식)를 먼저, 없으면 root의 직계 자식을 고릅니다.
fn point_target(
    model: &DocumentModel,
    root: StableId,
    current: &[StableId],
    leaf: StableId,
    deep: bool,
    enter: bool,
) -> Option<StableId> {
    let chain = ancestry(model, leaf, root)?;
    if deep {
        return Some(leaf);
    }
    for i in 1..chain.len() {
        let scope = chain[i];
        let entered = enter && current.contains(&scope);
        let sibling = current
            .iter()
            .any(|&s| s != scope && parent_of(model, s) == Some(scope));
        if entered || sibling {
            return Some(chain[i - 1]);
        }
    }
    Some(match chain.len() {
        1 => root,
        n => chain[n - 2],
    })
}

fn resolve_pick(
    window: &ProjectWindowSession,
    scene: Option<&CanvasScene>,
    view: StableId,
    pick: CanvasPick,
) -> Result<Selection, Failure> {
    if window.tabs().active_view() != Some(view) {
        return Err(stale_view());
    }
    let model = window.document().ok_or_else(stale_view)?.model();
    let root = view_root(model, view).ok_or_else(stale_view)?;
    let current = window.view().selection().clone();
    let ids = current.ids();
    match pick {
        CanvasPick::Point {
            leaf,
            toggle,
            deep,
            enter,
        } => {
            let target = leaf.and_then(|leaf| point_target(model, root, ids, leaf, deep, enter));
            Ok(match (target, toggle) {
                (None, false) => Selection::empty(),
                (None, true) => current,
                (Some(target), false) => Selection::new([target], Some(target)),
                (Some(target), true) if current.contains(target) => {
                    let rest: Vec<StableId> =
                        ids.iter().copied().filter(|&id| id != target).collect();
                    let primary = current
                        .primary()
                        .filter(|&p| p != target)
                        .or(rest.last().copied());
                    Selection::new(rest, primary)
                }
                (Some(target), true) => {
                    Selection::new(ids.iter().copied().chain([target]), Some(target))
                }
            })
        }
        CanvasPick::Area {
            from,
            to,
            contain,
            add,
        } => {
            let area = Aabb::new(
                from.x().min(to.x()),
                from.y().min(to.y()),
                from.x().max(to.x()),
                from.y().max(to.y()),
            )
            .ok_or_else(stale_view)?;
            let bounds = scene.map(world_bounds).unwrap_or_default();
            let hits: Vec<StableId> = model
                .children_in_order(root)
                .into_iter()
                .filter(|child| {
                    let open = model
                        .runtime_id(*child)
                        .and_then(|id| model.columns().get(id))
                        .is_some_and(|f| f.visible && !f.locked);
                    open && bounds.get(child).is_some_and(|b| {
                        if contain {
                            b.min_x() >= area.min_x()
                                && b.min_y() >= area.min_y()
                                && b.max_x() <= area.max_x()
                                && b.max_y() <= area.max_y()
                        } else {
                            b.intersects(&area)
                        }
                    })
                })
                .collect();
            Ok(if add {
                let primary = hits.last().copied().or(current.primary());
                Selection::new(ids.iter().copied().chain(hits), primary)
            } else {
                let primary = hits.last().copied();
                Selection::new(hits, primary)
            })
        }
    }
}

/// 문서 node별 world 경계. 같은 node의 행(인스턴스 투영)은 합칩니다.
fn world_bounds(scene: &CanvasScene) -> std::collections::BTreeMap<StableId, Aabb> {
    let mut bounds = std::collections::BTreeMap::new();
    for row in &scene.frames {
        let Some(b) = Aabb::from_rect(row.geometry.rect).transformed(&row.world) else {
            continue;
        };
        bounds
            .entry(row.id)
            .and_modify(|a: &mut Aabb| *a = a.union(&b))
            .or_insert(b);
    }
    bounds
}

/// 표시한 선택의 상자와 편집 가능 범위. 셸은 이 값으로 외곽선·핸들·크기 라벨만 그립니다.
pub(crate) fn selection_items(
    session: &ProjectWindowSession,
    scene: Option<&CanvasScene>,
) -> std::sync::Arc<[CanvasSelectionItem]> {
    let (Some(scene), Some(document), Some(view)) =
        (scene, session.document(), session.tabs().active_view())
    else {
        return std::sync::Arc::from([]);
    };
    let model = document.model();
    let ids = session.view().selection().ids();
    let mut items = Vec::with_capacity(ids.len());
    for &id in ids {
        let (Some(row), Some(payload)) = (
            scene.frames.iter().find(|row| row.id == id),
            model.payload(id),
        ) else {
            continue;
        };
        let (kind, radii, radius) = match payload {
            NodePayload::Frame(frame) => (
                CanvasNodeKind::Frame,
                frame.corner_radius,
                Some(RadiusHandles::PerCorner),
            ),
            NodePayload::Shape(ShapeGeometry::Rectangle(rect)) => (
                CanvasNodeKind::Rectangle,
                rect.corner_radius,
                Some(RadiusHandles::PerCorner),
            ),
            NodePayload::Shape(ShapeGeometry::Polygon(polygon)) => {
                let r = polygon.corner_radius();
                (
                    CanvasNodeKind::Polygon,
                    CornerRadius::new(r, r, r, r).unwrap_or(CornerRadius::ZERO),
                    Some(RadiusHandles::Uniform),
                )
            }
            NodePayload::Shape(ShapeGeometry::Ellipse(_)) => {
                (CanvasNodeKind::Ellipse, CornerRadius::ZERO, None)
            }
            NodePayload::Shape(ShapeGeometry::Line(_)) => {
                (CanvasNodeKind::Line, CornerRadius::ZERO, None)
            }
            NodePayload::Text(_) => (CanvasNodeKind::Text, CornerRadius::ZERO, None),
            NodePayload::Image(_) => (CanvasNodeKind::Image, CornerRadius::ZERO, None),
            NodePayload::ComponentInstance(_) => {
                (CanvasNodeKind::Instance, CornerRadius::ZERO, None)
            }
            _ => continue,
        };
        let frame = kind == CanvasNodeKind::Frame;
        let movable = editable_ancestry(model, view, id, !frame).is_some();
        let resizable = movable
            && !matches!(kind, CanvasNodeKind::Text | CanvasNodeKind::Instance)
            && resize_transform(model, id, frame).is_some();
        let fixed = kind != CanvasNodeKind::Text;
        let line = match payload {
            NodePayload::Shape(ShapeGeometry::Line(line)) => Some([line.start, line.end]),
            _ => None,
        };
        items.push(CanvasSelectionItem {
            id,
            kind,
            world: row.world,
            rect: row.geometry.rect,
            radii,
            movable,
            resizable,
            radius: radius.filter(|_| resizable),
            fixed: [fixed, fixed],
            line,
        });
    }
    items.into()
}

/// 크기를 바꿀 때 원점 이동을 기록할 변환. Frame은 자기 transform, 그 밖은 하나뿐인 Transform modifier(없으면 항등)입니다.
pub(super) fn resize_transform(
    model: &DocumentModel,
    node: StableId,
    frame: bool,
) -> Option<(Option<ModifierId>, Transform2D)> {
    if frame {
        let Some(NodePayload::Frame(payload)) = model.payload(node) else {
            return None;
        };
        return Some((None, payload.transform));
    }
    let stack = model.modifier_stack(node);
    let style = crate::canvas::shape::shape_style(stack)?;
    let transforms = stack.map_or(0, |stack| {
        stack
            .entries()
            .iter()
            .filter(|e| e.enabled && matches!(e.payload, ModifierPayload::Transform(_)))
            .count()
    });
    match (transforms, style.last_transform) {
        (0, None) => Some((None, Transform2D::identity())),
        (1, Some((id, transform))) => Some((Some(id), transform)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_window_tests::{next_event, open_project, open_window};
    use crate::canvas::CanvasCache;
    use crate::window_session::test_fixtures::{
        identity, sample_document, saved_project_with, sid, window,
    };
    use stellar_core_types::value::Size2;
    use stellar_document::frame::FramePayload;
    use stellar_document::records::NodeRecord;

    fn placed(w: f32, h: f32, x: f32, y: f32) -> NodePayload {
        NodePayload::Frame(FramePayload {
            size: Size2::new(w, h).unwrap(),
            transform: Transform2D::new(1., 0., 0., 1., x, y).unwrap(),
            ..FramePayload::default()
        })
    }

    /// View 3의 root 10(400×300) 아래 11(10,10 100×100)과 그 자식 13(5,5 20×20), 12(200,50 50×50).
    pub(super) fn nested_document() -> DocumentModel {
        let mut model = sample_document();
        model
            .set_payload(sid(10), placed(400., 300., 0., 0.))
            .unwrap();
        model
            .set_payload(sid(11), placed(100., 100., 10., 10.))
            .unwrap();
        model
            .set_payload(sid(12), placed(50., 50., 200., 50.))
            .unwrap();
        let mut child = model.record(sid(11)).unwrap();
        child.id = sid(13);
        child.parent = Some(sid(11));
        child.payload = placed(20., 20., 5., 5.);
        model
            .insert_node(NodeRecord {
                display_name: "n13".into(),
                ..child
            })
            .unwrap();
        model
    }

    fn pick(
        window: &mut ProjectWindowSession,
        view: u64,
        pick: CanvasPick,
    ) -> Option<Vec<StableId>> {
        let scene = CanvasCache::default().current(window, None, &mut Default::default());
        let selection = resolve_pick(window, scene.as_deref(), sid(view), pick).ok()?;
        window.select(selection).unwrap();
        Some(window.view().selection().ids().to_vec())
    }

    fn point(leaf: Option<u64>, toggle: bool, deep: bool, enter: bool) -> CanvasPick {
        CanvasPick::Point {
            leaf: leaf.map(sid),
            toggle,
            deep,
            enter,
        }
    }

    fn area(from: (f32, f32), to: (f32, f32), contain: bool, add: bool) -> CanvasPick {
        CanvasPick::Area {
            from: Vec2::new(from.0, from.1).unwrap(),
            to: Vec2::new(to.0, to.1).unwrap(),
            contain,
            add,
        }
    }

    #[test]
    fn click_selects_at_selection_depth_with_toggle_deep_enter_and_area() {
        let dir = saved_project_with("canvas-pick", &[&nested_document()]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        let revision = session.revision();
        let ids = |v: &[u64]| v.iter().copied().map(sid).collect::<Vec<_>>();
        // 깊은 행을 눌러도 root의 직계 자식을 고르고, 더블클릭은 한 단계 들어갑니다.
        assert_eq!(
            pick(&mut session, 3, point(Some(13), false, false, false)).unwrap(),
            ids(&[11])
        );
        assert_eq!(
            pick(&mut session, 3, point(Some(13), false, false, true)).unwrap(),
            ids(&[13])
        );
        // 선택과 같은 깊이가 없으면 다시 root의 직계 자식입니다.
        assert_eq!(
            pick(&mut session, 3, point(Some(12), false, false, false)).unwrap(),
            ids(&[12])
        );
        // ⌘는 깊은 선택, Shift는 넣고 빼기입니다.
        assert_eq!(
            pick(&mut session, 3, point(Some(13), false, true, false)).unwrap(),
            ids(&[13])
        );
        assert_eq!(
            pick(&mut session, 3, point(Some(12), true, false, false)).unwrap(),
            ids(&[13, 12])
        );
        assert_eq!(session.view().selection().primary(), Some(sid(12)));
        assert_eq!(
            pick(&mut session, 3, point(Some(12), true, false, false)).unwrap(),
            ids(&[13])
        );
        // root 배경 클릭은 root, 모든 Frame 밖 클릭은 비움, Shift 빈 곳은 그대로입니다.
        assert_eq!(
            pick(&mut session, 3, point(Some(10), true, false, false)).unwrap(),
            ids(&[13, 10])
        );
        assert_eq!(
            pick(&mut session, 3, point(None, true, false, false)).unwrap(),
            ids(&[13, 10])
        );
        assert_eq!(
            pick(&mut session, 3, point(None, false, false, false)).unwrap(),
            ids(&[])
        );
        // 영역: 겹치는 root 자식, Option은 완전히 포함된 것만, Shift는 더하기.
        assert_eq!(
            pick(&mut session, 3, area((150., 150.), (0., 0.), false, false)).unwrap(),
            ids(&[11])
        );
        assert_eq!(
            pick(&mut session, 3, area((0., 0.), (105., 105.), true, false)).unwrap(),
            ids(&[])
        );
        assert_eq!(
            pick(&mut session, 3, area((0., 0.), (300., 120.), true, false)).unwrap(),
            ids(&[11, 12])
        );
        pick(&mut session, 3, point(Some(12), false, false, false)).unwrap();
        assert_eq!(
            pick(&mut session, 3, area((0., 0.), (50., 50.), false, true)).unwrap(),
            ids(&[12, 11])
        );
        // 다른 View로 보낸 요청은 거부하고 선택을 바꾸지 않습니다.
        let before = session.view().selection().clone();
        assert!(pick(&mut session, 4, point(Some(11), false, false, false)).is_none());
        assert_eq!(session.view().selection(), &before);
        assert_eq!(session.revision(), revision);
        assert!(!session.has_unsaved_changes());
    }

    #[test]
    fn area_skips_locked_and_hidden_children() {
        let mut model = nested_document();
        model.replace_flags(sid(12), true, true).unwrap();
        model.replace_flags(sid(11), false, false).unwrap();
        let dir = saved_project_with("canvas-pick-locked", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        assert!(
            pick(&mut session, 3, area((0., 0.), (400., 300.), false, false))
                .unwrap()
                .is_empty()
        );
    }

    /// nested_document에 root 아래 Rectangle 14(300,200 40×30, Transform·Fill)를 더하고 11을 2배로 키웁니다.
    fn shape_document() -> DocumentModel {
        use stellar_core_types::color::{Color, ColorSpace};
        use stellar_core_types::order_key::OrderKey;
        use stellar_document::modifier::{ModifierEntry, ModifierStack};
        use stellar_document::shape::RectangleParams;
        let mut model = nested_document();
        model
            .set_payload(
                sid(11),
                NodePayload::Frame(FramePayload {
                    size: Size2::new(100., 100.).unwrap(),
                    transform: Transform2D::new(2., 0., 0., 2., 10., 10.).unwrap(),
                    ..FramePayload::default()
                }),
            )
            .unwrap();
        let mut record = model.record(sid(12)).unwrap();
        record.id = sid(14);
        record.order_key = OrderKey::between(Some(&record.order_key), None);
        record.display_name = "n14".into();
        record.payload = NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
            size: Size2::new(40., 30.).unwrap(),
            corner_radius: CornerRadius::ZERO,
        }));
        model.insert_node(record).unwrap();
        let mut stack = ModifierStack::new(sid(14));
        let gray = Color::new(ColorSpace::Srgb, 0.85, 0.85, 0.85, 1.).unwrap();
        for (n, payload) in [
            ModifierPayload::Transform(Transform2D::new(1., 0., 0., 1., 300., 200.).unwrap()),
            ModifierPayload::Fill { color: gray },
        ]
        .into_iter()
        .enumerate()
        {
            let order = OrderKey::between(stack.entries().last().map(|e| &e.order), None);
            stack
                .insert(ModifierEntry {
                    id: ModifierId::new(sid(140 + n as u64)),
                    order,
                    kind: payload.kind(),
                    enabled: true,
                    payload,
                })
                .unwrap();
        }
        model.set_modifier_stack(stack).unwrap();
        model
    }

    fn frame_transform(window: &ProjectWindowSession, node: u64) -> (f32, f32, f32, f32, f32, f32) {
        match window
            .document()
            .unwrap()
            .model()
            .payload(sid(node))
            .unwrap()
        {
            NodePayload::Frame(frame) => frame.transform.components(),
            _ => panic!("Frame"),
        }
    }

    fn shape_translation(window: &ProjectWindowSession, node: StableId) -> (f32, f32) {
        let stack = window
            .document()
            .unwrap()
            .model()
            .modifier_stack(node)
            .unwrap();
        match stack.entries()[0].payload {
            ModifierPayload::Transform(t) => {
                let (_, _, _, _, x, y) = t.components();
                (x, y)
            }
            _ => panic!("Transform"),
        }
    }

    fn drag(
        window: &mut ProjectWindowSession,
        request: u64,
        anchor: Option<u64>,
        delta: (f32, f32),
        parent_space: bool,
        duplicate: bool,
    ) -> bool {
        let scene = CanvasCache::default().current(window, None, &mut Default::default()).unwrap();
        apply_edit(
            window,
            &scene,
            RequestId(request),
            sid(3),
            CanvasEdit::Move {
                anchor: anchor.map(sid),
                delta: Vec2::new(delta.0, delta.1).unwrap(),
                parent_space,
                duplicate,
            },
        )
        .is_ok()
    }

    #[test]
    fn moving_a_selection_moves_every_root_once_in_parent_space_and_undoes_once() {
        let dir = saved_project_with("canvas-move-many", &[&shape_document()]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        // 부모(11)와 그 자식(13)을 함께 골라도 자식은 부모와 함께 한 번만 움직입니다.
        session
            .select(Selection::new([sid(11), sid(13), sid(14)], Some(sid(14))))
            .unwrap();
        let undo = session.history().history().len();
        assert!(drag(&mut session, 1, Some(13), (10., 5.), false, false));
        assert_eq!(frame_transform(&session, 11), (2., 0., 0., 2., 20., 15.));
        assert_eq!(frame_transform(&session, 13), (1., 0., 0., 1., 5., 5.));
        assert_eq!(shape_translation(&session, sid(14)), (310., 205.));
        assert_eq!(session.history().history().len(), undo + 1);
        assert!(session.undo().unwrap());
        assert_eq!(frame_transform(&session, 11), (2., 0., 0., 2., 10., 10.));
        assert_eq!(shape_translation(&session, sid(14)), (300., 200.));
        // 2배 부모 안의 자식은 world 10px이 부모 공간 5px입니다. 화살표 이동은 부모 공간 그대로입니다.
        session
            .select(Selection::new([sid(13)], Some(sid(13))))
            .unwrap();
        assert!(drag(&mut session, 2, Some(13), (10., 0.), false, false));
        assert_eq!(frame_transform(&session, 13), (1., 0., 0., 1., 10., 5.));
        assert!(drag(&mut session, 3, None, (0., -10.), true, false));
        assert_eq!(frame_transform(&session, 13), (1., 0., 0., 1., 10., -5.));
        // 선택이 드래그를 시작한 행을 덮지 않으면 옮기지 않습니다.
        session
            .select(Selection::new([sid(12)], Some(sid(12))))
            .unwrap();
        let revision = session.revision();
        assert!(!drag(&mut session, 4, Some(13), (10., 0.), false, false));
        assert_eq!(session.revision(), revision);
        session.save_now().unwrap();
        let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        assert_eq!(frame_transform(&reopened, 13), (1., 0., 0., 1., 10., -5.));
    }

    #[test]
    fn option_drag_duplicates_right_above_each_source_and_selects_the_copies() {
        let dir = saved_project_with("canvas-move-duplicate", &[&shape_document()]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session
            .select(Selection::new([sid(11), sid(14)], Some(sid(14))))
            .unwrap();
        let before = session
            .document()
            .unwrap()
            .model()
            .children_in_order(sid(10));
        assert!(drag(&mut session, 7, Some(14), (20., 0.), false, true));
        let model = session.document().unwrap().model();
        let after = model.children_in_order(sid(10));
        assert_eq!(after.len(), before.len() + 2);
        let copies = session.view().selection().ids().to_vec();
        assert_eq!(copies.len(), 2);
        // 복제본은 각 원본 바로 위(다음 형제 앞)에 있고, 원본은 그대로입니다.
        let at = |id: StableId| after.iter().position(|&n| n == id).unwrap();
        assert_eq!(at(copies[0]), at(sid(11)) + 1);
        assert_eq!(at(copies[1]), at(sid(14)) + 1);
        assert_eq!(frame_transform(&session, 11), (2., 0., 0., 2., 10., 10.));
        assert_eq!(shape_translation(&session, sid(14)), (300., 200.));
        let NodePayload::Frame(copy) = model.payload(copies[0]).unwrap() else {
            panic!("Frame copy");
        };
        assert_eq!(copy.transform.components(), (2., 0., 0., 2., 30., 10.));
        assert_eq!(model.children_in_order(copies[0]).len(), 1);
        assert_eq!(shape_translation(&session, copies[1]), (320., 200.));
        // 복제·이동·선택은 Undo 한 번입니다.
        assert!(session.undo().unwrap());
        let model = session.document().unwrap().model();
        assert_eq!(model.children_in_order(sid(10)), before);
        assert_eq!(session.view().selection().ids(), [sid(11), sid(14)]);
        // root는 복제할 수 없습니다.
        session
            .select(Selection::new([sid(10)], Some(sid(10))))
            .unwrap();
        assert!(!drag(&mut session, 8, Some(10), (5., 0.), false, true));
    }

    /// shape_document에 root 아래 Ellipse 15, Line 16, Polygon 17(각 Transform 하나)을 더합니다.
    fn all_shapes_document() -> DocumentModel {
        use stellar_core_types::color::{Color, ColorSpace};
        use stellar_core_types::order_key::OrderKey;
        use stellar_core_types::value::Angle;
        use stellar_document::modifier::{ModifierEntry, ModifierStack};
        use stellar_document::shape::{EllipseParams, LineCap, LineParams, PolygonParams};
        let mut model = shape_document();
        let point = |x, y| Vec2::new(x, y).unwrap();
        let bounds = Rect::new(Vec2::ZERO, Size2::new(40., 20.).unwrap());
        let shapes = [
            (
                15,
                ShapeGeometry::Ellipse(EllipseParams::new(bounds).unwrap()),
            ),
            (
                16,
                ShapeGeometry::Line(LineParams {
                    start: Vec2::ZERO,
                    end: point(30., 10.),
                    cap: LineCap::Butt,
                }),
            ),
            (
                17,
                ShapeGeometry::Polygon(
                    PolygonParams::new(bounds, 6, Angle::new(0.).unwrap(), 3.).unwrap(),
                ),
            ),
        ];
        let gray = Color::new(ColorSpace::Srgb, 0.85, 0.85, 0.85, 1.).unwrap();
        for (n, geometry) in shapes {
            let mut record = model.record(sid(14)).unwrap();
            record.id = sid(n);
            record.order_key = model.next_child_order_key(sid(10));
            record.display_name = format!("n{n}");
            record.payload = NodePayload::Shape(geometry);
            model.insert_node(record).unwrap();
            let mut stack = ModifierStack::new(sid(n));
            let paint = if n == 16 {
                ModifierPayload::Stroke {
                    color: gray,
                    width: stellar_core_types::value::FiniteF32::ONE,
                }
            } else {
                ModifierPayload::Fill { color: gray }
            };
            for (slot, payload) in [
                ModifierPayload::Transform(Transform2D::new(1., 0., 0., 1., 20., 220.).unwrap()),
                paint,
            ]
            .into_iter()
            .enumerate()
            {
                let order = OrderKey::between(stack.entries().last().map(|e| &e.order), None);
                stack
                    .insert(ModifierEntry {
                        id: ModifierId::new(sid(n * 10 + slot as u64)),
                        order,
                        kind: payload.kind(),
                        enabled: true,
                        payload,
                    })
                    .unwrap();
            }
            model.set_modifier_stack(stack).unwrap();
        }
        model
    }

    fn edit(window: &mut ProjectWindowSession, request: u64, edit: CanvasEdit) -> bool {
        let scene = CanvasCache::default().current(window, None, &mut Default::default()).unwrap();
        apply_edit(window, &scene, RequestId(request), sid(3), edit).is_ok()
    }

    fn resize(node: u64, x: f32, y: f32, w: f32, h: f32) -> CanvasEdit {
        CanvasEdit::Resize {
            node: sid(node),
            rect: Rect::new(Vec2::new(x, y).unwrap(), Size2::new(w, h).unwrap()),
        }
    }

    fn shape(window: &ProjectWindowSession, node: u64) -> ShapeGeometry {
        match window
            .document()
            .unwrap()
            .model()
            .payload(sid(node))
            .unwrap()
        {
            NodePayload::Shape(shape) => *shape,
            _ => panic!("Shape"),
        }
    }

    #[test]
    fn resize_handles_set_fixed_size_and_origin_for_each_node_kind_in_one_undo() {
        let dir = saved_project_with("canvas-resize", &[&all_shapes_document()]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        let select = |session: &mut ProjectWindowSession, n: u64| {
            session
                .select(Selection::new([sid(n)], Some(sid(n))))
                .unwrap();
        };
        // Frame: 왼쪽 위 핸들은 width·height와 원점을 함께 바꾸고 Undo 한 번입니다.
        select(&mut session, 12);
        let undo = session.history().history().len();
        assert!(edit(&mut session, 1, resize(12, -10., -10., 60., 60.)));
        assert_eq!(session.history().history().len(), undo + 1);
        let NodePayload::Frame(frame) = session
            .document()
            .unwrap()
            .model()
            .payload(sid(12))
            .unwrap()
            .clone()
        else {
            panic!("Frame");
        };
        assert_eq!(frame.size, Size2::new(60., 60.).unwrap());
        assert_eq!(frame.transform.components(), (1., 0., 0., 1., 190., 40.));
        assert!(session.undo().unwrap());
        assert_eq!(frame_transform(&session, 12), (1., 0., 0., 1., 200., 50.));
        // Rectangle: 크기와 Transform 원점.
        select(&mut session, 14);
        assert!(edit(&mut session, 2, resize(14, -5., 0., 45., 30.)));
        assert!(
            matches!(shape(&session, 14), ShapeGeometry::Rectangle(r) if r.size == Size2::new(45., 30.).unwrap())
        );
        assert_eq!(shape_translation(&session, sid(14)), (295., 200.));
        // Ellipse·Polygon: local bounds를 그대로 바꾸고 transform은 그대로입니다.
        select(&mut session, 15);
        assert!(edit(&mut session, 3, resize(15, 2., 3., 50., 25.)));
        assert!(matches!(shape(&session, 15), ShapeGeometry::Ellipse(e)
            if e.bounds == Rect::new(Vec2::new(2., 3.).unwrap(), Size2::new(50., 25.).unwrap())));
        assert_eq!(shape_translation(&session, sid(15)), (20., 220.));
        select(&mut session, 17);
        assert!(edit(&mut session, 4, resize(17, 0., 0., 80., 40.)));
        let ShapeGeometry::Polygon(polygon) = shape(&session, 17) else {
            panic!("Polygon");
        };
        assert_eq!(
            (
                polygon.bounds().size(),
                polygon.sides(),
                polygon.corner_radius()
            ),
            (Size2::new(80., 40.).unwrap(), 6, 3.)
        );
        // Line: 상자 비율로 두 끝점을 옮기고, 끝점 핸들은 한 점만 옮깁니다.
        select(&mut session, 16);
        assert!(edit(&mut session, 5, resize(16, 0., 0., 60., 20.)));
        assert!(
            matches!(shape(&session, 16), ShapeGeometry::Line(l) if l.end == Vec2::new(60., 20.).unwrap())
        );
        let end = Vec2::new(-5., 40.).unwrap();
        assert!(edit(
            &mut session,
            6,
            CanvasEdit::LineEnd {
                node: sid(16),
                end: true,
                point: end
            }
        ));
        assert!(
            matches!(shape(&session, 16), ShapeGeometry::Line(l) if l.start == Vec2::ZERO && l.end == end)
        );
        // 선택과 다른 node, 0 크기, Text가 아닌데도 선택이 여럿이면 거부합니다.
        assert!(!edit(&mut session, 7, resize(14, 0., 0., 10., 10.)));
        session
            .select(Selection::new([sid(14), sid(15)], Some(sid(15))))
            .unwrap();
        assert!(!edit(&mut session, 8, resize(15, 0., 0., 10., 10.)));
        select(&mut session, 15);
        assert!(!edit(
            &mut session,
            9,
            CanvasEdit::Resize {
                node: sid(15),
                rect: Rect::new(Vec2::ZERO, Size2::new(0., 10.).unwrap()),
            }
        ));
        session.save_now().unwrap();
        let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        assert!(matches!(shape(&reopened, 16), ShapeGeometry::Line(l) if l.end == end));
        assert_eq!(shape_translation(&reopened, sid(14)), (295., 200.));
    }

    fn radius(node: u64, corner: Option<usize>, value: f32) -> CanvasEdit {
        CanvasEdit::Radius {
            node: sid(node),
            corner,
            radius: stellar_core_types::value::FiniteF32::new(value).unwrap(),
        }
    }

    #[test]
    fn radius_handles_write_border_radius_longhands_and_polygon_radius_in_one_undo() {
        let dir = saved_project_with("canvas-radius", &[&all_shapes_document()]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        let select = |session: &mut ProjectWindowSession, n: u64| {
            session
                .select(Selection::new([sid(n)], Some(sid(n))))
                .unwrap();
        };
        let frame_radii = |session: &ProjectWindowSession| match session
            .document()
            .unwrap()
            .model()
            .payload(sid(12))
        {
            Some(NodePayload::Frame(frame)) => frame.corner_radius.components(),
            _ => panic!("Frame"),
        };
        select(&mut session, 12);
        let undo = session.history().history().len();
        assert!(edit(&mut session, 1, radius(12, None, 8.)));
        assert_eq!(frame_radii(&session), (8., 8., 8., 8.));
        assert_eq!(session.history().history().len(), undo + 1);
        // Option: 그 모서리 longhand 하나만 바꿉니다.
        assert!(edit(&mut session, 2, radius(12, Some(1), 4.)));
        assert_eq!(frame_radii(&session), (8., 4., 8., 8.));
        assert!(session.undo().unwrap());
        assert_eq!(frame_radii(&session), (8., 8., 8., 8.));
        select(&mut session, 14);
        assert!(edit(&mut session, 3, radius(14, Some(0), 6.)));
        assert!(
            matches!(shape(&session, 14), ShapeGeometry::Rectangle(r) if r.corner_radius.components() == (6., 0., 0., 0.))
        );
        select(&mut session, 17);
        assert!(edit(&mut session, 4, radius(17, Some(2), 5.)));
        assert!(
            matches!(shape(&session, 17), ShapeGeometry::Polygon(p) if p.corner_radius() == 5. && p.sides() == 6)
        );
        // 반경이 없는 Ellipse, 선택과 다른 node는 거부합니다.
        select(&mut session, 15);
        assert!(!edit(&mut session, 5, radius(15, None, 3.)));
        assert!(!edit(&mut session, 6, radius(14, None, 3.)));
        session.save_now().unwrap();
        let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        assert!(
            matches!(shape(&reopened, 14), ShapeGeometry::Rectangle(r) if r.corner_radius.components() == (6., 0., 0., 0.))
        );
    }

    #[test]
    fn selection_items_report_boxes_and_handles_from_the_core() {
        let dir = saved_project_with("canvas-selection-items", &[&nested_document()]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session
            .select(Selection::new([sid(13), sid(10)], Some(sid(13))))
            .unwrap();
        let scene = CanvasCache::default().current(&session, None, &mut Default::default());
        let items = selection_items(&session, scene.as_deref());
        assert_eq!(items.len(), 2);
        let child = &items[0];
        assert_eq!(child.id, sid(13));
        assert_eq!(child.kind, CanvasNodeKind::Frame);
        assert_eq!(child.world.components(), (1., 0., 0., 1., 15., 15.));
        assert_eq!(child.rect.size(), Size2::new(20., 20.).unwrap());
        assert!(child.movable && child.resizable);
        assert_eq!(child.radius, Some(RadiusHandles::PerCorner));
        assert_eq!(child.fixed, [true, true]);
        assert_eq!(items[1].id, sid(10));
    }

    #[test]
    fn continuous_zoom_keeps_anchor_and_leaves_document_and_history_alone() {
        let dir = saved_project_with("canvas-zoom-by", &[&sample_document()]);
        let app = AppRuntime::start().unwrap();
        let owner = open_project(&app, dir.path());
        let (session, _, _) = open_window(&app, owner);
        let before = app.state().window(session).unwrap().clone();
        let camera = before.viewport.unwrap();
        app.zoom_viewport_by(session, [40.0, -20.0], 1.25).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        let after = app.state().window(session).unwrap().clone();
        let zoomed = after.viewport.unwrap();
        assert_eq!(zoomed.zoom(), camera.zoom() * 1.25);
        // anchor 화면 점 아래의 문서 점이 그대로입니다.
        let world = |c: CanvasViewport, x: f64| c.center_x() + x / c.zoom();
        assert!((world(camera, 40.0) - world(zoomed, 40.0)).abs() < 1e-9);
        assert_eq!(
            (after.revision, after.undo_depth, after.has_unsaved_changes),
            (
                before.revision,
                before.undo_depth,
                before.has_unsaved_changes
            )
        );
        app.zoom_viewport_by(session, [0.0, 0.0], 1e9).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        assert_eq!(
            app.state()
                .window(session)
                .unwrap()
                .viewport
                .unwrap()
                .zoom(),
            stellar_commands::view_tabs::MAX_ZOOM
        );
        app.zoom_viewport_by(session, [0.0, 0.0], 0.0).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::RequestFailed { .. }
        ));
    }
}

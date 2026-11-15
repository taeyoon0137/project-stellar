//! 캔버스 직접 조작(ADR-0071).
//!
//! 셸은 그린 행·화면 결과(포인터 아래 행, world delta, local 상자)만 보내고, 카메라·선택·편집의 의미와 command
//! 구성은 여기서 정합니다. 카메라 조작은 문서·history를 바꾸지 않습니다. 크기·반경·끝점은 속성 탭(ADR-0070)과
//! 같은 longhand key의 `SetProperty`로 씁니다.

use super::frame_move::editable_ancestry;
use super::*;
use crate::canvas::CanvasScene;
use std::sync::Arc;
use stellar_commands::command::Command;
use stellar_commands::property::{read_property, PropertyValue, SetProperty};
use stellar_commands::selection::Selection;
use stellar_core_types::property::PropertyKey;
use stellar_core_types::value::{FiniteF32, Rect, Transform2D, Vec2};
use stellar_document::frame::CornerRadius;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierId, ModifierPayload};
use stellar_document::records::NodePayload;
use stellar_document::shape::ShapeGeometry;
use stellar_scene::bounds::Aabb;

/// 클릭 선택의 단계(UX 명세 5.3·5.4).
///
/// 이미 고른 node를 누르면 끌어 옮길 수 있도록 선택을 그대로 두고, 움직이지 않고 놓았을 때만 바꿉니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickPhase {
    /// 한 번에 정합니다(빈 곳 클릭 등).
    Click,
    /// 누름. 고를 node가 이미 선택에 있으면 선택을 그대로 두고 바꿀 내용(하나로 줄이기, Shift 빼기)을 미룹니다.
    Press,
    /// 움직이지 않고 놓음. 같은 창의 바로 앞 누름이 미룬 변경을 적용합니다. 미룬 것이 없으면 그대로입니다.
    Release,
}

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
        /// 누름·놓음 단계.
        phase: PickPhase,
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
    /// 이동할 수 있는지. 이동 command와 같은 판정(잠금·숨김·흐름 부모·modifier 구성)입니다.
    pub movable: bool,
    /// 크기 핸들로 `width`·`height`를 바꿀 수 있는지.
    pub resizable: bool,
    /// 반경 핸들이 있는지와 그 방식.
    pub radius: Option<RadiusHandles>,
    /// 가로·세로가 고정 크기인지. 아니면 자동 크기(Text의 `width: max-content`, `height: auto`)입니다.
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
        radius: FiniteF32,
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

/// 누를 때 미룬 선택 변경.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DeferredPick {
    view: StableId,
    target: StableId,
    toggle: bool,
}

/// 창 하나의 캔버스 조작 상태(document thread 소유).
#[derive(Debug, Default)]
pub(crate) struct CanvasGestureState {
    /// 바로 앞 누름이 미룬 선택 변경.
    deferred: Option<DeferredPick>,
    /// 마지막으로 계산한 선택 상자. scene과 선택이 같으면 다시 계산하지 않습니다(팬·줌 게시).
    items: Option<(Arc<CanvasScene>, Selection, Arc<[CanvasSelectionItem]>)>,
}

impl CanvasGestureState {
    /// 표시한 선택의 상자. scene과 선택이 바뀔 때만 다시 계산합니다.
    pub(crate) fn selection_items(
        &mut self,
        session: &ProjectWindowSession,
        scene: Option<&Arc<CanvasScene>>,
    ) -> Arc<[CanvasSelectionItem]> {
        let Some(scene) = scene else {
            self.items = None;
            return Arc::from([]);
        };
        let selection = session.view().selection();
        if let Some((cached, shown, items)) = &self.items {
            if Arc::ptr_eq(cached, scene) && shown == selection {
                return items.clone();
            }
        }
        let items = selection_items(session, Some(scene));
        self.items = Some((scene.clone(), selection.clone(), items.clone()));
        items
    }
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
            let deferred = entry.gesture.deferred.take();
            let (selection, next) =
                resolve_pick(&entry.session, scene.as_deref(), view, pick, deferred)?;
            entry.session.select(selection).map_err(view_failure)?;
            entry.gesture.deferred = next;
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
            entry.gesture.deferred = None;
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
    let commands = match edit {
        CanvasEdit::Move {
            anchor,
            delta,
            parent_space,
            duplicate,
        } => {
            return move_selection(
                window,
                scene,
                request,
                view,
                anchor,
                delta,
                parent_space,
                duplicate,
            )
        }
        CanvasEdit::Resize { node, rect } => {
            let (model, payload) = sole_editable(window, view, node)?;
            let mut ids = super::frame_move::FreshIds::default();
            let commands = resize_commands(model, node, &payload, rect, &mut || ids.next());
            ids.finish()?;
            commands?
        }
        CanvasEdit::LineEnd { node, end, point } => {
            let (model, payload) = sole_editable(window, view, node)?;
            line_end_commands(model, node, &payload, end, point)?
        }
        CanvasEdit::Radius {
            node,
            corner,
            radius,
        } => {
            let (model, payload) = sole_editable(window, view, node)?;
            radius_commands(model, node, &payload, corner, radius)?
        }
    };
    apply_commands(window, request, commands)
}

fn set_property(node: StableId, key: PropertyKey, value: PropertyValue) -> Command {
    Command::SetProperty(SetProperty { node, key, value })
}

fn length(value: f32) -> Result<PropertyValue, Failure> {
    FiniteF32::new(value)
        .ok()
        .filter(|v| v.get() >= 0.0)
        .map(PropertyValue::NonNegative)
        .ok_or_else(stale_view)
}

fn number(value: f32) -> Result<PropertyValue, Failure> {
    FiniteF32::new(value)
        .map(PropertyValue::Number)
        .map_err(|_| stale_view())
}

/// 값이 바뀐 `(key, value)`만 `SetProperty`로 만듭니다(속성 탭과 같은 key).
fn changed_properties(
    model: &DocumentModel,
    node: StableId,
    values: impl IntoIterator<Item = (PropertyKey, PropertyValue)>,
) -> Result<Vec<Command>, Failure> {
    let record = model.record(node).ok_or_else(stale_view)?;
    Ok(values
        .into_iter()
        .filter(|(key, value)| read_property(&record, *key).as_ref() != Some(value))
        .map(|(key, value)| set_property(node, key, value))
        .collect())
}

/// 반경 핸들 한 번. Frame·Rectangle은 바뀐 `border-*-radius` longhand만, Polygon은 반경 key 하나입니다.
fn radius_commands(
    model: &DocumentModel,
    node: StableId,
    payload: &NodePayload,
    corner: Option<usize>,
    radius: FiniteF32,
) -> Result<Vec<Command>, Failure> {
    use stellar_commands::document::frame::keys;
    use stellar_document::payload_metadata::shape_keys;
    if radius.get() < 0.0 || corner.is_some_and(|c| c > 3) {
        return Err(stale_view());
    }
    let value = PropertyValue::NonNegative(radius);
    let longhands = match payload {
        NodePayload::Frame(_) => keys::BORDER_RADIUS_LONGHANDS,
        NodePayload::Shape(ShapeGeometry::Rectangle(_)) => shape_keys::BORDER_RADIUS_LONGHANDS,
        NodePayload::Shape(ShapeGeometry::Polygon(_)) => {
            return changed_properties(model, node, [(shape_keys::POLYGON_CORNER_RADIUS, value)])
        }
        _ => return Err(stale_view()),
    };
    let chosen = longhands
        .into_iter()
        .enumerate()
        .filter(|(index, _)| corner.is_none_or(|c| c == *index))
        .map(|(_, key)| (key, value.clone()));
    changed_properties(model, node, chosen)
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

fn apply_commands(
    window: &mut ProjectWindowSession,
    request: RequestId,
    commands: Vec<Command>,
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

/// 크기 핸들 결과 상자 `rect`(지금 local 좌표)를 쓰는 command.
///
/// 크기는 속성 탭과 같은 `width`·`height`(Line은 `x1`·`y1`·`x2`·`y2`) longhand의 `SetProperty`입니다. 상자 원점이
/// 바뀌면 Frame은 `transform`, Shape·Image는 위치 Transform modifier(없으면 맨 앞에 새로)를 바꿉니다.
fn resize_commands(
    model: &DocumentModel,
    node: StableId,
    payload: &NodePayload,
    rect: Rect,
    new_modifier: &mut dyn FnMut() -> StableId,
) -> Result<Vec<Command>, Failure> {
    use stellar_commands::document::frame::keys;
    use stellar_document::payload_metadata::{image_keys, shape_keys};
    let size = rect.size();
    if size.width() <= 0.0 || size.height() <= 0.0 {
        return Err(stale_view());
    }
    let origin = rect.origin();
    let sized = |width: PropertyKey, height: PropertyKey| -> Result<_, Failure> {
        Ok([
            (width, length(size.width())?),
            (height, length(size.height())?),
        ])
    };
    // (바꿀 longhand, 지금 local 상자의 원점)
    let (values, old_origin) = match payload {
        NodePayload::Frame(frame) => {
            let mut commands = changed_properties(model, node, sized(keys::WIDTH, keys::HEIGHT)?)?;
            if origin != Vec2::ZERO {
                let next = shifted(frame.transform, origin).ok_or_else(stale_view)?;
                commands.push(set_property(
                    node,
                    keys::TRANSFORM,
                    PropertyValue::Transform(next),
                ));
            }
            return Ok(commands);
        }
        NodePayload::Shape(ShapeGeometry::Line(line)) => {
            // 끝점을 이전 상자에서 새 상자로 같은 비율로 옮깁니다(길이 0인 축은 그대로 평행 이동).
            let min_x = line.start.x().min(line.end.x());
            let min_y = line.start.y().min(line.end.y());
            let old_w = (line.end.x() - line.start.x()).abs();
            let old_h = (line.end.y() - line.start.y()).abs();
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
            let x = |v: f32| number(origin.x() + (v - min_x) * sx);
            let y = |v: f32| number(origin.y() + (v - min_y) * sy);
            let values = [
                (shape_keys::X1, x(line.start.x())?),
                (shape_keys::Y1, y(line.start.y())?),
                (shape_keys::X2, x(line.end.x())?),
                (shape_keys::Y2, y(line.end.y())?),
            ];
            return changed_properties(model, node, values);
        }
        NodePayload::Shape(ShapeGeometry::Rectangle(_)) => {
            (sized(shape_keys::WIDTH, shape_keys::HEIGHT)?, Vec2::ZERO)
        }
        NodePayload::Shape(ShapeGeometry::Ellipse(ellipse)) => (
            sized(shape_keys::WIDTH, shape_keys::HEIGHT)?,
            ellipse.bounds.origin(),
        ),
        NodePayload::Shape(ShapeGeometry::Polygon(polygon)) => (
            sized(shape_keys::WIDTH, shape_keys::HEIGHT)?,
            polygon.bounds().origin(),
        ),
        NodePayload::Image(_) => (sized(image_keys::WIDTH, image_keys::HEIGHT)?, Vec2::ZERO),
        _ => return Err(stale_view()),
    };
    let mut commands = changed_properties(model, node, values)?;
    let shift = Vec2::new(origin.x() - old_origin.x(), origin.y() - old_origin.y())
        .map_err(|_| stale_view())?;
    if shift != Vec2::ZERO {
        let (modifier, transform) = resize_transform(model, node, false).ok_or_else(stale_view)?;
        let next = shifted(transform, shift).ok_or_else(stale_view)?;
        commands.push(transform_command(model, node, modifier, next, new_modifier));
    }
    Ok(commands)
}

/// 위치 Transform modifier를 `next`로 바꾸거나(없으면 맨 앞에 새로) 넣는 command.
///
/// `new_modifier`는 새 modifier를 넣을 때만 부릅니다. 제품 경로는 UUIDv7(`FreshIds`)을 넘깁니다.
fn transform_command(
    model: &DocumentModel,
    node: StableId,
    modifier: Option<ModifierId>,
    next: Transform2D,
    new_modifier: &mut dyn FnMut() -> StableId,
) -> Command {
    use stellar_commands::modifier_ops::{InsertModifier, SetModifierProperty};
    use stellar_core_types::order_key::OrderKey;
    use stellar_document::modifier::{ModifierEntry, ModifierKind};
    use stellar_document::modifier_metadata::keys::TRANSFORM_MATRIX;
    match modifier {
        Some(modifier) => Command::SetModifierProperty(SetModifierProperty {
            node,
            modifier,
            key: TRANSFORM_MATRIX,
            value: PropertyValue::Transform(next),
        }),
        None => {
            let order = OrderKey::between(
                None,
                model
                    .modifier_stack(node)
                    .and_then(|s| s.entries().first())
                    .map(|e| &e.order),
            );
            Command::InsertModifier(InsertModifier {
                node,
                entry: ModifierEntry {
                    id: ModifierId::new(new_modifier()),
                    order,
                    kind: ModifierKind::Transform,
                    enabled: true,
                    payload: ModifierPayload::Transform(next),
                },
            })
        }
    }
}

/// Line 끝점 핸들 한 번. 바뀐 `x1`·`y1`(시작) 또는 `x2`·`y2`(끝)만 씁니다.
fn line_end_commands(
    model: &DocumentModel,
    node: StableId,
    payload: &NodePayload,
    end: bool,
    point: Vec2,
) -> Result<Vec<Command>, Failure> {
    use stellar_document::payload_metadata::shape_keys;
    if !matches!(payload, NodePayload::Shape(ShapeGeometry::Line(_))) {
        return Err(stale_view());
    }
    let (x, y) = if end {
        (shape_keys::X2, shape_keys::Y2)
    } else {
        (shape_keys::X1, shape_keys::Y1)
    };
    changed_properties(
        model,
        node,
        [(x, number(point.x())?), (y, number(point.y())?)],
    )
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

/// `node`의 위치 Transform modifier와 그 값(속성 탭 `left`·`top`과 같은 규칙, ADR-0070).
fn position_transform(model: &DocumentModel, node: StableId) -> Option<(ModifierId, Transform2D)> {
    let entry = stellar_commands::inspector::position_modifier(model, node)?;
    match entry.payload {
        ModifierPayload::Transform(transform) => Some((entry.id, transform)),
        _ => None,
    }
}

/// `source`를 `delta`(부모 공간)만큼 옮기는 command를 `target`(복제본이면 remap 적용)에 대해 만듭니다.
///
/// Frame은 `transform`, 배치한 인스턴스는 배치가 기록한 Transform 하나, 그 밖의 node는 속성 탭 `left`·`top`과 같은
/// 위치 Transform modifier(없으면 맨 앞에 새로)를 바꿉니다. 옮길 수 없는 node는 `Err`입니다.
fn move_command(
    model: &DocumentModel,
    source: StableId,
    target: StableId,
    delta: Vec2,
    remap: Option<&std::collections::BTreeMap<StableId, StableId>>,
    new_modifier: &mut dyn FnMut() -> StableId,
) -> Result<Option<Command>, Failure> {
    use stellar_commands::document::frame::keys;
    let modifier_of = |id: ModifierId| -> Result<ModifierId, Failure> {
        match remap {
            None => Ok(id),
            Some(map) => map
                .get(&id.stable_id())
                .map(|&copy| ModifierId::new(copy))
                .ok_or_else(stale_view),
        }
    };
    let position = match model.payload(source).ok_or_else(stale_view)? {
        NodePayload::Frame(frame) => {
            let next = translated(frame.transform, delta).ok_or_else(stale_view)?;
            if next == frame.transform {
                return Ok(None);
            }
            return Ok(Some(set_property(
                target,
                keys::TRANSFORM,
                PropertyValue::Transform(next),
            )));
        }
        NodePayload::ComponentInstance(instance) => {
            // 배치가 기록한 이동 하나만 바꿉니다. 별도 변환·override에는 손대지 않습니다.
            let stack = model.modifier_stack(source).ok_or_else(stale_view)?;
            let [entry] = stack.entries() else {
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
            position_transform(model, source)
        }
        _ => return Err(stale_view()),
    };
    let (modifier, next) = match position {
        Some((modifier, current)) => {
            let next = translated(current, delta).ok_or_else(stale_view)?;
            if next == current {
                return Ok(None);
            }
            (Some(modifier_of(modifier)?), next)
        }
        None => (
            None,
            translated(Transform2D::identity(), delta).ok_or_else(stale_view)?,
        ),
    };
    // 복제본의 새 modifier 자리는 원본 stack과 같습니다(DuplicateSubtree가 순서를 그대로 복사).
    let command = transform_command(model, source, modifier, next, new_modifier);
    Ok(Some(retarget(command, target)))
}

/// 원본 기준으로 만든 command의 대상 node를 `target`으로 바꿉니다.
fn retarget(command: Command, target: StableId) -> Command {
    match command {
        Command::SetModifierProperty(mut set) => {
            set.node = target;
            Command::SetModifierProperty(set)
        }
        Command::InsertModifier(mut insert) => {
            insert.node = target;
            Command::InsertModifier(insert)
        }
        other => other,
    }
}

/// 이동 command와 같은 판정으로 `node`를 옮길 수 있는지.
fn can_move(model: &DocumentModel, view: StableId, node: StableId) -> bool {
    let frame = matches!(model.payload(node), Some(NodePayload::Frame(_)));
    editable_ancestry(model, view, node, !frame).is_some()
        && Vec2::new(1.0, 0.0)
            .ok()
            .is_some_and(|probe| move_command(model, node, node, probe, None, &mut || node).is_ok())
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
    use stellar_commands::duplicate::DuplicateSubtree;
    use stellar_commands::id::CommandOrigin;
    use stellar_commands::transaction::CommandTransaction;
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
    // 복제본과 새 위치 Transform modifier의 ID는 UUIDv7(OS 난수)입니다. command ID에서 파생하지 않습니다.
    let mut ids = super::frame_move::FreshIds::default();
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
            let plan = DuplicateSubtree::plan_with(model, node, parent, order, &mut || ids.next())
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
        if let Some(command) =
            move_command(model, node, target, local, remap.as_ref(), &mut || {
                ids.next()
            })?
        {
            transaction = transaction.with_command(revision, command);
            commands += 1;
        }
    }
    // 새 ID를 만들지 못했으면(NIL) 적용하지 않습니다.
    ids.finish()?;
    if commands == 0 {
        return Ok(());
    }
    // 화살표 이동(부모 공간, 누른 행 없음)은 속성 탭 `left`·`top` 반복 입력과 같은 grouping으로 기록합니다. 0.5초 안에
    // 이어 누르면 Undo 한 번이고, 선택·다른 편집이 끼면 새 항목입니다.
    let repeat = (parent_space && anchor.is_none() && !duplicate).then(|| {
        super::inspector::repeat_key(
            model,
            &selected,
            stellar_commands::inspector::InspectorProperty::Left,
        )
    });
    if duplicate {
        let primary = copies.last().copied();
        window.apply_and_select(&transaction, Selection::new(copies, primary))
    } else if let Some(key) = repeat {
        window.apply_grouped(&transaction, key, &super::inspector::InstantClock)
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

/// `target`을 뺀 선택. 주 선택이 빠지면 남은 마지막 node가 주 선택입니다.
fn without(current: &Selection, target: StableId) -> Selection {
    let rest: Vec<StableId> = current
        .ids()
        .iter()
        .copied()
        .filter(|&id| id != target)
        .collect();
    let primary = current
        .primary()
        .filter(|&p| p != target)
        .or(rest.last().copied());
    Selection::new(rest, primary)
}

/// 고른 선택과 다음 누름·놓음까지 미룰 변경.
fn resolve_pick(
    window: &ProjectWindowSession,
    scene: Option<&CanvasScene>,
    view: StableId,
    pick: CanvasPick,
    deferred: Option<DeferredPick>,
) -> Result<(Selection, Option<DeferredPick>), Failure> {
    if window.tabs().active_view() != Some(view) {
        return Err(stale_view());
    }
    let model = window.document().ok_or_else(stale_view)?.model();
    let root = view_root(model, view).ok_or_else(stale_view)?;
    let current = window.view().selection().clone();
    let ids = current.ids();
    match pick {
        CanvasPick::Point {
            phase: PickPhase::Release,
            ..
        } => {
            // 누를 때 이미 선택에 있던 node를 끌지 않고 놓았습니다. 미룬 변경을 적용합니다.
            let selection = match deferred.filter(|d| d.view == view) {
                Some(d) if d.toggle => without(&current, d.target),
                Some(d) if current.contains(d.target) => Selection::new([d.target], Some(d.target)),
                _ => current,
            };
            Ok((selection, None))
        }
        CanvasPick::Point {
            leaf,
            toggle,
            deep,
            enter,
            phase,
        } => {
            let target = leaf.and_then(|leaf| point_target(model, root, ids, leaf, deep, enter));
            if let Some(target) =
                target.filter(|&t| phase == PickPhase::Press && current.contains(t))
            {
                // 이미 고른 node를 누르면 여러 개를 함께 끌 수 있도록 선택을 그대로 둡니다.
                let deferred = DeferredPick {
                    view,
                    target,
                    toggle,
                };
                return Ok((current, Some(deferred)));
            }
            Ok((
                match (target, toggle) {
                    (None, false) => Selection::empty(),
                    (None, true) => current,
                    (Some(target), false) => Selection::new([target], Some(target)),
                    (Some(target), true) if current.contains(target) => without(&current, target),
                    (Some(target), true) => {
                        Selection::new(ids.iter().copied().chain([target]), Some(target))
                    }
                },
                None,
            ))
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
            let selection = if add {
                let primary = hits.last().copied().or(current.primary());
                Selection::new(ids.iter().copied().chain(hits), primary)
            } else {
                let primary = hits.last().copied();
                Selection::new(hits, primary)
            };
            Ok((selection, None))
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
) -> Arc<[CanvasSelectionItem]> {
    let (Some(scene), Some(document), Some(view)) =
        (scene, session.document(), session.tabs().active_view())
    else {
        return Arc::from([]);
    };
    let model = document.model();
    let ids = session.view().selection().ids();
    if ids.is_empty() {
        return Arc::from([]);
    }
    let rows = row_index(scene);
    let mut items = Vec::with_capacity(ids.len());
    for &id in ids {
        let (Some(row), Some(payload)) = (
            rows.get(&id).map(|&index| &scene.frames[index]),
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
        let movable = can_move(model, view, id);
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

/// 크기를 바꿀 때 원점 이동을 기록할 변환. Frame은 자기 transform, 그 밖은 위치 Transform modifier가 하나뿐인
/// Transform일 때 그 값(없으면 항등)입니다. Transform이 여럿이면 어느 것을 바꿀지 모호해 크기 핸들이 없습니다.
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
    crate::canvas::shape::shape_style(stack)?;
    let transforms = stack.map_or(0, |stack| {
        stack
            .entries()
            .iter()
            .filter(|e| e.enabled && matches!(e.payload, ModifierPayload::Transform(_)))
            .count()
    });
    match (transforms, position_transform(model, node)) {
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

    /// runtime `canvas_pick`처럼 미룬 변경을 다음 요청에 넘기며 고릅니다.
    fn pick_with(
        window: &mut ProjectWindowSession,
        view: u64,
        pick: CanvasPick,
        deferred: &mut Option<DeferredPick>,
    ) -> Option<Vec<StableId>> {
        let scene = CanvasCache::default().current(window, None, &mut Default::default());
        let (selection, next) =
            resolve_pick(window, scene.as_deref(), sid(view), pick, deferred.take()).ok()?;
        window.select(selection).unwrap();
        *deferred = next;
        Some(window.view().selection().ids().to_vec())
    }

    fn pick(
        window: &mut ProjectWindowSession,
        view: u64,
        pick: CanvasPick,
    ) -> Option<Vec<StableId>> {
        pick_with(window, view, pick, &mut None)
    }

    fn point(leaf: Option<u64>, toggle: bool, deep: bool, enter: bool) -> CanvasPick {
        CanvasPick::Point {
            leaf: leaf.map(sid),
            toggle,
            deep,
            enter,
            phase: PickPhase::Click,
        }
    }

    fn press(leaf: u64, toggle: bool) -> CanvasPick {
        CanvasPick::Point {
            leaf: Some(sid(leaf)),
            toggle,
            deep: false,
            enter: false,
            phase: PickPhase::Press,
        }
    }

    fn release(leaf: u64, toggle: bool) -> CanvasPick {
        CanvasPick::Point {
            leaf: Some(sid(leaf)),
            toggle,
            deep: false,
            enter: false,
            phase: PickPhase::Release,
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
        let scene = CanvasCache::default()
            .current(window, None, &mut Default::default())
            .unwrap();
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

    /// 화살표 이동을 이어 누르면 속성 탭 반복 입력처럼 Undo 한 번으로 묶이고, 끄는 이동은 묶지 않습니다.
    #[test]
    fn repeated_arrow_moves_group_into_one_undo_entry() {
        let dir = saved_project_with("canvas-arrow-group", &[&shape_document()]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session
            .select(Selection::new([sid(14)], Some(sid(14))))
            .unwrap();
        let undo = session.history().history().len();
        for request in 1..=3 {
            assert!(drag(&mut session, request, None, (1., 0.), true, false));
        }
        assert_eq!(shape_translation(&session, sid(14)), (303., 200.));
        assert_eq!(session.history().history().len(), undo + 1);
        // 끄는 이동은 새 항목입니다.
        assert!(drag(&mut session, 4, Some(14), (2., 0.), false, false));
        assert_eq!(session.history().history().len(), undo + 2);
        assert!(session.undo().unwrap());
        assert!(session.undo().unwrap());
        assert_eq!(shape_translation(&session, sid(14)), (300., 200.));
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
        let scene = CanvasCache::default()
            .current(window, None, &mut Default::default())
            .unwrap();
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
        // Ellipse·Polygon: `width`·`height`만 바꾸고 원점 이동은 위치 Transform으로 옮깁니다(속성 탭과 같은 key).
        select(&mut session, 15);
        assert!(edit(&mut session, 3, resize(15, 2., 3., 50., 25.)));
        assert!(matches!(shape(&session, 15), ShapeGeometry::Ellipse(e)
            if e.bounds == Rect::new(Vec2::ZERO, Size2::new(50., 25.).unwrap())));
        assert_eq!(shape_translation(&session, sid(15)), (22., 223.));
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

    #[test]
    fn pressing_a_selected_node_keeps_area_and_shift_selections_for_drag_and_option_drag() {
        let dir = saved_project_with("canvas-press-drag", &[&shape_document()]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        let ids = |v: &[u64]| v.iter().copied().map(sid).collect::<Vec<_>>();
        let mut deferred = None;
        // 영역 선택 다음 그 안의 행(11의 자식 13)을 눌러도 선택이 그대로이고, 끌면 둘 다 함께 움직입니다.
        assert_eq!(
            pick_with(
                &mut session,
                3,
                area((0., 0.), (260., 120.), false, false),
                &mut deferred
            )
            .unwrap(),
            ids(&[11, 12])
        );
        assert_eq!(
            pick_with(&mut session, 3, press(13, false), &mut deferred).unwrap(),
            ids(&[11, 12])
        );
        let undo = session.history().history().len();
        assert!(drag(&mut session, 1, Some(13), (10., 5.), false, false));
        assert_eq!(frame_transform(&session, 11), (2., 0., 0., 2., 20., 15.));
        assert_eq!(frame_transform(&session, 12), (1., 0., 0., 1., 210., 55.));
        assert_eq!(session.history().history().len(), undo + 1);
        assert!(session.undo().unwrap());
        assert_eq!(frame_transform(&session, 12), (1., 0., 0., 1., 200., 50.));
        // 움직이지 않고 놓으면 그때 하나로 줄입니다.
        assert_eq!(
            pick_with(&mut session, 3, press(12, false), &mut deferred).unwrap(),
            ids(&[11, 12])
        );
        assert_eq!(
            pick_with(&mut session, 3, release(12, false), &mut deferred).unwrap(),
            ids(&[12])
        );
        // Shift로 더한 node는 누를 때 바로 들어가고, 놓아도 다시 빠지지 않습니다.
        assert_eq!(
            pick_with(&mut session, 3, press(14, true), &mut deferred).unwrap(),
            ids(&[12, 14])
        );
        assert_eq!(
            pick_with(&mut session, 3, release(14, true), &mut deferred).unwrap(),
            ids(&[12, 14])
        );
        // Shift 선택 다음 Option 끌기는 고른 node 전부를 복제합니다.
        assert_eq!(
            pick_with(&mut session, 3, press(14, false), &mut deferred).unwrap(),
            ids(&[12, 14])
        );
        let before = session
            .document()
            .unwrap()
            .model()
            .children_in_order(sid(10))
            .len();
        assert!(drag(&mut session, 2, Some(14), (0., 30.), false, true));
        let copies = session.view().selection().ids().to_vec();
        assert_eq!(copies.len(), 2);
        assert!(!copies.contains(&sid(12)) && !copies.contains(&sid(14)));
        assert_eq!(
            session
                .document()
                .unwrap()
                .model()
                .children_in_order(sid(10))
                .len(),
            before + 2
        );
        assert!(session.undo().unwrap());
        assert_eq!(session.view().selection().ids(), ids(&[12, 14]));
        // Shift로 이미 고른 node를 누르면 끌 수 있게 그대로 두고, 끌지 않고 놓을 때 뺍니다.
        assert_eq!(
            pick_with(&mut session, 3, press(12, true), &mut deferred).unwrap(),
            ids(&[12, 14])
        );
        assert_eq!(
            pick_with(&mut session, 3, release(12, true), &mut deferred).unwrap(),
            ids(&[14])
        );
        // 미룬 변경이 없으면 놓음은 선택을 바꾸지 않습니다.
        assert_eq!(
            pick_with(&mut session, 3, release(11, false), &mut deferred).unwrap(),
            ids(&[14])
        );
    }

    /// all_shapes_document에 root 아래 Text 18과 Image 19(각 Transform 하나)를 더합니다.
    /// 새 복제본·위치 Transform modifier ID는 UUIDv7(OS 난수)이고, 창 ID·요청 순번으로 정해지지 않습니다(B4).
    #[test]
    fn copies_and_new_position_modifiers_get_fresh_uuidv7_ids() {
        use stellar_document::modifier::{ModifierKind, ModifierStack};
        // Ellipse 15는 Fill만 둡니다. 이동·원점이 바뀌는 크기 조절은 위치 Transform을 새로 넣습니다.
        let mut model = all_shapes_document();
        let fill = model.modifier_stack(sid(15)).unwrap().entries()[1].clone();
        let mut stack = ModifierStack::new(sid(15));
        stack.insert(fill).unwrap();
        model.set_modifier_stack(stack).unwrap();
        let dir = saved_project_with("canvas-fresh-ids", &[&model]);
        let inserted_transform = |session: &ProjectWindowSession| {
            let model = session.document().unwrap().model();
            let entries = model.modifier_stack(sid(15)).unwrap().entries().to_vec();
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].kind, ModifierKind::Transform);
            entries[0].id.stable_id()
        };
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session
            .select(Selection::new([sid(15)], Some(sid(15))))
            .unwrap();
        assert!(drag(&mut session, 1, None, (5., 0.), true, false));
        let moved = inserted_transform(&session);
        assert!(session.undo().unwrap());
        assert!(edit(&mut session, 2, resize(15, 3., 0., 40., 20.)));
        let resized = inserted_transform(&session);
        assert_eq!((moved.version(), resized.version()), (7, 7));
        assert_ne!(moved, resized);
        // 같은 창 ID·같은 요청 순번으로 두 번 복제해도 새 ID의 난수 부분이 다릅니다.
        let duplicate = || {
            let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
            session
                .select(Selection::new([sid(14)], Some(sid(14))))
                .unwrap();
            assert!(drag(&mut session, 7, Some(14), (20., 0.), false, true));
            let copy = session.view().selection().ids()[0];
            let model = session.document().unwrap().model();
            let mut ids = vec![copy];
            ids.extend(
                model
                    .modifier_stack(copy)
                    .unwrap()
                    .entries()
                    .iter()
                    .map(|e| e.id.stable_id()),
            );
            ids
        };
        let (first, second) = (duplicate(), duplicate());
        assert_eq!(first.len(), 3);
        for (a, b) in first.iter().zip(&second) {
            assert_eq!((a.version(), b.version()), (7, 7));
            assert_ne!(a.to_bytes()[8..], b.to_bytes()[8..]);
        }
    }

    fn every_kind_document() -> DocumentModel {
        use stellar_core_types::order_key::OrderKey;
        use stellar_core_types::reference::AssetRef;
        use stellar_document::image::ImagePayload;
        use stellar_document::modifier::{ModifierEntry, ModifierStack};
        use stellar_document::text::TextPayload;
        let mut model = all_shapes_document();
        for (n, payload) in [
            (18, NodePayload::Text(TextPayload::empty(0))),
            (
                19,
                NodePayload::Image(ImagePayload::new(
                    AssetRef::new(sid(90)),
                    Size2::new(30., 20.).unwrap(),
                )),
            ),
        ] {
            let mut record = model.record(sid(14)).unwrap();
            record.id = sid(n);
            record.order_key = model.next_child_order_key(sid(10));
            record.display_name = format!("n{n}");
            record.payload = payload;
            model.insert_node(record).unwrap();
            let mut stack = ModifierStack::new(sid(n));
            let order = OrderKey::between(None, None);
            stack
                .insert(ModifierEntry {
                    id: ModifierId::new(sid(n * 10)),
                    order,
                    kind: stellar_document::modifier::ModifierKind::Transform,
                    enabled: true,
                    payload: ModifierPayload::Transform(
                        Transform2D::new(1., 0., 0., 1., 50., 60.).unwrap(),
                    ),
                })
                .unwrap();
            model.set_modifier_stack(stack).unwrap();
        }
        model
    }

    /// 이동·크기 핸들이 처음 넣는 위치 Transform modifier의 ID는 서로 독립인 UUIDv7이고, 같은 창·요청 순번으로
    /// 다시 해도 같은 ID가 나오지 않습니다(command ID에서 파생하지 않음, ADR-0028 "새 영속 ID").
    #[test]
    fn new_position_modifiers_from_move_and_resize_get_independent_uuid_v7_ids() {
        use stellar_document::modifier::ModifierKind;
        let mut model = all_shapes_document();
        // Ellipse(15)와 Polygon(17)은 위치 Transform 없이 채우기만 둡니다.
        for n in [15, 17] {
            let mut stack = model.modifier_stack(sid(n)).unwrap().clone();
            stack.remove(ModifierId::new(sid(n * 10))).unwrap();
            model.set_modifier_stack(stack).unwrap();
        }
        let dir = saved_project_with("canvas-new-ids", &[&model]);
        let transform_id = |session: &ProjectWindowSession, n: u64| {
            let model = session.document().unwrap().model();
            let entries = model.modifier_stack(sid(n)).unwrap().entries().to_vec();
            let found: Vec<StableId> = entries
                .iter()
                .filter(|e| e.kind == ModifierKind::Transform)
                .map(|e| e.id.stable_id())
                .collect();
            assert_eq!(found.len(), 1, "n{n}");
            found[0]
        };
        let run = || {
            let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
            session
                .select(Selection::new([sid(15), sid(17)], Some(sid(17))))
                .unwrap();
            assert!(drag(&mut session, 1, None, (5., 5.), true, false));
            let moved = [transform_id(&session, 15), transform_id(&session, 17)];
            let mut resized = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
            resized
                .select(Selection::new([sid(15)], Some(sid(15))))
                .unwrap();
            assert!(edit(&mut resized, 1, resize(15, 4., 0., 36., 20.)));
            let ids = [moved[0], moved[1], transform_id(&resized, 15)];
            assert!(ids.iter().all(|id| (id.version(), id.variant()) == (7, 2)));
            assert!(ids[0] != ids[1] && ids[0] != ids[2] && ids[1] != ids[2]);
            ids
        };
        let first = run();
        let second = run();
        assert!(first.iter().all(|id| !second.contains(id)));
    }

    #[test]
    fn every_node_kind_moves_with_its_position_transform_in_one_undo() {
        let dir = saved_project_with("canvas-move-kinds", &[&every_kind_document()]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        let kinds = [14, 15, 16, 17, 18, 19];
        let before: Vec<_> = kinds
            .iter()
            .map(|&n| shape_translation(&session, sid(n)))
            .collect();
        session
            .select(Selection::new(kinds.map(sid), Some(sid(19))))
            .unwrap();
        let undo = session.history().history().len();
        // 화살표 이동(부모 공간 px): Rectangle·Ellipse·Line·Polygon·Text·Image가 모두 한 번에 움직입니다.
        assert!(drag(&mut session, 1, None, (-10., 4.), true, false));
        for (&n, &(x, y)) in kinds.iter().zip(&before) {
            assert_eq!(
                shape_translation(&session, sid(n)),
                (x - 10., y + 4.),
                "n{n}"
            );
        }
        assert_eq!(session.history().history().len(), undo + 1);
        assert!(session.undo().unwrap());
        for (&n, &xy) in kinds.iter().zip(&before) {
            assert_eq!(shape_translation(&session, sid(n)), xy, "n{n}");
        }
        assert!(session.redo().unwrap());
        session.save_now().unwrap();
        let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        for (&n, &(x, y)) in kinds.iter().zip(&before) {
            assert_eq!(
                shape_translation(&reopened, sid(n)),
                (x - 10., y + 4.),
                "n{n}"
            );
        }
    }

    #[test]
    fn image_resize_writes_size_longhands_and_origin_in_one_undo_and_persists() {
        use stellar_document::payload_metadata::image_keys;
        let dir = saved_project_with("canvas-resize-image", &[&every_kind_document()]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session
            .select(Selection::new([sid(19)], Some(sid(19))))
            .unwrap();
        let undo = session.history().history().len();
        assert!(edit(&mut session, 1, resize(19, -4., 0., 34., 25.)));
        let image = |session: &ProjectWindowSession| match session
            .document()
            .unwrap()
            .model()
            .payload(sid(19))
        {
            Some(NodePayload::Image(image)) => image.size,
            _ => panic!("Image"),
        };
        assert_eq!(image(&session), Size2::new(34., 25.).unwrap());
        assert_eq!(shape_translation(&session, sid(19)), (46., 60.));
        assert_eq!(session.history().history().len(), undo + 1);
        assert!(session.undo().unwrap());
        assert_eq!(image(&session), Size2::new(30., 20.).unwrap());
        assert_eq!(shape_translation(&session, sid(19)), (50., 60.));
        assert!(session.redo().unwrap());
        session.save_now().unwrap();
        let reopened = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        assert_eq!(image(&reopened), Size2::new(34., 25.).unwrap());
        let model = reopened.document().unwrap().model();
        let record = model.record(sid(19)).unwrap();
        assert_eq!(
            read_property(&record, image_keys::WIDTH),
            Some(PropertyValue::NonNegative(FiniteF32::new(34.).unwrap()))
        );
    }

    #[test]
    fn canvas_handles_and_the_property_tab_build_the_same_commands() {
        use stellar_commands::css_value::CssValue;
        use stellar_commands::inspector::{edit_commands, InspectorInput, InspectorProperty};
        let model = every_kind_document();
        let tab = |node: u64, property: InspectorProperty, value: f32| {
            edit_commands(
                &model,
                &[sid(node)],
                property,
                &InspectorInput::Set(CssValue::Length(value)),
                &mut || sid(98),
            )
            .unwrap()
        };
        let payload = |node: u64| model.payload(sid(node)).unwrap().clone();
        let box_of = |w: f32, h: f32| Rect::new(Vec2::ZERO, Size2::new(w, h).unwrap());
        // Rectangle 오른쪽 핸들 = `width`, Image 아래 핸들 = `height`.
        assert_eq!(
            resize_commands(
                &model,
                sid(14),
                &payload(14),
                box_of(45., 30.),
                &mut || sid(99)
            )
            .ok()
            .unwrap(),
            tab(14, InspectorProperty::Width, 45.)
        );
        assert_eq!(
            resize_commands(
                &model,
                sid(19),
                &payload(19),
                box_of(30., 44.),
                &mut || sid(99)
            )
            .ok()
            .unwrap(),
            tab(19, InspectorProperty::Height, 44.)
        );
        // Rectangle Option 반경 = 그 모서리 longhand, Polygon 반경 = 반경 key.
        assert_eq!(
            radius_commands(
                &model,
                sid(14),
                &payload(14),
                Some(2),
                FiniteF32::new(6.).unwrap()
            )
            .ok()
            .unwrap(),
            tab(14, InspectorProperty::Corner(2), 6.)
        );
        assert_eq!(
            radius_commands(
                &model,
                sid(17),
                &payload(17),
                None,
                FiniteF32::new(9.).unwrap()
            )
            .ok()
            .unwrap(),
            tab(17, InspectorProperty::PolygonCornerRadius, 9.)
        );
        // Line 끝점 핸들 = `x2`(`y2`는 그대로라 빠짐).
        assert_eq!(
            line_end_commands(
                &model,
                sid(16),
                &payload(16),
                true,
                Vec2::new(44., 10.).unwrap()
            )
            .ok()
            .unwrap(),
            tab(16, InspectorProperty::X2, 44.)
        );
    }

    #[test]
    fn placed_instance_moves_only_its_single_active_translation() {
        use stellar_core_types::reference::ComponentRef;
        use stellar_document::component::instance::ComponentInstancePayload;
        use stellar_document::modifier::{ModifierKind, ModifierStack};
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
            // 선택 상자의 이동 가능 표시는 실제 이동 판정과 같습니다.
            let model = session.document().unwrap().model();
            assert_eq!(can_move(model, sid(3), node), case == "placed", "{case}");
            let moved = drag(&mut session, 1, None, (7., -4.), true, false);
            if case != "placed" {
                assert!(!moved, "{case}");
                assert_eq!(session.revision(), revision);
                assert_eq!(
                    session.document().unwrap().model().to_records().unwrap(),
                    scene.to_records().unwrap()
                );
                continue;
            }
            assert!(moved);
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
    fn rectangle_move_edits_the_position_transform_and_keeps_other_modifiers() {
        use stellar_document::modifier::{ModifierKind, ModifierStack};
        use stellar_document::shape::RectangleParams;
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
            assert!(drag(&mut session, 11, None, (7., -3.), true, false));
            let changed = session
                .document()
                .unwrap()
                .model()
                .modifier_stack(id)
                .unwrap()
                .clone();
            let mut expected = stack.clone();
            if has_transform {
                // 앞부분 Transform 가운데 마지막 것(속성 탭 `left`·`top`과 같은 modifier)만 바뀝니다.
                expected
                    .set_payload(
                        last,
                        ModifierPayload::Transform(translated(before, delta).unwrap()),
                    )
                    .unwrap();
            } else {
                let inserted = &changed.entries()[0];
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
            assert!(!drag(&mut session, 12, None, (7., -3.), true, false));
            assert!(!session.has_unsaved_changes());
        }
    }

    #[test]
    fn selection_items_are_reused_until_the_scene_or_selection_changes() {
        let dir = saved_project_with("canvas-selection-cache", &[&nested_document()]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session
            .select(Selection::new([sid(11)], Some(sid(11))))
            .unwrap();
        let mut cache = CanvasCache::default();
        let mut shared = Default::default();
        let mut state = CanvasGestureState::default();
        let scene = cache.current(&session, None, &mut shared);
        let first = state.selection_items(&session, scene.as_ref());
        let again = state.selection_items(
            &session,
            cache.current(&session, None, &mut shared).as_ref(),
        );
        assert!(Arc::ptr_eq(&first, &again));
        session
            .select(Selection::new([sid(12)], Some(sid(12))))
            .unwrap();
        let changed = state.selection_items(
            &session,
            cache.current(&session, None, &mut shared).as_ref(),
        );
        assert_eq!(changed[0].id, sid(12));
        assert!(state.selection_items(&session, None).is_empty());
    }
}

//! Frame·Rectangle·Ellipse·Line·Polygon 생성 도구 (XII.10.26, ADR-0117 결정 6).
//!
//! - [`CreateGesture`]: 생성 드래그의 순수 계산. 좌표는 대상 부모 local CSS px입니다. Shift는 상자 정비례·Line 45도,
//!   Option은 시작점이 중심, 드래그 중 Space를 누르는 동안은 크기를 지킨 채 시작점을 옮깁니다. 끌기 시작 거리(3pt)
//!   미만은 클릭이고 기본 크기([`DEFAULT_SIZE_PX`])로 만듭니다.
//! - [`creation_parent`]: pointer 아래 가장 깊은 잠기지 않은 Frame(인스턴스 투영 행 제외), 없으면 View root.
//! - [`commit`]: `InsertNode`와 새 선택을 한 history 항목으로 기록하고 도구 기계를 Move로 돌립니다. 흐름 컨테이너면
//!   pointer 자리의 흐름 순서(OrderKey)에 넣고 위치를 쓰지 않습니다.
//!
//! 칠은 셸이 넘기는 앱 설정 값([`ShapePaint`])이고 코어는 기본 색을 두지 않습니다. 앞선 셸 경로(`app::frame_insert`,
//! ADR-0071)는 새 UI가 이 module에 연결될 때 지웁니다.

use stellar_commands::command::Command;
use stellar_commands::lock::{edit_permission, EditIntent};
use stellar_commands::node_ops::InsertNode;
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::value::{Angle, Rect, Size2, Transform2D, Vec2};
use stellar_document::frame::{CornerRadius, FramePayload};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierPayload, ModifierStack};
use stellar_document::records::{NodePayload, NodeRecord};
use stellar_document::shape::{
    EllipseParams, LineCap, LineParams, PolygonParams, RectangleParams, ShapeGeometry,
};

pub use crate::app::ShapePaint;
use crate::canvas::flow_drag::{flow_insert_key, is_flow_container};
use crate::canvas::tool::{CanvasTool, CanvasToolState, ToolInput};
use crate::canvas::tool_units::DRAG_THRESHOLD_PT;
use crate::canvas::transform::{node_geometry, transaction, FreshIds, TransformError};
use crate::canvas::CanvasScene;
use crate::window_session::{history_label, ProjectWindowSession};

/// 클릭으로 만드는 기본 크기(px). Line은 이 길이의 가로선입니다.
pub const DEFAULT_SIZE_PX: f32 = 100.0;
/// 새 Polygon의 변 수.
pub const DEFAULT_POLYGON_SIDES: u32 = 5;

/// 생성 도구.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateTool {
    /// Frame.
    Frame,
    /// Rectangle.
    Rectangle,
    /// Ellipse.
    Ellipse,
    /// Line.
    Line,
    /// Polygon.
    Polygon,
}

impl CreateTool {
    /// 캔버스 도구의 생성 도구. 상자·선 생성 도구가 아니면 `None`입니다(Text·Image는 각자 행).
    pub const fn of(tool: CanvasTool) -> Option<CreateTool> {
        match tool {
            CanvasTool::Frame => Some(CreateTool::Frame),
            CanvasTool::Rectangle => Some(CreateTool::Rectangle),
            CanvasTool::Ellipse => Some(CreateTool::Ellipse),
            CanvasTool::Line => Some(CreateTool::Line),
            CanvasTool::Polygon => Some(CreateTool::Polygon),
            _ => None,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            CreateTool::Frame => "Frame",
            CreateTool::Rectangle => "Rectangle",
            CreateTool::Ellipse => "Ellipse",
            CreateTool::Line => "Line",
            CreateTool::Polygon => "Polygon",
        }
    }
}

/// 만들 모양(부모 local px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CreateShape {
    /// 상자 x, y, width, height.
    Box([f32; 4]),
    /// 선 시작·끝.
    Line([f32; 2], [f32; 2]),
}

/// 생성 수식키.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CreateModifiers {
    /// 정비례·45도.
    pub shift: bool,
    /// 시작점이 중심.
    pub option: bool,
    /// Space를 누르는 동안 시작점 이동.
    pub space: bool,
}

/// 생성 드래그 상태.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateGesture {
    tool: CreateTool,
    start: [f32; 2],
    pointer: [f32; 2],
    /// 부모 local 1px의 화면 pt(끌기 시작 거리 판정).
    scale: f32,
    dragging: bool,
    /// Space를 누른 시점의 (pointer, start).
    space: Option<([f32; 2], [f32; 2])>,
    modifiers: CreateModifiers,
}

impl CreateGesture {
    /// 누른 점 `start`(부모 local)에서 시작합니다. `scale`은 부모 local 1px의 화면 pt입니다.
    pub fn begin(tool: CreateTool, start: [f32; 2], scale: f32) -> Option<CreateGesture> {
        (start.iter().all(|v| v.is_finite()) && scale.is_finite() && scale > 0.0).then_some(
            CreateGesture {
                tool,
                start,
                pointer: start,
                scale,
                dragging: false,
                space: None,
                modifiers: CreateModifiers::default(),
            },
        )
    }

    /// 도구.
    pub fn tool(&self) -> CreateTool {
        self.tool
    }

    /// 끌기 시작 거리를 넘었는지.
    pub fn dragging(&self) -> bool {
        self.dragging
    }

    /// pointer update(부모 local)와 지금 수식키. 지금 preview 모양을 돌려줍니다.
    pub fn update(&mut self, pointer: [f32; 2], modifiers: CreateModifiers) -> CreateShape {
        if pointer.iter().all(|v| v.is_finite()) {
            match (modifiers.space, self.space) {
                (true, None) => self.space = Some((pointer, self.start)),
                (true, Some((at, start))) => {
                    self.start = [start[0] + pointer[0] - at[0], start[1] + pointer[1] - at[1]];
                }
                (false, _) => self.space = None,
            }
            self.pointer = pointer;
            let moved = (pointer[0] - self.start[0]).hypot(pointer[1] - self.start[1]) * self.scale;
            if moved >= DRAG_THRESHOLD_PT {
                self.dragging = true;
            }
        }
        self.modifiers = modifiers;
        self.shape()
    }

    /// 지금 모양: 끌기 전이면 클릭 기본 크기, 끈 뒤면 드래그 영역.
    pub fn shape(&self) -> CreateShape {
        let CreateModifiers { shift, option, .. } = self.modifiers;
        let s = self.start;
        if !self.dragging {
            let half = DEFAULT_SIZE_PX / 2.0;
            return match (self.tool, option) {
                (CreateTool::Line, false) => CreateShape::Line(s, [s[0] + DEFAULT_SIZE_PX, s[1]]),
                (CreateTool::Line, true) => {
                    CreateShape::Line([s[0] - half, s[1]], [s[0] + half, s[1]])
                }
                (_, false) => CreateShape::Box([s[0], s[1], DEFAULT_SIZE_PX, DEFAULT_SIZE_PX]),
                (_, true) => {
                    CreateShape::Box([s[0] - half, s[1] - half, DEFAULT_SIZE_PX, DEFAULT_SIZE_PX])
                }
            };
        }
        let mut d = [self.pointer[0] - s[0], self.pointer[1] - s[1]];
        if self.tool == CreateTool::Line {
            if shift {
                let length = d[0].hypot(d[1]);
                let step = std::f32::consts::FRAC_PI_4;
                let angle = (d[1].atan2(d[0]) / step).round() * step;
                let (sin, cos) = angle.sin_cos();
                // 직각 배수는 정확한 0·±1로 둡니다.
                let exact = |v: f32| if v.abs() < 1e-6 { 0.0 } else { v };
                d = [exact(cos) * length, exact(sin) * length];
            }
            return if option {
                CreateShape::Line([s[0] - d[0], s[1] - d[1]], [s[0] + d[0], s[1] + d[1]])
            } else {
                CreateShape::Line(s, [s[0] + d[0], s[1] + d[1]])
            };
        }
        if shift {
            let side = d[0].abs().max(d[1].abs());
            let sign = |v: f32| if v < 0.0 { -1.0 } else { 1.0 };
            d = [side * sign(d[0]), side * sign(d[1])];
        }
        if option {
            let [w, h] = [d[0].abs(), d[1].abs()];
            CreateShape::Box([s[0] - w, s[1] - h, 2.0 * w, 2.0 * h])
        } else {
            let (x0, x1) = (s[0].min(s[0] + d[0]), s[0].max(s[0] + d[0]));
            let (y0, y1) = (s[1].min(s[1] + d[1]), s[1].max(s[1] + d[1]));
            CreateShape::Box([x0, y0, x1 - x0, y1 - y0])
        }
    }
}

/// pointer(world) 아래 새 node의 부모: 가장 위(깊은) 잠기지 않은 Frame 행(인스턴스 투영 행·잠김·숨김 제외), 없으면
/// View root.
pub fn creation_parent(
    model: &DocumentModel,
    scene: &CanvasScene,
    root: StableId,
    pointer: [f32; 2],
) -> StableId {
    let Ok(point) = Vec2::new(pointer[0], pointer[1]) else {
        return root;
    };
    for row in scene.frames.iter().rev() {
        if row.instance_frame || !row.hit_target {
            continue;
        }
        if !matches!(model.payload(row.id), Some(NodePayload::Frame(_))) {
            continue;
        }
        let Some(local) = row.world.inverse().and_then(|inv| inv.apply(point).ok()) else {
            continue;
        };
        let r = row.geometry.rect;
        let inside = local.x() >= r.origin().x()
            && local.y() >= r.origin().y()
            && local.x() <= r.origin().x() + r.size().width()
            && local.y() <= r.origin().y() + r.size().height();
        if inside && edit_permission(model, row.id, EditIntent::Edit).is_ok() {
            return row.id;
        }
    }
    root
}

/// world 점을 `parent` local로. 부모 local 1px의 화면 배율도 함께 돌려줍니다(`zoom`은 camera 배율).
pub fn parent_local(
    scene: &CanvasScene,
    parent: StableId,
    world: [f32; 2],
    zoom: f32,
) -> Option<([f32; 2], f32)> {
    let geometry = node_geometry(scene, parent)?;
    let to_world = geometry.local.then(&geometry.parent_world).ok()?;
    let local = to_world
        .inverse()?
        .apply(Vec2::new(world[0], world[1]).ok()?)
        .ok()?;
    let (a, b, _, _, _, _) = to_world.components();
    Some(([local.x(), local.y()], a.hypot(b) * zoom))
}

fn invalid(reason: &'static str) -> TransformError {
    TransformError::Disabled(reason)
}

/// 확정: `shape`(부모 local)로 node를 만들고 선택해 history 한 항목으로 기록한 뒤 `tools`를 Move로 돌립니다.
/// `pointer`는 흐름 순서를 정할 놓은 world 점입니다. 새 node id를 돌려줍니다.
#[allow(clippy::too_many_arguments)]
pub fn commit(
    window: &mut ProjectWindowSession,
    scene: &CanvasScene,
    tools: &mut CanvasToolState,
    tool: CreateTool,
    parent: StableId,
    shape: CreateShape,
    pointer: [f32; 2],
    paint: ShapePaint,
) -> Result<StableId, TransformError> {
    let model = window.document().ok_or(TransformError::Stale)?.model();
    if !matches!(model.payload(parent), Some(NodePayload::Frame(_))) {
        return Err(invalid("parent-not-frame"));
    }
    edit_permission(model, parent, EditIntent::Edit).map_err(|_| invalid("locked"))?;
    let painted = match tool {
        CreateTool::Frame => true,
        CreateTool::Line => paint.stroke.is_some(),
        _ => paint.fill.is_some() || paint.stroke.is_some(),
    };
    if !painted || paint.stroke.is_some_and(|(_, width)| width.get() <= 0.0) {
        return Err(invalid("paint"));
    }
    let flow = is_flow_container(model, parent);
    let order_key: OrderKey = if flow {
        flow_insert_key(model, scene, parent, pointer).ok_or(TransformError::Stale)?
    } else {
        model.next_child_order_key(parent)
    };
    // 모양의 원점과 local geometry.
    let (origin, geometry) = match (tool, shape) {
        (CreateTool::Line, CreateShape::Line(a, b)) => {
            let end = Vec2::new(b[0] - a[0], b[1] - a[1]).map_err(|_| invalid("non-finite"))?;
            if end == Vec2::ZERO {
                return Err(invalid("empty"));
            }
            (
                a,
                Some(ShapeGeometry::Line(LineParams {
                    start: Vec2::ZERO,
                    end,
                    cap: LineCap::Butt,
                })),
            )
        }
        (CreateTool::Line, CreateShape::Box(_)) | (_, CreateShape::Line(..)) => {
            return Err(invalid("shape-kind"))
        }
        (tool, CreateShape::Box([x, y, w, h])) => {
            let size = Size2::new(w, h).map_err(|_| invalid("non-finite"))?;
            if size.is_empty() {
                return Err(invalid("empty"));
            }
            let bounds = Rect::new(Vec2::ZERO, size);
            let geometry = match tool {
                CreateTool::Frame => None,
                CreateTool::Rectangle => Some(ShapeGeometry::Rectangle(RectangleParams {
                    size,
                    corner_radius: CornerRadius::ZERO,
                })),
                CreateTool::Ellipse => Some(ShapeGeometry::Ellipse(
                    EllipseParams::new(bounds).map_err(|_| invalid("shape"))?,
                )),
                CreateTool::Polygon => Some(ShapeGeometry::Polygon(
                    PolygonParams::new(
                        bounds,
                        DEFAULT_POLYGON_SIDES,
                        Angle::new(0.0).map_err(|_| invalid("shape"))?,
                        0.0,
                    )
                    .map_err(|_| invalid("shape"))?,
                )),
                CreateTool::Line => unreachable!("Line은 위 갈래"),
            };
            ([x, y], geometry)
        }
    };
    // 흐름 자식은 위치를 쓰지 않습니다(흐름이 자리를 정함).
    let position = if flow { [0.0, 0.0] } else { origin };
    let transform = Transform2D::new(1.0, 0.0, 0.0, 1.0, position[0], position[1])
        .map_err(|_| invalid("non-finite"))?;
    let mut ids = FreshIds::default();
    let node = ids.fresh();
    let (payload, modifiers) = match geometry {
        None => (
            NodePayload::Frame(FramePayload {
                size: match shape {
                    CreateShape::Box([_, _, w, h]) => {
                        Size2::new(w, h).map_err(|_| invalid("non-finite"))?
                    }
                    CreateShape::Line(..) => return Err(invalid("shape-kind")),
                },
                transform,
                background: paint.fill,
                ..FramePayload::default()
            }),
            Vec::new(),
        ),
        Some(geometry) => {
            // 캔버스 평가 순서(Transform → Fill → Stroke)대로 쌓습니다.
            let mut list = Vec::new();
            if !flow {
                list.push(ModifierPayload::Transform(transform));
            }
            if let Some(color) = paint.fill {
                list.push(ModifierPayload::Fill { color });
            }
            if let Some((color, width)) = paint.stroke {
                list.push(ModifierPayload::Stroke { color, width });
            }
            (NodePayload::Shape(geometry), list)
        }
    };
    let mut stack = ModifierStack::new(node);
    for payload in modifiers {
        let order = OrderKey::between(stack.entries().last().map(|entry| &entry.order), None);
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(ids.fresh()),
                order,
                kind: payload.kind(),
                enabled: true,
                payload,
            })
            .map_err(|_| invalid("modifier"))?;
    }
    ids.finish()?;
    let mut insert = InsertNode::single(NodeRecord {
        id: node,
        parent: Some(parent),
        order_key,
        visible: true,
        locked: false,
        display_name: tool.name().into(),
        payload,
    });
    if !stack.entries().is_empty() {
        insert.modifier_stacks.push(stack);
    }
    let tx = transaction(window, vec![Command::InsertNode(insert)])?;
    window
        .insert_and_select(&tx, node, history_label::INSERT)
        .map_err(|error| TransformError::Edit(format!("{error:?}")))?;
    tools.handle(ToolInput::CreationFinished);
    Ok(node)
}

#[cfg(test)]
mod tests;

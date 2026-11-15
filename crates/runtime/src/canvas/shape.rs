//! 고정 도형의 canonical path와 순서가 있는 Transform·단색 Fill·Stroke를 같은 장면에 투영합니다.
use super::*;
use stellar_core_types::value::{Rect, Vec2};
use stellar_document::modifier::ModifierStack;
use stellar_document::path::PathCommand as DocumentPath;
use stellar_document::shape::{
    evaluate_shape_modifiers, evaluate_shape_path, ShapeGeometry, ShapeModifierStyle,
};
use stellar_scene::builder::{SceneScope, ShapeInput, StrokeInput};
use stellar_scene::geometry::{PathCommand, PathGeometry};
use stellar_scene::stack::{Paint, StrokeCap};

/// 캔버스가 평가하는 transform·fill·stroke. 규칙은 웹 출력과 같은 [`evaluate_shape_modifiers`] 한 곳에 있고,
/// 캔버스는 평가하지 못하는 stack을 그리지 않습니다. 부모 공간 이동은 마지막 Transform에 적용합니다.
pub(crate) fn shape_style(stack: Option<&ModifierStack>) -> Option<ShapeModifierStyle> {
    evaluate_shape_modifiers(stack).ok()
}

/// Free Frame 목록은 경로를 할당하지 않습니다. 이 정보는 도형 행에만 있습니다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanvasShapePath {
    /// SceneBuilder가 확정한 local path.
    pub geometry: PathGeometry,
    /// stroke cap. Line은 저장된 cap을 따릅니다.
    pub cap: StrokeCap,
    /// 길이 0인 선의 cap을 그릴 정확한 local bounds.
    pub point_cap: Option<Rect>,
}

pub(super) fn project(
    model: &DocumentModel,
    id: StableId,
    shape: &ShapeGeometry,
    parent_scope: &SceneScope,
    parent: Option<u32>,
    locked: bool,
    builder: &mut SceneBuilder,
) -> Option<(CanvasFrame, Option<CanvasShapePath>)> {
    let style = shape_style(model.modifier_stack(id))?;
    let mut node = NodeInput::new(id, style.transform);
    node.locked = locked;
    if let Some(color) = style.fill {
        node.paint.fills.push(Paint::Solid(color));
    }
    if let Some(stroke) = style.stroke {
        node.paint.strokes.push(StrokeInput {
            paint: Paint::Solid(stroke.color),
            width: stroke.thickness(),
        });
    }
    let mut cap = StrokeCap::Butt;
    let mut point_cap = None;
    let mut radii = CornerRadii::ZERO;
    let bounds = match shape {
        ShapeGeometry::Rectangle(rect) => {
            let (tl, tr, br, bl) = rect.effective_corner_radius();
            radii = CornerRadii::new(tl, tr, br, bl).ok()?;
            Rect::new(Vec2::ZERO, rect.size)
        }
        ShapeGeometry::Ellipse(ellipse) => ellipse.bounds,
        ShapeGeometry::Polygon(polygon) => polygon.bounds(),
        ShapeGeometry::Line(line) => {
            cap = StrokeCap::from_tag(line.cap as u8)?;
            if line.is_zero_length() {
                point_cap = style
                    .stroke
                    .and_then(|stroke| line.stroked_bounds(stroke.thickness()));
            }
            let min = Vec2::new(
                line.start.x().min(line.end.x()),
                line.start.y().min(line.end.y()),
            )
            .ok()?;
            let size = stellar_core_types::value::Size2::new(
                (line.end.x() - line.start.x()).abs(),
                (line.end.y() - line.start.y()).abs(),
            )
            .ok()?;
            Rect::new(min, size)
        }
    };
    let path = evaluate_shape_path(shape)
        .ok()?
        .into_iter()
        .map(|command| match command {
            DocumentPath::MoveTo(p) => PathCommand::MoveTo(p),
            DocumentPath::LineTo(p) => PathCommand::LineTo(p),
            DocumentPath::CubicTo { c1, c2, to } => PathCommand::CubicTo { c1, c2, to },
            DocumentPath::Close => PathCommand::Close,
        })
        .collect();
    let item = builder
        .build_shape(
            &mut parent_scope.clone(),
            &ShapeInput {
                node,
                layout_rect: bounds,
                geometry: PathGeometry::new(path).ok()?,
                cap,
            },
        )
        .ok()?;
    let world = item.world_transform();
    world.inverse()?;
    Aabb::from_rect(bounds).transformed(&world)?;
    let rectangle = matches!(shape, ShapeGeometry::Rectangle(_));
    // 보통 Rectangle은 기존 rounded-rect 행을 그대로 사용합니다. 가운데 stroke가 있으면 실제 path가 필요합니다.
    let path = if !rectangle || style.stroke.is_some() {
        let SceneContent::Path(path) = item.content() else {
            return None;
        };
        Some(CanvasShapePath {
            geometry: path.geometry.clone(),
            cap,
            point_cap,
        })
    } else {
        None
    };
    let frame = CanvasFrame {
        rectangle_shape: rectangle,
        path_shape: path.is_some(),
        instance_frame: false,
        text_node: false,
        image_asset: None,
        id,
        parent,
        world,
        geometry: RectPrimitive {
            rect: bounds,
            radii,
        },
        background: style.fill,
        border: style.stroke,
        clip_children: false,
        hit_target: item.hit().is_hit_target(),
    };
    Some((frame, path))
}

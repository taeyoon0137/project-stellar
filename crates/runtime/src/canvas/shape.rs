//! 고정 Rectangle Shape의 Transform·단색 Fill을 문서 순서대로 평가합니다.
use super::*;
use stellar_core_types::value::{Rect, Vec2};
use stellar_document::modifier::ModifierPayload;
use stellar_document::path::PathCommand as DocumentPath;
use stellar_scene::builder::{SceneScope, ShapeInput};
use stellar_scene::geometry::{PathCommand, PathGeometry};
use stellar_scene::stack::{Paint, StrokeCap};

pub(super) fn rectangle(
    model: &DocumentModel,
    id: StableId,
    shape: &RectangleParams,
    parent_scope: &SceneScope,
    parent: Option<u32>,
    locked: bool,
    builder: &mut SceneBuilder,
) -> Option<CanvasFrame> {
    let mut transform = Transform2D::identity();
    let mut fill = None;
    if let Some(stack) = model.modifier_stack(id) {
        for entry in stack.entries().iter().filter(|e| e.enabled) {
            match entry.payload {
                ModifierPayload::Transform(next) if fill.is_none() => {
                    transform = transform.then(&next).ok()?
                }
                ModifierPayload::Fill { color } if fill.is_none() => fill = Some(color),
                _ => return None,
            }
        }
    }
    let mut node = NodeInput::new(id, transform);
    node.locked = locked;
    if let Some(color) = fill {
        node.paint.fills.push(Paint::Solid(color));
    }
    let bounds = Rect::new(Vec2::ZERO, shape.size);
    let path = shape
        .to_path()
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
                cap: StrokeCap::Butt,
            },
        )
        .ok()?;
    let world = item.world_transform();
    world.inverse()?;
    Aabb::from_rect(bounds).transformed(&world)?;
    let r = shape.effective_corner_radius();
    Some(CanvasFrame {
        rectangle_shape: true,
        id,
        parent,
        world,
        geometry: RectPrimitive {
            rect: bounds,
            radii: CornerRadii::new(r.0, r.1, r.2, r.3).ok()?,
        },
        background: fill,
        border: None,
        clip_children: false,
        hit_target: item.hit().is_hit_target(),
    })
}

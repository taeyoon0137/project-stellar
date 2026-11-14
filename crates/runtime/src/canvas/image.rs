use super::*;
use stellar_core_types::value::{Rect, Vec2};
use stellar_document::{
    image::{CropRect, FitMode, ImagePayload, SamplingMode},
    modifier::ModifierPayload,
};
use stellar_scene::builder::{ImageInput, SceneScope};
use stellar_scene::node::{ImageCrop, ImageFit, ImageSampling};

pub(super) fn project(
    model: &DocumentModel,
    id: StableId,
    image: &ImagePayload,
    scope: &SceneScope,
    parent: Option<u32>,
    locked: bool,
    builder: &mut SceneBuilder,
) -> Option<CanvasFrame> {
    if image.fit != FitMode::Contain
        || image.crop != CropRect::FULL
        || image.sampling != SamplingMode::Linear
        || image.color_space_override.is_some()
    {
        return None;
    }
    let mut transform = Transform2D::identity();
    for modifier in model
        .modifier_stack(id)
        .into_iter()
        .flat_map(|s| s.entries())
        .filter(|m| m.enabled)
    {
        let ModifierPayload::Transform(next) = modifier.payload else {
            return None;
        };
        transform = transform.then(&next).ok()?;
    }
    let mut node = NodeInput::new(id, transform);
    node.locked = locked;
    let built = builder
        .build_image(
            &mut scope.clone(),
            &ImageInput {
                node,
                asset: image.asset,
                size: image.size,
                fit: ImageFit::Contain,
                crop: ImageCrop::new(0., 0., 1., 1.).ok()?,
                sampling: ImageSampling::Linear,
                color_space: None,
            },
        )
        .ok()?;
    let world = built.world_transform();
    world.inverse()?;
    let rect = Rect::new(Vec2::ZERO, image.size);
    Aabb::from_rect(rect).transformed(&world)?;
    Some(CanvasFrame {
        rectangle_shape: false,
        instance_frame: false,
        image_asset: Some(image.asset.id()),
        text_node: false,
        id,
        parent,
        world,
        geometry: RectPrimitive {
            rect,
            radii: CornerRadii::ZERO,
        },
        background: None,
        border: None,
        clip_children: false,
        hit_target: built.hit().is_hit_target(),
    })
}

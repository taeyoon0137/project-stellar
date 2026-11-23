use super::*;
use stellar_core_types::value::{Rect, Vec2};
use stellar_document::{
    image::{CropRect, FitMode, ImagePayload, SamplingMode},
    modifier::ModifierPayload,
};
use stellar_scene::builder::{ImageInput, SceneScope};
use stellar_scene::node::{ImageCrop, ImageFit, ImageSampling};

/// 문서 `FitMode`를 scene `ImageFit`으로 옮깁니다(두 tag는 같습니다).
pub(super) fn scene_fit(fit: FitMode) -> ImageFit {
    match fit {
        FitMode::Fill => ImageFit::Fill,
        FitMode::Contain => ImageFit::Contain,
        FitMode::Cover => ImageFit::Cover,
        FitMode::None => ImageFit::None,
    }
}

/// 이미지 행의 object-fit 배치(`ImageFit::place`). `fit`은 표시 행의 tag, `bounds`는 행의 local 상자, `intrinsic`은
/// 셸이 디코드한 원본 픽셀 크기입니다. (그릴 사각형, 상자로 자를지)를 돌려주며 셸은 그 사각형에 그리기만 합니다.
pub fn image_placement(fit: u8, bounds: [f32; 4], intrinsic: [f32; 2]) -> Option<([f32; 4], bool)> {
    let placed = ImageFit::from_tag(fit)?.place(bounds, intrinsic)?;
    Some((placed.dest, placed.clip_to_bounds))
}

pub(super) fn project(
    model: &DocumentModel,
    id: StableId,
    image: &ImagePayload,
    scope: &SceneScope,
    parent: Option<u32>,
    locked: bool,
    builder: &mut SceneBuilder,
) -> Option<CanvasFrame> {
    if image.crop != CropRect::FULL
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
                fit: scene_fit(image.fit),
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
        path_shape: false,
        instance_frame: false,
        image_asset: Some(image.asset.id()),
        image_fit: Some(image.fit),
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

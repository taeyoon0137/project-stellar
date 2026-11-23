use super::*;
use stellar_core_types::value::{Rect, Vec2};
use stellar_document::image::ImagePayload;
use stellar_scene::builder::SceneScope;
use stellar_scene::node::ImageFit;

/// 이미지 행의 object-fit 배치(`ImageFit::place`). `fit`은 표시 행의 tag, `bounds`는 행의 local 상자, `intrinsic`은
/// 셸이 디코드한 원본 픽셀 크기입니다. (그릴 사각형, 상자로 자를지)를 돌려주며 셸은 그 사각형에 그리기만 합니다.
pub fn image_placement(fit: u8, bounds: [f32; 4], intrinsic: [f32; 2]) -> Option<([f32; 4], bool)> {
    let placed = ImageFit::from_tag(fit)?.place(bounds, intrinsic)?;
    Some((placed.dest, placed.clip_to_bounds))
}

/// 이미지 payload와 modifier stack을 scene 입력으로 바꾸는 scene crate 함수(ADR-0116 결정 1).
pub(crate) use stellar_scene::document::inputs::image_input;

pub(super) fn project(
    model: &DocumentModel,
    id: StableId,
    image: &ImagePayload,
    scope: &SceneScope,
    parent: Option<u32>,
    locked: bool,
    builder: &mut SceneBuilder,
) -> Option<CanvasFrame> {
    let input = image_input(id, image, model.modifier_stack(id), locked)?;
    let built = builder.build_image(&mut scope.clone(), &input).ok()?;
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

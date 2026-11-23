//! 고정 도형의 canonical path와 순서가 있는 Transform·단색 Fill·Stroke를 같은 장면에 투영합니다.
use super::*;
use stellar_core_types::value::Rect;
use stellar_document::shape::ShapeGeometry;
use stellar_scene::builder::SceneScope;
use stellar_scene::geometry::PathGeometry;
use stellar_scene::stack::StrokeCap;

/// 캔버스가 평가하는 transform·fill·stroke와 도형 scene 입력. 규칙은 scene crate의 문서 변환 함수 한 곳에
/// 있습니다(ADR-0116 결정 1).
pub(crate) use stellar_scene::document::inputs::{shape_source, shape_style, ShapeSource};

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
    let ShapeSource {
        input,
        style,
        radii,
        point_cap,
    } = shape_source(id, shape, model.modifier_stack(id), locked)?;
    let bounds = input.layout_rect;
    let cap = input.cap;
    let item = builder
        .build_shape(&mut parent_scope.clone(), &input)
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
        image_fit: None,
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

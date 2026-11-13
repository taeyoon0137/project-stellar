//! 테스트 전용 scene fixture. 공개 [`SceneBuilder`]로 document 평가 결과를 흉내 내 만듭니다.

use std::sync::Arc;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::value::{Rect, Size2, Transform2D, Vec2};
use stellar_scene::builder::{
    BorderInput, FrameClipInput, FrameInput, ImageInput, NodeInput, PathInput, SceneBuilder,
    ShapeInput, StrokeInput,
};
use stellar_scene::geometry::{FillRule, PathCommand, PathGeometry};
use stellar_scene::node::{CornerRadii, ImageCrop, ImageFit, ImageSampling, SceneNode};
use stellar_scene::revision::SceneRevisionMap;
use stellar_scene::stack::{Paint, StrokeCap};
use stellar_scene::SceneSnapshot;

use crate::request::{
    ExportBackground, ExportFormat, ExportRequest, ExportScale, ExportSource, OutputPath,
    TargetProfileKey,
};

/// 고정 View id.
pub(crate) fn view_id() -> StableId {
    StableId::new_v7(0x0195_c530_3acd, 0x7bc, 0x0414_c213_f61c_b965)
}

/// `n`번 document node id(UUIDv7 레이아웃, 결정론).
pub(crate) fn node_id(n: u64) -> StableId {
    StableId::new_v7(0x0195_c530_3acd, 0x7bc, 0x1000 + n)
}

/// 이미지 asset id.
pub(crate) fn asset(n: u64) -> AssetRef {
    AssetRef::new(StableId::new_v7(0x0195_c530_3acd, 0x7bd, n))
}

pub(crate) fn srgb(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color::new(ColorSpace::Srgb, r, g, b, a).unwrap()
}

pub(crate) fn translate(x: f32, y: f32) -> Transform2D {
    Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y).unwrap()
}

pub(crate) fn p(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y).unwrap()
}

pub(crate) fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(p(x, y), Size2::new(w, h).unwrap())
}

pub(crate) fn triangle() -> PathGeometry {
    PathGeometry::new(vec![
        PathCommand::MoveTo(p(0.0, 0.0)),
        PathCommand::LineTo(p(40.0, 0.0)),
        PathCommand::LineTo(p(20.0, 30.0)),
        PathCommand::Close,
    ])
    .unwrap()
}

/// 기본 fixture 설정.
#[derive(Debug, Clone)]
pub(crate) struct Basic {
    /// Root Frame이 bounds clip을 거는지.
    pub clip: bool,
    /// Shape 위치(Frame local).
    pub shape_at: (f32, f32),
    /// Image asset.
    pub image: AssetRef,
    /// Shape fill.
    pub shape_fill: Color,
}

impl Default for Basic {
    fn default() -> Basic {
        Basic {
            clip: true,
            shape_at: (10.0, 10.0),
            image: asset(1),
            shape_fill: srgb(1.0, 0.0, 0.0, 1.0),
        }
    }
}

/// Root Frame(100×80, 배경·테두리, 둥근 모서리) 아래 Shape·Path·Image가 있는 View scene.
///
/// node id: Frame 1, Shape 2, Path 3, Image 4.
pub(crate) fn basic_roots(config: &Basic) -> Vec<Arc<SceneNode>> {
    let mut builder = SceneBuilder::new();
    let mut root = builder.root_scope();
    let frame_input = FrameInput {
        node: NodeInput::new(node_id(1), translate(20.0, 10.0)),
        size: Size2::new(100.0, 80.0).unwrap(),
        corner_radii: CornerRadii::new(8.0, 8.0, 8.0, 8.0).unwrap(),
        background: Some(srgb(1.0, 1.0, 1.0, 1.0)),
        border: Some(BorderInput {
            color: srgb(0.0, 0.0, 0.0, 0.5),
            thickness: 2.0,
        }),
        clip: if config.clip {
            FrameClipInput::Bounds
        } else {
            FrameClipInput::None
        },
    };
    let mut frame = builder.begin_frame(&root, &frame_input).unwrap();
    let mut shape = NodeInput::new(node_id(2), translate(config.shape_at.0, config.shape_at.1));
    shape.paint.fills.push(Paint::Solid(config.shape_fill));
    shape.paint.strokes.push(StrokeInput {
        paint: Paint::Solid(srgb(0.0, 0.0, 1.0, 1.0)),
        width: 3.0,
    });
    builder
        .build_shape(
            frame.scope_mut(),
            &ShapeInput {
                node: shape,
                layout_rect: rect(0.0, 0.0, 40.0, 30.0),
                geometry: triangle(),
                cap: StrokeCap::Round,
            },
        )
        .unwrap();
    let mut path = NodeInput::new(node_id(3), translate(50.0, 10.0));
    path.paint
        .fills
        .push(Paint::Solid(srgb(0.0, 0.5, 0.0, 1.0)));
    builder
        .build_path(
            frame.scope_mut(),
            &PathInput {
                node: path,
                layout_rect: rect(0.0, 0.0, 40.0, 30.0),
                geometry: triangle(),
                fill_rule: FillRule::EvenOdd,
            },
        )
        .unwrap();
    builder
        .build_image(
            frame.scope_mut(),
            &ImageInput {
                node: NodeInput::new(node_id(4), translate(10.0, 45.0)),
                asset: config.image,
                size: Size2::new(30.0, 20.0).unwrap(),
                fit: ImageFit::Cover,
                crop: ImageCrop::FULL,
                sampling: ImageSampling::Linear,
                color_space: None,
            },
        )
        .unwrap();
    builder.finish_frame(&mut root, frame).unwrap();
    builder.finish(root)
}

/// [`basic_roots`] snapshot(document revision 1, scene revision 1).
pub(crate) fn basic_snapshot(config: &Basic) -> SceneSnapshot {
    SceneRevisionMap::new()
        .generate(DocumentRevision(1), basic_roots(config))
        .unwrap()
}

/// target profile key.
pub(crate) fn profile() -> TargetProfileKey {
    TargetProfileKey::new("web-modern").unwrap()
}

/// View 전체 요청.
pub(crate) fn view_request(format: ExportFormat) -> ExportRequest {
    request(ExportSource::view(view_id()).unwrap(), format)
}

/// 임의 source 요청.
pub(crate) fn request(source: ExportSource, format: ExportFormat) -> ExportRequest {
    ExportRequest::new(
        source,
        profile(),
        ExportScale::ONE,
        ExportBackground::Transparent,
        OutputPath::new(format!("/tmp/stellar-export/out.{}", format.name())).unwrap(),
        format,
    )
    .unwrap()
}

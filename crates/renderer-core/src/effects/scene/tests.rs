//! scene frame encoder 테스트 (ADR-0116 결정 4).
//!
//! scene → frame plan → 그리기 목록 → CPU 기준 rasterizer. 결과 pixel이 scene 의미(painter 순서, layer 합성, blend, shadow,
//! clip)와 같은지 봅니다. GPU와 CPU 기준의 일치는 renderer-metal 적합성 test가 봅니다.

use super::*;
use crate::effects::artifact::RgbaImage;
use crate::effects::fixture::{srgb, Category, Fixture};
use crate::effects::raster::render_reference;
use crate::pass_compiler::tests::{input, viewport_with};
use crate::pass_compiler::{compile_frame, EffectBoundaries};
use crate::render_graph::{SampleCount, TextureFormat};
use crate::test_scene::{group, opaque, rect, snapshot, ClipFx, NodeFx, ShadowFx, IDENTITY};

struct NoResources;

impl FrameResources for NoResources {
    fn image(
        &self,
        _: &SceneNode,
        _: &ImagePrimitive,
    ) -> Result<Option<Arc<ImageData>>, EncodeError> {
        Ok(None)
    }

    fn text(
        &self,
        _: &SceneNode,
        _: &TextPrimitive,
    ) -> Result<Option<Vec<GlyphShape>>, EncodeError> {
        Ok(None)
    }
}

/// scene을 `size`² 선형 8-bit target에 그립니다(premultiplied 저장 값).
fn render(roots: &[NodeFx], size: u32, view: [f32; 6], samples: SampleCount) -> RgbaImage {
    let scene = snapshot(roots);
    let viewport = viewport_with(size as f32, size as f32, view);
    let mut compile = input(&scene, &viewport, EffectBoundaries::Minimal);
    compile.clip_caps.samples = samples;
    let plan = compile_frame(&compile).unwrap();
    let ops = encode_plan(&plan, &viewport, &NoResources).unwrap();
    let mut fixture = Fixture::new("scene", Category::Frame, size, size);
    fixture.format = TextureFormat::Rgba8Unorm;
    fixture.clear = srgb(0.0, 0.0, 0.0, 0.0);
    fixture.ops = ops;
    render_reference(&fixture).unwrap()
}

#[test]
fn painter_order_and_pixel_aligned_rects() {
    let image = render(
        &[
            rect(1, [0.0, 0.0, 8.0, 8.0], opaque(1.0, 0.0, 0.0)),
            rect(2, [4.0, 4.0, 4.0, 4.0], opaque(0.0, 0.0, 1.0)),
        ],
        8,
        IDENTITY,
        SampleCount::X4,
    );
    assert_eq!(image.pixel(1, 1), [255, 0, 0, 255]);
    assert_eq!(image.pixel(5, 5), [0, 0, 255, 255], "뒤 draw가 위");
}

#[test]
fn group_opacity_composites_overlap_once() {
    let image = render(
        &[group(
            1,
            [0.0, 0.0, 8.0, 8.0],
            vec![
                rect(2, [0.0, 0.0, 6.0, 6.0], opaque(1.0, 0.0, 0.0)),
                rect(3, [2.0, 2.0, 6.0, 6.0], opaque(0.0, 1.0, 0.0)),
            ],
        )
        .with_opacity(0.5)],
        8,
        IDENTITY,
        SampleCount::X4,
    );
    // 겹친 곳도 위 내용(초록)만 불투명도 0.5로 한 번 합성합니다.
    assert_eq!(image.pixel(3, 3), [0, 128, 0, 128]);
    assert_eq!(image.pixel(1, 1), [128, 0, 0, 128]);
}

#[test]
fn blend_layer_reads_the_backdrop() {
    let image = render(
        &[
            rect(1, [0.0, 0.0, 8.0, 8.0], [0.5, 0.5, 1.0, 1.0]),
            group(
                2,
                [0.0, 0.0, 8.0, 8.0],
                vec![
                    rect(3, [0.0, 0.0, 4.0, 8.0], opaque(1.0, 0.0, 0.0)),
                    rect(4, [2.0, 0.0, 4.0, 8.0], opaque(1.0, 1.0, 0.0)),
                ],
            )
            .with_blend(SceneBlendMode::Multiply.tag()),
        ],
        8,
        IDENTITY,
        SampleCount::X4,
    );
    // backdrop sRGB 0.5 → 선형 0.214, multiply 빨강(1, 0, 0): (0.214, 0, 0) → 저장 55.
    let backdrop = crate::color::ColorSpace::Srgb.decode_channel(0.5);
    let red = (backdrop * 255.0).round() as u8;
    assert_eq!(image.pixel(1, 1), [red, 0, 0, 255]);
    // 노랑(1, 1, 0) × backdrop(0.214, 0.214, 1) = (0.214, 0.214, 0).
    assert_eq!(image.pixel(5, 1), [red, red, 0, 255]);
    // group 밖은 backdrop 그대로.
    assert_eq!(image.pixel(7, 7), [red, red, 255, 255]);
}

#[test]
fn shadow_is_drawn_below_content_at_device_offset() {
    let image = render(
        &[
            rect(1, [1.0, 1.0, 2.0, 2.0], opaque(1.0, 1.0, 1.0)).with_shadow(ShadowFx {
                offset: [1.0, 1.0],
                blur: 0.0,
                color: [0.0, 0.0, 1.0, 1.0],
            }),
        ],
        16,
        // 배율 2: offset 2 device pixel, 내용 (2..6).
        [2.0, 0.0, 0.0, 2.0, 0.0, 0.0],
        SampleCount::X4,
    );
    assert_eq!(image.pixel(3, 3), [255, 255, 255, 255], "내용이 위");
    assert_eq!(image.pixel(6, 6), [0, 0, 255, 255], "shadow만");
    assert_eq!(image.pixel(1, 1), [0, 0, 0, 0]);
}

#[test]
fn complex_clips_cut_content_with_stencil_or_mask_plans() {
    let clip = ClipFx {
        radii: [4.0; 4],
        ..crate::test_scene::rect_clip([0.0, 0.0, 8.0, 8.0])
    };
    let node = rect(1, [0.0, 0.0, 16.0, 16.0], opaque(1.0, 0.0, 0.0)).with_clip(clip);
    for samples in [SampleCount::X4, SampleCount::X1] {
        let image = render(std::slice::from_ref(&node), 16, IDENTITY, samples);
        assert_eq!(image.pixel(4, 4), [255, 0, 0, 255], "{samples:?}");
        // 둥근 모서리 밖과 clip 상자 밖.
        assert_eq!(image.pixel(0, 0), [0, 0, 0, 0], "{samples:?}");
        assert_eq!(image.pixel(10, 4), [0, 0, 0, 0], "{samples:?}");
    }
}

#[test]
fn rounded_rect_path_and_transforms() {
    let rect = Rect::new(
        Vec2::new(0.0, 0.0).unwrap(),
        stellar_scene::Size2::new(4.0, 2.0).unwrap(),
    );
    let path = rounded_rect_path(rect, (1.0, 0.0, 1.0, 0.0)).unwrap();
    // MoveTo + (LineTo, Cubic) × 반경 있는 두 모서리 + LineTo × 반경 없는 두 모서리 + Close.
    assert_eq!(path.commands().len(), 1 + 2 + 1 + 2 + 1 + 1);
    let moved = transform_path(
        &path,
        &Transform2D::new(2.0, 0.0, 0.0, 2.0, 1.0, 1.0).unwrap(),
    )
    .unwrap();
    assert_eq!(
        moved.commands()[0],
        PathCommand::MoveTo(Vec2::new(3.0, 1.0).unwrap())
    );
}

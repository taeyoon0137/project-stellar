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

#[test]
fn frame_box_shadow_ignores_background_alpha_and_stays_outside_the_box() {
    // 배경 없는 Frame과 반투명 배경 Frame: CSS box-shadow는 배경과 관계없이 상자 모양이고 상자 안에는 비치지 않습니다
    // (ADR-0130 결정 1).
    let shadow = ShadowFx {
        offset: [2.0, 2.0],
        blur: 0.0,
        color: [0.0, 0.0, 1.0, 1.0],
    };
    let empty = group(1, [2.0, 2.0, 6.0, 6.0], Vec::new()).with_shadow(shadow);
    let image = render(&[empty], 16, IDENTITY, SampleCount::X4);
    // 상자 (2..8) 안은 비고, offset 자리(4..10) 중 상자 밖은 shadow 색입니다.
    assert_eq!(image.pixel(5, 5), [0, 0, 0, 0], "상자 안은 비움");
    assert_eq!(image.pixel(9, 9), [0, 0, 255, 255], "상자 밖 shadow");
    assert_eq!(image.pixel(9, 5), [0, 0, 255, 255]);
    assert_eq!(image.pixel(1, 1), [0, 0, 0, 0]);
    let translucent = rect(1, [2.0, 2.0, 6.0, 6.0], [1.0, 1.0, 1.0, 0.5]).with_shadow(shadow);
    let image = render(&[translucent], 16, IDENTITY, SampleCount::X4);
    // 상자 안은 배경만(shadow가 비치지 않음), 상자 밖 shadow는 불투명(배경 alpha 0.5와 무관).
    assert_eq!(image.pixel(5, 5), [128, 128, 128, 128]);
    assert_eq!(image.pixel(9, 9), [0, 0, 255, 255]);
}

#[test]
fn text_and_path_shadows_follow_content_alpha() {
    // Frame이 아닌 내용(text)은 drop-shadow(): 내용 alpha 그대로입니다. glyph 모양이 없으면 내용 alpha도 없어 그림자가
    // 없습니다(배경 없는 Frame과 다름).
    let text = rect(1, [2.0, 2.0, 6.0, 6.0], opaque(1.0, 1.0, 1.0))
        .with_content(crate::test_scene::ContentFx::Text {
            key: 1,
            bounds: [2.0, 2.0, 6.0, 6.0],
        })
        .with_shadow(ShadowFx {
            offset: [2.0, 2.0],
            blur: 0.0,
            color: [0.0, 0.0, 1.0, 1.0],
        });
    let image = render(&[text], 16, IDENTITY, SampleCount::X4);
    // fill layer가 glyph 합(빈 모양)을 칠하므로 아무것도 없습니다.
    assert_eq!(image.pixel(9, 9), [0, 0, 0, 0]);
}

#[test]
fn fractional_shadow_offset_is_not_rounded() {
    // device offset 0.5: 반올림하면 한 pixel 옮겨지거나 그대로지만, bilinear 합성은 경계 pixel을 반씩 덮습니다
    // (ADR-0130 결정 3).
    let image = render(
        &[
            rect(1, [2.0, 2.0, 4.0, 4.0], opaque(1.0, 1.0, 1.0)).with_shadow(ShadowFx {
                offset: [0.5, 0.0],
                blur: 0.0,
                color: [0.0, 0.0, 1.0, 1.0],
            }),
        ],
        16,
        IDENTITY,
        SampleCount::X4,
    );
    // 상자 (2..6) 오른쪽 바로 밖 pixel 6: shadow가 x 2.5..6.5를 덮으므로 절반.
    let edge = image.pixel(6, 3);
    assert!((126..=129).contains(&edge[3]), "{edge:?}");
    assert_eq!(image.pixel(7, 3), [0, 0, 0, 0]);
}

//! 적합성 fixture 목록 (XI.10.2~XI.10.4, XI.11.3).
//!
//! 기능별 묶음 함수가 fixture를 만들고, 모든 backend harness가 같은 목록을 그립니다. fixture 이름은 artifact 파일 이름이라
//! 목록 전체에서 유일해야 합니다.

use super::fixture::{
    fill_mesh, linear_gradient, p3, solid, srgb, Category, ClipSpec, Coverage, CoverageShape,
    Feature, FillRule, Fixture, LayerOp, Mesh, Op, PathBuilder, SceneBlendMode,
};
use super::gradient::GradientStop;
use crate::color::ColorSpace;
use crate::output_color::OutputColor;

/// 사각형 단색 fill.
pub fn rect(x0: f32, y0: f32, x1: f32, y1: f32, color: OutputColor) -> Op {
    solid(Mesh::rect(x0, y0, x1, y1), color, Coverage::Disjoint)
}

/// layer 그리기.
pub fn layer(content: Vec<Op>, blend: SceneBlendMode) -> Op {
    Op::Layer(Box::new(LayerOp::new(content, blend)))
}

/// blend backdrop: 가로 띠 네 개(불투명·반투명).
fn backdrop(translucent: bool) -> Vec<Op> {
    let a = if translucent { 0.6 } else { 1.0 };
    vec![
        rect(0.0, 0.0, 32.0, 8.0, srgb(0.9, 0.1, 0.1, a)),
        rect(0.0, 8.0, 32.0, 16.0, srgb(0.1, 0.8, 0.2, a)),
        rect(0.0, 16.0, 32.0, 24.0, srgb(0.2, 0.3, 0.9, a * 0.5)),
        rect(0.0, 24.0, 32.0, 32.0, srgb(0.45, 0.45, 0.45, a)),
    ]
}

/// blend source: 세로 띠 세 개(불투명·반투명), 오른쪽 열은 비어 있음.
fn blend_source() -> Vec<Op> {
    vec![
        rect(0.0, 0.0, 8.0, 32.0, srgb(1.0, 0.8, 0.1, 1.0)),
        rect(8.0, 0.0, 16.0, 32.0, srgb(0.1, 0.6, 0.9, 0.5)),
        rect(16.0, 0.0, 24.0, 32.0, srgb(0.3, 0.3, 0.3, 0.75)),
    ]
}

/// blend mode 기준 image(XI.10.2): mode 6개 × 불투명·반투명 backdrop, 중첩 blend.
pub fn blend_fixtures() -> Vec<Fixture> {
    let mut out = Vec::new();
    for mode in SceneBlendMode::ALL {
        for translucent in [false, true] {
            let mut fixture = Fixture::new(
                format!(
                    "blend-{mode}-{}",
                    if translucent { "translucent" } else { "opaque" }
                ),
                Category::Modifier,
                32,
                32,
            )
            .feature(Feature::Blend);
            fixture.tolerance = "blend";
            if translucent {
                fixture.clear = srgb(0.0, 0.0, 0.0, 0.0);
            }
            fixture.ops = backdrop(translucent);
            fixture.ops.push(layer(blend_source(), mode));
            out.push(fixture);
        }
    }
    // 중첩: multiply layer 안에 screen layer.
    let mut nested = Fixture::new("blend-nested-multiply-screen", Category::Modifier, 32, 32)
        .feature(Feature::Blend);
    nested.tolerance = "blend";
    nested.ops = backdrop(false);
    nested.ops.push(layer(
        vec![
            rect(4.0, 4.0, 28.0, 28.0, srgb(0.8, 0.7, 0.2, 1.0)),
            layer(
                vec![rect(10.0, 0.0, 22.0, 32.0, srgb(0.2, 0.4, 0.9, 0.8))],
                SceneBlendMode::Screen,
            ),
        ],
        SceneBlendMode::Multiply,
    ));
    out.push(nested);
    // 선형 저장 target에서도 같은 식입니다.
    let mut linear = Fixture::new("blend-overlay-linear-target", Category::Modifier, 32, 32)
        .feature(Feature::Blend);
    linear.format = crate::render_graph::TextureFormat::Rgba8Unorm;
    linear.tolerance = "blend";
    linear.ops = backdrop(false);
    linear
        .ops
        .push(layer(blend_source(), SceneBlendMode::Overlay));
    out.push(linear);
    out
}

/// 시각 순서 stop.
pub fn stop(position: f32, color: OutputColor) -> GradientStop {
    GradientStop { position, color }
}

fn gradient_fixture(name: &str, category: Category, ops: Vec<Op>) -> Fixture {
    let mut fixture = Fixture::new(name, category, 32, 32).feature(Feature::Gradient);
    fixture.tolerance = "gradient";
    fixture.ops = ops;
    fixture
}

/// gradient pixel fixture(XI.10.3): 선형 2·3 stop, 대각선, 같은 position tie, 범위 밖 stop, pad, 투명 stop, P3, 길이 0, 도형 안.
pub fn gradient_fixtures() -> Vec<Fixture> {
    let red = srgb(0.9, 0.1, 0.1, 1.0);
    let blue = srgb(0.1, 0.2, 0.9, 1.0);
    let yellow = srgb(0.95, 0.85, 0.2, 1.0);
    let green = srgb(0.1, 0.7, 0.3, 1.0);
    let full = || Mesh::rect(0.0, 0.0, 32.0, 32.0);
    let horizontal = |stops| linear_gradient(full(), [0.0, 0.0], [32.0, 0.0], stops);
    let mut out = vec![
        gradient_fixture(
            "gradient-linear-two-stop",
            Category::Frame,
            vec![linear_gradient(
                Mesh::rect(2.0, 2.0, 30.0, 30.0),
                [2.0, 0.0],
                [30.0, 0.0],
                vec![stop(0.0, red), stop(1.0, blue)],
            )],
        ),
        gradient_fixture(
            "gradient-linear-three-stop-vertical",
            Category::Frame,
            vec![linear_gradient(
                full(),
                [0.0, 2.0],
                [0.0, 30.0],
                vec![stop(0.0, yellow), stop(0.5, green), stop(1.0, blue)],
            )],
        ),
        gradient_fixture(
            "gradient-linear-diagonal",
            Category::Frame,
            vec![linear_gradient(
                full(),
                [0.0, 0.0],
                [32.0, 32.0],
                vec![stop(0.0, red), stop(0.6, yellow), stop(1.0, green)],
            )],
        ),
        // 같은 position 두 stop: x = 16에서 앞(파랑) → 뒤(초록) hard edge.
        gradient_fixture(
            "gradient-tie-hard-edge",
            Category::Frame,
            vec![horizontal(vec![
                stop(0.0, red),
                stop(0.5, blue),
                stop(0.5, green),
                stop(1.0, yellow),
            ])],
        ),
        // 같은 position 세 stop: 가운데 stop은 폭 0이라 보이지 않습니다.
        gradient_fixture(
            "gradient-tie-three-stops",
            Category::Frame,
            vec![horizontal(vec![
                stop(0.0, red),
                stop(0.4, blue),
                stop(0.4, srgb(1.0, 0.0, 1.0, 1.0)),
                stop(0.4, green),
                stop(1.0, yellow),
            ])],
        ),
        gradient_fixture(
            "gradient-stops-outside-unit-range",
            Category::Frame,
            vec![horizontal(vec![stop(-0.5, red), stop(1.5, blue)])],
        ),
        gradient_fixture(
            "gradient-pad-inner-stops",
            Category::Frame,
            vec![horizontal(vec![stop(0.3, red), stop(0.7, blue)])],
        ),
        gradient_fixture(
            "gradient-transparent-stop",
            Category::Frame,
            vec![horizontal(vec![
                stop(0.0, red),
                stop(1.0, srgb(0.1, 0.2, 0.9, 0.0)),
            ])],
        ),
        gradient_fixture(
            "gradient-zero-length",
            Category::Frame,
            vec![linear_gradient(
                full(),
                [16.0, 16.0],
                [16.0, 16.0],
                vec![stop(0.0, red), stop(1.0, blue)],
            )],
        ),
        gradient_fixture(
            "gradient-in-ellipse",
            Category::Shape,
            vec![linear_gradient(
                fill_mesh(
                    &PathBuilder::new()
                        .ellipse(16.0, 16.0, 13.0, 9.0)
                        .build()
                        .expect("ellipse"),
                    FillRule::NonZero,
                )
                .expect("mesh"),
                [3.0, 0.0],
                [29.0, 0.0],
                vec![stop(0.0, green), stop(1.0, red)],
            )],
        ),
        gradient_fixture(
            "gradient-in-rounded-rect",
            Category::Frame,
            vec![linear_gradient(
                fill_mesh(
                    &PathBuilder::new()
                        .rounded_rect(3.0, 3.0, 29.0, 29.0, 7.0)
                        .build()
                        .expect("rounded"),
                    FillRule::NonZero,
                )
                .expect("mesh"),
                [0.0, 3.0],
                [0.0, 29.0],
                vec![stop(0.0, yellow), stop(1.0, blue)],
            )],
        ),
    ];
    let mut p3_surface = gradient_fixture(
        "gradient-p3-on-p3-surface",
        Category::P3,
        vec![horizontal(vec![
            stop(0.0, p3(1.0, 0.0, 0.0, 1.0)),
            stop(1.0, p3(0.0, 1.0, 0.0, 1.0)),
        ])],
    )
    .feature(Feature::DisplayP3);
    p3_surface.requested = ColorSpace::DisplayP3;
    p3_surface.display_p3 = true;
    out.push(p3_surface);
    let mut p3_fallback = gradient_fixture(
        "gradient-p3-on-srgb-surface",
        Category::P3,
        vec![horizontal(vec![
            stop(0.0, p3(1.0, 0.0, 0.0, 1.0)),
            stop(1.0, p3(0.0, 1.0, 0.0, 1.0)),
        ])],
    )
    .feature(Feature::DisplayP3);
    p3_fallback.requested = ColorSpace::DisplayP3;
    out.push(p3_fallback);
    let mut linear = gradient_fixture(
        "gradient-linear-target",
        Category::Frame,
        vec![horizontal(vec![
            stop(0.0, red),
            stop(0.5, yellow),
            stop(1.0, blue),
        ])],
    );
    linear.format = crate::render_graph::TextureFormat::Rgba8Unorm;
    out.push(linear);
    out
}

/// 효과 layer 그리기.
pub fn effect(layer: LayerOp) -> Op {
    Op::Layer(Box::new(layer))
}

fn effect_fixture(name: &str, feature: Feature, tolerance: &'static str, ops: Vec<Op>) -> Fixture {
    let category = match feature {
        Feature::ScissorClip | Feature::StencilClip | Feature::MaskClip => Category::Clip,
        _ => Category::Modifier,
    };
    let mut fixture = Fixture::new(name, category, 32, 32).feature(feature);
    fixture.tolerance = tolerance;
    fixture.ops = ops;
    fixture
}

/// 겹치는 두 사각형(그룹 불투명도와 개별 불투명도가 다른 내용).
fn overlapping() -> Vec<Op> {
    vec![
        rect(4.0, 4.0, 20.0, 20.0, srgb(0.9, 0.2, 0.1, 1.0)),
        rect(12.0, 12.0, 28.0, 28.0, srgb(0.1, 0.3, 0.9, 1.0)),
    ]
}

/// 배경 줄무늬(효과가 backdrop과 섞이는지 보이게).
fn stripes() -> Vec<Op> {
    (0..4)
        .map(|i| {
            let x = i as f32 * 8.0;
            rect(x, 0.0, x + 4.0, 32.0, srgb(0.2, 0.2, 0.2, 1.0))
        })
        .collect()
}

fn with_backdrop(mut ops: Vec<Op>) -> Vec<Op> {
    let mut all = stripes();
    all.append(&mut ops);
    all
}

fn star_mesh(cx: f32, cy: f32, r: f32) -> Mesh {
    let points: Vec<[f32; 2]> = (0..5)
        .map(|i| {
            let a =
                std::f32::consts::PI * 2.0 * (i as f32 * 2.0) / 5.0 - std::f32::consts::FRAC_PI_2;
            [cx + r * a.cos(), cy + r * a.sin()]
        })
        .collect();
    fill_mesh(
        &PathBuilder::new().polygon(&points).build().expect("star"),
        FillRule::NonZero,
    )
    .expect("star mesh")
}

/// clip·mask·opacity·shadow·blur 기능별 독립 fixture(XI.10.4).
pub fn effect_fixtures() -> Vec<Fixture> {
    let base = || LayerOp::new(overlapping(), SceneBlendMode::Normal);
    let rounded = fill_mesh(
        &PathBuilder::new()
            .rounded_rect(6.0, 6.0, 26.0, 26.0, 6.0)
            .build()
            .expect("rounded"),
        FillRule::NonZero,
    )
    .expect("rounded mesh");
    let ellipse = fill_mesh(
        &PathBuilder::new()
            .ellipse(16.0, 16.0, 12.0, 8.0)
            .build()
            .expect("ellipse"),
        FillRule::NonZero,
    )
    .expect("ellipse mesh");
    let shadow_color = srgb(0.0, 0.0, 0.0, 0.6);
    vec![
        // --- opacity ---
        effect_fixture(
            "opacity-group-half",
            Feature::Opacity,
            "default",
            with_backdrop(vec![effect(base().with_opacity(0.5))]),
        ),
        effect_fixture(
            "opacity-group-quarter",
            Feature::Opacity,
            "default",
            with_backdrop(vec![effect(base().with_opacity(0.25))]),
        ),
        effect_fixture(
            "opacity-zero",
            Feature::Opacity,
            "default",
            with_backdrop(vec![effect(base().with_opacity(0.0))]),
        ),
        effect_fixture(
            "opacity-nested",
            Feature::Opacity,
            "default",
            with_backdrop(vec![effect(
                LayerOp::new(
                    vec![
                        rect(2.0, 2.0, 30.0, 12.0, srgb(0.1, 0.7, 0.3, 1.0)),
                        effect(base().with_opacity(0.5)),
                    ],
                    SceneBlendMode::Normal,
                )
                .with_opacity(0.6),
            )]),
        ),
        // --- scissor clip ---
        effect_fixture(
            "clip-scissor-rect",
            Feature::ScissorClip,
            "default",
            with_backdrop(vec![effect(base().with_clip(ClipSpec::Scissor {
                x: 8,
                y: 6,
                width: 14,
                height: 18,
            }))]),
        ),
        effect_fixture(
            "clip-scissor-outside",
            Feature::ScissorClip,
            "default",
            with_backdrop(vec![effect(base().with_clip(ClipSpec::Scissor {
                x: 30,
                y: 30,
                width: 8,
                height: 8,
            }))]),
        ),
        // --- stencil path clip ---
        effect_fixture(
            "clip-stencil-rounded-rect",
            Feature::StencilClip,
            "curve",
            with_backdrop(vec![effect(
                base().with_clip(ClipSpec::Stencil(rounded.clone())),
            )]),
        ),
        effect_fixture(
            "clip-stencil-star",
            Feature::StencilClip,
            "curve",
            with_backdrop(vec![effect(
                base().with_clip(ClipSpec::Stencil(star_mesh(16.0, 16.0, 14.0))),
            )]),
        ),
        effect_fixture(
            "clip-stencil-nested",
            Feature::StencilClip,
            "curve",
            with_backdrop(vec![effect(
                LayerOp::new(
                    vec![effect(base().with_clip(ClipSpec::Stencil(ellipse.clone())))],
                    SceneBlendMode::Normal,
                )
                .with_clip(ClipSpec::Stencil(Mesh::rect(10.0, 0.0, 32.0, 32.0))),
            )]),
        ),
        // --- mask clip(coverage) ---
        effect_fixture(
            "clip-mask-soft",
            Feature::MaskClip,
            "default",
            with_backdrop(vec![effect(base().with_clip(ClipSpec::Mask(vec![
                CoverageShape {
                    mesh: Mesh::rect(0.0, 0.0, 16.0, 32.0),
                    coverage: 1.0,
                },
                CoverageShape {
                    mesh: Mesh::rect(16.0, 0.0, 32.0, 32.0),
                    coverage: 0.4,
                },
            ])))]),
        ),
        effect_fixture(
            "clip-mask-overlap-union",
            Feature::MaskClip,
            "curve",
            with_backdrop(vec![effect(base().with_clip(ClipSpec::Mask(vec![
                CoverageShape {
                    mesh: ellipse.clone(),
                    coverage: 0.5,
                },
                CoverageShape {
                    mesh: Mesh::rect(8.0, 8.0, 24.0, 24.0),
                    coverage: 0.5,
                },
            ])))]),
        ),
        // --- mask ---
        effect_fixture(
            "mask-stripes",
            Feature::Mask,
            "default",
            with_backdrop(vec![effect(
                base()
                    .with_mask(Mesh::rect(0.0, 0.0, 32.0, 10.0), 1.0)
                    .with_mask(Mesh::rect(0.0, 10.0, 32.0, 20.0), 0.3)
                    .with_mask(Mesh::rect(0.0, 20.0, 32.0, 32.0), 0.7),
            )]),
        ),
        effect_fixture(
            "mask-with-opacity",
            Feature::Mask,
            "default",
            with_backdrop(vec![effect(
                base().with_mask(rounded.clone(), 0.8).with_opacity(0.7),
            )]),
        ),
        // --- shadow ---
        effect_fixture(
            "shadow-offset-blur",
            Feature::Shadow,
            "blur",
            vec![effect(
                LayerOp::new(
                    vec![rect(8.0, 8.0, 20.0, 20.0, srgb(0.9, 0.6, 0.1, 1.0))],
                    SceneBlendMode::Normal,
                )
                .with_shadow([3, 4], 6.0, shadow_color),
            )],
        ),
        effect_fixture(
            "shadow-hard",
            Feature::Shadow,
            "blur",
            vec![effect(
                LayerOp::new(
                    vec![rect(6.0, 6.0, 18.0, 18.0, srgb(0.2, 0.6, 0.9, 1.0))],
                    SceneBlendMode::Normal,
                )
                .with_shadow([5, 5], 0.0, srgb(0.4, 0.0, 0.4, 0.8)),
            )],
        ),
        effect_fixture(
            "shadow-two-stacked",
            Feature::Shadow,
            "blur",
            vec![effect(
                LayerOp::new(
                    vec![rect(10.0, 10.0, 22.0, 22.0, srgb(0.95, 0.95, 0.95, 1.0))],
                    SceneBlendMode::Normal,
                )
                .with_shadow([-4, -3], 4.0, srgb(0.9, 0.1, 0.1, 0.7))
                .with_shadow([4, 3], 4.0, srgb(0.1, 0.1, 0.9, 0.7)),
            )],
        ),
        effect_fixture(
            "shadow-translucent-content",
            Feature::Shadow,
            "blur",
            vec![effect(
                LayerOp::new(
                    vec![
                        rect(6.0, 6.0, 26.0, 16.0, srgb(0.1, 0.7, 0.3, 0.4)),
                        rect(6.0, 16.0, 26.0, 26.0, srgb(0.1, 0.7, 0.3, 1.0)),
                    ],
                    SceneBlendMode::Normal,
                )
                .with_shadow([2, 2], 3.0, shadow_color),
            )],
        ),
        effect_fixture(
            "shadow-of-curve",
            Feature::Shadow,
            "blur",
            vec![effect(
                LayerOp::new(
                    vec![solid(
                        ellipse.clone(),
                        srgb(0.9, 0.3, 0.5, 1.0),
                        Coverage::Disjoint,
                    )],
                    SceneBlendMode::Normal,
                )
                .with_shadow([0, 3], 5.0, shadow_color),
            )],
        ),
        // --- layer blur ---
        effect_fixture(
            "blur-sigma-1",
            Feature::Blur,
            "blur",
            with_backdrop(vec![effect(base().with_blur(1.0))]),
        ),
        effect_fixture(
            "blur-sigma-3",
            Feature::Blur,
            "blur",
            with_backdrop(vec![effect(base().with_blur(3.0))]),
        ),
        effect_fixture(
            "blur-edge-content",
            Feature::Blur,
            "blur",
            vec![effect(
                LayerOp::new(
                    vec![rect(0.0, 0.0, 10.0, 32.0, srgb(0.1, 0.3, 0.9, 1.0))],
                    SceneBlendMode::Normal,
                )
                .with_blur(2.0),
            )],
        ),
        effect_fixture(
            "blur-with-opacity",
            Feature::Blur,
            "blur",
            with_backdrop(vec![effect(base().with_blur(2.0).with_opacity(0.6))]),
        ),
        // --- 조합: 효과 순서(shadow → blur → clip → mask → opacity → blend) ---
        effect_fixture(
            "effects-combined-order",
            Feature::Shadow,
            "blur",
            with_backdrop(vec![effect(
                LayerOp::new(overlapping(), SceneBlendMode::Multiply)
                    .with_shadow([2, 2], 4.0, shadow_color)
                    .with_blur(1.0)
                    .with_clip(ClipSpec::Stencil(rounded))
                    .with_opacity(0.8),
            )]),
        )
        .feature(Feature::Blur)
        .feature(Feature::StencilClip)
        .feature(Feature::Opacity)
        .feature(Feature::Blend),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blend::composite;
    use crate::color::{decode_pixel, to_working};
    use crate::effects::raster::render_reference;

    #[test]
    fn blend_reference_pixels_follow_blend_formula() {
        // 기준 image 한 pixel을 식으로 다시 계산합니다: (2, 2) = 빨강 띠 위 노랑 source.
        for fixture in blend_fixtures()
            .into_iter()
            .filter(|f| f.name.ends_with("-opaque"))
        {
            let image = render_reference(&fixture).unwrap();
            let plan = fixture.plan();
            let mode = SceneBlendMode::ALL
                .into_iter()
                .find(|m| fixture.name == format!("blend-{m}-opaque"))
                .unwrap();
            let dst = to_working(&plan, srgb(0.9, 0.1, 0.1, 1.0));
            let src = to_working(&plan, srgb(1.0, 0.8, 0.1, 1.0));
            let expected = composite(mode, src, dst);
            let actual = decode_pixel(&image.pixel(2, 2), fixture.format, plan.surface()).unwrap();
            for i in 0..4 {
                assert!(
                    (actual.rgba[i] - expected.rgba[i]).abs() < 0.01,
                    "{}: {actual:?} vs {expected:?}",
                    fixture.name
                );
            }
            // 빈 열(x = 28)은 backdrop 그대로입니다.
            let untouched =
                decode_pixel(&image.pixel(28, 2), fixture.format, plan.surface()).unwrap();
            assert!((untouched.rgba[0] - dst.rgba[0]).abs() < 0.01);
        }
        let names: std::collections::BTreeSet<_> =
            blend_fixtures().into_iter().map(|f| f.name).collect();
        assert_eq!(names.len(), 14);
    }
}

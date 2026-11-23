//! 적합성 fixture 목록 (XI.10.2~XI.10.4, XI.11.3).
//!
//! 기능별 묶음 함수가 fixture를 만들고, 모든 backend harness가 같은 목록을 그립니다. fixture 이름은 artifact 파일 이름이라
//! 목록 전체에서 유일해야 합니다.

use std::sync::Arc;

use super::compare::ToleranceConfig;
use super::fixture::{
    fill_mesh, linear_gradient, p3, solid, srgb, stroke_mesh, Category, ClipSpec, Coverage,
    CoverageMask, CoverageShape, Feature, FillRule, Fixture, GlyphOp, ImageData, ImageOp, LayerOp,
    Mesh, Op, PathBuilder, SceneBlendMode, StrokeCap,
};
use super::gradient::GradientStop;
use crate::color::ColorSpace;
use crate::output_color::OutputColor;
use crate::placeholder::{placeholder_texture, PlaceholderKind, TextureFilter, TextureWrap};
use crate::stroke::{DashPattern, StrokeJoin, StrokeStyle};

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
        effect_fixture(
            "clip-stencil-ring-evenodd",
            Feature::StencilClip,
            "curve",
            with_backdrop(vec![effect(
                base().with_clip(ClipSpec::Stencil(
                    fill_mesh(
                        &PathBuilder::new()
                            .ellipse(16.0, 16.0, 14.0, 14.0)
                            .ellipse(16.0, 16.0, 7.0, 7.0)
                            .build()
                            .expect("ring"),
                        FillRule::EvenOdd,
                    )
                    .expect("ring mesh"),
                )),
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

fn basic(
    name: &str,
    category: Category,
    feature: Feature,
    tolerance: &'static str,
    ops: Vec<Op>,
) -> Fixture {
    let mut fixture = Fixture::new(name, category, 32, 32).feature(feature);
    fixture.tolerance = tolerance;
    fixture.ops = ops;
    fixture
}

fn path_fill(path: PathBuilder, rule: FillRule, color: OutputColor) -> Op {
    solid(
        fill_mesh(&path.build().expect("path"), rule).expect("fill mesh"),
        color,
        Coverage::Disjoint,
    )
}

fn path_stroke(path: PathBuilder, style: StrokeStyle, color: OutputColor) -> Op {
    solid(
        stroke_mesh(&path.build().expect("path"), &style).expect("stroke mesh"),
        color,
        Coverage::Union,
    )
}

fn regular_polygon(cx: f32, cy: f32, r: f32, sides: usize, rotation: f32) -> PathBuilder {
    let points: Vec<[f32; 2]> = (0..sides)
        .map(|i| {
            let a = rotation + std::f32::consts::PI * 2.0 * i as f32 / sides as f32;
            [cx + r * a.cos(), cy + r * a.sin()]
        })
        .collect();
    PathBuilder::new().polygon(&points)
}

fn star(cx: f32, cy: f32, r: f32) -> PathBuilder {
    let points: Vec<[f32; 2]> = (0..5)
        .map(|i| {
            let a =
                std::f32::consts::PI * 2.0 * (i as f32 * 2.0) / 5.0 - std::f32::consts::FRAC_PI_2;
            [cx + r * a.cos(), cy + r * a.sin()]
        })
        .collect();
    PathBuilder::new().polygon(&points)
}

fn width(w: f32) -> StrokeStyle {
    StrokeStyle::new(w).expect("stroke width")
}

/// Frame: 사각형 fill·모서리·테두리(inside border는 even-odd 고리).
pub fn frame_fixtures() -> Vec<Fixture> {
    let fill = srgb(0.2, 0.5, 0.8, 1.0);
    let border = srgb(0.1, 0.1, 0.15, 1.0);
    let ring = |x0: f32, y0: f32, x1: f32, y1: f32, w: f32| {
        PathBuilder::new()
            .polygon(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]])
            .polygon(&[
                [x0 + w, y0 + w],
                [x1 - w, y0 + w],
                [x1 - w, y1 - w],
                [x0 + w, y1 - w],
            ])
    };
    let rounded_ring = |r: f32, w: f32| {
        PathBuilder::new()
            .rounded_rect(4.0, 4.0, 28.0, 28.0, r)
            .rounded_rect(4.0 + w, 4.0 + w, 28.0 - w, 28.0 - w, (r - w).max(0.0))
    };
    let f = |name: &str, tolerance, ops| {
        basic(name, Category::Frame, Feature::SolidFill, tolerance, ops)
    };
    vec![
        f(
            "frame-solid",
            "default",
            vec![rect(4.0, 4.0, 28.0, 28.0, fill)],
        ),
        f(
            "frame-translucent",
            "default",
            vec![rect(4.0, 4.0, 28.0, 28.0, srgb(0.2, 0.5, 0.8, 0.5))],
        ),
        f(
            "frame-subpixel-edges",
            "curve",
            vec![rect(4.25, 3.5, 27.75, 28.5, fill)],
        ),
        f(
            "frame-rounded-2",
            "curve",
            vec![path_fill(
                PathBuilder::new().rounded_rect(4.0, 4.0, 28.0, 28.0, 2.0),
                FillRule::NonZero,
                fill,
            )],
        ),
        f(
            "frame-rounded-8",
            "curve",
            vec![path_fill(
                PathBuilder::new().rounded_rect(4.0, 4.0, 28.0, 28.0, 8.0),
                FillRule::NonZero,
                fill,
            )],
        ),
        f(
            "frame-rounded-pill",
            "curve",
            vec![path_fill(
                PathBuilder::new().rounded_rect(2.0, 10.0, 30.0, 22.0, 16.0),
                FillRule::NonZero,
                fill,
            )],
        ),
        f(
            "frame-border-inside",
            "default",
            vec![
                rect(4.0, 4.0, 28.0, 28.0, fill),
                path_fill(ring(4.0, 4.0, 28.0, 28.0, 3.0), FillRule::EvenOdd, border),
            ],
        ),
        f(
            "frame-border-only",
            "default",
            vec![path_fill(
                ring(6.0, 6.0, 26.0, 26.0, 2.0),
                FillRule::EvenOdd,
                border,
            )],
        ),
        f(
            "frame-border-rounded",
            "curve",
            vec![
                path_fill(
                    PathBuilder::new().rounded_rect(4.0, 4.0, 28.0, 28.0, 8.0),
                    FillRule::NonZero,
                    fill,
                ),
                path_fill(rounded_ring(8.0, 3.0), FillRule::EvenOdd, border),
            ],
        ),
        f(
            "frame-translucent-border",
            "default",
            vec![
                rect(4.0, 4.0, 28.0, 28.0, fill),
                path_fill(
                    ring(4.0, 4.0, 28.0, 28.0, 4.0),
                    FillRule::EvenOdd,
                    srgb(1.0, 1.0, 1.0, 0.5),
                ),
            ],
        ),
        f(
            "frame-nested",
            "default",
            vec![
                rect(2.0, 2.0, 30.0, 30.0, srgb(0.9, 0.9, 0.85, 1.0)),
                rect(6.0, 6.0, 26.0, 18.0, srgb(0.85, 0.3, 0.2, 1.0)),
                rect(6.0, 20.0, 26.0, 26.0, srgb(0.3, 0.7, 0.3, 0.8)),
            ],
        ),
        f("frame-dark-background", "default", {
            let mut ops = vec![rect(0.0, 0.0, 32.0, 32.0, srgb(0.05, 0.05, 0.08, 1.0))];
            ops.push(rect(8.0, 8.0, 24.0, 24.0, srgb(0.95, 0.8, 0.3, 0.7)));
            ops
        }),
    ]
}

/// Shape: 타원·원·다각형·별·선.
pub fn shape_fixtures() -> Vec<Fixture> {
    let color = srgb(0.85, 0.35, 0.2, 1.0);
    let s = |name: &str, ops| basic(name, Category::Shape, Feature::SolidFill, "curve", ops);
    let line = |cap: StrokeCap| {
        path_stroke(
            PathBuilder::new().move_to(6.0, 16.0).line_to(26.0, 16.0),
            width(5.0).with_cap(cap),
            srgb(0.1, 0.2, 0.3, 1.0),
        )
    };
    vec![
        s(
            "shape-ellipse",
            vec![path_fill(
                PathBuilder::new().ellipse(16.0, 16.0, 13.0, 8.0),
                FillRule::NonZero,
                color,
            )],
        ),
        s(
            "shape-circle",
            vec![path_fill(
                PathBuilder::new().ellipse(16.0, 16.0, 11.0, 11.0),
                FillRule::NonZero,
                color,
            )],
        ),
        s(
            "shape-triangle",
            vec![path_fill(
                PathBuilder::new().polygon(&[[16.0, 3.0], [29.0, 28.0], [3.0, 28.0]]),
                FillRule::NonZero,
                color,
            )],
        ),
        s(
            "shape-pentagon",
            vec![path_fill(
                regular_polygon(16.0, 16.0, 13.0, 5, -std::f32::consts::FRAC_PI_2),
                FillRule::NonZero,
                color,
            )],
        ),
        s(
            "shape-hexagon",
            vec![path_fill(
                regular_polygon(16.0, 16.0, 13.0, 6, 0.0),
                FillRule::NonZero,
                color,
            )],
        ),
        s(
            "shape-octagon",
            vec![path_fill(
                regular_polygon(16.0, 16.0, 13.0, 8, 0.3),
                FillRule::NonZero,
                color,
            )],
        ),
        s(
            "shape-star-nonzero",
            vec![path_fill(star(16.0, 16.0, 14.0), FillRule::NonZero, color)],
        ),
        s(
            "shape-star-evenodd",
            vec![path_fill(star(16.0, 16.0, 14.0), FillRule::EvenOdd, color)],
        ),
        s(
            "shape-rotated-rect",
            vec![path_fill(
                PathBuilder::new().polygon(&[[16.0, 2.0], [30.0, 16.0], [16.0, 30.0], [2.0, 16.0]]),
                FillRule::NonZero,
                color,
            )],
        ),
        s("shape-line-butt", vec![line(StrokeCap::Butt)]),
        s("shape-line-round", vec![line(StrokeCap::Round)]),
        s("shape-line-square", vec![line(StrokeCap::Square)]),
        s(
            "shape-translucent-overlap",
            vec![
                path_fill(
                    PathBuilder::new().ellipse(12.0, 16.0, 9.0, 9.0),
                    FillRule::NonZero,
                    srgb(0.9, 0.1, 0.1, 0.6),
                ),
                path_fill(
                    PathBuilder::new().ellipse(20.0, 16.0, 9.0, 9.0),
                    FillRule::NonZero,
                    srgb(0.1, 0.1, 0.9, 0.6),
                ),
            ],
        ),
    ]
}

/// Path: 곡선 fill·fill 규칙·구멍·stroke join·cap·dash.
pub fn path_fixtures() -> Vec<Fixture> {
    let ink = srgb(0.15, 0.15, 0.2, 1.0);
    let p = |name: &str, feature, ops| basic(name, Category::Path, feature, "curve", ops);
    let zigzag = || {
        PathBuilder::new()
            .move_to(4.0, 26.0)
            .line_to(10.0, 6.0)
            .line_to(16.0, 26.0)
            .line_to(22.0, 6.0)
            .line_to(28.0, 26.0)
    };
    let wave = || {
        PathBuilder::new()
            .move_to(3.0, 16.0)
            .cubic_to([9.0, 2.0], [14.0, 30.0], [20.0, 16.0])
            .cubic_to([23.0, 9.0], [27.0, 9.0], [29.0, 16.0])
    };
    vec![
        p(
            "path-cubic-fill",
            Feature::SolidFill,
            vec![path_fill(
                PathBuilder::new()
                    .move_to(4.0, 28.0)
                    .cubic_to([4.0, 4.0], [28.0, 4.0], [28.0, 28.0])
                    .close(),
                FillRule::NonZero,
                srgb(0.3, 0.6, 0.2, 1.0),
            )],
        ),
        p(
            "path-compound-hole-evenodd",
            Feature::SolidFill,
            vec![path_fill(
                PathBuilder::new()
                    .ellipse(16.0, 16.0, 13.0, 13.0)
                    .ellipse(16.0, 16.0, 6.0, 6.0),
                FillRule::EvenOdd,
                srgb(0.6, 0.2, 0.7, 1.0),
            )],
        ),
        p(
            "path-self-intersect-nonzero",
            Feature::SolidFill,
            vec![path_fill(
                PathBuilder::new().polygon(&[[4.0, 4.0], [28.0, 28.0], [28.0, 4.0], [4.0, 28.0]]),
                FillRule::NonZero,
                srgb(0.9, 0.5, 0.1, 1.0),
            )],
        ),
        p(
            "path-overlap-evenodd",
            Feature::SolidFill,
            vec![path_fill(
                PathBuilder::new()
                    .polygon(&[[3.0, 3.0], [20.0, 3.0], [20.0, 20.0], [3.0, 20.0]])
                    .polygon(&[[12.0, 12.0], [29.0, 12.0], [29.0, 29.0], [12.0, 29.0]]),
                FillRule::EvenOdd,
                srgb(0.1, 0.5, 0.6, 1.0),
            )],
        ),
        p(
            "path-stroke-miter",
            Feature::StrokeUnion,
            vec![path_stroke(
                zigzag(),
                width(3.0).with_join(StrokeJoin::Miter),
                ink,
            )],
        ),
        p(
            "path-stroke-round-join",
            Feature::StrokeUnion,
            vec![path_stroke(
                zigzag(),
                width(4.0).with_join(StrokeJoin::Round),
                ink,
            )],
        ),
        p(
            "path-stroke-bevel",
            Feature::StrokeUnion,
            vec![path_stroke(
                zigzag(),
                width(4.0).with_join(StrokeJoin::Bevel),
                ink,
            )],
        ),
        p(
            "path-stroke-translucent-union",
            Feature::StrokeUnion,
            vec![path_stroke(
                zigzag(),
                width(6.0).with_join(StrokeJoin::Round),
                srgb(0.8, 0.1, 0.3, 0.5),
            )],
        ),
        p(
            "path-stroke-dash",
            Feature::StrokeUnion,
            vec![path_stroke(
                PathBuilder::new().move_to(3.0, 16.0).line_to(29.0, 16.0),
                width(4.0).with_dash(Some(DashPattern::new(&[5.0, 3.0], 0.0).expect("dash"))),
                ink,
            )],
        ),
        p(
            "path-stroke-cubic",
            Feature::StrokeUnion,
            vec![path_stroke(
                wave(),
                width(3.0).with_cap(StrokeCap::Round),
                ink,
            )],
        ),
        p(
            "path-stroke-closed",
            Feature::StrokeUnion,
            vec![path_stroke(
                regular_polygon(16.0, 16.0, 11.0, 6, 0.0),
                width(3.0),
                srgb(0.2, 0.4, 0.9, 1.0),
            )],
        ),
        p(
            "path-fill-and-stroke",
            Feature::StrokeUnion,
            vec![
                path_fill(
                    star(16.0, 16.0, 12.0),
                    FillRule::NonZero,
                    srgb(0.95, 0.8, 0.2, 1.0),
                ),
                path_stroke(
                    star(16.0, 16.0, 12.0),
                    width(2.0).with_join(StrokeJoin::Round),
                    ink,
                ),
            ],
        ),
    ]
}

/// supersampling(4×4)한 합성 glyph coverage.
fn glyph_mask(width: u32, height: u32, inside: impl Fn(f32, f32) -> bool) -> CoverageMask {
    let mut data = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let mut hits = 0u32;
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = x as f32 + (sx as f32 + 0.5) / 4.0;
                    let py = y as f32 + (sy as f32 + 0.5) / 4.0;
                    hits += u32::from(inside(px, py));
                }
            }
            data.push(((hits * 255 + 8) / 16) as u8);
        }
    }
    CoverageMask {
        width,
        height,
        data,
    }
}

/// 합성 glyph 세 개: 막대(I), 고리(O), 기울어진 사선(/).
fn glyphs() -> Vec<Arc<CoverageMask>> {
    vec![
        Arc::new(glyph_mask(6, 12, |x, _| (2.0..4.0).contains(&x))),
        Arc::new(glyph_mask(10, 12, |x, y| {
            let d = ((x - 5.0) / 4.5).powi(2) + ((y - 6.0) / 5.5).powi(2);
            (0.45..=1.0).contains(&d)
        })),
        Arc::new(glyph_mask(8, 12, |x, y| (x - (8.0 - y * 0.6)).abs() < 1.2)),
    ]
}

fn glyph_run(origin: [i32; 2], color: OutputColor) -> Vec<Op> {
    let mut x = origin[0];
    glyphs()
        .into_iter()
        .map(|mask| {
            let op = Op::Glyphs(GlyphOp {
                origin: [x, origin[1]],
                mask: mask.clone(),
                color,
            });
            x += mask.width as i32 + 1;
            op
        })
        .collect()
}

/// Text: glyph coverage 합성(합성 coverage bitmap, ADR-0102 Consequences).
pub fn text_fixtures() -> Vec<Fixture> {
    let ink = srgb(0.1, 0.1, 0.12, 1.0);
    let t = |name: &str, ops| basic(name, Category::Text, Feature::Glyph, "default", ops);
    let mut out = vec![
        t("text-run-black", glyph_run([3, 10], ink)),
        t(
            "text-run-color",
            glyph_run([3, 10], srgb(0.85, 0.2, 0.25, 1.0)),
        ),
        t(
            "text-run-translucent",
            glyph_run([3, 10], srgb(0.1, 0.3, 0.8, 0.5)),
        ),
        t("text-on-dark", {
            let mut ops = vec![rect(0.0, 0.0, 32.0, 32.0, srgb(0.08, 0.08, 0.1, 1.0))];
            ops.extend(glyph_run([3, 10], srgb(0.95, 0.95, 0.9, 1.0)));
            ops
        }),
        t("text-two-lines", {
            let mut ops = glyph_run([2, 2], ink);
            ops.extend(glyph_run([6, 17], srgb(0.2, 0.5, 0.3, 1.0)));
            ops
        }),
        t(
            "text-overlapping-glyphs",
            vec![
                Op::Glyphs(GlyphOp {
                    origin: [8, 10],
                    mask: glyphs()[1].clone(),
                    color: srgb(0.9, 0.1, 0.1, 0.7),
                }),
                Op::Glyphs(GlyphOp {
                    origin: [12, 10],
                    mask: glyphs()[1].clone(),
                    color: srgb(0.1, 0.1, 0.9, 0.7),
                }),
            ],
        ),
        t(
            "text-in-opacity-layer",
            vec![effect(
                LayerOp::new(glyph_run([3, 10], ink), SceneBlendMode::Normal).with_opacity(0.5),
            )],
        )
        .feature(Feature::Opacity),
        t(
            "text-with-shadow",
            vec![effect(
                LayerOp::new(
                    glyph_run([3, 10], srgb(1.0, 1.0, 1.0, 1.0)),
                    SceneBlendMode::Normal,
                )
                .with_shadow([1, 1], 2.0, srgb(0.0, 0.0, 0.0, 0.8)),
            )],
        )
        .feature(Feature::Shadow),
        t(
            "text-clipped",
            vec![effect(
                LayerOp::new(glyph_run([3, 10], ink), SceneBlendMode::Normal).with_clip(
                    ClipSpec::Scissor {
                        x: 0,
                        y: 0,
                        width: 32,
                        height: 16,
                    },
                ),
            )],
        )
        .feature(Feature::ScissorClip),
        t("text-partially-outside", glyph_run([24, 25], ink)),
    ];
    let mut p3_text =
        t("text-p3-color", glyph_run([3, 10], p3(0.0, 0.7, 0.3, 1.0))).feature(Feature::DisplayP3);
    p3_text.requested = ColorSpace::DisplayP3;
    p3_text.display_p3 = true;
    out.push(p3_text);
    out[7].tolerance = "blur";
    out
}

fn checker_image(w: u32, h: u32, a: [u8; 4], b: [u8; 4], space: ColorSpace) -> Arc<ImageData> {
    Arc::new(ImageData {
        width: w,
        height: h,
        space,
        texels: (0..w * h)
            .map(|i| if (i % w + i / w).is_multiple_of(2) { a } else { b })
            .collect(),
    })
}

fn gradient_image(w: u32, h: u32) -> Arc<ImageData> {
    Arc::new(ImageData {
        width: w,
        height: h,
        space: ColorSpace::Srgb,
        texels: (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                [
                    (x * 255 / (w - 1)) as u8,
                    (y * 255 / (h - 1)) as u8,
                    160,
                    255,
                ]
            })
            .collect(),
    })
}

fn image_op(rect: [f32; 4], uv: [f32; 4], image: Arc<ImageData>, filter: TextureFilter) -> Op {
    Op::Image(ImageOp {
        rect,
        uv,
        image,
        filter,
        wrap: TextureWrap::ClampToEdge,
        opacity: 1.0,
    })
}

fn placeholder_image(kind: PlaceholderKind) -> Arc<ImageData> {
    let tile = placeholder_texture(kind);
    Arc::new(ImageData {
        width: tile.width(),
        height: tile.height(),
        space: tile.color_space(),
        texels: tile
            .pixels()
            .chunks_exact(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect(),
    })
}

/// Image: nearest·linear sampling, 확대, crop, 반투명, P3, placeholder(XI.7.4 data).
pub fn image_fixtures() -> Vec<Fixture> {
    let checker = checker_image(
        8,
        8,
        [230, 60, 40, 255],
        [30, 70, 200, 255],
        ColorSpace::Srgb,
    );
    let alpha_checker = checker_image(
        8,
        8,
        [230, 60, 40, 255],
        [30, 70, 200, 96],
        ColorSpace::Srgb,
    );
    let ramp = gradient_image(8, 8);
    let full = [0.0, 0.0, 1.0, 1.0];
    let i =
        |name: &str, tolerance, ops| basic(name, Category::Image, Feature::Image, tolerance, ops);
    let mut out = vec![
        i(
            "image-nearest-1x",
            "default",
            vec![image_op(
                [8.0, 8.0, 16.0, 16.0],
                full,
                checker.clone(),
                TextureFilter::Nearest,
            )],
        ),
        i(
            "image-nearest-4x",
            "default",
            vec![image_op(
                [0.0, 0.0, 32.0, 32.0],
                full,
                checker.clone(),
                TextureFilter::Nearest,
            )],
        ),
        i(
            "image-linear-4x",
            "filtered",
            vec![image_op(
                [0.0, 0.0, 32.0, 32.0],
                full,
                ramp.clone(),
                TextureFilter::Linear,
            )],
        ),
        i(
            "image-linear-checker-3x",
            "filtered",
            vec![image_op(
                [4.0, 4.0, 28.0, 28.0],
                full,
                checker.clone(),
                TextureFilter::Linear,
            )],
        ),
        i(
            "image-crop-uv",
            "default",
            vec![image_op(
                [0.0, 0.0, 32.0, 32.0],
                [0.25, 0.25, 0.75, 0.75],
                checker.clone(),
                TextureFilter::Nearest,
            )],
        ),
        i(
            "image-translucent-texels",
            "default",
            vec![image_op(
                [0.0, 0.0, 32.0, 32.0],
                full,
                alpha_checker,
                TextureFilter::Nearest,
            )],
        ),
        i(
            "image-opacity",
            "default",
            vec![Op::Image(ImageOp {
                rect: [0.0, 0.0, 32.0, 32.0],
                uv: full,
                image: checker.clone(),
                filter: TextureFilter::Nearest,
                wrap: TextureWrap::ClampToEdge,
                opacity: 0.4,
            })],
        ),
        i(
            "image-repeat",
            "default",
            vec![Op::Image(ImageOp {
                rect: [0.0, 0.0, 32.0, 32.0],
                uv: [0.0, 0.0, 2.0, 2.0],
                image: ramp.clone(),
                filter: TextureFilter::Nearest,
                wrap: TextureWrap::Repeat,
                opacity: 1.0,
            })],
        ),
        i(
            "image-placeholder-missing",
            "default",
            vec![Op::Image(ImageOp {
                rect: [0.0, 0.0, 32.0, 32.0],
                uv: [0.0, 0.0, 2.0, 2.0],
                image: placeholder_image(PlaceholderKind::Missing),
                filter: TextureFilter::Nearest,
                wrap: TextureWrap::Repeat,
                opacity: 1.0,
            })],
        ),
        i(
            "image-placeholder-decoding",
            "default",
            vec![Op::Image(ImageOp {
                rect: [0.0, 0.0, 32.0, 32.0],
                uv: [0.0, 0.0, 2.0, 2.0],
                image: placeholder_image(PlaceholderKind::Decoding),
                filter: TextureFilter::Nearest,
                wrap: TextureWrap::Repeat,
                opacity: 1.0,
            })],
        ),
        i(
            "image-clipped-rounded",
            "curve",
            vec![effect(
                LayerOp::new(
                    vec![image_op(
                        [0.0, 0.0, 32.0, 32.0],
                        full,
                        ramp.clone(),
                        TextureFilter::Nearest,
                    )],
                    SceneBlendMode::Normal,
                )
                .with_clip(ClipSpec::Stencil(
                    fill_mesh(
                        &PathBuilder::new()
                            .rounded_rect(3.0, 3.0, 29.0, 29.0, 9.0)
                            .build()
                            .expect("rounded"),
                        FillRule::NonZero,
                    )
                    .expect("mesh"),
                )),
            )],
        )
        .feature(Feature::StencilClip),
    ];
    let p3_image = checker_image(
        8,
        8,
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        ColorSpace::DisplayP3,
    );
    let mut on_srgb = i(
        "image-p3-on-srgb-surface",
        "default",
        vec![image_op(
            [0.0, 0.0, 32.0, 32.0],
            full,
            p3_image.clone(),
            TextureFilter::Nearest,
        )],
    )
    .feature(Feature::DisplayP3);
    on_srgb.requested = ColorSpace::DisplayP3;
    out.push(on_srgb);
    let mut on_p3 = i(
        "image-p3-on-p3-surface",
        "default",
        vec![image_op(
            [0.0, 0.0, 32.0, 32.0],
            full,
            p3_image,
            TextureFilter::Nearest,
        )],
    )
    .feature(Feature::DisplayP3);
    on_p3.requested = ColorSpace::DisplayP3;
    on_p3.display_p3 = true;
    out.push(on_p3);
    out
}

/// P3: P3 색과 surface 조합(gamut 자르기·sRGB on P3·확장 값·선형 target).
pub fn p3_fixtures() -> Vec<Fixture> {
    let pr = |name: &str, surface_p3: bool, ops| {
        let mut fixture = basic(name, Category::P3, Feature::DisplayP3, "default", ops);
        fixture.requested = ColorSpace::DisplayP3;
        fixture.display_p3 = surface_p3;
        fixture
    };
    let swatches = |colors: [OutputColor; 4]| {
        colors
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let x = (i as f32) * 8.0;
                rect(x, 0.0, x + 8.0, 32.0, *c)
            })
            .collect::<Vec<_>>()
    };
    let saturated = [
        p3(1.0, 0.0, 0.0, 1.0),
        p3(0.0, 1.0, 0.0, 1.0),
        p3(0.0, 0.0, 1.0, 1.0),
        p3(1.0, 1.0, 0.0, 1.0),
    ];
    let mut out = vec![
        pr("p3-saturated-on-p3-surface", true, swatches(saturated)),
        pr("p3-saturated-on-srgb-fallback", false, swatches(saturated)),
        pr(
            "p3-srgb-colors-on-p3-surface",
            true,
            swatches([
                srgb(1.0, 0.0, 0.0, 1.0),
                srgb(0.0, 1.0, 0.0, 1.0),
                srgb(0.0, 0.0, 1.0, 1.0),
                srgb(0.5, 0.5, 0.5, 1.0),
            ]),
        ),
        pr("p3-translucent-over-srgb", true, {
            let mut ops = swatches([
                srgb(1.0, 1.0, 1.0, 1.0),
                srgb(0.2, 0.2, 0.2, 1.0),
                srgb(0.9, 0.6, 0.1, 1.0),
                srgb(0.1, 0.4, 0.8, 1.0),
            ]);
            ops.push(rect(0.0, 8.0, 32.0, 24.0, p3(0.1, 0.8, 0.4, 0.5)));
            ops
        }),
        pr(
            "p3-extended-range-clamped",
            true,
            swatches([
                p3(1.3, 0.2, 0.2, 1.0),
                p3(-0.2, 0.9, 0.3, 1.0),
                p3(0.3, 0.3, 1.4, 1.0),
                p3(1.0, 1.0, 1.0, 1.0),
            ]),
        ),
        pr(
            "p3-blend-multiply",
            true,
            vec![
                rect(0.0, 0.0, 32.0, 32.0, p3(0.9, 0.85, 0.2, 1.0)),
                effect(LayerOp::new(
                    vec![rect(4.0, 4.0, 28.0, 28.0, p3(0.2, 0.5, 0.9, 0.8))],
                    SceneBlendMode::Multiply,
                )),
            ],
        )
        .feature(Feature::Blend),
        pr(
            "p3-stroke",
            true,
            vec![path_stroke(
                PathBuilder::new()
                    .move_to(4.0, 26.0)
                    .line_to(16.0, 6.0)
                    .line_to(28.0, 26.0),
                width(4.0).with_join(StrokeJoin::Round),
                p3(0.0, 0.6, 1.0, 1.0),
            )],
        )
        .feature(Feature::StrokeUnion),
    ];
    let mut linear = pr("p3-linear-target", true, swatches(saturated));
    linear.format = crate::render_graph::TextureFormat::Rgba8Unorm;
    out.push(linear);
    out[5].tolerance = "blend";
    out[6].tolerance = "curve";
    out
}

/// inventory 분류별 최소 fixture 수.
pub const MIN_PER_CATEGORY: usize = 8;

/// inventory 전체 최소 fixture 수(XI.11.3).
pub const MIN_TOTAL: usize = 100;

/// 모든 적합성 fixture(XI.11.3).
pub fn inventory() -> Vec<Fixture> {
    let mut all = Vec::new();
    all.extend(frame_fixtures());
    all.extend(shape_fixtures());
    all.extend(path_fixtures());
    all.extend(text_fixtures());
    all.extend(image_fixtures());
    all.extend(p3_fixtures());
    all.extend(gradient_fixtures());
    all.extend(blend_fixtures());
    all.extend(effect_fixtures());
    all
}

/// inventory 검사 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryReport {
    /// 전체 fixture 수.
    pub total: usize,
    /// 분류별 수.
    pub per_category: Vec<(Category, usize)>,
    /// 기능별 수.
    pub per_feature: Vec<(Feature, usize)>,
    /// 문제 목록(비면 통과).
    pub problems: Vec<String>,
}

/// 모든 기능 표시.
pub const ALL_FEATURES: [Feature; 14] = [
    Feature::SolidFill,
    Feature::StrokeUnion,
    Feature::Image,
    Feature::Glyph,
    Feature::DisplayP3,
    Feature::Blend,
    Feature::Gradient,
    Feature::Opacity,
    Feature::ScissorClip,
    Feature::StencilClip,
    Feature::MaskClip,
    Feature::Mask,
    Feature::Shadow,
    Feature::Blur,
];

/// inventory를 검사합니다: 전체 수·분류별 최소 수, 모든 기능 포함, 이름 유일, fixture 계약, 허용 오차 class 존재.
pub fn check_inventory(fixtures: &[Fixture], tolerances: &ToleranceConfig) -> InventoryReport {
    let mut problems = Vec::new();
    let mut names = std::collections::BTreeSet::new();
    for fixture in fixtures {
        if !names.insert(fixture.name.as_str()) {
            problems.push(format!("이름 중복: {}", fixture.name));
        }
        if fixture.name.is_empty()
            || !fixture
                .name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            problems.push(format!("이름은 소문자·숫자·'-'만: {}", fixture.name));
        }
        if let Err(error) = fixture.validate() {
            problems.push(format!("{}: {error:?}", fixture.name));
        }
        if tolerances.get(fixture.tolerance).is_none() {
            problems.push(format!(
                "{}: 허용 오차 class {} 없음",
                fixture.name, fixture.tolerance
            ));
        }
        if fixture.features.is_empty() {
            problems.push(format!("{}: 기능 표시 없음", fixture.name));
        }
    }
    let per_category: Vec<(Category, usize)> = Category::ALL
        .iter()
        .map(|c| (*c, fixtures.iter().filter(|f| f.category == *c).count()))
        .collect();
    for (category, count) in &per_category {
        if *count < MIN_PER_CATEGORY {
            problems.push(format!(
                "{} fixture {count}개 < {MIN_PER_CATEGORY}",
                category.name()
            ));
        }
    }
    let per_feature: Vec<(Feature, usize)> = ALL_FEATURES
        .iter()
        .map(|f| {
            (
                *f,
                fixtures.iter().filter(|x| x.features.contains(f)).count(),
            )
        })
        .collect();
    for (feature, count) in &per_feature {
        if *count == 0 {
            problems.push(format!("기능 {feature:?} fixture 없음"));
        }
    }
    if fixtures.len() < MIN_TOTAL {
        problems.push(format!("전체 fixture {}개 < {MIN_TOTAL}", fixtures.len()));
    }
    InventoryReport {
        total: fixtures.len(),
        per_category,
        per_feature,
        problems,
    }
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

    #[test]
    fn inventory_check_passes_with_required_categories_and_features() {
        // 검증 기준(XI.11.3): fixture inventory 검사 통과(Frame·Shape·Path·Text·Image·Modifier·clip·P3, 전체 100개 이상).
        let fixtures = inventory();
        let report = check_inventory(&fixtures, &ToleranceConfig::builtin());
        assert!(report.problems.is_empty(), "{:#?}", report.problems);
        eprintln!(
            "inventory: total {} {:?}",
            report.total, report.per_category
        );
        assert!(report.total >= MIN_TOTAL, "{}", report.total);
        for (category, count) in &report.per_category {
            assert!(*count >= MIN_PER_CATEGORY, "{category:?} {count}");
        }
        assert!(report.per_feature.iter().all(|(_, count)| *count > 0));
        // 모든 fixture의 CPU 기준을 만들 수 있습니다.
        for fixture in &fixtures {
            let image =
                render_reference(fixture).unwrap_or_else(|e| panic!("{}: {e:?}", fixture.name));
            assert!(image.is_well_formed());
        }
    }

    #[test]
    fn inventory_check_reports_each_problem_kind() {
        let mut fixtures = vec![frame_fixtures()[0].clone(), frame_fixtures()[0].clone()];
        fixtures[1].tolerance = "missing-class";
        fixtures[1].name = "Bad Name".into();
        fixtures[0].features.clear();
        fixtures[0].width = 0;
        let report = check_inventory(&fixtures, &ToleranceConfig::builtin());
        let text = report.problems.join("\n");
        for needle in [
            "이름은 소문자",
            "허용 오차 class missing-class",
            "기능 표시 없음",
            "InvalidSize",
            "fixture 2개 < 100",
            "text fixture 0개",
        ] {
            assert!(text.contains(needle), "{needle}: {text}");
        }
        let duplicate = check_inventory(
            &[frame_fixtures()[0].clone(), frame_fixtures()[0].clone()],
            &ToleranceConfig::builtin(),
        );
        assert!(duplicate
            .problems
            .iter()
            .any(|p| p.starts_with("이름 중복")));
    }
}

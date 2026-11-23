//! 적합성 fixture 목록 (XI.10.2~XI.10.4, XI.11.3).
//!
//! 기능별 묶음 함수가 fixture를 만들고, 모든 backend harness가 같은 목록을 그립니다. fixture 이름은 artifact 파일 이름이라
//! 목록 전체에서 유일해야 합니다.

use super::fixture::{
    fill_mesh, linear_gradient, p3, solid, srgb, Category, Coverage, Feature, FillRule, Fixture,
    LayerOp, Mesh, Op, PathBuilder, SceneBlendMode,
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

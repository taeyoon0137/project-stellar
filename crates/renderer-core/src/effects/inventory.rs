//! 적합성 fixture 목록 (XI.10.2~XI.10.4, XI.11.3).
//!
//! 기능별 묶음 함수가 fixture를 만들고, 모든 backend harness가 같은 목록을 그립니다. fixture 이름은 artifact 파일 이름이라
//! 목록 전체에서 유일해야 합니다.

use super::fixture::{
    solid, srgb, Category, Coverage, Feature, Fixture, LayerOp, Mesh, Op, SceneBlendMode,
};
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

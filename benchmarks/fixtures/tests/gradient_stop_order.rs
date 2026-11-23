//! gradient stop tie 순서: core-types 시각 순서(III.4.3) → renderer gradient(XI.10.3) 연결 검사.
//!
//! renderer는 stop을 다시 정렬하지 않고 `Gradient::visual_order`가 준 순서를 씁니다(ADR-0102 결정 4). 이 test는 같은
//! position stop을 저장 순서를 바꿔 넣어도 렌더 결과(hard edge 양쪽 색)가 안정 ID 순서로 고정되는지 확인합니다.
//! renderer-core는 core-types를 직접 의존하지 않으므로(crate graph) 두 crate를 함께 쓰는 이 crate에 둡니다.

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::gradient::{Gradient, GradientStop};
use stellar_core_types::ids::StableId;
use stellar_renderer_core::backend::{BackendKind, DeviceCapabilities};
use stellar_renderer_core::color::to_working;
use stellar_renderer_core::effects::fixture::{linear_gradient, Category, Fixture, Mesh};
use stellar_renderer_core::effects::gradient::{self as render_gradient, GradientRamp};
use stellar_renderer_core::effects::raster::render_reference;
use stellar_renderer_core::output_color::{OutputColor, OutputColorPlan};

fn color(r: f32, g: f32, b: f32) -> Color {
    Color::new(ColorSpace::Srgb, r, g, b, 1.0).expect("color")
}

fn output(color: Color) -> OutputColor {
    OutputColor {
        space: color.space(),
        rgb: [color.r(), color.g(), color.b()],
        alpha: color.alpha(),
    }
}

/// core-types 시각 순서 그대로 renderer stop으로 옮깁니다.
fn render_stops(gradient: &Gradient) -> Vec<render_gradient::GradientStop> {
    gradient
        .visual_order()
        .into_iter()
        .map(|stop| render_gradient::GradientStop {
            position: stop.position(),
            color: output(stop.color()),
        })
        .collect()
}

fn plan() -> OutputColorPlan {
    OutputColorPlan::negotiate(
        ColorSpace::Srgb,
        &DeviceCapabilities {
            backend: BackendKind::Mock,
            max_texture_dimension: 16_384,
            display_p3_surface: false,
            timestamp_query: false,
            msaa_x4: false,
            stencil_bits: 8,
        },
    )
}

#[test]
fn equal_position_stops_render_in_stable_id_order_regardless_of_storage_order() {
    let first = color(0.9, 0.1, 0.1);
    let low_id = color(0.1, 0.2, 0.9);
    let high_id = color(0.1, 0.7, 0.3);
    let last = color(0.95, 0.85, 0.2);
    let stop = |id: u64, position: f32, color: Color| {
        GradientStop::new(StableId::new_v7(id, 0, id), position, color).expect("stop")
    };
    let stored_a = Gradient::new(vec![
        stop(1, 0.0, first),
        stop(5, 0.5, low_id),
        stop(9, 0.5, high_id),
        stop(3, 1.0, last),
    ]);
    // 저장 순서만 바꾼 같은 gradient.
    let stored_b = Gradient::new(vec![
        stop(9, 0.5, high_id),
        stop(3, 1.0, last),
        stop(5, 0.5, low_id),
        stop(1, 0.0, first),
    ]);
    let plan = plan();
    for gradient in [&stored_a, &stored_b] {
        let stops = render_stops(gradient);
        let ramp = GradientRamp::resolve(&plan, &stops).expect("visual order");
        // t = 0.5 직전은 낮은 ID stop, t = 0.5부터는 높은 ID stop(뒤 stop이 이김).
        let before = ramp.evaluate(0.499);
        let at = ramp.evaluate(0.5);
        let low = to_working(&plan, output(low_id));
        let high = to_working(&plan, output(high_id));
        assert!((before.rgba[2] - low.rgba[2]).abs() < 0.01, "{before:?}");
        assert_eq!(at, high);
    }
    // 렌더 결과도 저장 순서와 무관하게 같습니다.
    let image = |gradient: &Gradient| {
        let fixture = Fixture::new("tie", Category::Frame, 32, 4).with(linear_gradient(
            Mesh::rect(0.0, 0.0, 32.0, 4.0),
            [0.0, 0.0],
            [32.0, 0.0],
            render_stops(gradient),
        ));
        render_reference(&fixture).expect("reference")
    };
    let a = image(&stored_a);
    assert_eq!(a, image(&stored_b));
    // x = 15(t < 0.5)는 파랑 쪽, x = 16(t > 0.5)은 초록 쪽입니다.
    assert!(a.pixel(15, 0)[2] > a.pixel(15, 0)[1]);
    assert!(a.pixel(16, 0)[1] > a.pixel(16, 0)[2]);
}

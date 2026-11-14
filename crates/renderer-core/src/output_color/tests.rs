//! 출력 색공간 협상·gamut 경고 테스트 (XI.10.5).

use super::*;
use crate::backend::mock::MockBackend;
use crate::backend::tests::device_config;
use crate::backend::{CheckedBackend, RendererBackend};
use crate::test_scene::{opaque, rect, snapshot};

/// mock backend를 `display_p3` capability로 초기화해 capability를 얻습니다.
fn mock_caps(display_p3: bool) -> DeviceCapabilities {
    let mut mock = MockBackend::new();
    mock.caps.display_p3_surface = display_p3;
    let mut backend = CheckedBackend::new(mock);
    backend.init(&device_config(32, 32)).unwrap()
}

fn p3(r: f32, g: f32, b: f32) -> OutputColor {
    OutputColor {
        space: ColorSpace::DisplayP3,
        rgb: [r, g, b],
        alpha: 1.0,
    }
}

fn srgb(r: f32, g: f32, b: f32) -> OutputColor {
    OutputColor {
        space: ColorSpace::Srgb,
        rgb: [r, g, b],
        alpha: 1.0,
    }
}

#[track_caller]
fn assert_close(actual: [f32; 3], expected: [f32; 3], eps: f32) {
    for (a, e) in actual.iter().zip(expected.iter()) {
        assert!((a - e).abs() <= eps, "{actual:?} vs {expected:?}");
    }
}

#[test]
fn mock_capability_selects_surface_color_space() {
    // 검증 기준(XI.10.5): mock capability fixture.
    let supported = mock_caps(true);
    let unsupported = mock_caps(false);
    assert!(supported.display_p3_surface && !unsupported.display_p3_surface);

    let plan = OutputColorPlan::negotiate(ColorSpace::DisplayP3, &supported);
    assert_eq!(plan.surface(), ColorSpace::DisplayP3);
    assert!(!plan.is_fallback());
    assert!(plan.negotiation_warnings().is_empty());

    let fallback = OutputColorPlan::negotiate(ColorSpace::DisplayP3, &unsupported);
    assert_eq!(fallback.requested(), ColorSpace::DisplayP3);
    assert_eq!(fallback.surface(), ColorSpace::Srgb);
    assert!(fallback.is_fallback());
    assert_eq!(
        fallback.negotiation_warnings(),
        vec![OutputColorWarning::DisplayP3Unsupported]
    );
    assert_eq!(
        fallback.negotiation_warnings()[0].code(),
        "render.color.display_p3_unsupported"
    );

    // sRGB 요청은 capability와 무관하게 sRGB이며 경고가 없습니다.
    for caps in [supported, unsupported] {
        let plan = OutputColorPlan::negotiate(ColorSpace::Srgb, &caps);
        assert_eq!(plan.surface(), ColorSpace::Srgb);
        assert!(plan.negotiation_warnings().is_empty());
    }
    // surface 설정에 결정된 색공간을 넣습니다.
    let config = fallback.surface_config(SurfaceConfig {
        color_space: ColorSpace::DisplayP3,
        ..device_config(8, 8).surface
    });
    assert_eq!(config.color_space, ColorSpace::Srgb);
}

#[test]
fn p3_on_srgb_surface_converts_and_flags_out_of_gamut() {
    let plan = OutputColorPlan::negotiate(ColorSpace::DisplayP3, &mock_caps(false));
    // P3 순빨강은 sRGB 밖: 선형 (1.2249, -0.0421, -0.0196) → 잘림 + 경고.
    let red = plan.convert(p3(1.0, 0.0, 0.0));
    assert_eq!(red.gamut, GamutStatus::Clipped);
    assert_close(red.linear, [1.0, 0.0, 0.0], 1e-6);
    assert_close(red.encoded, [1.0, 0.0, 0.0], 1e-6);
    // 흰색·회색·검정은 두 공간에서 같은 색(경계 반올림을 경고하지 않음).
    for v in [0.0f32, 0.5, 1.0] {
        let gray = plan.convert(p3(v, v, v));
        assert_eq!(gray.gamut, GamutStatus::InGamut, "{v}");
        assert_close(gray.encoded, [v, v, v], 1e-4);
    }
    // sRGB 안에 들어가는 P3 색(채도 낮음)은 경고 없이 변환합니다.
    let muted = plan.convert(p3(0.6, 0.5, 0.4));
    assert_eq!(muted.gamut, GamutStatus::InGamut);
    // 배정밀도 reference(같은 곡선·행렬): P3 (0.6, 0.5, 0.4) → sRGB (0.6197, 0.4952, 0.3854).
    assert_close(muted.encoded, [0.6197, 0.4952, 0.3854], 1e-3);
    // 알파는 그대로.
    let translucent = plan.convert(OutputColor {
        alpha: 0.25,
        ..p3(0.2, 0.2, 0.2)
    });
    assert_eq!(translucent.alpha, 0.25);
}

#[test]
fn srgb_on_p3_surface_is_always_in_gamut() {
    let plan = OutputColorPlan::negotiate(ColorSpace::DisplayP3, &mock_caps(true));
    // sRGB 순빨강 → P3 (0.9175, 0.2003, 0.1386)(VIII.4.4 공개 근사값과 같은 상수).
    let red = plan.convert(srgb(1.0, 0.0, 0.0));
    assert_eq!(red.gamut, GamutStatus::InGamut);
    assert_close(red.encoded, [0.9175, 0.2003, 0.1386], 1e-3);
    let mut state = 0x000a_0005u32;
    for _ in 0..500 {
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 8) as f32 / (1u32 << 24) as f32
        };
        let color = srgb(next(), next(), next());
        assert_eq!(plan.convert(color).gamut, GamutStatus::InGamut, "{color:?}");
    }
    // 같은 색공간은 그대로(transfer round-trip 오차 이내).
    let same = plan.convert(p3(0.3, 0.6, 0.9));
    assert_eq!(same.gamut, GamutStatus::InGamut);
    assert_close(same.encoded, [0.3, 0.6, 0.9], 1e-5);
}

#[test]
fn snapshot_scan_reports_clipped_nodes() {
    let scene = snapshot(&[
        // sRGB 색: 경고 없음.
        rect(1, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0)),
        // P3 순초록 fill: sRGB 밖.
        rect(2, [0.0, 0.0, 4.0, 4.0], opaque(0.0, 1.0, 0.0)).in_display_p3(),
        // P3 회색 fill + P3 순파랑 stroke + P3 순빨강 shadow: 색 2개가 밖.
        rect(3, [0.0, 0.0, 4.0, 4.0], opaque(0.5, 0.5, 0.5))
            .with_stroke(opaque(0.0, 0.0, 1.0), 2.0)
            .with_shadow_color(opaque(1.0, 0.0, 0.0))
            .in_display_p3(),
        // P3지만 sRGB 안의 색.
        rect(4, [0.0, 0.0, 4.0, 4.0], opaque(0.6, 0.5, 0.4)).in_display_p3(),
    ]);
    let fallback = OutputColorPlan::negotiate(ColorSpace::DisplayP3, &mock_caps(false));
    let report = fallback.scan(&scene);
    assert_eq!(report.checked, 1 + 1 + 3 + 1);
    assert_eq!(report.clipped_colors, 3);
    assert_eq!(report.clipped_nodes, 2);
    assert_eq!(report.node_ids, vec![[2u8; 16], [3u8; 16]]);
    let warnings = fallback.warnings(&scene);
    assert_eq!(warnings.len(), 2);
    assert_eq!(warnings[0], OutputColorWarning::DisplayP3Unsupported);
    assert_eq!(
        warnings[1],
        OutputColorWarning::GamutClipped {
            colors: 3,
            nodes: 2,
            node_ids: vec![[2u8; 16], [3u8; 16]],
        }
    );
    assert_eq!(warnings[1].code(), "render.color.gamut_clipped");

    // P3 지원 display에서는 같은 scene에 경고가 없습니다.
    let supported = OutputColorPlan::negotiate(ColorSpace::DisplayP3, &mock_caps(true));
    assert!(supported.warnings(&scene).is_empty());
}

#[test]
fn reported_node_ids_are_bounded() {
    let nodes: Vec<_> = (1..=100u8)
        .map(|id| rect(id, [0.0, 0.0, 1.0, 1.0], opaque(0.0, 1.0, 0.0)).in_display_p3())
        .collect();
    let scene = snapshot(&nodes);
    let plan = OutputColorPlan::negotiate(ColorSpace::DisplayP3, &mock_caps(false));
    let report = plan.scan(&scene);
    assert_eq!(report.clipped_nodes, 100);
    assert_eq!(report.node_ids.len(), MAX_REPORTED_NODES);
    assert_eq!(report.node_ids[0], [1u8; 16]);
}

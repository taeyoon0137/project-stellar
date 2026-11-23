//! 캔버스 이동·크기 단위, 스냅 허용 오차, 줌 단계, 핸들 숨김 기준 (XII.10.2, ADR-0104 결정 2).
//!
//! 값은 ADR-0104 결정 2 표가 원본입니다. 이 파일의 테스트가 ADR 표를 읽어 값이 같은지 확인합니다. 줌 상하한과 끌기
//! 시작 거리는 기존 정의(`view_tabs::MIN_ZOOM`·`MAX_ZOOM`, `InteractionConfig::DEFAULT_DRAG_THRESHOLD`)를 그대로 다시
//! 내보내며 따로 값을 두지 않습니다.

use stellar_scene::interaction::InteractionConfig;

/// 화살표 한 번의 이동량(부모 좌표 CSS px).
pub const NUDGE_PX: f32 = 1.0;
/// Shift-화살표 한 번의 이동량(부모 좌표 CSS px).
pub const NUDGE_LARGE_PX: f32 = 10.0;
/// ⌘-화살표 한 번의 크기 변화(CSS px). 오른쪽·아래 변을 옮깁니다.
pub const RESIZE_KEY_PX: f32 = 1.0;
/// ⌘⇧-화살표 한 번의 크기 변화(CSS px).
pub const RESIZE_KEY_LARGE_PX: f32 = 10.0;
/// 끌기로 바뀌는 화면 거리(pt). 상호작용 상태 기계(XII.3.6)의 기본값과 같은 정의입니다.
pub const DRAG_THRESHOLD_PT: f32 = InteractionConfig::DEFAULT_DRAG_THRESHOLD;
/// 스냅 허용 오차(화면 pt). 줌과 관계없이 화면 거리로 같습니다.
pub const SNAP_TOLERANCE_PT: f32 = 5.0;
/// 줌 하한(배율). viewport 경계와 같은 정의입니다.
pub const MIN_ZOOM: f64 = stellar_commands::view_tabs::MIN_ZOOM;
/// 줌 상한(배율).
pub const MAX_ZOOM: f64 = stellar_commands::view_tabs::MAX_ZOOM;
/// 단계 줌(확대·축소 명령)의 배율 목록. 오름차순이며 처음과 끝은 하한·상한입니다.
pub const ZOOM_STEPS: [f64; 14] = [
    0.02, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0,
];
/// 픽셀 그리드를 보이는 최소 배율(800%).
pub const PIXEL_GRID_MIN_ZOOM: f64 = 8.0;
/// 선택 상자 화면 짧은 변이 이보다 작으면 크기·반경 핸들을 숨깁니다(pt).
pub const HIDE_RESIZE_BELOW_PT: f32 = 8.0;
/// 선택 상자 화면 짧은 변이 이보다 작으면 변 핸들을 숨깁니다(pt).
pub const HIDE_EDGES_BELOW_PT: f32 = 24.0;
/// 반경 핸들을 보이는 최소 화면 짧은 변(pt, 단일 선택).
pub const RADIUS_MIN_SIDE_PT: f32 = 40.0;
/// 모서리 크기 핸들 정사각형 반변(pt).
pub const CORNER_HALF_PT: f32 = 4.0;
/// 변 크기 핸들 띠 반폭(pt).
pub const EDGE_HALF_WIDTH_PT: f32 = 4.0;
/// 모서리 바깥 회전 영역 반지름(pt).
pub const ROTATE_RADIUS_PT: f32 = 16.0;
/// 반경 핸들 반지름(pt).
pub const RADIUS_HANDLE_PT: f32 = 5.0;
/// 반경 핸들이 모서리에서 안쪽으로 들어가는 최소 거리(pt).
pub const RADIUS_INSET_PT: f32 = 12.0;

/// 이동 단위: Shift면 큰 단위.
pub const fn nudge(large: bool) -> f32 {
    if large {
        NUDGE_LARGE_PX
    } else {
        NUDGE_PX
    }
}

/// 키보드 크기 조절 단위: Shift면 큰 단위.
pub const fn resize_step(large: bool) -> f32 {
    if large {
        RESIZE_KEY_LARGE_PX
    } else {
        RESIZE_KEY_PX
    }
}

/// `zoom`보다 큰 다음 줌 단계. 상한 이상이면 상한입니다.
pub fn zoom_in_step(zoom: f64) -> f64 {
    ZOOM_STEPS
        .iter()
        .copied()
        .find(|step| *step > zoom * (1.0 + 1e-9))
        .unwrap_or(MAX_ZOOM)
}

/// `zoom`보다 작은 이전 줌 단계. 하한 이하이면 하한입니다.
pub fn zoom_out_step(zoom: f64) -> f64 {
    ZOOM_STEPS
        .iter()
        .rev()
        .copied()
        .find(|step| *step < zoom * (1.0 - 1e-9))
        .unwrap_or(MIN_ZOOM)
}

/// 배율 `zoom`에서 픽셀 그리드를 보이는지.
pub fn pixel_grid_visible(zoom: f64) -> bool {
    zoom >= PIXEL_GRID_MIN_ZOOM
}

/// 화면 pt 허용 오차를 배율 `zoom`의 world 단위로 바꿉니다. 배율이 0 이하·비유한이면 `None`입니다.
pub fn snap_tolerance_world(zoom: f64) -> Option<f64> {
    (zoom.is_finite() && zoom > 0.0).then(|| f64::from(SNAP_TOLERANCE_PT) / zoom)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADR: &str =
        include_str!("../../../../docs/adr/ADR-0104-canvas-tools-units-and-selection-core.md");

    fn row(name: &str) -> &'static str {
        let prefix = format!("| {name} | ");
        ADR.lines()
            .find(|line| line.starts_with(&prefix))
            .unwrap_or_else(|| panic!("ADR-0104 표에 '{name}' 행이 없습니다"))
    }

    fn has(name: &str, value: &str) {
        let line = row(name);
        assert!(
            line.contains(value),
            "ADR-0104 '{name}' 행({line})에 코어 값 '{value}'가 없습니다"
        );
    }

    fn pct(zoom: f64) -> String {
        format!("{}", (zoom * 100.0).round() as i64)
    }

    #[test]
    fn constants_match_the_adr_table() {
        // 검증 기준(XII.10.2): ADR 값과 기록이 일치.
        has("화살표 이동", &format!("{} px", NUDGE_PX));
        has("Shift-화살표 이동", &format!("{} px", NUDGE_LARGE_PX));
        has("키보드 크기 조절 단위", &format!("⌘ {} px", RESIZE_KEY_PX));
        has(
            "키보드 크기 조절 단위",
            &format!("⌘⇧ {} px", RESIZE_KEY_LARGE_PX),
        );
        has("끌기 시작 거리", &format!("{} pt", DRAG_THRESHOLD_PT));
        has("스냅 허용 오차", &format!("{} pt", SNAP_TOLERANCE_PT));
        has("줌 하한", &format!("{}%", pct(MIN_ZOOM)));
        has("줌 상한", &format!("{}%", pct(MAX_ZOOM)));
        let steps: Vec<String> = ZOOM_STEPS.iter().map(|z| pct(*z)).collect();
        has("줌 단계", &format!("| {} (%) |", steps.join(", ")));
        has(
            "픽셀 그리드 표시",
            &format!("{}% 이상", pct(PIXEL_GRID_MIN_ZOOM)),
        );
        has(
            "크기·반경 핸들 숨김",
            &format!("{} pt 미만", HIDE_RESIZE_BELOW_PT),
        );
        has("변 핸들 숨김", &format!("{} pt 미만", HIDE_EDGES_BELOW_PT));
        has("반경 핸들 표시", &format!("{} pt 이상", RADIUS_MIN_SIDE_PT));
        has("모서리 핸들 반변", &format!("{} pt", CORNER_HALF_PT));
        has("변 핸들 반폭", &format!("{} pt", EDGE_HALF_WIDTH_PT));
        has("회전 영역 반지름", &format!("{} pt", ROTATE_RADIUS_PT));
        has("반경 핸들 반지름", &format!("{} pt", RADIUS_HANDLE_PT));
        has("반경 핸들 안쪽 거리", &format!("{} pt", RADIUS_INSET_PT));
        assert!(ADR.contains("## Status\n\nAccepted"), "ADR-0104 Accepted");
    }

    #[test]
    fn zoom_steps_cover_the_viewport_range_in_order() {
        assert_eq!(ZOOM_STEPS[0], MIN_ZOOM);
        assert_eq!(ZOOM_STEPS[ZOOM_STEPS.len() - 1], MAX_ZOOM);
        assert!(ZOOM_STEPS.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(zoom_in_step(1.0), 2.0);
        assert_eq!(zoom_in_step(1.3), 2.0);
        assert_eq!(zoom_out_step(1.0), 0.5);
        assert_eq!(zoom_out_step(0.3), 0.25);
        assert_eq!(zoom_in_step(MAX_ZOOM), MAX_ZOOM);
        assert_eq!(zoom_out_step(MIN_ZOOM), MIN_ZOOM);
        assert!(pixel_grid_visible(8.0) && !pixel_grid_visible(7.99));
    }

    #[test]
    fn snap_tolerance_is_constant_on_screen() {
        // 줌 25%와 400%에서 화면 허용 오차가 같습니다(XII.10.15가 쓰는 변환).
        for zoom in [0.25, 4.0] {
            let world = snap_tolerance_world(zoom).unwrap();
            assert!((world * zoom - f64::from(SNAP_TOLERANCE_PT)).abs() < 1e-9);
        }
        assert_eq!(snap_tolerance_world(0.0), None);
        assert_eq!(nudge(false), 1.0);
        assert_eq!(nudge(true), 10.0);
        assert_eq!(resize_step(true), 10.0);
    }
}

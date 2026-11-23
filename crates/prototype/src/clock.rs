//! 프리뷰 고정 tick 논리 시계 `PreviewClock` (XXX.3.1, ADR-0025 결정 2.7, ADR-0094 결정 6).
//!
//! 프리뷰 평가는 시스템 시각을 읽지 않습니다. 값은 (상태, 논리 tick, 입력 이벤트 로그)의 순수 함수입니다. 논리 시계는
//! 초당 [`TICK_RATE_HZ`] tick으로 고정이며, 셸은 디스플레이 프레임 번호를 [`ticks_for_frame`]으로 tick 번호에 대응시키기만
//! 합니다. 표시 장치의 갱신율이 달라도 같은 tick 열은 같은 결과를 냅니다.

/// 논리 tick 번호.
pub type Tick = u64;

/// 고정 논리 tick rate(초당 tick).
pub const TICK_RATE_HZ: u32 = 120;

/// 프리뷰 논리 시계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PreviewClock {
    tick: Tick,
}

impl PreviewClock {
    /// tick 0에서 시작합니다.
    pub const fn new() -> PreviewClock {
        PreviewClock { tick: 0 }
    }

    /// 지금 tick.
    pub const fn tick(&self) -> Tick {
        self.tick
    }

    /// 한 tick 나아가고 새 tick을 돌려줍니다.
    pub fn advance(&mut self) -> Tick {
        self.tick = self.tick.saturating_add(1);
        self.tick
    }
}

/// tick 수를 논리 시간(ms)으로 바꿉니다. IEEE-754 기본 연산(곱·나눗셈)만 씁니다.
pub fn ticks_to_ms(ticks: Tick) -> f64 {
    ticks as f64 * 1000.0 / TICK_RATE_HZ as f64
}

/// 갱신율 `display_hz`인 디스플레이의 `frame`번째 프레임에 대응하는 tick(내림). 정수 연산만 씁니다.
pub fn ticks_for_frame(frame: u64, display_hz: u32) -> Tick {
    if display_hz == 0 {
        return 0;
    }
    ((frame as u128 * TICK_RATE_HZ as u128) / display_hz as u128) as Tick
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_is_monotonic_and_pure() {
        let mut clock = PreviewClock::new();
        assert_eq!(clock.tick(), 0);
        assert_eq!(clock.advance(), 1);
        assert_eq!(clock.advance(), 2);
        assert_eq!(ticks_to_ms(120), 1000.0);
        assert_eq!(ticks_to_ms(3), 25.0);
    }

    #[test]
    fn frames_map_to_ticks() {
        assert_eq!(ticks_for_frame(60, 60), 120);
        assert_eq!(ticks_for_frame(1, 60), 2);
        assert_eq!(ticks_for_frame(1, 120), 1);
        assert_eq!(ticks_for_frame(1, 144), 0);
        assert_eq!(ticks_for_frame(6, 144), 5);
        assert_eq!(ticks_for_frame(5, 0), 0);
    }
}

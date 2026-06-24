//! 단조 시계 추상화.
//!
//! 성능 측정과 히스토리 순서에는 시스템 wall clock을 사용하지 않고 [`MonotonicClock`]을
//! 사용합니다. 테스트는 [`ManualClock`]으로 결정론적 시간을 주입합니다.

/// 단조(monotonic) 시계. 되돌아가지 않는 nanosecond 카운터를 제공합니다.
pub trait MonotonicClock {
    /// 기준 시점부터의 경과 nanoseconds.
    fn now_nanos(&self) -> u64;
}

/// 테스트용 수동 시계. 같은 입력(시작값·advance)이면 항상 같은 값을 생성합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManualClock {
    nanos: u64,
}

impl ManualClock {
    /// 시작 nanoseconds로 시계를 만듭니다.
    pub fn new(start_nanos: u64) -> Self {
        ManualClock { nanos: start_nanos }
    }

    /// 시간을 delta nanoseconds만큼 진행합니다.
    pub fn advance(&mut self, delta_nanos: u64) {
        self.nanos = self.nanos.saturating_add(delta_nanos);
    }
}

impl MonotonicClock for ManualClock {
    fn now_nanos(&self) -> u64 {
        self.nanos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_is_deterministic() {
        let mut a = ManualClock::new(0);
        let start = a.now_nanos();
        a.advance(100);
        let dur_a = a.now_nanos() - start;

        let mut b = ManualClock::new(0);
        let start_b = b.now_nanos();
        b.advance(100);
        let dur_b = b.now_nanos() - start_b;

        assert_eq!(dur_a, dur_b);
        assert_eq!(dur_a, 100);
    }

    #[test]
    fn advance_is_monotonic() {
        let mut clock = ManualClock::new(5);
        clock.advance(3);
        clock.advance(2);
        assert_eq!(clock.now_nanos(), 10);
    }
}

//! 호출량 상한 판정 (XIX.9.10, ADR-0085 결정 4).
//!
//! 값은 `stellar_agent_protocol::limits`에 있습니다. 이 모듈은 연결 하나의 고정 창 판정만 합니다. 시각은 호출자가
//! 넘기므로 test가 시계를 흉내 낼 수 있습니다.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use stellar_agent_protocol::limits::{MAX_CALLS_PER_WINDOW, RATE_WINDOW};

/// 상한을 세지 않는 method(수명주기·생존 확인).
const EXEMPT: [&str; 2] = ["ping", "initialize"];

/// 최근 [`RATE_WINDOW`] 안의 요청 시각.
#[derive(Debug)]
pub struct RateLimiter {
    window: Duration,
    max: usize,
    recent: VecDeque<Instant>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        RateLimiter::new(RATE_WINDOW, MAX_CALLS_PER_WINDOW)
    }
}

impl RateLimiter {
    /// 창 길이와 창 안 최대 요청 수로 만듭니다.
    pub fn new(window: Duration, max: usize) -> Self {
        RateLimiter {
            window,
            max,
            recent: VecDeque::with_capacity(max + 1),
        }
    }

    /// `now`에 온 `method` 요청을 받을지 정합니다. 넘으면 가장 오래된 요청이 창을 벗어날 때까지의 시간을
    /// 돌려주고 그 요청은 세지 않습니다.
    pub fn admit(&mut self, method: &str, now: Instant) -> Result<(), Duration> {
        if EXEMPT.contains(&method) {
            return Ok(());
        }
        while self
            .recent
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= self.window)
        {
            self.recent.pop_front();
        }
        if self.recent.len() >= self.max {
            let oldest = *self.recent.front().expect("max > 0이면 비지 않음");
            return Err(self.window - now.saturating_duration_since(oldest));
        }
        self.recent.push_back(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_admits_up_to_the_limit_then_busy_until_the_oldest_expires() {
        let start = Instant::now();
        let mut limiter = RateLimiter::new(Duration::from_secs(1), 3);
        for i in 0..3 {
            assert!(limiter
                .admit("tools/call", start + Duration::from_millis(i * 10))
                .is_ok());
        }
        let retry = limiter
            .admit("resources/read", start + Duration::from_millis(500))
            .unwrap_err();
        assert_eq!(retry, Duration::from_millis(500));
        // ping·initialize는 세지 않습니다.
        assert!(limiter
            .admit("ping", start + Duration::from_millis(500))
            .is_ok());
        // 가장 오래된 요청이 창을 벗어나면 다시 받습니다.
        assert!(limiter
            .admit("tools/call", start + Duration::from_millis(1000))
            .is_ok());
        assert!(limiter
            .admit("tools/call", start + Duration::from_millis(1001))
            .is_err());
    }
}

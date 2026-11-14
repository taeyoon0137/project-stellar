//! benchmark 결과의 present queue 깊이·dropped stale frame 지표 (XI.11.5).
//!
//! frame scheduler(XI.5.5)는 present 대기열 깊이를 최대 1로 유지하고 오래된 snapshot을 버립니다. benchmark는 이 계약이
//! 실제 실행에서도 지켜졌는지를 결과 JSON에 남기고 검사해야 합니다(아키텍처 §6.3).
//!
//! - [`PresentRecorder`]는 tick마다 [`FrameScheduler::present_queue_depth`]를 표본으로 모아 깊이 분포를 만듭니다.
//! - [`PresentMetrics`]는 그 분포와 [`SchedulerStats`](offer·submit·encode 전/후 폐기·throttle·skip)를 합친
//!   결과입니다. [`PresentMetrics::assert_queue_depth`]가 "깊이 ≤ [`MAX_PRESENT_QUEUE_DEPTH`]" assertion이며,
//!   [`PresentMetrics::check_accounting`]은 offer 수 = 제출 + 폐기 + 남은 pending인지 확인합니다.
//! - [`PresentMetrics::to_json`]은 key 순서가 고정된 JSON 객체(`"present_queue"`)를 만듭니다. 값은 모두 정수이며
//!   프로젝트 내용은 담지 않습니다. 단계별 timing benchmark(XI.11.4)의 결과 JSON에 이 객체를 넣습니다.

use std::fmt::{self, Write as _};

use crate::frame_scheduler::{FrameScheduler, SchedulerStats, MAX_PRESENT_QUEUE_DEPTH};

/// 깊이 분포 칸 수: 깊이 `0..=MAX`와 "상한 초과" 칸.
pub const DEPTH_BUCKETS: usize = MAX_PRESENT_QUEUE_DEPTH + 2;

/// tick마다 present queue 깊이를 모읍니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PresentRecorder {
    ticks: u64,
    histogram: [u64; DEPTH_BUCKETS],
    max_depth: usize,
}

impl PresentRecorder {
    /// 빈 recorder.
    pub fn new() -> PresentRecorder {
        PresentRecorder::default()
    }

    /// tick 한 번 뒤의 queue 깊이를 기록합니다.
    pub fn record_depth(&mut self, depth: usize) {
        self.ticks += 1;
        let bucket = depth.min(DEPTH_BUCKETS - 1);
        self.histogram[bucket] += 1;
        self.max_depth = self.max_depth.max(depth);
    }

    /// scheduler의 현재 queue 깊이를 기록합니다(`tick` 직후 호출).
    pub fn record(&mut self, scheduler: &FrameScheduler) {
        self.record_depth(scheduler.present_queue_depth());
    }

    /// 기록한 tick 수.
    pub const fn ticks(&self) -> u64 {
        self.ticks
    }

    /// 지표를 만듭니다.
    pub fn finish(&self, scheduler: &FrameScheduler) -> PresentMetrics {
        let stats = scheduler.stats();
        PresentMetrics {
            limit: MAX_PRESENT_QUEUE_DEPTH,
            ticks: self.ticks,
            // scheduler가 직접 본 최대값과 표본 최대값 중 큰 값(어느 쪽도 놓치지 않음).
            max_depth: self.max_depth.max(stats.max_queue_depth),
            depth_histogram: self.histogram,
            stats,
            pending_at_end: u64::from(scheduler.pending_revision().is_some()),
        }
    }
}

/// queue 깊이 assertion 위반.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentMetricsError {
    /// present queue 깊이가 상한을 넘었습니다.
    QueueDepthExceeded {
        /// 관측 최대 깊이.
        observed: usize,
        /// 상한.
        limit: usize,
    },
    /// offer 수가 제출 + 폐기 + 남은 pending과 맞지 않습니다.
    AccountingMismatch {
        /// offer 수.
        offered: u64,
        /// 제출 + 폐기 + pending.
        accounted: u64,
    },
}

impl fmt::Display for PresentMetricsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PresentMetricsError::QueueDepthExceeded { observed, limit } => write!(
                f,
                "present queue 깊이 {observed}가 상한 {limit}을 넘었습니다"
            ),
            PresentMetricsError::AccountingMismatch { offered, accounted } => write!(
                f,
                "frame 집계 불일치: offer {offered}, 제출+폐기+pending {accounted}"
            ),
        }
    }
}

impl std::error::Error for PresentMetricsError {}

/// benchmark 결과의 present queue 지표.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentMetrics {
    /// queue 깊이 상한(= [`MAX_PRESENT_QUEUE_DEPTH`]).
    pub limit: usize,
    /// 기록한 tick 수.
    pub ticks: u64,
    /// 관측 최대 깊이.
    pub max_depth: usize,
    /// 깊이별 tick 수(`[0..=limit]`, 마지막 칸은 상한 초과).
    pub depth_histogram: [u64; DEPTH_BUCKETS],
    /// scheduler 통계.
    pub stats: SchedulerStats,
    /// 끝날 때 남은 pending snapshot 수(0 또는 1).
    pub pending_at_end: u64,
}

impl PresentMetrics {
    /// 버린 오래된 frame 총수(encode 전 + 후).
    pub const fn dropped_stale_frames(&self) -> u64 {
        self.stats.dropped_stale()
    }

    /// queue 깊이 ≤ 상한 assertion.
    pub fn assert_queue_depth(&self) -> Result<(), PresentMetricsError> {
        let overflow = self.depth_histogram[DEPTH_BUCKETS - 1];
        if self.max_depth > self.limit || overflow > 0 {
            // 상한 초과 칸에 표본이 있으면 관측 깊이는 적어도 상한 + 1입니다.
            let observed = if overflow > 0 {
                self.max_depth.max(self.limit + 1)
            } else {
                self.max_depth
            };
            return Err(PresentMetricsError::QueueDepthExceeded {
                observed,
                limit: self.limit,
            });
        }
        Ok(())
    }

    /// offer = 제출 + 폐기 + pending 인지 확인합니다.
    pub fn check_accounting(&self) -> Result<(), PresentMetricsError> {
        let accounted = self.stats.submitted + self.dropped_stale_frames() + self.pending_at_end;
        if accounted != self.stats.offered {
            return Err(PresentMetricsError::AccountingMismatch {
                offered: self.stats.offered,
                accounted,
            });
        }
        Ok(())
    }

    /// (정적 key, 값) 목록. 순서는 고정입니다.
    pub fn fields(&self) -> [(&'static str, u64); 11] {
        [
            ("limit", self.limit as u64),
            ("max_depth", self.max_depth as u64),
            ("ticks", self.ticks),
            ("offered", self.stats.offered),
            ("submitted", self.stats.submitted),
            ("dropped_stale_frames", self.dropped_stale_frames()),
            ("dropped_before_encode", self.stats.dropped_before_encode),
            ("dropped_after_encode", self.stats.dropped_after_encode),
            ("throttled_ticks", self.stats.throttled),
            ("skipped_ticks", self.stats.skipped),
            ("pending_at_end", self.pending_at_end),
        ]
    }

    /// 고정 key 순서 JSON 객체 `{"present_queue":{...,"depth_histogram":[...]}}`.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\"present_queue\":{");
        for (index, (key, value)) in self.fields().iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            // String에 쓰기는 실패하지 않습니다.
            let _ = write!(out, "\"{key}\":{value}");
        }
        out.push_str(",\"depth_histogram\":[");
        for (index, count) in self.depth_histogram.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let _ = write!(out, "{count}");
        }
        out.push_str("]}}");
        out
    }
}

#[cfg(test)]
mod tests;

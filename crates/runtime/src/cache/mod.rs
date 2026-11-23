//! 앱 runtime의 cache 예산: memory pressure 증분 제거와 썸네일 cache (XVI.5.3·XVI.5.4, ADR-0108 결정 6).
//!
//! 공통 budget manager·priority·제거 순서는 renderer-core([`stellar_renderer_core::cache`])에 있습니다. 이 모듈은 앱
//! 쪽 정책을 정합니다.
//!
//! - **압박 단계 → 목표**([`PressureLevel`], [`CacheLimits`]): Normal이면 지울 수 있는 사용량을 `limit` 이하로,
//!   Warning이면 `limit`의 절반 이하로, Critical이면 0으로 줄입니다. Document/CurrentScene 사용량은 목표에 더해 둡니다
//!   (지우지 않으므로). 단계는 진단 crate의 `MemoryPressure`(XVI.5.1, OS 압박 단계)와 같은 뜻이며, 앱 연결 코드가 그
//!   값을 그대로 옮겨 넘깁니다(runtime은 crate graph상 diagnostics에 직접 의존하지 않음).
//! - **UI thread 2 ms chunk**([`PressureEvictor`]): 제거를 한 번에 끝내지 않고 UI thread tick마다 [`run_chunk`]로 최대
//!   [`UI_CHUNK_BUDGET`]만 씁니다. [`ChunkDeadline`]은 다음 단위를 지우기 전에 "지금까지 쓴 시간 + 단위 하나의 최대
//!   비용"이 예산 안인지 확인하므로, 단위 하나가 [`UNIT_RESERVE`] 이하인 한 chunk는 2 ms를 넘지 않습니다(아키텍처 6.3
//!   UI thread 단일 점유 2 ms).
//! - **해제는 pass 끝에 UI thread 밖에서**: 지운 resource의 메모리 반환은 chunk 안에서 하지 않습니다. pass 동안 모아
//!   두었다가 pass가 끝나는 chunk의 [`ChunkReport::released`]로 한 번에 내주고, 호출자가 worker thread로 보내 버립니다.
//!   chunk 안에서 해제하거나 pass 도중 다른 thread가 해제하면 allocator 경합으로 단위 하나가 수 ms 걸리는 경우가 run마다
//!   생겼습니다(보고서 XVI.5.4 측정). 사용량은 지운 즉시 줄어든 것으로 봅니다.
//!
//! [`run_chunk`]: PressureEvictor::run_chunk

/// 문서 썸네일 cache (XVI.5.3).
pub mod thumbnail;

#[cfg(test)]
mod tests;

use std::time::{Duration, Instant};

use stellar_renderer_core::cache::{
    CacheBudgetManager, CachePriority, ChunkOutcome, EvictionDeadline, EvictionPass,
    ReleasedResources,
};

/// UI thread chunk 하나의 시간 예산(아키텍처 6.3 단일 점유 2 ms).
pub const UI_CHUNK_BUDGET: Duration = Duration::from_millis(2);

/// 단위(entry·page) 하나를 지우는 최대 비용으로 미리 잡아 두는 시간. 실제 단위 비용은 이보다 훨씬 작습니다(보고서
/// XVI.5.4 측정). 관측한 단위 비용이 더 크면 그 값을 씁니다.
pub const UNIT_RESERVE: Duration = Duration::from_micros(100);

/// OS memory pressure 단계(진단 `MemoryPressure`와 같은 뜻).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PressureLevel {
    /// 압박 없음.
    Normal,
    /// 경고.
    Warning,
    /// 심각.
    Critical,
}

/// 지울 수 있는 cache 사용량 상한.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheLimits {
    /// 압박이 없을 때 지울 수 있는 cache(Document/CurrentScene 제외) 사용량 상한(byte).
    pub evictable_bytes: u64,
}

impl CacheLimits {
    /// 압박 단계별로 남겨 둘 지울 수 있는 사용량(byte).
    pub const fn allowance(&self, level: PressureLevel) -> u64 {
        match level {
            PressureLevel::Normal => self.evictable_bytes,
            PressureLevel::Warning => self.evictable_bytes / 2,
            PressureLevel::Critical => 0,
        }
    }

    /// 현재 manager 사용량 기준 전체 목표(byte): Document/CurrentScene 사용량 + 단계별 허용량.
    pub fn target(&self, level: PressureLevel, manager: &CacheBudgetManager) -> u64 {
        manager
            .usage()
            .get(CachePriority::DocumentScene)
            .saturating_add(self.allowance(level))
    }
}

/// 단조 증가 시계. 시험은 가짜 시계로 단위 비용을 흉내 냅니다.
pub trait Clock {
    /// 임의 기준점부터 지난 시간.
    fn now(&self) -> Duration;
}

/// 실제 단조 시계.
#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    /// 지금을 기준점으로 하는 시계.
    pub fn new() -> SystemClock {
        SystemClock {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> SystemClock {
        SystemClock::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// chunk 하나의 시간 예산(XVI.5.4).
pub struct ChunkDeadline<'a> {
    clock: &'a dyn Clock,
    budget: Duration,
    started: Duration,
    last: Option<Duration>,
    unit_cost: Duration,
}

impl<'a> ChunkDeadline<'a> {
    /// 지금부터 `budget`을 쓰는 deadline. 단위 비용 추정은 [`UNIT_RESERVE`]에서 시작합니다.
    pub fn new(clock: &'a dyn Clock, budget: Duration) -> ChunkDeadline<'a> {
        ChunkDeadline {
            clock,
            budget,
            started: clock.now(),
            last: None,
            unit_cost: UNIT_RESERVE,
        }
    }

    /// chunk 시작부터 지난 시간.
    pub fn elapsed(&self) -> Duration {
        self.clock.now().saturating_sub(self.started)
    }

    /// 지금까지 관측한 단위 비용 최대값(최소 [`UNIT_RESERVE`]).
    pub fn unit_cost(&self) -> Duration {
        self.unit_cost
    }
}

impl EvictionDeadline for ChunkDeadline<'_> {
    fn allows_another(&mut self) -> bool {
        let now = self.clock.now();
        if let Some(last) = self.last {
            self.unit_cost = self.unit_cost.max(now.saturating_sub(last));
        }
        self.last = Some(now);
        now.saturating_sub(self.started) + self.unit_cost <= self.budget
    }
}

/// chunk 하나의 보고.
#[derive(Debug)]
pub struct ChunkReport {
    /// manager 결과(지운 단위·byte·완료 여부).
    pub outcome: ChunkOutcome,
    /// chunk가 쓴 시간.
    pub elapsed: Duration,
    /// chunk 뒤 전체 사용량(byte).
    pub usage_after: u64,
    /// chunk 안에서 관측한 단위 하나의 최대 비용(최소 [`UNIT_RESERVE`]).
    pub unit_cost: Duration,
    /// pass가 끝나는 chunk에서만 채워지는, 이 pass가 지운 resource. 호출자는 UI thread가 아닌 thread(worker pool)로 보내
    /// 버립니다. UI thread에서 버리면 큰 buffer 해제가 UI thread를 붙잡습니다.
    pub released: ReleasedResources,
}

/// memory pressure event를 받아 UI thread chunk 단위로 cache를 줄입니다(XVI.5.4).
#[derive(Debug)]
pub struct PressureEvictor {
    limits: CacheLimits,
    level: PressureLevel,
    pass: Option<EvictionPass>,
    /// 진행 중인 pass에서 지운 resource. pass가 끝나는 chunk에서 한 번에 내줍니다.
    held: ReleasedResources,
}

impl PressureEvictor {
    /// 압박 없는 상태.
    pub fn new(limits: CacheLimits) -> PressureEvictor {
        PressureEvictor {
            limits,
            level: PressureLevel::Normal,
            pass: None,
            held: ReleasedResources::default(),
        }
    }

    /// 현재 압박 단계.
    pub const fn level(&self) -> PressureLevel {
        self.level
    }

    /// 진행 중인 제거가 있으면 `true`(다음 UI tick에 [`PressureEvictor::run_chunk`]를 불러야 함).
    pub fn is_pending(&self) -> bool {
        self.pass.is_some()
    }

    /// 진행 중인 제거의 목표(byte).
    pub fn pending_target(&self) -> Option<u64> {
        self.pass.as_ref().map(EvictionPass::target)
    }

    /// memory pressure event. 목표를 다시 정하고, 사용량이 목표를 넘으면 제거를 시작합니다. 이 호출은 아무것도 지우지
    /// 않습니다(제거는 [`PressureEvictor::run_chunk`]가 chunk로 합니다).
    pub fn on_pressure(&mut self, level: PressureLevel, manager: &CacheBudgetManager) {
        self.level = level;
        self.schedule(manager);
    }

    /// cache가 커진 뒤(삽입) 현재 단계의 목표를 넘으면 제거를 시작합니다.
    pub fn on_usage_changed(&mut self, manager: &CacheBudgetManager) {
        self.schedule(manager);
    }

    fn schedule(&mut self, manager: &CacheBudgetManager) {
        let target = self.limits.target(self.level, manager);
        self.pass = (manager.usage().total() > target).then(|| EvictionPass::new(target));
    }

    /// UI thread chunk 하나: `budget` 안에서 낮은 priority부터 지웁니다. 진행 중인 제거가 없으면 `None`입니다.
    pub fn run_chunk(
        &mut self,
        manager: &mut CacheBudgetManager,
        clock: &dyn Clock,
        budget: Duration,
    ) -> Option<ChunkReport> {
        let Some(pass) = self.pass.as_mut() else {
            // 목표가 바뀌어 pass가 사라졌어도 이미 지운 resource는 내줍니다.
            if self.held.is_empty() {
                return None;
            }
            return Some(ChunkReport {
                outcome: ChunkOutcome {
                    finished: true,
                    ..ChunkOutcome::default()
                },
                elapsed: Duration::ZERO,
                usage_after: manager.usage().total(),
                unit_cost: Duration::ZERO,
                released: std::mem::take(&mut self.held),
            });
        };
        let mut deadline = ChunkDeadline::new(clock, budget);
        let outcome = manager.evict_chunk(pass, &mut deadline);
        let elapsed = deadline.elapsed();
        let unit_cost = deadline.unit_cost();
        self.held.append(manager.take_released());
        let released = if outcome.finished {
            self.pass = None;
            std::mem::take(&mut self.held)
        } else {
            ReleasedResources::default()
        };
        Some(ChunkReport {
            outcome,
            elapsed,
            usage_after: manager.usage().total(),
            unit_cost,
            released,
        })
    }
}

//! memory pressure 증분 제거·썸네일 cache 시험 (XVI.5.3·XVI.5.4).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use stellar_core_types::ids::StableId;
use stellar_renderer_core::cache::{
    BudgetedCache, CacheBudgetManager, CacheKind, CachePriority, CacheUsage, ReleasedResources,
    ReportedUsage,
};

use super::thumbnail::{Thumbnail, ThumbnailCache, ThumbnailKey};
use super::*;

/// 가짜 시계: 시험 cache가 단위 하나를 지울 때마다 정해진 비용만큼 앞으로 갑니다.
#[derive(Clone, Default)]
struct FakeClock(Rc<Cell<Duration>>);

impl FakeClock {
    fn advance(&self, by: Duration) {
        self.0.set(self.0.get() + by);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        self.0.get()
    }
}

/// 같은 크기 entry를 들고, 지울 때마다 가짜 시계를 단위 비용(결정적 난수 5~100 µs)만큼 움직이는 cache.
struct CostlyCache {
    kind: CacheKind,
    priority: CachePriority,
    entries: usize,
    entry_bytes: u64,
    clock: FakeClock,
    seed: u64,
}

impl BudgetedCache for CostlyCache {
    fn kind(&self) -> CacheKind {
        self.kind
    }

    fn usage(&self) -> CacheUsage {
        CacheUsage::of(self.priority, self.entries as u64 * self.entry_bytes)
    }

    fn evict_one(&mut self, priority: CachePriority) -> u64 {
        if priority != self.priority || self.entries == 0 {
            return 0;
        }
        self.seed = self
            .seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let micros = 5 + (self.seed >> 33) % 96;
        self.clock.advance(Duration::from_micros(micros));
        self.entries -= 1;
        self.entry_bytes
    }
}

const ENTRY: u64 = 1_024;
const PER_CACHE: usize = 2_000;
const DOCUMENT: u64 = 1 << 20;

fn simulated_manager(clock: &FakeClock) -> CacheBudgetManager {
    let mut manager = CacheBudgetManager::new();
    manager.register(Box::new(ReportedUsage::new(
        CacheKind::DocumentModel,
        CachePriority::DocumentScene,
        DOCUMENT,
    )));
    for (index, (kind, priority)) in [
        (CacheKind::Tessellation, CachePriority::VisibleGlyphGeometry),
        (CacheKind::ImageMipmap, CachePriority::NearViewport),
        (CacheKind::Thumbnail, CachePriority::BackgroundThumbnail),
    ]
    .into_iter()
    .enumerate()
    {
        manager.register(Box::new(CostlyCache {
            kind,
            priority,
            entries: PER_CACHE,
            entry_bytes: ENTRY,
            clock: clock.clone(),
            seed: index as u64 + 1,
        }));
    }
    manager
}

/// 제거가 끝날 때까지 chunk를 돌리고 (chunk 수, 지운 priority 순서)를 돌려줍니다. chunk마다 2 ms 이하를 확인합니다.
fn drain(
    evictor: &mut PressureEvictor,
    manager: &mut CacheBudgetManager,
    clock: &FakeClock,
) -> (usize, Vec<CachePriority>) {
    let mut chunks = 0;
    let mut order = Vec::new();
    while let Some(report) = evictor.run_chunk(manager, clock, UI_CHUNK_BUDGET) {
        chunks += 1;
        assert!(
            report.elapsed <= UI_CHUNK_BUDGET,
            "chunk {chunks}: {:?} > 2 ms",
            report.elapsed
        );
        order.extend(report.outcome.evicted.iter().map(|e| e.priority));
        // UI thread가 다른 일을 하는 동안 시간이 흐릅니다(다음 tick).
        clock.advance(Duration::from_millis(8));
        assert!(chunks < 10_000, "끝나지 않는 제거");
    }
    (chunks, order)
}

#[test]
fn simulated_pressure_evicts_incrementally_within_ui_chunk_budget() {
    // 검증 기준(XVI.5.4): simulated pressure에서 latency(chunk ≤ 2 ms)·usage(목표 도달) 기준 통과.
    let clock = FakeClock::default();
    let mut manager = simulated_manager(&clock);
    let evictable = 3 * PER_CACHE as u64 * ENTRY;
    let limits = CacheLimits {
        evictable_bytes: evictable * 3 / 4,
    };
    let mut evictor = PressureEvictor::new(limits);

    // 1. 압박 없음: 상한(3/4)을 넘은 만큼만 낮은 priority(썸네일)부터 줄입니다.
    evictor.on_usage_changed(&manager);
    assert!(evictor.is_pending());
    let (chunks, order) = drain(&mut evictor, &mut manager, &clock);
    assert!(chunks >= 2, "한 번에 끝내지 않음: {chunks}");
    assert!(order
        .iter()
        .all(|p| *p == CachePriority::BackgroundThumbnail));
    assert_eq!(
        manager.usage().total(),
        DOCUMENT + limits.allowance(PressureLevel::Normal)
    );

    // 2. Warning: 절반까지. 썸네일을 마저 지운 뒤 화면 밖(NearViewport) resource를 지웁니다.
    evictor.on_pressure(PressureLevel::Warning, &manager);
    let (_, order) = drain(&mut evictor, &mut manager, &clock);
    assert!(order.windows(2).all(|w| w[0] <= w[1]), "낮은 priority부터");
    assert!(order.contains(&CachePriority::NearViewport));
    assert!(!order.contains(&CachePriority::VisibleGlyphGeometry));
    assert!(manager.usage().total() <= DOCUMENT + limits.allowance(PressureLevel::Warning));

    // 3. Critical: 지울 수 있는 것을 모두 지우고 Document/CurrentScene만 남깁니다.
    evictor.on_pressure(PressureLevel::Critical, &manager);
    let started = clock.now();
    let (chunks, order) = drain(&mut evictor, &mut manager, &clock);
    assert!(order.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(
        manager.usage().total(),
        DOCUMENT,
        "Document/CurrentScene 비축출"
    );
    assert!(!evictor.is_pending());
    // chunk 하나는 예산의 상당 부분을 씁니다(단위 비용 평균 약 52 µs, 예산 2 ms - 예비 100 µs).
    let units = order.len();
    assert!(
        units / chunks >= 20,
        "chunk당 단위 {} (chunk {chunks}, 단위 {units})",
        units / chunks
    );
    assert!(clock.now() > started);
}

#[test]
fn pressure_below_target_does_not_schedule_work() {
    let clock = FakeClock::default();
    let manager = simulated_manager(&clock);
    let mut evictor = PressureEvictor::new(CacheLimits {
        evictable_bytes: u64::MAX,
    });
    evictor.on_pressure(PressureLevel::Normal, &manager);
    assert!(!evictor.is_pending());
    let mut manager = manager;
    assert!(evictor
        .run_chunk(&mut manager, &clock, UI_CHUNK_BUDGET)
        .is_none());
    evictor.on_pressure(PressureLevel::Warning, &manager);
    assert_eq!(evictor.level(), PressureLevel::Warning);
    assert_eq!(evictor.pending_target(), None, "절반 상한도 넘지 않음");
    evictor.on_pressure(PressureLevel::Critical, &manager);
    assert_eq!(evictor.pending_target(), Some(DOCUMENT));
}

#[test]
fn slow_units_raise_the_unit_cost_estimate() {
    // 관측한 단위 비용이 예비값보다 크면 그 값으로 다음 단위를 판단합니다.
    let clock = FakeClock::default();
    let mut deadline = ChunkDeadline::new(&clock, UI_CHUNK_BUDGET);
    assert!(stellar_renderer_core::cache::EvictionDeadline::allows_another(&mut deadline));
    clock.advance(Duration::from_micros(700));
    assert!(stellar_renderer_core::cache::EvictionDeadline::allows_another(&mut deadline));
    assert_eq!(deadline.unit_cost(), Duration::from_micros(700));
    clock.advance(Duration::from_micros(700));
    // 1.4 ms + 0.7 ms > 2 ms이므로 더 지우지 않습니다.
    assert!(!stellar_renderer_core::cache::EvictionDeadline::allows_another(&mut deadline));
}

fn thumbnail(document: u64, side: u32) -> Thumbnail {
    let key = ThumbnailKey {
        document: StableId::new_v7(document, 0, document),
        width: side,
        height: side,
    };
    Thumbnail::new(key, vec![0x40; (side * side * 4) as usize]).unwrap()
}

#[test]
fn released_resources_are_handed_over_once_when_the_pass_ends() {
    // 지운 resource는 pass 도중에는 내주지 않고(해제 경합 방지), pass가 끝나거나 목표가 바뀌어 사라질 때 한 번 내줍니다.
    let clock = FakeClock::default();
    let mut manager = CacheBudgetManager::new();
    let mut thumbnails = ThumbnailCache::new();
    for document in 1..=10 {
        thumbnails.insert(thumbnail(document, 4));
    }
    manager.register(Box::new(thumbnails));
    manager.register(Box::new(CostlyCache {
        kind: CacheKind::ImageMipmap,
        priority: CachePriority::NearViewport,
        entries: 200,
        entry_bytes: ENTRY,
        clock: clock.clone(),
        seed: 7,
    }));
    let mut evictor = PressureEvictor::new(CacheLimits {
        evictable_bytes: 1 << 40,
    });
    evictor.on_pressure(PressureLevel::Critical, &manager);
    let first = evictor
        .run_chunk(&mut manager, &clock, UI_CHUNK_BUDGET)
        .expect("chunk");
    assert!(!first.outcome.finished, "썸네일 10개 + mipmap 일부만");
    assert!(first.released.is_empty(), "pass 도중에는 내주지 않음");
    // 압박이 풀려 pass가 사라져도 이미 지운 썸네일은 다음 chunk 보고로 한 번 나옵니다.
    evictor.on_pressure(PressureLevel::Normal, &manager);
    assert!(!evictor.is_pending());
    let handed = evictor
        .run_chunk(&mut manager, &clock, UI_CHUNK_BUDGET)
        .expect("남은 해제 묶음");
    assert!(handed.outcome.finished && handed.outcome.evicted.is_empty());
    assert_eq!(handed.released.len(), 1);
    assert!(evictor
        .run_chunk(&mut manager, &clock, UI_CHUNK_BUDGET)
        .is_none());
}

#[test]
fn pressure_evictor_drives_thumbnail_eviction() {
    let mut cache = ThumbnailCache::new();
    for document in 1..=3 {
        cache.insert(thumbnail(document, 8));
    }
    assert!(
        Thumbnail::new(thumbnail(9, 2).key(), vec![0; 3]).is_none(),
        "크기 검사"
    );
    let first = thumbnail(1, 8).key();
    assert!(cache.get(&first).is_some());
    let mut manager = CacheBudgetManager::new();
    let id = manager.register(Box::new(cache));
    assert_eq!(
        manager.usage(),
        CacheUsage::of(CachePriority::BackgroundThumbnail, 3 * 8 * 8 * 4)
    );
    let mut evictor = PressureEvictor::new(CacheLimits {
        evictable_bytes: 8 * 8 * 4,
    });
    evictor.on_usage_changed(&manager);
    let report = evictor
        .run_chunk(&mut manager, &SystemClock::new(), Duration::from_secs(1))
        .expect("제거");
    assert_eq!(report.outcome.evicted.len(), 2);
    assert!(report.outcome.finished);
    // 지운 썸네일 둘은 한 묶음으로 나와 호출자가 UI thread 밖에서 버립니다.
    assert_eq!(report.released.len(), 1);
    assert!(manager.take_released().is_empty(), "한 번만 나옴");
    let cache = manager.get_mut::<ThumbnailCache>(id).unwrap();
    assert_eq!(cache.len(), 1);
    assert!(cache.get(&first).is_some(), "최근에 본 썸네일이 남음");
}

#[test]
#[ignore = "release 측정: cargo test -p stellar-runtime --release cache::tests::measure -- --ignored --nocapture"]
fn measure_thumbnail_pressure_chunks_on_real_clock() {
    // 실제 시계로 썸네일 200,000개(각 1 KiB)를 Critical에서 모두 지울 때 chunk 시간과 단위 비용을 잽니다.
    let mut cache = ThumbnailCache::new();
    for document in 0..200_000u64 {
        cache.insert(thumbnail(document + 1, 16));
    }
    let mut manager = CacheBudgetManager::new();
    manager.register(Box::new(cache));
    let mut evictor = PressureEvictor::new(CacheLimits {
        evictable_bytes: 1 << 30,
    });
    evictor.on_pressure(PressureLevel::Critical, &manager);
    let clock = SystemClock::new();
    // 지운 resource는 worker 역할 thread가 버립니다(UI thread chunk 밖 해제).
    let (release, released) = std::sync::mpsc::channel::<ReleasedResources>();
    let dropper =
        std::thread::spawn(move || released.into_iter().map(|batch| batch.len()).sum::<usize>());
    let mut chunks = Vec::new();
    let mut units = 0usize;
    let mut unit_max = Duration::ZERO;
    while let Some(report) = evictor.run_chunk(&mut manager, &clock, UI_CHUNK_BUDGET) {
        units += report.outcome.evicted.len();
        unit_max = unit_max.max(report.unit_cost);
        chunks.push(report.elapsed);
        release.send(report.released).unwrap();
    }
    drop(release);
    assert!(dropper.join().unwrap() > 0, "해제 묶음이 worker로 감");
    chunks.sort_unstable();
    let max = *chunks.last().unwrap();
    let p50 = chunks[chunks.len() / 2];
    let p99 = chunks[chunks.len() * 99 / 100];
    let over = chunks.iter().filter(|c| **c > UI_CHUNK_BUDGET).count();
    println!(
        "썸네일 200,000개 Critical 제거: chunk {}개, 단위 {units}개(chunk당 평균 {}), chunk p50 {p50:?}, p99 {p99:?}, 최대 {max:?}, 2 ms 초과 chunk {over}개, 단위 최대 {unit_max:?}",
        chunks.len(),
        units / chunks.len()
    );
    assert_eq!(manager.usage().total(), 0);
    // deadline은 단위 사이에서만 멈출 수 있으므로 OS 선점(아키텍처 6.3의 통제 불가 지연)은 막지 못합니다. 그래서
    // 중앙값으로 판정하고, 초과 chunk 수와 그때의 단위 최대 비용(선점 여부)은 보고서에 따로 적습니다. 정책 자체의 상한은
    // 가짜 시계 시험(`simulated_pressure_evicts_incrementally_within_ui_chunk_budget`)이 확인합니다.
    assert!(p50 <= UI_CHUNK_BUDGET, "chunk 중앙값 {p50:?} > 2 ms");
}

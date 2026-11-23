//! cache budget manager 시험 (XVI.5.2·XVI.5.3·XVI.5.6).

use std::cell::Cell;
use std::rc::Rc;

use super::*;
use crate::backend::BackendErrorKind;
use crate::cache_adapters::tests::fixture;

/// priority별 entry를 들고 priority 안에서 넣은 순서(LRU)대로 지우는 시험 cache.
struct MockCache {
    kind: CacheKind,
    entries: Vec<(CachePriority, u64, &'static str)>,
}

impl BudgetedCache for MockCache {
    fn kind(&self) -> CacheKind {
        self.kind
    }

    fn usage(&self) -> CacheUsage {
        let mut usage = CacheUsage::new();
        for (priority, bytes, _) in &self.entries {
            usage.add(*priority, *bytes);
        }
        usage
    }

    fn evict_one(&mut self, priority: CachePriority) -> u64 {
        match self.entries.iter().position(|(p, _, _)| *p == priority) {
            Some(index) => self.entries.remove(index).1,
            None => 0,
        }
    }
}

/// 지울 것이 남아 있어도 지금은 지울 수 없다고 답하는 cache(현재 frame·GPU 진행 중 page 흉내).
struct PinnedCache;

impl BudgetedCache for PinnedCache {
    fn kind(&self) -> CacheKind {
        CacheKind::GlyphAtlas
    }

    fn usage(&self) -> CacheUsage {
        CacheUsage::of(CachePriority::NearViewport, 500)
    }

    fn evict_one(&mut self, _priority: CachePriority) -> u64 {
        0
    }
}

#[test]
fn eviction_order_fixture() {
    // 검증 기준(XVI.5.2): eviction order fixture. 낮은 priority부터, 같은 priority 안에서는 §10.4 종류 순서, 같은
    // 종류는 등록 순서로 지우고 Document/CurrentScene은 지우지 않습니다.
    use CacheKind as K;
    use CachePriority as P;
    let mut manager = CacheBudgetManager::new();
    manager.register(Box::new(ReportedUsage::new(
        K::DocumentModel,
        P::DocumentScene,
        9_000,
    )));
    manager.register(Box::new(ReportedUsage::new(
        K::SceneSnapshot,
        P::DocumentScene,
        4_000,
    )));
    // 등록 순서를 §10.4 순서와 반대로 둡니다.
    manager.register(Box::new(MockCache {
        kind: K::GlyphAtlas,
        entries: vec![(P::VisibleGlyphGeometry, 100, "glyph page")],
    }));
    manager.register(Box::new(PinnedCache));
    manager.register(Box::new(MockCache {
        kind: K::Tessellation,
        entries: vec![
            (P::VisibleGlyphGeometry, 100, "visible mesh"),
            (P::NearViewport, 100, "near mesh a"),
            (P::NearViewport, 100, "near mesh b"),
        ],
    }));
    manager.register(Box::new(MockCache {
        kind: K::ImageMipmap,
        entries: vec![
            (P::NearViewport, 100, "near image"),
            (P::VisibleGlyphGeometry, 100, "visible image"),
        ],
    }));
    manager.register(Box::new(MockCache {
        kind: K::Thumbnail,
        entries: vec![
            (P::BackgroundThumbnail, 100, "thumbnail 1"),
            (P::BackgroundThumbnail, 100, "thumbnail 2"),
        ],
    }));
    manager.register(Box::new(MockCache {
        kind: K::Thumbnail,
        entries: vec![(P::BackgroundThumbnail, 100, "later thumbnail cache")],
    }));
    let before = manager.usage();
    assert_eq!(before.get(P::DocumentScene), 13_000);
    let mut pass = EvictionPass::new(0);
    let outcome = manager.evict_chunk(&mut pass, &mut Unbounded);
    let order: Vec<(CachePriority, CacheKind, u32)> = outcome
        .evicted
        .iter()
        .map(|e| (e.priority, e.kind, e.cache.get()))
        .collect();
    assert_eq!(
        order,
        vec![
            (P::BackgroundThumbnail, K::Thumbnail, 6),
            (P::BackgroundThumbnail, K::Thumbnail, 6),
            (P::BackgroundThumbnail, K::Thumbnail, 7),
            (P::NearViewport, K::ImageMipmap, 5),
            (P::NearViewport, K::Tessellation, 4),
            (P::NearViewport, K::Tessellation, 4),
            (P::VisibleGlyphGeometry, K::ImageMipmap, 5),
            (P::VisibleGlyphGeometry, K::Tessellation, 4),
            (P::VisibleGlyphGeometry, K::GlyphAtlas, 2),
        ]
    );
    assert!(outcome.finished && outcome.over_target);
    assert_eq!(outcome.freed, 900);
    let after = manager.usage();
    assert_eq!(
        after.get(P::DocumentScene),
        13_000,
        "Document/CurrentScene 비축출"
    );
    assert_eq!(
        after.get(P::NearViewport),
        500,
        "지울 수 없다고 답한 단위는 남김"
    );
    assert_eq!(after.total(), 13_500);
}

#[test]
fn eviction_stops_at_the_target_and_resumes_across_chunks() {
    use CacheKind as K;
    use CachePriority as P;
    let mut manager = CacheBudgetManager::new();
    manager.register(Box::new(MockCache {
        kind: K::Thumbnail,
        entries: (0..10).map(|_| (P::BackgroundThumbnail, 10, "t")).collect(),
    }));
    manager.register(Box::new(MockCache {
        kind: K::Tessellation,
        entries: (0..10).map(|_| (P::NearViewport, 10, "m")).collect(),
    }));
    let mut pass = EvictionPass::new(55);
    // chunk마다 단위 3개까지.
    let first = manager.evict_chunk(&mut pass, &mut UnitLimit(3));
    assert_eq!((first.evicted.len(), first.finished), (3, false));
    let mut chunks = 1;
    loop {
        let outcome = manager.evict_chunk(&mut pass, &mut UnitLimit(3));
        chunks += 1;
        if outcome.finished {
            assert!(!outcome.over_target);
            break;
        }
    }
    assert_eq!(manager.usage().total(), 50, "목표 55 이하에서 멈춤");
    assert_eq!(manager.usage().get(P::BackgroundThumbnail), 0);
    assert_eq!(manager.usage().get(P::NearViewport), 50);
    assert_eq!(chunks, 5);
}
/// 호출마다 정해진 결과를 돌려주는 GPU 할당 작업.
fn allocation(
    results: Vec<Result<u32, BackendErrorKind>>,
) -> (Rc<Cell<u32>>, impl FnMut() -> Result<u32, BackendError>) {
    let calls = Rc::new(Cell::new(0u32));
    let counter = Rc::clone(&calls);
    let operation = move || {
        let index = counter.get() as usize;
        counter.set(counter.get() + 1);
        match results
            .get(index)
            .copied()
            .unwrap_or(Err(BackendErrorKind::OutOfDeviceMemory))
        {
            Ok(value) => Ok(value),
            Err(kind) => Err(BackendError::new(kind, "시험 할당")),
        }
    };
    (calls, operation)
}

#[test]
fn mock_gpu_oom_retries_exactly_once() {
    // 검증 기준(XVI.5.6): mock OOM retry count = 1, 반복 loop 금지.
    let (mut manager, [_, _, glyphs, tessellation, mipmaps, thumbnails]) = fixture();
    let before = manager.usage();
    let mut tracker = RecoveryTracker::new();
    let (calls, operation) = allocation(vec![]);
    let outcome = with_oom_retry(&mut tracker, &mut manager, operation);
    assert_eq!(calls.get(), 2, "처음 + 재시도 한 번");
    assert_eq!(outcome.retries, 1);
    assert_eq!(
        outcome.result.unwrap_err().kind(),
        BackendErrorKind::OutOfDeviceMemory
    );
    // GPU cache만 비웠고 Document/CurrentScene·썸네일(GPU 밖)은 남았습니다.
    let after = manager.usage();
    assert_eq!(after.get(CachePriority::DocumentScene), 60_000);
    assert_eq!(manager.usage_of(thumbnails).unwrap().total(), 3_000);
    assert_eq!(manager.usage_of(tessellation).unwrap().total(), 0);
    assert_eq!(manager.usage_of(mipmaps).unwrap().total(), 0);
    assert_eq!(manager.usage_of(glyphs).unwrap().total(), 64 * 64);
    assert_eq!(
        outcome.purged_bytes,
        before.total() - after.total(),
        "purge byte = 줄어든 사용량"
    );
    // 다음 작업(다음 frame)은 다시 한 번만 재시도합니다. tracker가 재시도 대기 상태에 갇히지 않습니다.
    let (calls, operation) = allocation(vec![]);
    let outcome = with_oom_retry(&mut tracker, &mut manager, operation);
    assert_eq!((calls.get(), outcome.retries), (2, 1));
    assert!(!tracker.is_disabled());
}

#[test]
fn oom_retry_succeeds_once_and_non_memory_errors_are_not_retried() {
    let (mut manager, _) = fixture();
    let mut tracker = RecoveryTracker::new();
    // purge 뒤 성공.
    let (calls, operation) = allocation(vec![Err(BackendErrorKind::OutOfDeviceMemory), Ok(7)]);
    let outcome = with_oom_retry(&mut tracker, &mut manager, operation);
    assert_eq!((calls.get(), outcome.retries), (2, 1));
    assert_eq!(outcome.result.unwrap(), 7);
    // 성공을 알렸으므로 다음 OOM도 다시 한 번 재시도합니다(frame마다 무한 반복이 아니라 작업마다 한 번).
    let (calls, operation) = allocation(vec![Err(BackendErrorKind::OutOfHostMemory), Ok(8)]);
    let outcome = with_oom_retry(&mut tracker, &mut manager, operation);
    assert_eq!(
        (calls.get(), outcome.retries, outcome.result.unwrap()),
        (2, 1, 8)
    );
    // 처음부터 성공하면 purge하지 않습니다.
    let (manager_before, _) = fixture();
    let mut fresh = fixture().0;
    let (calls, operation) = allocation(vec![Ok(1)]);
    let outcome = with_oom_retry(&mut tracker, &mut fresh, operation);
    assert_eq!(
        (calls.get(), outcome.retries, outcome.purged_bytes),
        (1, 0, 0)
    );
    assert_eq!(fresh.usage(), manager_before.usage());
    // 메모리 부족이 아닌 오류는 재시도하지 않습니다.
    let (calls, operation) = allocation(vec![Err(BackendErrorKind::DeviceLost), Ok(2)]);
    let outcome = with_oom_retry(&mut tracker, &mut fresh, operation);
    assert_eq!((calls.get(), outcome.retries), (1, 0));
    assert_eq!(
        outcome.result.unwrap_err().kind(),
        BackendErrorKind::DeviceLost
    );
    assert_eq!(fresh.usage(), manager_before.usage());
}

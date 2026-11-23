//! cache budget manager 시험 (XVI.5.2·XVI.5.3·XVI.5.6).

use super::*;

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

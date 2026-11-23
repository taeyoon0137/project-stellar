//! 문서 썸네일 cache (XVI.5.3).
//!
//! 비활성 문서의 썸네일 RGBA pixel을 문서·크기별로 들고, budget manager에 [`CachePriority::BackgroundThumbnail`]로
//! 사용량을 보고합니다(가장 먼저 지움). 지우는 순서는 가장 오래 보지 않은 썸네일부터입니다. 썸네일은 다시 그릴 수
//! 있는 파생 값이라 지워도 문서 내용은 그대로입니다.

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use stellar_core_types::ids::StableId;
use stellar_renderer_core::cache::{BudgetedCache, CacheKind, CachePriority, CacheUsage};
use stellar_renderer_core::cache_adapters::{LruBudget, ReleaseQueue};

/// 썸네일 key: 문서와 pixel 크기.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ThumbnailKey {
    /// 문서 ID.
    pub document: StableId,
    /// 폭(px).
    pub width: u32,
    /// 높이(px).
    pub height: u32,
}

/// 썸네일 pixel(RGBA8, 행 우선).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    key: ThumbnailKey,
    pixels: Arc<[u8]>,
}

impl Thumbnail {
    /// pixel 길이가 `width × height × 4`가 아니면 `None`.
    pub fn new(key: ThumbnailKey, pixels: Vec<u8>) -> Option<Thumbnail> {
        let expected = u64::from(key.width)
            .checked_mul(u64::from(key.height))?
            .checked_mul(4)?;
        (pixels.len() as u64 == expected).then(|| Thumbnail {
            key,
            pixels: pixels.into(),
        })
    }

    /// key.
    pub fn key(&self) -> ThumbnailKey {
        self.key
    }

    /// pixel.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// pixel byte.
    pub fn bytes(&self) -> u64 {
        self.pixels.len() as u64
    }
}

/// 문서 썸네일 cache.
#[derive(Debug, Default)]
pub struct ThumbnailCache {
    entries: HashMap<ThumbnailKey, Thumbnail>,
    budget: LruBudget<ThumbnailKey>,
    /// 지웠지만 아직 해제하지 않은 썸네일(XVI.5.4, worker thread에서 버림).
    released: ReleaseQueue<Thumbnail>,
}

impl ThumbnailCache {
    /// 빈 cache.
    pub fn new() -> ThumbnailCache {
        ThumbnailCache::default()
    }

    /// 썸네일을 넣습니다(같은 key는 교체). 가장 최근 사용이 됩니다.
    pub fn insert(&mut self, thumbnail: Thumbnail) {
        let key = thumbnail.key();
        self.budget
            .insert(key, CachePriority::BackgroundThumbnail, thumbnail.bytes());
        self.entries.insert(key, thumbnail);
    }

    /// 썸네일을 찾고 가장 최근 사용으로 표시합니다.
    pub fn get(&mut self, key: &ThumbnailKey) -> Option<&Thumbnail> {
        self.budget.touch(key);
        self.entries.get(key)
    }

    /// 썸네일을 지웁니다(문서 닫기·내용 변경).
    pub fn remove(&mut self, key: &ThumbnailKey) -> Option<Thumbnail> {
        self.budget.remove(key);
        self.entries.remove(key)
    }

    /// 썸네일 수.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 썸네일이 없으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl BudgetedCache for ThumbnailCache {
    fn kind(&self) -> CacheKind {
        CacheKind::Thumbnail
    }

    fn usage(&self) -> CacheUsage {
        self.budget.usage()
    }

    fn evict_one(&mut self, priority: CachePriority) -> u64 {
        match self.budget.pop_lru(priority) {
            Some((key, bytes)) => {
                if let Some(thumbnail) = self.entries.remove(&key) {
                    self.released.push(thumbnail);
                }
                bytes
            }
            None => 0,
        }
    }

    fn take_released(&mut self) -> Vec<Box<dyn Any + Send>> {
        self.released.take()
    }
}

#[cfg(test)]
mod tests {
    use stellar_renderer_core::cache::{CacheBudgetManager, EvictionPass, Unbounded};

    use super::*;

    fn thumbnail(document: u64, side: u32) -> Thumbnail {
        let key = ThumbnailKey {
            document: StableId::new_v7(document, 0, document),
            width: side,
            height: side,
        };
        Thumbnail::new(key, vec![0x40; (side * side * 4) as usize]).unwrap()
    }

    #[test]
    fn thumbnail_cache_registers_usage_and_evicts_least_recent_first() {
        // 검증 기준(XVI.5.3): 썸네일 cache가 공통 budget manager에 usage를 등록(실제 pixel byte 합).
        let mut cache = ThumbnailCache::new();
        for document in 1..=3 {
            cache.insert(thumbnail(document, 8));
        }
        assert!(
            Thumbnail::new(thumbnail(9, 2).key(), vec![0; 3]).is_none(),
            "크기 검사"
        );
        // 같은 key를 다시 넣으면 교체합니다(사용량 중복 없음).
        cache.insert(thumbnail(2, 8));
        let first = thumbnail(1, 8).key();
        assert!(cache.get(&first).is_some());
        let mut manager = CacheBudgetManager::new();
        let id = manager.register(Box::new(cache));
        assert_eq!(
            manager.usage(),
            CacheUsage::of(CachePriority::BackgroundThumbnail, 3 * 8 * 8 * 4)
        );
        let mut pass = EvictionPass::new(8 * 8 * 4);
        let outcome = manager.evict_chunk(&mut pass, &mut Unbounded);
        assert_eq!(outcome.evicted.len(), 2);
        assert!(outcome
            .evicted
            .iter()
            .all(|e| e.priority == CachePriority::BackgroundThumbnail));
        let cache = manager.get_mut::<ThumbnailCache>(id).unwrap();
        assert_eq!(cache.len(), 1);
        assert!(cache.get(&first).is_some(), "최근에 본 썸네일이 남음");
        assert_eq!(cache.remove(&first).map(|t| t.bytes()), Some(8 * 8 * 4));
        assert_eq!(manager.usage().total(), 0);
    }
}

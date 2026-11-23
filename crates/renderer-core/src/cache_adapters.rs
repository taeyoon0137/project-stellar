//! glyph atlas·tessellation·이미지 mipmap cache의 budget manager 등록 (XVI.5.3, ADR-0108 결정 5).
//!
//! 각 cache를 [`BudgetedCache`]로 감쌉니다. 감싼 타입은 원래 cache를 그대로 들고, 원래 cache의 조회·삽입 API를
//! 거치면서 entry마다 priority·byte·마지막 사용 순번을 함께 기록합니다. 그래서 manager가 보는 사용량은 cache가
//! 실제로 들고 있는 resource의 byte 합이고, 제거는 priority 안 LRU 순서입니다.
//!
//! - [`BudgetedTessellationCache`]: [`TessellationCache`] + mesh byte(vertex 8 byte + index 4 byte).
//! - [`BudgetedMipmapCache`]: [`TextureCache<MipChain>`] + mip chain byte([`MipChain::total_bytes`]).
//! - [`BudgetedGlyphAtlas`]: [`ShelfGlyphAtlas`] page texture byte([`ShelfGlyphAtlas::memory_bytes`]). page 단위로
//!   [`ShelfGlyphAtlas::apply_memory_pressure`]의 LRU page 제거를 씁니다. 현재 frame·GPU 진행 중 page는 atlas가 지우지
//!   않으므로 그때는 0 byte를 돌려줍니다.
//!
//! 썸네일 cache는 runtime(`stellar_runtime::cache::thumbnail`)에 있습니다.

use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::sync::Arc;

use crate::cache::{BudgetedCache, CacheKind, CachePriority, CacheUsage};
use crate::glyph::atlas::ShelfGlyphAtlas;
use crate::mipmap::MipChain;
use stellar_scene::geometry::PathGeometry;

use crate::stroke::{StrokeStyle, StrokeTessellator};
use crate::tessellation::{
    CacheOutcome, CachedMesh, FillMesh, FillStyle, FillTessellator, TessellationCache,
    TessellationError, TessellationKey,
};
use crate::texture_cache::{CachedTexture, TextureCache, TextureCacheKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    priority: CachePriority,
    tick: u64,
    bytes: u64,
}

/// key별 priority·byte와 priority 안 LRU 순서. 이 모듈의 cache와 runtime 썸네일 cache가 같이 씁니다.
#[derive(Debug)]
pub struct LruBudget<K> {
    slots: HashMap<K, Slot>,
    order: BTreeMap<(CachePriority, u64), K>,
    usage: CacheUsage,
    next_tick: u64,
}

impl<K> Default for LruBudget<K> {
    fn default() -> LruBudget<K> {
        LruBudget {
            slots: HashMap::new(),
            order: BTreeMap::new(),
            usage: CacheUsage::new(),
            next_tick: 0,
        }
    }
}

impl<K: Clone + Eq + Hash> LruBudget<K> {
    fn tick(&mut self) -> u64 {
        self.next_tick += 1;
        self.next_tick
    }

    /// key를 기록합니다(이미 있으면 byte·priority를 바꾸고 가장 최근 사용으로).
    pub fn insert(&mut self, key: K, priority: CachePriority, bytes: u64) {
        self.remove(&key);
        let tick = self.tick();
        self.order.insert((priority, tick), key.clone());
        self.slots.insert(
            key,
            Slot {
                priority,
                tick,
                bytes,
            },
        );
        self.usage.add(priority, bytes);
    }

    /// 가장 최근 사용으로 표시합니다.
    pub fn touch(&mut self, key: &K) {
        let Some(slot) = self.slots.get(key).copied() else {
            return;
        };
        let tick = self.tick();
        if let Some(key) = self.order.remove(&(slot.priority, slot.tick)) {
            self.order.insert((slot.priority, tick), key);
        }
        if let Some(slot) = self.slots.get_mut(key) {
            slot.tick = tick;
        }
    }

    /// priority를 바꿉니다(viewport 안팎 이동). 없으면 `false`.
    pub fn set_priority(&mut self, key: &K, priority: CachePriority) -> bool {
        let Some(slot) = self.slots.get(key).copied() else {
            return false;
        };
        self.insert(key.clone(), priority, slot.bytes);
        true
    }

    /// key의 priority.
    pub fn priority(&self, key: &K) -> Option<CachePriority> {
        self.slots.get(key).map(|slot| slot.priority)
    }

    /// 기록을 지우고 byte를 돌려줍니다.
    pub fn remove(&mut self, key: &K) -> Option<u64> {
        let slot = self.slots.remove(key)?;
        self.order.remove(&(slot.priority, slot.tick));
        self.usage.subtract(slot.priority, slot.bytes);
        Some(slot.bytes)
    }

    /// `priority`의 가장 오래 쓰지 않은 key를 꺼냅니다.
    pub fn pop_lru(&mut self, priority: CachePriority) -> Option<(K, u64)> {
        let (_, key) = self
            .order
            .range((priority, 0)..=(priority, u64::MAX))
            .next()?;
        let key = key.clone();
        let bytes = self.remove(&key)?;
        Some((key, bytes))
    }

    /// priority별 사용량.
    pub fn usage(&self) -> CacheUsage {
        self.usage
    }

    /// 빈 기록.
    pub fn new() -> LruBudget<K> {
        LruBudget::default()
    }

    /// 기록 수.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// 기록이 없으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// 모두 지웁니다.
    pub fn clear(&mut self) {
        self.slots.clear();
        self.order.clear();
        self.usage = CacheUsage::new();
    }
}

/// 지운 값을 [`ReleaseQueue`]에 모을 때 한 묶음의 크기.
const RELEASE_BATCH: usize = 256;

/// 지웠지만 아직 해제하지 않은 값(XVI.5.4). [`BudgetedCache::take_released`] 구현용입니다.
///
/// 고정 크기 묶음에 넣어, chunk 안에서 큰 `Vec`을 다시 할당(복사·새 memory region)하지 않습니다.
#[derive(Debug)]
pub struct ReleaseQueue<T> {
    current: Vec<T>,
    full: Vec<Vec<T>>,
}

impl<T> Default for ReleaseQueue<T> {
    fn default() -> ReleaseQueue<T> {
        ReleaseQueue {
            current: Vec::new(),
            full: Vec::new(),
        }
    }
}

impl<T: Send + 'static> ReleaseQueue<T> {
    /// 값 하나를 넣습니다.
    pub fn push(&mut self, value: T) {
        if self.current.capacity() == 0 {
            self.current = Vec::with_capacity(RELEASE_BATCH);
        }
        self.current.push(value);
        if self.current.len() == RELEASE_BATCH {
            self.full.push(std::mem::take(&mut self.current));
        }
    }

    /// 모은 값을 한 묶음으로 꺼냅니다(비었으면 없음).
    pub fn take(&mut self) -> Vec<Box<dyn Any + Send>> {
        if !self.current.is_empty() {
            self.full.push(std::mem::take(&mut self.current));
        }
        if self.full.is_empty() {
            return Vec::new();
        }
        vec![Box::new(std::mem::take(&mut self.full))]
    }
}

/// mesh가 차지하는 byte(vertex `Vec2` 8 byte + index `u32` 4 byte).
pub fn mesh_bytes(mesh: &FillMesh) -> u64 {
    (mesh.vertices().len() as u64) * 8 + (mesh.indices().len() as u64) * 4
}

/// budget manager에 등록하는 tessellation cache.
#[derive(Debug, Default)]
pub struct BudgetedTessellationCache {
    inner: TessellationCache,
    budget: LruBudget<TessellationKey>,
    /// 지웠지만 아직 해제하지 않은 mesh(XVI.5.4, worker thread에서 버림).
    released: ReleaseQueue<Arc<FillMesh>>,
}

impl BudgetedTessellationCache {
    /// 빈 cache.
    pub fn new() -> BudgetedTessellationCache {
        BudgetedTessellationCache::default()
    }

    /// 원래 cache(통계 조회용).
    pub fn inner(&self) -> &TessellationCache {
        &self.inner
    }

    fn adopt(&mut self, key: TessellationKey, cached: &CachedMesh, priority: CachePriority) {
        match cached.outcome {
            CacheOutcome::Miss => self.budget.insert(key, priority, mesh_bytes(&cached.mesh)),
            CacheOutcome::Hit => {
                if self.budget.priority(&key) == Some(priority) {
                    self.budget.touch(&key);
                } else {
                    self.budget.set_priority(&key, priority);
                }
            }
        }
    }

    /// fill mesh를 조회하거나 만들고, entry를 `priority`의 가장 최근 사용으로 둡니다.
    pub fn fill(
        &mut self,
        path: &PathGeometry,
        style: FillStyle,
        tessellator: &dyn FillTessellator,
        priority: CachePriority,
    ) -> Result<CachedMesh, TessellationError> {
        let cached = self.inner.fill(path, style, tessellator)?;
        self.adopt(TessellationKey::fill(path, style), &cached, priority);
        Ok(cached)
    }

    /// stroke mesh를 조회하거나 만들고, entry를 `priority`의 가장 최근 사용으로 둡니다.
    pub fn stroke(
        &mut self,
        path: &PathGeometry,
        style: &StrokeStyle,
        tessellator: &dyn StrokeTessellator,
        priority: CachePriority,
    ) -> Result<CachedMesh, TessellationError> {
        let cached = self.inner.stroke(path, style, tessellator)?;
        self.adopt(TessellationKey::stroke(path, style), &cached, priority);
        Ok(cached)
    }

    /// entry의 priority를 바꿉니다(viewport 안팎 이동). 없으면 `false`.
    pub fn set_priority(&mut self, key: &TessellationKey, priority: CachePriority) -> bool {
        self.budget.set_priority(key, priority)
    }

    /// entry를 지웁니다.
    pub fn remove(&mut self, key: &TessellationKey) -> Option<Arc<FillMesh>> {
        self.budget.remove(key);
        self.inner.remove(key)
    }

    /// entry 수.
    pub fn len(&self) -> usize {
        self.budget.len()
    }

    /// entry가 없으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl BudgetedCache for BudgetedTessellationCache {
    fn kind(&self) -> CacheKind {
        CacheKind::Tessellation
    }

    fn usage(&self) -> CacheUsage {
        self.budget.usage()
    }

    fn evict_one(&mut self, priority: CachePriority) -> u64 {
        match self.budget.pop_lru(priority) {
            Some((key, bytes)) => {
                if let Some(mesh) = self.inner.remove(&key) {
                    self.released.push(mesh);
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

/// budget manager에 등록하는 이미지 mipmap texture cache.
#[derive(Debug, Default)]
pub struct BudgetedMipmapCache {
    inner: TextureCache<MipChain>,
    budget: LruBudget<TextureCacheKey>,
    /// 지웠지만 아직 해제하지 않은 mip chain(XVI.5.4, worker thread에서 버림).
    released: ReleaseQueue<Arc<MipChain>>,
}

impl BudgetedMipmapCache {
    /// 빈 cache.
    pub fn new() -> BudgetedMipmapCache {
        BudgetedMipmapCache::default()
    }

    /// 조회하고, 없으면 `create`로 만들어 저장합니다. entry를 `priority`의 가장 최근 사용으로 둡니다.
    pub fn get_or_create<E>(
        &mut self,
        key: TextureCacheKey,
        priority: CachePriority,
        create: impl FnOnce(&TextureCacheKey) -> Result<MipChain, E>,
    ) -> Result<CachedTexture<MipChain>, E> {
        let cached = self.inner.get_or_create(key, create)?;
        match cached.outcome {
            CacheOutcome::Miss => self
                .budget
                .insert(key, priority, cached.texture.total_bytes()),
            CacheOutcome::Hit => {
                if self.budget.priority(&key) == Some(priority) {
                    self.budget.touch(&key);
                } else {
                    self.budget.set_priority(&key, priority);
                }
            }
        }
        Ok(cached)
    }

    /// entry의 priority를 바꿉니다(viewport 안팎 이동). 없으면 `false`.
    pub fn set_priority(&mut self, key: &TextureCacheKey, priority: CachePriority) -> bool {
        self.budget.set_priority(key, priority)
    }

    /// entry를 지웁니다.
    pub fn remove(&mut self, key: &TextureCacheKey) -> Option<Arc<MipChain>> {
        self.budget.remove(key);
        self.inner.remove(key)
    }

    /// 모든 entry를 지웁니다.
    pub fn clear(&mut self) {
        self.budget.clear();
        self.inner.clear();
    }

    /// entry 수.
    pub fn len(&self) -> usize {
        self.budget.len()
    }

    /// entry가 없으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl BudgetedCache for BudgetedMipmapCache {
    fn kind(&self) -> CacheKind {
        CacheKind::ImageMipmap
    }

    fn usage(&self) -> CacheUsage {
        self.budget.usage()
    }

    fn evict_one(&mut self, priority: CachePriority) -> u64 {
        match self.budget.pop_lru(priority) {
            Some((key, bytes)) => {
                if let Some(chain) = self.inner.remove(&key) {
                    self.released.push(chain);
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

/// budget manager에 등록하는 glyph atlas. 모든 page는 [`CachePriority::VisibleGlyphGeometry`]입니다(화면 글자).
#[derive(Debug)]
pub struct BudgetedGlyphAtlas {
    atlas: ShelfGlyphAtlas,
    configured_max_pages: u32,
}

impl BudgetedGlyphAtlas {
    /// atlas를 감쌉니다. 지금 설정의 page 상한을 압박 해제 때 되돌릴 값으로 기억합니다.
    pub fn new(atlas: ShelfGlyphAtlas) -> BudgetedGlyphAtlas {
        let configured_max_pages = atlas.config().max_pages;
        BudgetedGlyphAtlas {
            atlas,
            configured_max_pages,
        }
    }

    /// atlas(조회).
    pub fn atlas(&self) -> &ShelfGlyphAtlas {
        &self.atlas
    }

    /// atlas(삽입·frame 진행). [`GlyphAtlas`](crate::glyph::GlyphAtlas) API를 그대로 씁니다.
    pub fn atlas_mut(&mut self) -> &mut ShelfGlyphAtlas {
        &mut self.atlas
    }

    /// 압박 때 줄인 page 상한을 처음 설정으로 되돌립니다.
    pub fn restore_page_budget(&mut self) {
        // 제거는 page 수를 줄이기만 하므로 지금 page 수는 설정값 이하입니다. 상한만 되돌리고 page는 지우지 않습니다.
        self.atlas.apply_memory_pressure(self.configured_max_pages);
    }
}

impl BudgetedCache for BudgetedGlyphAtlas {
    fn kind(&self) -> CacheKind {
        CacheKind::GlyphAtlas
    }

    fn usage(&self) -> CacheUsage {
        CacheUsage::of(
            CachePriority::VisibleGlyphGeometry,
            self.atlas.memory_bytes(),
        )
    }

    fn evict_one(&mut self, priority: CachePriority) -> u64 {
        if priority != CachePriority::VisibleGlyphGeometry || self.atlas.page_count() <= 1 {
            return 0;
        }
        let before = self.atlas.memory_bytes();
        self.atlas
            .apply_memory_pressure(self.atlas.page_count().saturating_sub(1));
        before.saturating_sub(self.atlas.memory_bytes())
    }
}

#[cfg(test)]
pub(crate) mod tests;

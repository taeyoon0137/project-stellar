//! 공통 cache budget manager와 cache priority (XVI.5.2·XVI.5.3·XVI.5.4·XVI.5.6, ADR-0108 결정 4~7).
//!
//! 아키텍처 §10.4의 예산 기반 다단계 cache를 구현합니다. 각 cache는 [`BudgetedCache`]로 manager에 등록하고, 자기
//! 사용량을 priority별 byte로 보고하며, priority 하나의 가장 오래 쓰지 않은 단위(entry·page) 하나를 지우는 방법을
//! 제공합니다. manager는 등록한 cache의 사용량을 합하고(XVI.5.3), 낮은 priority부터 정해진 순서로 지웁니다(XVI.5.2).
//!
//! ## priority (XVI.5.2)
//!
//! | priority | 예 | 제거 |
//! |---|---|---|
//! | [`CachePriority::DocumentScene`] | 현재 문서 모델, 현재 Scene Snapshot | 하지 않음 |
//! | [`CachePriority::VisibleGlyphGeometry`] | 화면에 보이는 glyph atlas page·tessellation·이미지 | 마지막 |
//! | [`CachePriority::NearViewport`] | 화면 밖이지만 가까운 이미지 mipmap·tessellation | 두 번째 |
//! | [`CachePriority::BackgroundThumbnail`] | 비활성 문서 썸네일 | 처음 |
//!
//! 같은 priority 안에서는 §10.4 "우선 제거" 목록 순서([`CacheKind::eviction_rank`]: 썸네일 → 이미지 mipmap →
//! tessellation → glyph atlas page)로, 같은 종류 안에서는 등록 순서로, 한 cache 안에서는 cache가 정한 LRU 순서로 지웁니다.
//!
//! ## 증분 제거 (XVI.5.4)
//!
//! [`CacheBudgetManager::evict_chunk`]는 [`EvictionDeadline`]이 허락하는 동안만 한 단위씩 지우고 돌아옵니다. 진행 상태
//! ([`EvictionPass`])는 호출자가 들고 다음 chunk에서 이어 갑니다. UI thread 2 ms chunk 정책과 memory pressure 단계 →
//! 목표 사용량 변환은 runtime(`stellar_runtime::cache`)이 정합니다.
//!
//! ## GPU OOM (XVI.5.6)
//!
//! [`with_oom_retry`]는 GPU 메모리 부족 오류([`BackendErrorKind::OutOfDeviceMemory`](crate::backend::BackendErrorKind)·
//! `OutOfHostMemory`)가 나면 GPU에 올라가는 cache를 비우고([`CacheBudgetManager::purge_gpu`]) 작업을 **한 번만** 다시 합니다.
//! 다시 실패해도 반복하지 않습니다. 판단은 XI.5.4 [`RecoveryTracker`]의 `PurgeCachesAndRetry` 정책을 그대로 씁니다.

use std::any::Any;

use crate::backend::recovery::{Recovery, RecoveryAction, RecoveryTracker};
use crate::backend::BackendError;

/// cache 보존 priority(XVI.5.2). 값이 작을수록 먼저 지웁니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CachePriority {
    /// 비활성 문서 썸네일. 가장 먼저 지웁니다.
    BackgroundThumbnail,
    /// 화면 밖이지만 viewport 가까이에 있는 resource.
    NearViewport,
    /// 화면에 보이는 glyph·geometry(tessellation)·이미지. 마지막에 지웁니다.
    VisibleGlyphGeometry,
    /// 현재 문서 모델과 현재 Scene Snapshot. 지우지 않습니다.
    DocumentScene,
}

impl CachePriority {
    /// 모든 priority(제거 순서).
    pub const ALL: [CachePriority; 4] = [
        CachePriority::BackgroundThumbnail,
        CachePriority::NearViewport,
        CachePriority::VisibleGlyphGeometry,
        CachePriority::DocumentScene,
    ];

    /// 지울 수 있는 priority를 지우는 순서.
    pub const EVICTION_ORDER: [CachePriority; 3] = [
        CachePriority::BackgroundThumbnail,
        CachePriority::NearViewport,
        CachePriority::VisibleGlyphGeometry,
    ];

    /// 지울 수 있으면 `true`(Document/CurrentScene만 `false`).
    pub const fn is_evictable(self) -> bool {
        !matches!(self, CachePriority::DocumentScene)
    }

    const fn index(self) -> usize {
        match self {
            CachePriority::BackgroundThumbnail => 0,
            CachePriority::NearViewport => 1,
            CachePriority::VisibleGlyphGeometry => 2,
            CachePriority::DocumentScene => 3,
        }
    }
}

/// manager에 등록하는 cache 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CacheKind {
    /// 현재 문서 모델(사용량만 보고, 제거 없음).
    DocumentModel,
    /// 현재 Scene Snapshot(사용량만 보고, 제거 없음).
    SceneSnapshot,
    /// 문서 썸네일.
    Thumbnail,
    /// 이미지 mipmap texture.
    ImageMipmap,
    /// path tessellation mesh.
    Tessellation,
    /// glyph atlas page.
    GlyphAtlas,
}

impl CacheKind {
    /// 같은 priority 안 제거 순서(아키텍처 §10.4 "우선 제거" 목록: 썸네일 → 이미지 mipmap → 재생성 가능한
    /// tessellation → 사용 빈도가 낮은 glyph atlas page). 작을수록 먼저 지웁니다.
    pub const fn eviction_rank(self) -> u8 {
        match self {
            CacheKind::Thumbnail => 0,
            CacheKind::ImageMipmap => 1,
            CacheKind::Tessellation => 2,
            CacheKind::GlyphAtlas => 3,
            CacheKind::SceneSnapshot => 4,
            CacheKind::DocumentModel => 5,
        }
    }

    /// GPU 메모리에 올라가는 resource를 가진 cache면 `true`(GPU OOM purge 대상).
    pub const fn gpu_resident(self) -> bool {
        matches!(
            self,
            CacheKind::ImageMipmap | CacheKind::Tessellation | CacheKind::GlyphAtlas
        )
    }
}

/// priority별 사용량(byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheUsage {
    bytes: [u64; 4],
}

impl CacheUsage {
    /// 빈 사용량.
    pub const fn new() -> CacheUsage {
        CacheUsage { bytes: [0; 4] }
    }

    /// `priority` 하나에 `bytes`만 있는 사용량.
    pub fn of(priority: CachePriority, bytes: u64) -> CacheUsage {
        let mut usage = CacheUsage::new();
        usage.add(priority, bytes);
        usage
    }

    /// `priority`에 `bytes`를 더합니다(포화).
    pub fn add(&mut self, priority: CachePriority, bytes: u64) {
        let slot = &mut self.bytes[priority.index()];
        *slot = slot.saturating_add(bytes);
    }

    /// `priority`에서 `bytes`를 뺍니다(0 아래로 내려가지 않음).
    pub fn subtract(&mut self, priority: CachePriority, bytes: u64) {
        let slot = &mut self.bytes[priority.index()];
        *slot = slot.saturating_sub(bytes);
    }

    /// `priority`의 사용량.
    pub const fn get(&self, priority: CachePriority) -> u64 {
        self.bytes[priority.index()]
    }

    /// 전체 사용량.
    pub fn total(&self) -> u64 {
        self.bytes
            .iter()
            .fold(0u64, |sum, b| sum.saturating_add(*b))
    }

    /// 지울 수 있는 priority의 사용량.
    pub fn evictable(&self) -> u64 {
        self.total()
            .saturating_sub(self.get(CachePriority::DocumentScene))
    }

    /// 두 사용량의 합.
    pub fn plus(mut self, other: CacheUsage) -> CacheUsage {
        for priority in CachePriority::ALL {
            self.add(priority, other.get(priority));
        }
        self
    }
}

/// manager에 등록하는 cache(XVI.5.3).
///
/// `usage`는 지금 들고 있는 resource의 byte를 priority별로 정확히 보고해야 합니다(추정 상한이 아님). `evict_one`은
/// `priority`의 가장 오래 쓰지 않은 단위 하나를 지우고 줄어든 byte를 돌려줍니다. 지울 것이 없거나 지금 지울 수 없는
/// 단위(현재 frame·GPU 진행 중)만 남았으면 0입니다. manager는 [`CachePriority::DocumentScene`]에 대해 `evict_one`을
/// 부르지 않습니다. 한 번 호출은 짧아야 합니다(증분 제거의 단위, XVI.5.4).
///
/// 지운 resource의 메모리 반환(큰 buffer의 해제)은 UI thread chunk 안에서 하지 않습니다. cache는 지운 값을 들고 있다가
/// [`BudgetedCache::take_released`]로 내주고, 호출자가 그 묶음([`ReleasedResources`])을 worker thread로 보내 버립니다.
/// 사용량은 지운 즉시 줄어든 것으로 봅니다.
pub trait BudgetedCache: Any {
    /// cache 종류.
    fn kind(&self) -> CacheKind;
    /// priority별 사용량.
    fn usage(&self) -> CacheUsage;
    /// `priority`의 단위 하나를 지우고 줄어든 byte를 돌려줍니다.
    fn evict_one(&mut self, priority: CachePriority) -> u64;
    /// 지운 뒤 아직 메모리를 돌려주지 않은 값을 꺼냅니다. 기본은 없음(지울 때 바로 해제하는 cache).
    fn take_released(&mut self) -> Vec<Box<dyn Any + Send>> {
        Vec::new()
    }
}

/// 지운 resource 묶음. 버리면(drop) 메모리를 돌려주므로 UI thread가 아닌 thread에서 버립니다(XVI.5.4).
#[derive(Default)]
pub struct ReleasedResources(Vec<Box<dyn Any + Send>>);

impl ReleasedResources {
    /// 다른 묶음을 뒤에 붙입니다.
    pub fn append(&mut self, mut other: ReleasedResources) {
        self.0.append(&mut other.0);
    }

    /// 묶음 안 값 수.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// 비었으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for ReleasedResources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ReleasedResources")
            .field(&self.0.len())
            .finish()
    }
}

/// 제거 없이 사용량만 보고하는 항목(현재 문서 모델·Scene Snapshot처럼 manager가 지우지 않는 resource).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportedUsage {
    kind: CacheKind,
    priority: CachePriority,
    bytes: u64,
}

impl ReportedUsage {
    /// 사용량 보고 항목.
    pub const fn new(kind: CacheKind, priority: CachePriority, bytes: u64) -> ReportedUsage {
        ReportedUsage {
            kind,
            priority,
            bytes,
        }
    }

    /// 사용량을 바꿉니다(문서·scene이 바뀔 때 소유자가 갱신).
    pub fn set_bytes(&mut self, bytes: u64) {
        self.bytes = bytes;
    }
}

impl BudgetedCache for ReportedUsage {
    fn kind(&self) -> CacheKind {
        self.kind
    }

    fn usage(&self) -> CacheUsage {
        CacheUsage::of(self.priority, self.bytes)
    }

    fn evict_one(&mut self, _priority: CachePriority) -> u64 {
        0
    }
}

/// 등록한 cache의 번호(등록 순서).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CacheId(u32);

impl CacheId {
    /// 0부터 시작하는 등록 순서.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// 지운 단위 하나의 기록.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Eviction {
    /// 지운 cache.
    pub cache: CacheId,
    /// cache 종류.
    pub kind: CacheKind,
    /// 지운 단위의 priority.
    pub priority: CachePriority,
    /// 줄어든 byte.
    pub bytes: u64,
}

/// 증분 제거 진행 상태. 목표 하나에 하나를 만들어 chunk 사이에 들고 다닙니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvictionPass {
    target: u64,
    /// 이번 pass에서 더 지울 것이 없다고 답한 (cache, priority).
    exhausted: Vec<(CacheId, CachePriority)>,
}

impl EvictionPass {
    /// 전체 사용량을 `target` byte 이하로 줄이는 pass.
    pub fn new(target: u64) -> EvictionPass {
        EvictionPass {
            target,
            exhausted: Vec::new(),
        }
    }

    /// 목표 사용량.
    pub const fn target(&self) -> u64 {
        self.target
    }
}

/// chunk 하나의 결과.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChunkOutcome {
    /// 이번 chunk에서 지운 단위(순서대로).
    pub evicted: Vec<Eviction>,
    /// 이번 chunk에서 줄어든 byte.
    pub freed: u64,
    /// 목표에 닿았거나 더 지울 것이 없어 pass가 끝났으면 `true`.
    pub finished: bool,
    /// 끝난 뒤에도 목표를 넘으면 `true`(지울 수 없는 Document/CurrentScene·보호 단위만 남음).
    pub over_target: bool,
}

/// chunk 하나가 단위를 더 지워도 되는지 정합니다(XVI.5.4 UI thread 시간 예산).
pub trait EvictionDeadline {
    /// 다음 단위를 지워도 되면 `true`. 단위마다 지우기 직전에 부릅니다.
    fn allows_another(&mut self) -> bool;
}

/// 단위 수로 제한하는 deadline(시험·도구용).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitLimit(pub usize);

impl EvictionDeadline for UnitLimit {
    fn allows_another(&mut self) -> bool {
        if self.0 == 0 {
            return false;
        }
        self.0 -= 1;
        true
    }
}

/// 제한 없는 deadline(한 번에 끝까지).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Unbounded;

impl EvictionDeadline for Unbounded {
    fn allows_another(&mut self) -> bool {
        true
    }
}

/// 공통 cache budget manager(XVI.5.3).
#[derive(Default)]
pub struct CacheBudgetManager {
    caches: Vec<Box<dyn BudgetedCache>>,
}

impl std::fmt::Debug for CacheBudgetManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheBudgetManager")
            .field("caches", &self.caches.len())
            .field("usage", &self.usage())
            .finish()
    }
}

impl CacheBudgetManager {
    /// 빈 manager.
    pub fn new() -> CacheBudgetManager {
        CacheBudgetManager::default()
    }

    /// cache를 등록합니다. 이후 사용량 합계와 제거 대상에 들어갑니다.
    pub fn register(&mut self, cache: Box<dyn BudgetedCache>) -> CacheId {
        let id = CacheId(self.caches.len() as u32);
        self.caches.push(cache);
        id
    }

    /// 등록한 cache 수.
    pub fn len(&self) -> usize {
        self.caches.len()
    }

    /// 등록한 cache가 없으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.caches.is_empty()
    }

    /// 등록한 cache를 원래 타입으로 읽습니다.
    pub fn get<T: BudgetedCache>(&self, id: CacheId) -> Option<&T> {
        let cache: &dyn Any = self.caches.get(id.0 as usize)?.as_ref();
        cache.downcast_ref::<T>()
    }

    /// 등록한 cache를 원래 타입으로 바꿉니다(조회·삽입은 cache 자신의 API로).
    pub fn get_mut<T: BudgetedCache>(&mut self, id: CacheId) -> Option<&mut T> {
        let cache: &mut dyn Any = self.caches.get_mut(id.0 as usize)?.as_mut();
        cache.downcast_mut::<T>()
    }

    /// cache 하나의 사용량.
    pub fn usage_of(&self, id: CacheId) -> Option<CacheUsage> {
        self.caches.get(id.0 as usize).map(|cache| cache.usage())
    }

    /// 등록한 모든 cache의 사용량 합(XVI.5.3).
    pub fn usage(&self) -> CacheUsage {
        self.caches
            .iter()
            .fold(CacheUsage::new(), |sum, cache| sum.plus(cache.usage()))
    }

    /// 모든 cache가 지운 뒤 아직 메모리를 돌려주지 않은 값을 모읍니다. 호출자는 이 묶음을 worker thread에서 버립니다
    /// (XVI.5.4: 큰 buffer 해제를 UI thread chunk 밖으로).
    pub fn take_released(&mut self) -> ReleasedResources {
        ReleasedResources(
            self.caches
                .iter_mut()
                .flat_map(|cache| cache.take_released())
                .collect(),
        )
    }

    /// 다음에 지울 (cache, priority). 낮은 priority → 종류 순서 → 등록 순서.
    fn next_victim(&self, pass: &EvictionPass) -> Option<(usize, CachePriority)> {
        for priority in CachePriority::EVICTION_ORDER {
            let mut best: Option<(u8, usize)> = None;
            for (index, cache) in self.caches.iter().enumerate() {
                if cache.usage().get(priority) == 0
                    || pass.exhausted.contains(&(CacheId(index as u32), priority))
                {
                    continue;
                }
                let rank = cache.kind().eviction_rank();
                if best.is_none_or(|(best_rank, _)| rank < best_rank) {
                    best = Some((rank, index));
                }
            }
            if let Some((_, index)) = best {
                return Some((index, priority));
            }
        }
        None
    }

    /// 전체 사용량이 `pass` 목표 이하가 될 때까지 `deadline`이 허락하는 만큼 낮은 priority부터 지웁니다(XVI.5.4).
    ///
    /// Document/CurrentScene은 지우지 않습니다. 목표에 닿거나 지울 것이 더 없으면 `finished`입니다.
    pub fn evict_chunk(
        &mut self,
        pass: &mut EvictionPass,
        deadline: &mut dyn EvictionDeadline,
    ) -> ChunkOutcome {
        let mut outcome = ChunkOutcome::default();
        loop {
            let usage = self.usage().total();
            if usage <= pass.target {
                outcome.finished = true;
                return outcome;
            }
            let Some((index, priority)) = self.next_victim(pass) else {
                outcome.finished = true;
                outcome.over_target = true;
                return outcome;
            };
            if !deadline.allows_another() {
                return outcome;
            }
            let cache = &mut self.caches[index];
            let bytes = cache.evict_one(priority);
            if bytes == 0 {
                pass.exhausted.push((CacheId(index as u32), priority));
                continue;
            }
            outcome.freed = outcome.freed.saturating_add(bytes);
            outcome.evicted.push(Eviction {
                cache: CacheId(index as u32),
                kind: cache.kind(),
                priority,
                bytes,
            });
        }
    }

    /// GPU에 올라가는 cache([`CacheKind::gpu_resident`])의 지울 수 있는 단위를 모두 지우고 줄어든 byte를 돌려줍니다
    /// (XVI.5.6 GPU OOM 직전 purge). Document/CurrentScene과 GPU 밖 cache는 그대로 둡니다. 바로 다시 할당하므로 지운
    /// 값은 미루지 않고 여기서 해제합니다.
    pub fn purge_gpu(&mut self) -> u64 {
        let mut freed = 0u64;
        for priority in CachePriority::EVICTION_ORDER {
            for cache in self
                .caches
                .iter_mut()
                .filter(|cache| cache.kind().gpu_resident())
            {
                loop {
                    let bytes = cache.evict_one(priority);
                    if bytes == 0 {
                        break;
                    }
                    freed = freed.saturating_add(bytes);
                }
            }
        }
        for cache in self
            .caches
            .iter_mut()
            .filter(|cache| cache.kind().gpu_resident())
        {
            drop(cache.take_released());
        }
        freed
    }
}

/// [`with_oom_retry`] 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OomRetry<T> {
    /// 작업 결과(재시도도 실패하면 마지막 오류).
    pub result: Result<T, BackendError>,
    /// GPU OOM 때문에 다시 한 횟수(0 또는 1).
    pub retries: u32,
    /// purge로 줄어든 byte.
    pub purged_bytes: u64,
}

/// GPU 메모리 할당 작업 `operation`을 실행하고, 메모리 부족이면 GPU cache를 비운 뒤 **한 번만** 다시 합니다(XVI.5.6).
///
/// 판단은 [`RecoveryTracker::on_error`]입니다: 첫 메모리 부족은 `PurgeCachesAndRetry`(purge 후 재시도), 그 재시도도
/// 메모리 부족이면 `SkipFrame`(반복하지 않고 오류 반환)입니다. 메모리 부족이 아닌 오류는 재시도하지 않고 그대로
/// 돌려줍니다(그 오류의 복구는 호출자가 같은 tracker로 정함). 성공하면 tracker에 알려 다음 OOM이 다시 한 번의
/// 재시도를 받게 합니다.
pub fn with_oom_retry<T>(
    tracker: &mut RecoveryTracker,
    caches: &mut CacheBudgetManager,
    mut operation: impl FnMut() -> Result<T, BackendError>,
) -> OomRetry<T> {
    let mut retries = 0u32;
    let mut purged_bytes = 0u64;
    loop {
        let error = match operation() {
            Ok(value) => {
                tracker.operation_succeeded();
                return OomRetry {
                    result: Ok(value),
                    retries,
                    purged_bytes,
                };
            }
            Err(error) => error,
        };
        let retry = error.recovery() == Recovery::PurgeCachesAndRetryOnce
            && retries == 0
            && tracker.on_error(&error) == RecoveryAction::PurgeCachesAndRetry;
        if !retry {
            if error.recovery() == Recovery::PurgeCachesAndRetryOnce && retries > 0 {
                // 재시도도 메모리 부족: tracker에 알려 frame을 버리게 합니다(SkipFrame). 다시 반복하지 않습니다.
                tracker.on_error(&error);
            }
            return OomRetry {
                result: Err(error),
                retries,
                purged_bytes,
            };
        }
        purged_bytes = purged_bytes.saturating_add(caches.purge_gpu());
        retries += 1;
    }
}

#[cfg(test)]
mod tests;

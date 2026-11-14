//! glyph cache miss 정책 (XI.6.6).
//!
//! glyph가 cache에 없으면(rasterize·outline 추출이 끝나지 않음) [`GlyphResolver::resolve`]는 다음 순서로
//! 그릴 것을 정합니다. **placeholder(체커·tofu·`.notdef`)는 그리지 않습니다.**
//!
//! 1. **정확한 glyph**([`GlyphDraw::Exact`]): 요청 key의 atlas 배치 또는 outline.
//! 2. **이전 valid glyph**([`GlyphDraw::Substitute`]): 같은 glyph 모양 — 폰트 hash·glyph id·variation·
//!    rendering mode가 모두 같고 **크기만 다른** key 중 지금 cache에 유효하게 있는 것을 크기 비율로 늘이거나
//!    줄여 그립니다(zoom 중 이전 크기 glyph). 요청 크기 이상 중 가장 작은 크기, 없으면 요청 크기 미만 중 가장
//!    큰 크기를 고릅니다(축소가 확대보다 선명함). 다른 폰트·다른 glyph id·다른 variation·다른 mode의 glyph는
//!    절대 대체로 쓰지 않습니다.
//! 3. **빈 glyph**([`GlyphDraw::Empty`]): 대체가 없거나, 잉크가 없는 glyph(공백 등)이거나, rasterize가 실패한
//!    경우입니다. 아무것도 그리지 않습니다(layout advance는 text engine이 이미 정했으므로 줄 배치는 변하지
//!    않음).
//!
//! miss인 key는 rasterize 요청 목록([`GlyphResolver::take_requests`])에 한 번만 들어갑니다. worker 결과는
//! [`GlyphResolver::complete`]로 반영하며, mode와 맞지 않는 결과는 실패로 처리합니다. 대체로 고른 glyph도
//! atlas `lookup`으로 현재 frame 사용 표시가 되므로 그 frame 동안 eviction되지 않습니다(XI.6.5).
//!
//! 모든 내부 집합에는 상한이 있습니다(오래된 항목부터 버림).

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::hash::Hash;

use stellar_scene::geometry::PathGeometry;

use super::{
    check_image_matches_mode, AtlasError, AtlasPlacement, FontHash, GlyphAtlas, GlyphError,
    GlyphId, GlyphImage, GlyphKey, GlyphRenderMode, GlyphSize, VariationCoords,
};

/// 대기 중 rasterize 요청 상한.
pub const MAX_PENDING_GLYPHS: usize = 4096;

/// 실패·무잉크·outline 기록 상한(각각).
pub const MAX_GLYPH_RECORDS: usize = 4096;

/// glyph 모양 하나가 기억하는 크기 수 상한.
pub const MAX_SIZES_PER_SHAPE: usize = 16;

/// 그릴 glyph 원천.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlyphSource {
    /// atlas bitmap.
    Atlas(AtlasPlacement),
    /// pen 원점 기준 px outline(Outline mode).
    Outline(PathGeometry),
}

/// 빈 glyph 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmptyReason {
    /// 잉크가 없는 glyph(정상).
    NoInk,
    /// rasterize 대기 중이고 대체할 이전 valid glyph가 없습니다.
    Pending,
    /// rasterize가 실패했습니다.
    Failed,
}

/// glyph 하나를 이번 frame에 어떻게 그릴지.
#[derive(Debug, Clone, PartialEq)]
pub enum GlyphDraw {
    /// 요청 key 그대로.
    Exact(GlyphSource),
    /// 같은 glyph 모양의 다른 크기를 `scale`배로 그립니다.
    Substitute {
        /// 원천.
        source: GlyphSource,
        /// 원천 크기.
        from: GlyphSize,
        /// 요청 크기 ÷ 원천 크기.
        scale: f32,
    },
    /// 그리지 않습니다.
    Empty(EmptyReason),
}

/// glyph 모양 식별(크기 제외).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ShapeKey {
    font: FontHash,
    glyph: GlyphId,
    variations: VariationCoords,
    mode: GlyphRenderMode,
}

impl ShapeKey {
    fn of(key: &GlyphKey) -> ShapeKey {
        ShapeKey {
            font: key.font,
            glyph: key.glyph,
            variations: key.variations.clone(),
            mode: key.mode,
        }
    }

    fn with_size(&self, size: GlyphSize) -> GlyphKey {
        GlyphKey {
            font: self.font,
            glyph: self.glyph,
            size,
            variations: self.variations.clone(),
            mode: self.mode,
        }
    }
}

/// 삽입 순서로 오래된 것부터 버리는 상한 있는 map.
#[derive(Debug, Clone)]
struct BoundedMap<K, V> {
    map: HashMap<K, V>,
    order: VecDeque<K>,
    limit: usize,
}

impl<K: Clone + Eq + Hash, V> BoundedMap<K, V> {
    fn new(limit: usize) -> BoundedMap<K, V> {
        BoundedMap {
            map: HashMap::new(),
            order: VecDeque::new(),
            limit,
        }
    }

    fn insert(&mut self, key: K, value: V) {
        if self.map.insert(key.clone(), value).is_none() {
            self.order.push_back(key);
            while self.map.len() > self.limit {
                match self.order.pop_front() {
                    Some(old) => {
                        self.map.remove(&old);
                    }
                    None => break,
                }
            }
        }
    }

    fn get(&self, key: &K) -> Option<&V> {
        self.map.get(key)
    }

    fn contains(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    fn len(&self) -> usize {
        self.map.len()
    }
}

/// 통계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResolverStats {
    /// 정확한 glyph.
    pub exact: u64,
    /// 이전 valid glyph 대체.
    pub substituted: u64,
    /// 빈 glyph(대기·실패·무잉크).
    pub empty: u64,
    /// rasterize 요청 수.
    pub requested: u64,
}

/// [`GlyphResolver::complete`] 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompleteOutcome {
    /// atlas에 배치했습니다.
    Placed(AtlasPlacement),
    /// outline을 저장했습니다.
    OutlineStored,
    /// 잉크 없는 glyph로 기록했습니다.
    NoInk,
    /// 실패로 기록했습니다(이후 resolve는 빈 glyph).
    Failed(GlyphError),
    /// atlas에 공간이 없어 버렸습니다(다음 resolve에서 다시 요청).
    Deferred,
}

/// glyph cache miss 정책 관리자(모듈 문서).
#[derive(Debug, Clone)]
pub struct GlyphResolver {
    pending: HashSet<GlyphKey>,
    requests: Vec<GlyphKey>,
    sizes: BoundedMap<ShapeKey, BTreeSet<GlyphSize>>,
    outlines: BoundedMap<GlyphKey, PathGeometry>,
    no_ink: BoundedMap<GlyphKey, ()>,
    failed: BoundedMap<GlyphKey, GlyphError>,
    stats: ResolverStats,
}

impl Default for GlyphResolver {
    fn default() -> GlyphResolver {
        GlyphResolver::new()
    }
}

impl GlyphResolver {
    /// 빈 관리자.
    pub fn new() -> GlyphResolver {
        GlyphResolver {
            pending: HashSet::new(),
            requests: Vec::new(),
            sizes: BoundedMap::new(MAX_GLYPH_RECORDS),
            outlines: BoundedMap::new(MAX_GLYPH_RECORDS),
            no_ink: BoundedMap::new(MAX_GLYPH_RECORDS),
            failed: BoundedMap::new(MAX_GLYPH_RECORDS),
            stats: ResolverStats::default(),
        }
    }

    /// 통계.
    pub fn stats(&self) -> ResolverStats {
        self.stats
    }

    /// rasterize 대기 중인 key 수.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// 새로 생긴 rasterize 요청을 꺼냅니다(worker에 보낼 목록).
    pub fn take_requests(&mut self) -> Vec<GlyphKey> {
        std::mem::take(&mut self.requests)
    }

    /// 실패 기록을 지웁니다(폰트 재등록 등으로 다시 시도할 때).
    pub fn forget_failures(&mut self) {
        self.failed = BoundedMap::new(MAX_GLYPH_RECORDS);
    }

    fn exact(&mut self, atlas: &mut dyn GlyphAtlas, key: &GlyphKey) -> Option<GlyphSource> {
        if key.mode == GlyphRenderMode::Outline {
            self.outlines.get(key).cloned().map(GlyphSource::Outline)
        } else {
            atlas.lookup(key).map(GlyphSource::Atlas)
        }
    }

    fn request(&mut self, key: &GlyphKey) {
        if self.pending.len() < MAX_PENDING_GLYPHS && self.pending.insert(key.clone()) {
            self.requests.push(key.clone());
            self.stats.requested += 1;
        }
    }

    /// 같은 모양의 다른 크기 중 지금 유효한 것을 고릅니다(모듈 문서 순서).
    fn substitute(
        &mut self,
        atlas: &mut dyn GlyphAtlas,
        key: &GlyphKey,
    ) -> Option<(GlyphSource, GlyphSize)> {
        let shape = ShapeKey::of(key);
        let sizes: Vec<GlyphSize> = self.sizes.get(&shape)?.iter().copied().collect();
        let (larger, smaller): (Vec<GlyphSize>, Vec<GlyphSize>) = sizes
            .into_iter()
            .filter(|size| *size != key.size)
            .partition(|size| *size > key.size);
        // 요청 이상 중 가장 작은 것부터, 그다음 요청 미만 중 가장 큰 것부터.
        let candidates = larger.into_iter().chain(smaller.into_iter().rev());
        let mut gone = Vec::new();
        let mut found = None;
        for size in candidates {
            let candidate = shape.with_size(size);
            match self.exact(atlas, &candidate) {
                Some(source) => {
                    found = Some((source, size));
                    break;
                }
                None => gone.push(size),
            }
        }
        // eviction된 크기는 기록에서 지웁니다.
        if !gone.is_empty() {
            if let Some(set) = self.sizes.map.get_mut(&shape) {
                for size in gone {
                    set.remove(&size);
                }
            }
        }
        found
    }

    /// 이번 frame에 `key`를 어떻게 그릴지 정합니다(모듈 문서).
    pub fn resolve(&mut self, atlas: &mut dyn GlyphAtlas, key: &GlyphKey) -> GlyphDraw {
        if self.no_ink.contains(key) {
            self.stats.empty += 1;
            return GlyphDraw::Empty(EmptyReason::NoInk);
        }
        if let Some(source) = self.exact(atlas, key) {
            self.stats.exact += 1;
            return GlyphDraw::Exact(source);
        }
        let failed = self.failed.contains(key);
        if !failed {
            self.request(key);
        }
        if let Some((source, from)) = self.substitute(atlas, key) {
            self.stats.substituted += 1;
            return GlyphDraw::Substitute {
                source,
                from,
                scale: key.size.px() / from.px(),
            };
        }
        self.stats.empty += 1;
        GlyphDraw::Empty(if failed {
            EmptyReason::Failed
        } else {
            EmptyReason::Pending
        })
    }

    fn remember_size(&mut self, key: &GlyphKey) {
        let shape = ShapeKey::of(key);
        let target = key.size.sixty_fourths();
        let trim = |set: &mut BTreeSet<GlyphSize>| {
            while set.len() > MAX_SIZES_PER_SHAPE {
                // 요청 크기에서 가장 먼 끝을 버립니다.
                let (Some(first), Some(last)) = (set.first().copied(), set.last().copied()) else {
                    break;
                };
                let far = if target.abs_diff(first.sixty_fourths())
                    >= target.abs_diff(last.sixty_fourths())
                {
                    first
                } else {
                    last
                };
                set.remove(&far);
            }
        };
        if let Some(set) = self.sizes.map.get_mut(&shape) {
            set.insert(key.size);
            trim(set);
            return;
        }
        let mut set = BTreeSet::new();
        set.insert(key.size);
        self.sizes.insert(shape, set);
    }

    /// worker의 rasterize 결과를 반영합니다.
    pub fn complete(
        &mut self,
        atlas: &mut dyn GlyphAtlas,
        key: GlyphKey,
        result: Result<GlyphImage, GlyphError>,
    ) -> CompleteOutcome {
        self.pending.remove(&key);
        let image = match result.and_then(|image| {
            check_image_matches_mode(&key, &image)?;
            Ok(image)
        }) {
            Ok(image) => image,
            Err(error) => {
                self.failed.insert(key, error);
                return CompleteOutcome::Failed(error);
            }
        };
        match image {
            GlyphImage::Empty => {
                self.no_ink.insert(key, ());
                CompleteOutcome::NoInk
            }
            GlyphImage::Outline(path) => {
                self.remember_size(&key);
                self.outlines.insert(key, path);
                CompleteOutcome::OutlineStored
            }
            GlyphImage::Bitmap(bitmap) => match atlas.insert(key.clone(), &bitmap) {
                Ok(placement) => {
                    self.remember_size(&key);
                    CompleteOutcome::Placed(placement)
                }
                Err(AtlasError::Full) => CompleteOutcome::Deferred,
                Err(AtlasError::TooLarge { .. }) => {
                    let error = GlyphError::Rasterize {
                        reason: "bitmap이 atlas page보다 큽니다(Outline mode 필요)",
                    };
                    self.failed.insert(key, error);
                    CompleteOutcome::Failed(error)
                }
                Err(_) => {
                    let error = GlyphError::Rasterize {
                        reason: "atlas 배치 실패",
                    };
                    self.failed.insert(key, error);
                    CompleteOutcome::Failed(error)
                }
            },
        }
    }

    /// 기억하는 outline 수(진단용).
    pub fn outline_count(&self) -> usize {
        self.outlines.len()
    }
}

#[cfg(test)]
mod tests;

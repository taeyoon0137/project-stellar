//! node별 layout input hash와 output cache (X.1.4).
//!
//! - [`LayoutInputHash`]: node 하나의 layout 입력(크기 모드·제약·자식 결과 등)을 요약한 64비트
//!   값입니다. [`LayoutInputHasher`]가 고정 알고리즘(FNV-1a 64)과 canonical little-endian
//!   바이트로 계산하므로 같은 입력은 플랫폼·실행과 무관하게 같은 hash를 만듭니다.
//! - [`LayoutCache`]: node([`StableId`])별로 마지막 입력 hash, 계산한 [`LayoutRevision`],
//!   output을 보관합니다. 같은 입력 hash로 조회하면 cache hit입니다.
//!
//! 이 hash는 **런타임 cache key 전용**이며 저장하지 않습니다. 저장용 콘텐츠 해시(V.1.5
//! `ContentHash`)를 대신하지 않습니다. output 타입은 generic이라 `LayoutResult`(X.1.1)가
//! 정의되면 그대로 담을 수 있습니다.

use std::collections::HashMap;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Length};

use crate::revision::LayoutRevision;

/// node 하나의 layout 입력을 요약한 64비트 hash (X.1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayoutInputHash(u64);

impl LayoutInputHash {
    /// raw 64비트 값으로부터 만듭니다(테스트·진단용).
    pub const fn from_raw(raw: u64) -> LayoutInputHash {
        LayoutInputHash(raw)
    }

    /// raw 64비트 값.
    pub const fn raw(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for LayoutInputHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// 결정적 layout input hasher (FNV-1a 64).
///
/// 모든 다중 바이트 값은 little-endian으로 공급하고, float은 [`FiniteF32`] 비트(`-0.0`은 `+0.0`
/// 으로 정규화됨)를 사용합니다. 가변 길이 바이트열은 `u64` LE 길이 프리픽스를 붙여 경계를
/// 모호하지 않게 합니다. 필드 순서가 다르면 다른 hash가 됩니다(순서는 호출자가 고정).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutInputHasher {
    state: u64,
}

impl LayoutInputHasher {
    /// FNV-1a 64 offset basis.
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    /// FNV-1a 64 prime.
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    /// 빈 hasher를 만듭니다.
    pub const fn new() -> LayoutInputHasher {
        LayoutInputHasher {
            state: LayoutInputHasher::OFFSET_BASIS,
        }
    }

    /// 바이트를 그대로(프리픽스 없이) 섞습니다.
    fn mix(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.state ^= u64::from(*byte);
            self.state = self.state.wrapping_mul(LayoutInputHasher::PRIME);
        }
    }

    /// `u8` 하나를 섞습니다(필드 tag·enum 판별자 용도).
    pub fn write_u8(&mut self, value: u8) -> &mut LayoutInputHasher {
        self.mix(&[value]);
        self
    }

    /// 불리언을 `0x00`/`0x01` 1바이트로 섞습니다.
    pub fn write_bool(&mut self, value: bool) -> &mut LayoutInputHasher {
        self.write_u8(u8::from(value))
    }

    /// `u32`를 little-endian 4바이트로 섞습니다.
    pub fn write_u32(&mut self, value: u32) -> &mut LayoutInputHasher {
        self.mix(&value.to_le_bytes());
        self
    }

    /// `u64`를 little-endian 8바이트로 섞습니다.
    pub fn write_u64(&mut self, value: u64) -> &mut LayoutInputHasher {
        self.mix(&value.to_le_bytes());
        self
    }

    /// `i32`를 little-endian 4바이트(2의 보수)로 섞습니다.
    pub fn write_i32(&mut self, value: i32) -> &mut LayoutInputHasher {
        self.mix(&value.to_le_bytes());
        self
    }

    /// 유한 float을 little-endian 비트로 섞습니다(`-0.0`은 이미 `+0.0`).
    pub fn write_finite_f32(&mut self, value: FiniteF32) -> &mut LayoutInputHasher {
        self.mix(&value.to_le_bytes());
        self
    }

    /// 길이 값을 유한 float 비트로 섞습니다.
    pub fn write_length(&mut self, value: Length) -> &mut LayoutInputHasher {
        self.write_finite_f32(value.raw())
    }

    /// `StableId` 16바이트를 섞습니다.
    pub fn write_stable_id(&mut self, id: StableId) -> &mut LayoutInputHasher {
        self.mix(&id.to_bytes());
        self
    }

    /// 가변 길이 바이트열을 `u64` LE 길이 프리픽스 + bytes로 섞습니다.
    pub fn write_bytes(&mut self, bytes: &[u8]) -> &mut LayoutInputHasher {
        // usize → u64는 지원 플랫폼(64비트)에서 손실이 없습니다.
        self.write_u64(bytes.len() as u64);
        self.mix(bytes);
        self
    }

    /// 다른 input hash(예: 자식 결과 요약)를 섞습니다.
    pub fn write_hash(&mut self, hash: LayoutInputHash) -> &mut LayoutInputHasher {
        self.write_u64(hash.raw())
    }

    /// 지금까지 섞은 입력의 hash.
    pub const fn finish(&self) -> LayoutInputHash {
        LayoutInputHash(self.state)
    }
}

impl Default for LayoutInputHasher {
    fn default() -> LayoutInputHasher {
        LayoutInputHasher::new()
    }
}

/// cache에 저장된 node 하나의 layout output (X.1.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutCacheEntry<T> {
    input_hash: LayoutInputHash,
    revision: LayoutRevision,
    output: T,
}

impl<T> LayoutCacheEntry<T> {
    /// output을 계산한 입력 hash.
    pub const fn input_hash(&self) -> LayoutInputHash {
        self.input_hash
    }

    /// output을 계산한 layout revision.
    pub const fn revision(&self) -> LayoutRevision {
        self.revision
    }

    /// 캐시된 output.
    pub const fn output(&self) -> &T {
        &self.output
    }

    /// output을 소유권과 함께 꺼냅니다.
    pub fn into_output(self) -> T {
        self.output
    }
}

/// cache miss 사유 (X.1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMissReason {
    /// 해당 node의 entry가 없습니다.
    NotCached,
    /// entry는 있으나 입력 hash가 다릅니다(입력 변경).
    InputChanged {
        /// 캐시된 entry의 입력 hash.
        cached: LayoutInputHash,
    },
}

/// cache 조회 결과 (X.1.4).
#[derive(Debug, PartialEq, Eq)]
pub enum CacheLookup<'a, T> {
    /// 같은 입력 hash의 entry가 있습니다.
    Hit(&'a LayoutCacheEntry<T>),
    /// 재계산이 필요합니다.
    Miss(CacheMissReason),
}

impl<'a, T> CacheLookup<'a, T> {
    /// hit이면 `true`.
    pub const fn is_hit(&self) -> bool {
        matches!(self, CacheLookup::Hit(_))
    }

    /// hit이면 캐시된 output.
    pub fn output(&self) -> Option<&'a T> {
        match self {
            CacheLookup::Hit(entry) => Some(entry.output()),
            CacheLookup::Miss(_) => None,
        }
    }
}

/// cache hit/miss 집계(벤치마크·진단용). 값은 포화 증가합니다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LayoutCacheStats {
    /// hit 횟수.
    pub hits: u64,
    /// miss 횟수.
    pub misses: u64,
}

/// layout cache 오류 (X.1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutCacheError {
    /// 기존 entry보다 오래된 revision의 output을 저장하려 했습니다.
    RevisionRegressed {
        /// 대상 node.
        node: StableId,
        /// 기존 entry의 revision.
        cached: LayoutRevision,
        /// 저장하려던 revision.
        attempted: LayoutRevision,
    },
}

impl fmt::Display for LayoutCacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutCacheError::RevisionRegressed {
                node,
                cached,
                attempted,
            } => write!(
                f,
                "layout cache revision이 역행합니다: node {node}, 기존 {cached}, 시도 {attempted}"
            ),
        }
    }
}

impl std::error::Error for LayoutCacheError {}

/// node별 layout output cache (X.1.4).
///
/// node마다 entry 하나(마지막으로 계산한 입력 hash·revision·output)를 보관합니다. 조회는 입력
/// hash가 같을 때만 hit입니다. 순회 API는 제공하지 않으므로 내부 map 순서가 결과에 영향을
/// 주지 않습니다.
#[derive(Debug, Clone)]
pub struct LayoutCache<T> {
    entries: HashMap<StableId, LayoutCacheEntry<T>>,
    stats: LayoutCacheStats,
}

impl<T> LayoutCache<T> {
    /// 빈 cache를 만듭니다.
    pub fn new() -> LayoutCache<T> {
        LayoutCache {
            entries: HashMap::new(),
            stats: LayoutCacheStats::default(),
        }
    }

    /// node의 output을 입력 hash로 조회하고 hit/miss를 집계합니다.
    pub fn lookup(&mut self, node: StableId, input_hash: LayoutInputHash) -> CacheLookup<'_, T> {
        match self.entries.get(&node) {
            Some(entry) if entry.input_hash == input_hash => {
                self.stats.hits = self.stats.hits.saturating_add(1);
                CacheLookup::Hit(entry)
            }
            Some(entry) => {
                self.stats.misses = self.stats.misses.saturating_add(1);
                CacheLookup::Miss(CacheMissReason::InputChanged {
                    cached: entry.input_hash,
                })
            }
            None => {
                self.stats.misses = self.stats.misses.saturating_add(1);
                CacheLookup::Miss(CacheMissReason::NotCached)
            }
        }
    }

    /// 집계 없이 node의 entry를 읽습니다.
    pub fn peek(&self, node: StableId) -> Option<&LayoutCacheEntry<T>> {
        self.entries.get(&node)
    }

    /// node의 output을 저장하고 이전 entry를 반환합니다.
    ///
    /// 기존 entry보다 오래된 `revision`은 [`LayoutCacheError::RevisionRegressed`]로 거부하며,
    /// 이때 cache는 바뀌지 않습니다. 같은 revision의 재저장은 허용합니다.
    pub fn insert(
        &mut self,
        node: StableId,
        input_hash: LayoutInputHash,
        revision: LayoutRevision,
        output: T,
    ) -> Result<Option<LayoutCacheEntry<T>>, LayoutCacheError> {
        if let Some(existing) = self.entries.get(&node) {
            if revision < existing.revision {
                return Err(LayoutCacheError::RevisionRegressed {
                    node,
                    cached: existing.revision,
                    attempted: revision,
                });
            }
        }
        Ok(self.entries.insert(
            node,
            LayoutCacheEntry {
                input_hash,
                revision,
                output,
            },
        ))
    }

    /// node의 entry를 제거하고 반환합니다.
    pub fn invalidate(&mut self, node: StableId) -> Option<LayoutCacheEntry<T>> {
        self.entries.remove(&node)
    }

    /// 모든 entry를 제거합니다(집계는 유지).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// entry 수.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 비어 있는지 여부.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 지금까지의 hit/miss 집계.
    pub const fn stats(&self) -> LayoutCacheStats {
        self.stats
    }
}

impl<T> Default for LayoutCache<T> {
    fn default() -> LayoutCache<T> {
        LayoutCache::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(seed: u64) -> StableId {
        StableId::new_v7(seed, 0, seed)
    }

    /// 테스트용 layout 입력(고정 폭·높이·자식 수).
    fn input_hash(width: f32, height: f32, children: u32) -> LayoutInputHash {
        let mut hasher = LayoutInputHasher::new();
        hasher
            .write_u8(0x01)
            .write_length(Length::new(width).unwrap())
            .write_length(Length::new(height).unwrap())
            .write_u32(children);
        hasher.finish()
    }

    #[test]
    fn same_input_hash_hits_cache() {
        let mut cache = LayoutCache::new();
        let id = node(1);
        let first = input_hash(120.0, 40.0, 3);
        cache
            .insert(id, first, LayoutRevision(1), "measured")
            .unwrap();

        // 같은 입력으로 다시 계산한 hash → 같은 값 → hit.
        let again = input_hash(120.0, 40.0, 3);
        assert_eq!(first, again);
        let lookup = cache.lookup(id, again);
        assert!(lookup.is_hit());
        assert_eq!(lookup.output(), Some(&"measured"));
        match lookup {
            CacheLookup::Hit(entry) => {
                assert_eq!(entry.input_hash(), first);
                assert_eq!(entry.revision(), LayoutRevision(1));
            }
            CacheLookup::Miss(reason) => panic!("hit이어야 함: {reason:?}"),
        }
        assert_eq!(cache.stats(), LayoutCacheStats { hits: 1, misses: 0 });
    }

    #[test]
    fn changed_input_misses_with_cached_hash() {
        let mut cache = LayoutCache::new();
        let id = node(2);
        let old = input_hash(120.0, 40.0, 3);
        cache.insert(id, old, LayoutRevision(1), 7u32).unwrap();

        let new = input_hash(121.0, 40.0, 3);
        assert_ne!(old, new);
        assert_eq!(
            cache.lookup(id, new),
            CacheLookup::Miss(CacheMissReason::InputChanged { cached: old })
        );
        assert_eq!(cache.stats(), LayoutCacheStats { hits: 0, misses: 1 });
    }

    #[test]
    fn unknown_node_misses_not_cached() {
        let mut cache: LayoutCache<u32> = LayoutCache::new();
        assert_eq!(
            cache.lookup(node(3), input_hash(1.0, 1.0, 0)),
            CacheLookup::Miss(CacheMissReason::NotCached)
        );
        assert!(cache.is_empty());
    }

    #[test]
    fn entries_are_per_node() {
        let mut cache = LayoutCache::new();
        let hash = input_hash(10.0, 10.0, 0);
        cache.insert(node(4), hash, LayoutRevision(1), 'a').unwrap();
        // 같은 hash라도 다른 node는 entry가 없음.
        assert_eq!(
            cache.lookup(node(5), hash),
            CacheLookup::Miss(CacheMissReason::NotCached)
        );
        assert_eq!(cache.lookup(node(4), hash).output(), Some(&'a'));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn invalidate_and_clear_remove_entries() {
        let mut cache = LayoutCache::new();
        let hash = input_hash(10.0, 10.0, 0);
        cache.insert(node(6), hash, LayoutRevision(1), 1u8).unwrap();
        cache.insert(node(7), hash, LayoutRevision(1), 2u8).unwrap();

        let removed = cache.invalidate(node(6)).expect("entry 존재");
        assert_eq!(removed.into_output(), 1u8);
        assert_eq!(
            cache.lookup(node(6), hash),
            CacheLookup::Miss(CacheMissReason::NotCached)
        );
        cache.clear();
        assert!(cache.is_empty());
        assert!(cache.peek(node(7)).is_none());
    }

    #[test]
    fn insert_replaces_and_returns_previous() {
        let mut cache = LayoutCache::new();
        let id = node(8);
        let a = input_hash(1.0, 1.0, 0);
        let b = input_hash(2.0, 1.0, 0);
        assert_eq!(cache.insert(id, a, LayoutRevision(1), 10i32), Ok(None));
        let previous = cache.insert(id, b, LayoutRevision(2), 20i32).unwrap();
        assert_eq!(previous.map(LayoutCacheEntry::into_output), Some(10));
        assert_eq!(
            cache.peek(id).map(LayoutCacheEntry::revision),
            Some(LayoutRevision(2))
        );
        // 같은 revision 재저장은 허용.
        assert!(cache.insert(id, b, LayoutRevision(2), 21i32).is_ok());
    }

    #[test]
    fn insert_rejects_revision_regression_without_change() {
        let mut cache = LayoutCache::new();
        let id = node(9);
        let hash = input_hash(5.0, 5.0, 1);
        cache.insert(id, hash, LayoutRevision(5), "new").unwrap();
        let err = cache
            .insert(id, input_hash(6.0, 5.0, 1), LayoutRevision(4), "old")
            .unwrap_err();
        assert_eq!(
            err,
            LayoutCacheError::RevisionRegressed {
                node: id,
                cached: LayoutRevision(5),
                attempted: LayoutRevision(4),
            }
        );
        assert!(err.to_string().contains("역행"));
        // 거부 시 기존 entry 유지.
        assert_eq!(cache.lookup(id, hash).output(), Some(&"new"));
    }

    #[test]
    fn hasher_matches_fnv1a_64_vectors() {
        // FNV-1a 64 공개 test vector. 알고리즘 고정 golden.
        assert_eq!(
            LayoutInputHasher::new().finish().raw(),
            0xcbf2_9ce4_8422_2325
        );
        let mut a = LayoutInputHasher::new();
        a.write_u8(b'a');
        assert_eq!(a.finish().raw(), 0xaf63_dc4c_8601_ec8c);
        let mut foobar = LayoutInputHasher::new();
        for byte in b"foobar" {
            foobar.write_u8(*byte);
        }
        assert_eq!(foobar.finish().raw(), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn hasher_is_deterministic_and_order_sensitive() {
        assert_eq!(input_hash(3.0, 4.0, 2), input_hash(3.0, 4.0, 2));
        // 폭·높이 순서가 바뀌면 다른 hash.
        assert_ne!(input_hash(3.0, 4.0, 2), input_hash(4.0, 3.0, 2));
    }

    #[test]
    fn hasher_treats_negative_zero_as_zero() {
        assert_eq!(input_hash(-0.0, 1.0, 0), input_hash(0.0, 1.0, 0));
    }

    #[test]
    fn hasher_length_prefix_disambiguates_bytes() {
        let mut left = LayoutInputHasher::new();
        left.write_bytes(b"ab").write_bytes(b"c");
        let mut right = LayoutInputHasher::new();
        right.write_bytes(b"a").write_bytes(b"bc");
        assert_ne!(left.finish(), right.finish());
    }

    #[test]
    fn hasher_composes_child_hashes_and_ids() {
        let child = input_hash(10.0, 20.0, 0);
        let mut parent_a = LayoutInputHasher::new();
        parent_a
            .write_stable_id(node(10))
            .write_hash(child)
            .write_bool(true)
            .write_i32(-3);
        let mut parent_b = parent_a.clone();
        assert_eq!(parent_a.finish(), parent_b.finish());
        parent_b.write_u8(0);
        assert_ne!(parent_a.finish(), parent_b.finish());
        assert_eq!(LayoutInputHasher::default(), LayoutInputHasher::new());
        assert_eq!(
            LayoutInputHash::from_raw(0xab).to_string(),
            "00000000000000ab"
        );
    }
}

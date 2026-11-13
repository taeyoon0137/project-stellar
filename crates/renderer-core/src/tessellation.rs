//! Shape/Path fill tessellation interface와 cache key (XI.6.1).
//!
//! - **입력**: Scene의 [`PathGeometry`]와 [`FillStyle`]. Shape는 document 단계의 순수 평가
//!   (VI.5.6)로 path command가 된 뒤 Scene에 들어오므로 Shape·Path가 같은 interface를 씁니다.
//! - **출력**: [`FillMesh`] — path local 좌표의 삼각형 목록.
//! - **cache key**: [`TessellationKey`] = path canonical bytes([`PathKey`]) + mesh에 영향을 주는
//!   style 값([`TessellationStyle`]). paint(색상·gradient)·opacity·transform은 mesh를 바꾸지 않으므로
//!   key에 넣지 않습니다. 같은 path/style이면 Shape든 Path든, 서로 다른 node든 같은 entry를
//!   공유합니다.
//! - 실제 tessellation 알고리즘은 [`FillTessellator`] 구현이 담당합니다. stroke(XI.6.2),
//!   worker pool 실행과 stale revision 폐기(XI.6.3), 예산 기반 eviction(XVI.5)은 후속 작업입니다.

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use stellar_scene::geometry::{FillRule, PathGeometry};
use stellar_scene::Vec2;

use crate::digest::fnv1a64;

/// tessellation 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TessellationError {
    /// 평탄화 허용 오차가 유한한 양수가 아닙니다.
    InvalidTolerance,
    /// tessellator 출력 mesh가 계약을 위반합니다.
    InvalidMesh {
        /// 원인.
        reason: &'static str,
    },
    /// tessellator가 입력을 처리하지 못했습니다.
    Failed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for TessellationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TessellationError::InvalidTolerance => {
                f.write_str("평탄화 허용 오차는 유한한 양수여야 합니다")
            }
            TessellationError::InvalidMesh { reason } => {
                write!(f, "tessellation mesh 계약 위반: {reason}")
            }
            TessellationError::Failed { reason } => write!(f, "tessellation 실패: {reason}"),
        }
    }
}

impl std::error::Error for TessellationError {}

/// 곡선 평탄화 허용 오차(path local 단위). 유한한 양수만 허용합니다.
///
/// bits로 보관하며, 생성 시 NaN·무한대·0 이하를 거부하므로 같은 값은 항상 같은 bits입니다
/// (`Eq`/`Hash`가 cache key에 안전).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlattenTolerance {
    bits: u32,
}

impl FlattenTolerance {
    /// 기본 허용 오차 0.25.
    pub const DEFAULT: FlattenTolerance = FlattenTolerance {
        bits: 0.25f32.to_bits(),
    };

    /// 유한한 양수 허용 오차를 만듭니다.
    pub fn new(value: f32) -> Result<FlattenTolerance, TessellationError> {
        if value.is_finite() && value > 0.0 {
            Ok(FlattenTolerance {
                bits: value.to_bits(),
            })
        } else {
            Err(TessellationError::InvalidTolerance)
        }
    }

    /// 허용 오차 값.
    pub fn get(&self) -> f32 {
        f32::from_bits(self.bits)
    }
}

impl Default for FlattenTolerance {
    fn default() -> FlattenTolerance {
        FlattenTolerance::DEFAULT
    }
}

/// fill tessellation에 영향을 주는 style.
///
/// paint(색상·gradient)·opacity는 mesh를 바꾸지 않으므로 여기에 두지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FillStyle {
    /// 채우기 규칙.
    pub fill_rule: FillRule,
    /// 곡선 평탄화 허용 오차.
    pub tolerance: FlattenTolerance,
}

/// cache key의 style 부분. stroke(XI.6.2)는 variant를 추가해 같은 cache를 공유합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TessellationStyle {
    /// fill tessellation.
    Fill(FillStyle),
}

/// path geometry key: canonical bytes와 그 FNV-1a 64 digest.
///
/// `Hash`는 digest만 쓰고, `Eq`는 digest와 **원본 bytes**를 모두 비교하므로 digest 충돌이
/// 있어도 서로 다른 path가 같은 entry로 hit되지 않습니다.
#[derive(Debug, Clone)]
pub struct PathKey {
    digest: u64,
    bytes: Arc<[u8]>,
}

impl PathKey {
    /// geometry의 canonical bytes로 key를 만듭니다.
    pub fn new(path: &PathGeometry) -> PathKey {
        let bytes = path.canonical_bytes();
        PathKey {
            digest: fnv1a64(&bytes),
            bytes: bytes.into(),
        }
    }

    /// FNV-1a 64 digest.
    pub fn digest(&self) -> u64 {
        self.digest
    }

    /// canonical bytes 길이.
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }
}

impl PartialEq for PathKey {
    fn eq(&self, other: &PathKey) -> bool {
        self.digest == other.digest
            && (Arc::ptr_eq(&self.bytes, &other.bytes) || self.bytes == other.bytes)
    }
}

impl Eq for PathKey {}

impl Hash for PathKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // 같은 bytes → 같은 digest이므로 Eq와 일관됩니다.
        self.digest.hash(state);
    }
}

/// tessellation cache key = path + style.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TessellationKey {
    path: PathKey,
    style: TessellationStyle,
}

impl TessellationKey {
    /// fill tessellation key.
    pub fn fill(path: &PathGeometry, style: FillStyle) -> TessellationKey {
        TessellationKey {
            path: PathKey::new(path),
            style: TessellationStyle::Fill(style),
        }
    }

    /// path 부분.
    pub fn path(&self) -> &PathKey {
        &self.path
    }

    /// style 부분.
    pub fn style(&self) -> TessellationStyle {
        self.style
    }
}

/// fill tessellation 결과: path local 좌표 정점과 삼각형 index(3개씩).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FillMesh {
    vertices: Vec<Vec2>,
    indices: Vec<u32>,
}

impl FillMesh {
    /// mesh를 검증해 만듭니다. index 개수는 3의 배수, 모든 index는 정점 범위 안이어야 합니다.
    pub fn new(vertices: Vec<Vec2>, indices: Vec<u32>) -> Result<FillMesh, TessellationError> {
        if !indices.len().is_multiple_of(3) {
            return Err(TessellationError::InvalidMesh {
                reason: "index 개수가 3의 배수가 아닙니다",
            });
        }
        if indices
            .iter()
            .any(|&index| usize::try_from(index).map_or(true, |i| i >= vertices.len()))
        {
            return Err(TessellationError::InvalidMesh {
                reason: "index가 정점 범위를 벗어납니다",
            });
        }
        Ok(FillMesh { vertices, indices })
    }

    /// 빈 mesh(빈 path·면적 0).
    pub fn empty() -> FillMesh {
        FillMesh::default()
    }

    /// 정점.
    pub fn vertices(&self) -> &[Vec2] {
        &self.vertices
    }

    /// 삼각형 index.
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// 삼각형 개수.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// fill tessellation interface.
///
/// 구현은 같은 입력에 같은 mesh를 반환해야 하며(결정론), worker thread에서 호출될 수 있도록
/// `Send + Sync`입니다. 입력이 잘못되어도 panic하지 않고 오류를 반환해야 합니다.
pub trait FillTessellator: Send + Sync {
    /// `path`를 `style`로 채우는 삼각형 mesh를 만듭니다.
    fn tessellate_fill(
        &self,
        path: &PathGeometry,
        style: &FillStyle,
    ) -> Result<FillMesh, TessellationError>;
}

/// cache 조회 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheOutcome {
    /// 기존 entry 재사용.
    Hit,
    /// 새로 tessellation해 저장.
    Miss,
}

/// cache에서 얻은 mesh와 조회 결과.
#[derive(Debug, Clone)]
pub struct CachedMesh {
    /// 공유 mesh.
    pub mesh: Arc<FillMesh>,
    /// hit/miss.
    pub outcome: CacheOutcome,
}

/// cache 통계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheStats {
    /// hit 횟수.
    pub hits: u64,
    /// miss 횟수.
    pub misses: u64,
    /// 저장된 entry 수.
    pub entries: usize,
}

/// tessellation 결과 cache.
///
/// 실패한 tessellation은 저장하지 않습니다(다음 조회에서 재시도).
#[derive(Debug, Default)]
pub struct TessellationCache {
    entries: HashMap<TessellationKey, Arc<FillMesh>>,
    hits: u64,
    misses: u64,
}

impl TessellationCache {
    /// 빈 cache.
    pub fn new() -> TessellationCache {
        TessellationCache::default()
    }

    /// key로 조회하고 hit/miss를 기록합니다.
    pub fn lookup(&mut self, key: &TessellationKey) -> Option<Arc<FillMesh>> {
        match self.entries.get(key) {
            Some(mesh) => {
                self.hits = self.hits.saturating_add(1);
                Some(Arc::clone(mesh))
            }
            None => {
                self.misses = self.misses.saturating_add(1);
                None
            }
        }
    }

    /// mesh를 저장하고 공유 handle을 반환합니다. 같은 key가 있으면 교체합니다.
    pub fn insert(&mut self, key: TessellationKey, mesh: FillMesh) -> Arc<FillMesh> {
        let mesh = Arc::new(mesh);
        self.entries.insert(key, Arc::clone(&mesh));
        mesh
    }

    /// fill mesh를 조회하고, 없으면 `tessellator`로 만들어 저장합니다.
    pub fn fill(
        &mut self,
        path: &PathGeometry,
        style: FillStyle,
        tessellator: &dyn FillTessellator,
    ) -> Result<CachedMesh, TessellationError> {
        let key = TessellationKey::fill(path, style);
        if let Some(mesh) = self.lookup(&key) {
            return Ok(CachedMesh {
                mesh,
                outcome: CacheOutcome::Hit,
            });
        }
        let mesh = tessellator.tessellate_fill(path, &style)?;
        Ok(CachedMesh {
            mesh: self.insert(key, mesh),
            outcome: CacheOutcome::Miss,
        })
    }

    /// entry를 제거합니다.
    pub fn remove(&mut self, key: &TessellationKey) -> Option<Arc<FillMesh>> {
        self.entries.remove(key)
    }

    /// 모든 entry를 제거합니다(통계는 유지).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// entry 수.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// entry가 없으면 참입니다.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 통계.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            entries: self.entries.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use stellar_scene::geometry::PathCommand;

    fn p(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y).unwrap()
    }

    /// 호출 횟수를 세는 테스트 tessellator: subpath anchor를 fan 삼각형으로 나눕니다.
    #[derive(Default)]
    struct CountingFan {
        calls: AtomicUsize,
    }

    impl CountingFan {
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl FillTessellator for CountingFan {
        fn tessellate_fill(
            &self,
            path: &PathGeometry,
            _style: &FillStyle,
        ) -> Result<FillMesh, TessellationError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let mut vertices = Vec::new();
            let mut indices = Vec::new();
            let mut start = 0u32;
            for command in path.commands() {
                match *command {
                    PathCommand::MoveTo(v) => {
                        start = vertices.len() as u32;
                        vertices.push(v);
                    }
                    PathCommand::LineTo(v) | PathCommand::CubicTo { to: v, .. } => {
                        vertices.push(v);
                        let last = vertices.len() as u32 - 1;
                        if last >= start + 2 {
                            indices.extend_from_slice(&[start, last - 1, last]);
                        }
                    }
                    PathCommand::Close => {}
                }
            }
            FillMesh::new(vertices, indices)
        }
    }

    /// 항상 실패하는 tessellator.
    struct Failing;

    impl FillTessellator for Failing {
        fn tessellate_fill(
            &self,
            _path: &PathGeometry,
            _style: &FillStyle,
        ) -> Result<FillMesh, TessellationError> {
            Err(TessellationError::Failed {
                reason: "테스트"
            })
        }
    }

    /// document Shape 평가(VI.5.6)가 Rectangle에 대해 내는 것과 같은 모양의 command 목록.
    fn rect_commands(x: f32, y: f32, w: f32, h: f32) -> Vec<PathCommand> {
        vec![
            PathCommand::MoveTo(p(x, y)),
            PathCommand::LineTo(p(x + w, y)),
            PathCommand::LineTo(p(x + w, y + h)),
            PathCommand::LineTo(p(x, y + h)),
            PathCommand::Close,
        ]
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> PathGeometry {
        PathGeometry::new(rect_commands(x, y, w, h)).unwrap()
    }

    #[test]
    fn same_path_and_style_hits_cache() {
        // 검증 기준(XI.6.1): 동일 path/style cache hit.
        let tessellator = CountingFan::default();
        let mut cache = TessellationCache::new();
        let style = FillStyle::default();

        let first = cache.fill(&rect(0.0, 0.0, 10.0, 20.0), style, &tessellator);
        let first = first.unwrap();
        assert_eq!(first.outcome, CacheOutcome::Miss);
        assert_eq!(first.mesh.triangle_count(), 2);

        // 독립적으로 다시 만든(다른 Arc) 동일 path + 동일 style → hit, tessellator 재호출 없음.
        let second = cache
            .fill(&rect(0.0, 0.0, 10.0, 20.0), style, &tessellator)
            .unwrap();
        assert_eq!(second.outcome, CacheOutcome::Hit);
        assert!(Arc::ptr_eq(&first.mesh, &second.mesh));
        assert_eq!(tessellator.calls(), 1);
        assert_eq!(
            cache.stats(),
            CacheStats {
                hits: 1,
                misses: 1,
                entries: 1
            }
        );

        // 명시 생성한 동일 style 값도 hit.
        let explicit = FillStyle {
            fill_rule: FillRule::NonZero,
            tolerance: FlattenTolerance::new(0.25).unwrap(),
        };
        let third = cache
            .fill(&rect(0.0, 0.0, 10.0, 20.0), explicit, &tessellator)
            .unwrap();
        assert_eq!(third.outcome, CacheOutcome::Hit);
        assert_eq!(tessellator.calls(), 1);
    }

    #[test]
    fn shape_evaluated_and_authored_path_share_entry() {
        // Shape(평가 결과)와 사용자가 그린 Path가 같은 command면 같은 entry.
        let tessellator = CountingFan::default();
        let mut cache = TessellationCache::new();
        let from_shape = PathGeometry::new(rect_commands(0.0, 0.0, 4.0, 4.0)).unwrap();
        let authored = PathGeometry::new(vec![
            PathCommand::MoveTo(p(0.0, 0.0)),
            PathCommand::LineTo(p(4.0, 0.0)),
            PathCommand::LineTo(p(4.0, 4.0)),
            PathCommand::LineTo(p(0.0, 4.0)),
            PathCommand::Close,
        ])
        .unwrap();
        let style = FillStyle::default();
        assert_eq!(
            TessellationKey::fill(&from_shape, style),
            TessellationKey::fill(&authored, style)
        );
        cache.fill(&from_shape, style, &tessellator).unwrap();
        let hit = cache.fill(&authored, style, &tessellator).unwrap();
        assert_eq!(hit.outcome, CacheOutcome::Hit);
        assert_eq!(tessellator.calls(), 1);
    }

    #[test]
    fn path_or_style_change_misses() {
        let tessellator = CountingFan::default();
        let mut cache = TessellationCache::new();
        let base = rect(0.0, 0.0, 10.0, 10.0);
        let style = FillStyle::default();
        cache.fill(&base, style, &tessellator).unwrap();

        let variants = [
            // 좌표 1개 변경.
            (rect(0.0, 0.0, 10.0, 10.5), style),
            // command 추가(같은 좌표 + 다른 구조).
            (
                PathGeometry::new({
                    let mut c = rect_commands(0.0, 0.0, 10.0, 10.0);
                    c.insert(4, PathCommand::LineTo(p(0.0, 10.0)));
                    c
                })
                .unwrap(),
                style,
            ),
            // fill rule 변경.
            (
                base.clone(),
                FillStyle {
                    fill_rule: FillRule::EvenOdd,
                    ..style
                },
            ),
            // 허용 오차 변경.
            (
                base.clone(),
                FillStyle {
                    tolerance: FlattenTolerance::new(0.1).unwrap(),
                    ..style
                },
            ),
        ];
        for (index, (path, style)) in variants.iter().enumerate() {
            let result = cache.fill(path, *style, &tessellator).unwrap();
            assert_eq!(result.outcome, CacheOutcome::Miss, "variant {index}");
        }
        assert_eq!(tessellator.calls(), 1 + variants.len());
        assert_eq!(cache.len(), 1 + variants.len());

        // 원래 입력은 여전히 hit.
        assert_eq!(
            cache.fill(&base, style, &tessellator).unwrap().outcome,
            CacheOutcome::Hit
        );
    }

    #[test]
    fn negative_zero_coordinates_share_key() {
        let a = PathGeometry::new(vec![
            PathCommand::MoveTo(p(-0.0, 0.0)),
            PathCommand::LineTo(p(1.0, -0.0)),
        ])
        .unwrap();
        let b = PathGeometry::new(vec![
            PathCommand::MoveTo(p(0.0, 0.0)),
            PathCommand::LineTo(p(1.0, 0.0)),
        ])
        .unwrap();
        let style = FillStyle::default();
        assert_eq!(
            TessellationKey::fill(&a, style),
            TessellationKey::fill(&b, style)
        );
    }

    #[test]
    fn digest_collision_does_not_alias_entries() {
        // digest가 같아도 bytes가 다르면 다른 key(가짜 충돌 강제).
        let a = PathKey::new(&rect(0.0, 0.0, 1.0, 1.0));
        let b = PathKey::new(&rect(0.0, 0.0, 2.0, 2.0));
        let forged = PathKey {
            digest: a.digest(),
            bytes: Arc::clone(&b.bytes),
        };
        assert_ne!(a, forged);
        assert_eq!(a, PathKey::new(&rect(0.0, 0.0, 1.0, 1.0)));
    }

    #[test]
    fn failed_tessellation_is_not_cached() {
        let mut cache = TessellationCache::new();
        let path = rect(0.0, 0.0, 1.0, 1.0);
        let style = FillStyle::default();
        assert_eq!(
            cache.fill(&path, style, &Failing).unwrap_err(),
            TessellationError::Failed {
                reason: "테스트"
            }
        );
        assert!(cache.is_empty());
        // 다음 조회는 다시 tessellation합니다.
        let tessellator = CountingFan::default();
        assert_eq!(
            cache.fill(&path, style, &tessellator).unwrap().outcome,
            CacheOutcome::Miss
        );
        assert_eq!(tessellator.calls(), 1);
    }

    #[test]
    fn tolerance_and_mesh_contracts() {
        for bad in [0.0, -0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                FlattenTolerance::new(bad),
                Err(TessellationError::InvalidTolerance)
            );
        }
        assert_eq!(FlattenTolerance::DEFAULT.get(), 0.25);
        assert_eq!(
            FlattenTolerance::new(0.25).unwrap(),
            FlattenTolerance::DEFAULT
        );

        let vertices = vec![p(0.0, 0.0), p(1.0, 0.0), p(0.0, 1.0)];
        assert!(FillMesh::new(vertices.clone(), vec![0, 1, 2]).is_ok());
        assert!(matches!(
            FillMesh::new(vertices.clone(), vec![0, 1]),
            Err(TessellationError::InvalidMesh { .. })
        ));
        assert!(matches!(
            FillMesh::new(vertices, vec![0, 1, 3]),
            Err(TessellationError::InvalidMesh { .. })
        ));
        assert_eq!(FillMesh::empty().triangle_count(), 0);
    }

    #[test]
    fn remove_and_clear() {
        let tessellator = CountingFan::default();
        let mut cache = TessellationCache::new();
        let path = rect(0.0, 0.0, 3.0, 3.0);
        let style = FillStyle::default();
        cache.fill(&path, style, &tessellator).unwrap();
        let key = TessellationKey::fill(&path, style);
        assert!(cache.remove(&key).is_some());
        assert_eq!(
            cache.fill(&path, style, &tessellator).unwrap().outcome,
            CacheOutcome::Miss
        );
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.stats().misses, 2);
    }
}

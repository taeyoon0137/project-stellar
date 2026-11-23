//! world bounds 기반 공간 인덱스 (XI.3.1).
//!
//! [`SpatialIndex`]는 key마다 world [`Aabb`] 하나를 보관하고 사각형·점 질의로 교차 후보를 찾는
//! 인터페이스입니다. 가시성 culling(XI.3.2)과 hit test(XI.3.3)가 이 인터페이스만 사용합니다.
//!
//! 기본 구현 [`GridIndex`]는 **계층형 loose grid**입니다.
//!
//! - level `L`의 cell 크기는 `base_cell · 2^L`입니다(2의 거듭제곱 배라 나눗셈·floor가 정확).
//! - 항목은 최소·최대 꼭짓점 cell이 축마다 1칸 이내인 가장 작은 level에서, 최소 꼭짓점이 든
//!   cell 하나에만 저장됩니다. 그래서 항목은 그 cell에서 오른쪽·아래 이웃 cell까지만 걸칩니다.
//! - 질의는 level마다 `floor(min / size) - 1 ..= floor(max / size)` 범위 cell을 봅니다. 범위가
//!   그 level의 사용 중 cell 수보다 크면 사용 중 cell을 직접 훑습니다(축소 보기의 거대 viewport 대비).
//! - 가장 큰 level cell보다 큰 항목은 별도 목록에 두고 매 질의에서 검사합니다.
//! - insert·update·remove는 O(1) 평균(hash + swap-remove)이며 무한 canvas를 지원합니다.
//!
//! 질의 결과는 경계를 포함한 교차(닿기만 해도 포함)이고 **순서는 정하지 않습니다**. 순서가 필요한
//! 소비자(paint 순서, topmost hit)가 정렬합니다.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::Hash;
use std::sync::Arc;

use stellar_core_types::value::Vec2;

use crate::bounds::Aabb;
use crate::node::{SceneNode, ZOrder};
use crate::snapshot::SceneSnapshot;

/// 공간 인덱스 갱신 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpatialError {
    /// 이미 있는 key를 insert했습니다.
    DuplicateKey,
    /// 없는 key를 update했습니다.
    MissingKey,
}

impl fmt::Display for SpatialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpatialError::DuplicateKey => f.write_str("공간 인덱스에 이미 있는 key입니다"),
            SpatialError::MissingKey => f.write_str("공간 인덱스에 없는 key입니다"),
        }
    }
}

impl std::error::Error for SpatialError {}

/// world bounds 기반 공간 인덱스 인터페이스.
pub trait SpatialIndex<K> {
    /// 새 key를 넣습니다. 이미 있으면 [`SpatialError::DuplicateKey`].
    fn insert(&mut self, key: K, bounds: Aabb) -> Result<(), SpatialError>;

    /// 있는 key의 bounds를 바꿉니다. 없으면 [`SpatialError::MissingKey`].
    fn update(&mut self, key: K, bounds: Aabb) -> Result<(), SpatialError>;

    /// key를 지우고 마지막 bounds를 돌려줍니다. 없으면 `None`.
    fn remove(&mut self, key: K) -> Option<Aabb>;

    /// `area`와 교차(경계 포함)하는 key를 `out`에 덧붙입니다(순서 미정).
    fn query_rect(&self, area: &Aabb, out: &mut Vec<K>);

    /// `point`를 포함(경계 포함)하는 key를 `out`에 덧붙입니다(순서 미정).
    fn query_point(&self, point: Vec2, out: &mut Vec<K>) {
        self.query_rect(&Aabb::point(point), out);
    }

    /// key의 현재 bounds.
    fn bounds_of(&self, key: K) -> Option<Aabb>;

    /// 항목 수.
    fn len(&self) -> usize;

    /// 비어 있으면 참입니다.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// level 수. level 23 cell은 기본 base(64)에서 약 5.4×10⁸ 논리 픽셀입니다.
const LEVELS: usize = 24;

/// 기본 level 0 cell 크기(논리 픽셀, 2의 거듭제곱).
pub const DEFAULT_BASE_CELL: f32 = 64.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Cell {
        level: usize,
        cell: (i64, i64),
        slot: usize,
    },
    Huge {
        slot: usize,
    },
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    bounds: Aabb,
    place: Place,
}

/// level 하나의 cell 좌표 → (key, bounds) 목록.
type CellMap<K> = HashMap<(i64, i64), Vec<(K, Aabb)>>;

/// 계층형 loose grid 공간 인덱스.
#[derive(Debug, Clone)]
pub struct GridIndex<K> {
    base_cell: f32,
    levels: Vec<CellMap<K>>,
    huge: Vec<(K, Aabb)>,
    entries: HashMap<K, Entry>,
}

impl<K: Copy + Eq + Hash> Default for GridIndex<K> {
    fn default() -> GridIndex<K> {
        GridIndex::new()
    }
}

/// `value / size`의 floor cell 좌표(saturate).
fn cell_coord(value: f32, size: f32) -> i64 {
    // f32 → i64 변환은 범위를 넘으면 saturate합니다.
    (value / size).floor() as i64
}

impl<K: Copy + Eq + Hash> GridIndex<K> {
    /// 기본 cell 크기([`DEFAULT_BASE_CELL`])의 빈 인덱스.
    pub fn new() -> GridIndex<K> {
        GridIndex::with_base_cell(DEFAULT_BASE_CELL)
    }

    /// level 0 cell 크기를 지정합니다. 2의 거듭제곱으로 내림하고 `[1, 2^20]`로 제한합니다.
    pub fn with_base_cell(base_cell: f32) -> GridIndex<K> {
        let clamped = if base_cell.is_finite() {
            base_cell.clamp(1.0, 1_048_576.0)
        } else {
            DEFAULT_BASE_CELL
        };
        // 2의 거듭제곱으로 내림(정확한 나눗셈 보장).
        let mut power = 1.0f32;
        while power * 2.0 <= clamped {
            power *= 2.0;
        }
        GridIndex {
            base_cell: power,
            levels: (0..LEVELS).map(|_| HashMap::new()).collect(),
            huge: Vec::new(),
            entries: HashMap::new(),
        }
    }

    /// level 0 cell 크기.
    pub const fn base_cell(&self) -> f32 {
        self.base_cell
    }

    fn cell_size(&self, level: usize) -> f32 {
        self.base_cell * (1u32 << level) as f32
    }

    /// bounds가 들어갈 위치(slot 제외)를 정합니다.
    ///
    /// 너비 계산의 반올림에 기대지 않고, 질의와 **같은** 단조 함수 `cell_coord`로 최소·최대
    /// 꼭짓점 cell이 축마다 1칸 이내인 가장 작은 level을 고릅니다. 그래서 질의 범위
    /// `cell(min) - 1 ..= cell(max)`가 교차 항목을 놓치지 않습니다.
    fn locate(&self, bounds: &Aabb) -> Option<(usize, (i64, i64))> {
        (0..LEVELS).find_map(|level| {
            let size = self.cell_size(level);
            let x0 = cell_coord(bounds.min_x(), size);
            let y0 = cell_coord(bounds.min_y(), size);
            let x1 = cell_coord(bounds.max_x(), size);
            let y1 = cell_coord(bounds.max_y(), size);
            (x1.saturating_sub(x0) <= 1 && y1.saturating_sub(y0) <= 1).then_some((level, (x0, y0)))
        })
    }

    fn place(&mut self, key: K, bounds: Aabb) -> Place {
        match self.locate(&bounds) {
            Some((level, cell)) => {
                let items = self.levels[level].entry(cell).or_default();
                items.push((key, bounds));
                Place::Cell {
                    level,
                    cell,
                    slot: items.len() - 1,
                }
            }
            None => {
                self.huge.push((key, bounds));
                Place::Huge {
                    slot: self.huge.len() - 1,
                }
            }
        }
    }

    /// `place`에서 항목을 빼고, 자리를 메운 항목의 slot을 고칩니다.
    fn unplace(&mut self, place: Place) {
        let moved = match place {
            Place::Cell { level, cell, slot } => {
                let Some(items) = self.levels[level].get_mut(&cell) else {
                    return;
                };
                items.swap_remove(slot);
                let moved = items.get(slot).map(|(key, _)| *key);
                if items.is_empty() {
                    self.levels[level].remove(&cell);
                }
                moved
            }
            Place::Huge { slot } => {
                self.huge.swap_remove(slot);
                self.huge.get(slot).map(|(key, _)| *key)
            }
        };
        if let Some(moved_key) = moved {
            if let Some(entry) = self.entries.get_mut(&moved_key) {
                match &mut entry.place {
                    Place::Cell { slot: s, .. } | Place::Huge { slot: s } => {
                        *s = match place {
                            Place::Cell { slot, .. } | Place::Huge { slot } => slot,
                        }
                    }
                }
            }
        }
    }

    fn scan(items: &[(K, Aabb)], area: &Aabb, out: &mut Vec<K>) {
        out.extend(
            items
                .iter()
                .filter(|(_, bounds)| bounds.intersects(area))
                .map(|(key, _)| *key),
        );
    }
}

impl<K: Copy + Eq + Hash> SpatialIndex<K> for GridIndex<K> {
    fn insert(&mut self, key: K, bounds: Aabb) -> Result<(), SpatialError> {
        if self.entries.contains_key(&key) {
            return Err(SpatialError::DuplicateKey);
        }
        let place = self.place(key, bounds);
        self.entries.insert(key, Entry { bounds, place });
        Ok(())
    }

    fn update(&mut self, key: K, bounds: Aabb) -> Result<(), SpatialError> {
        let old = *self.entries.get(&key).ok_or(SpatialError::MissingKey)?;
        // 같은 cell이면 bounds만 바꿉니다.
        if let (Place::Cell { level, cell, slot }, Some((new_level, new_cell))) =
            (old.place, self.locate(&bounds))
        {
            if level == new_level && cell == new_cell {
                if let Some(item) = self.levels[level]
                    .get_mut(&cell)
                    .and_then(|items| items.get_mut(slot))
                {
                    item.1 = bounds;
                }
                if let Some(entry) = self.entries.get_mut(&key) {
                    entry.bounds = bounds;
                }
                return Ok(());
            }
        }
        self.unplace(old.place);
        let place = self.place(key, bounds);
        self.entries.insert(key, Entry { bounds, place });
        Ok(())
    }

    fn remove(&mut self, key: K) -> Option<Aabb> {
        let entry = self.entries.remove(&key)?;
        self.unplace(entry.place);
        Some(entry.bounds)
    }

    fn query_rect(&self, area: &Aabb, out: &mut Vec<K>) {
        for (level, cells) in self.levels.iter().enumerate() {
            if cells.is_empty() {
                continue;
            }
            let size = self.cell_size(level);
            let x0 = cell_coord(area.min_x(), size).saturating_sub(1);
            let y0 = cell_coord(area.min_y(), size).saturating_sub(1);
            let x1 = cell_coord(area.max_x(), size);
            let y1 = cell_coord(area.max_y(), size);
            let span = (x1.abs_diff(x0) as u128 + 1).saturating_mul(y1.abs_diff(y0) as u128 + 1);
            if span > cells.len() as u128 {
                for (&(cx, cy), items) in cells {
                    if (x0..=x1).contains(&cx) && (y0..=y1).contains(&cy) {
                        GridIndex::scan(items, area, out);
                    }
                }
            } else {
                for cx in x0..=x1 {
                    for cy in y0..=y1 {
                        if let Some(items) = cells.get(&(cx, cy)) {
                            GridIndex::scan(items, area, out);
                        }
                    }
                }
            }
        }
        GridIndex::scan(&self.huge, area, out);
    }

    fn bounds_of(&self, key: K) -> Option<Aabb> {
        self.entries.get(&key).map(|entry| entry.bounds)
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// snapshot node를 world 그리기 영역([`SceneNode::world_render_bounds`])으로 색인한 scene 공간
/// 인덱스. key는 snapshot 안에서 유일한 [`ZOrder`]입니다(XI.1.2 불변식).
#[derive(Debug, Clone, Default)]
pub struct SceneSpatialIndex {
    index: GridIndex<ZOrder>,
    nodes: HashMap<ZOrder, Arc<SceneNode>>,
    unindexed: HashSet<ZOrder>,
}

impl SceneSpatialIndex {
    /// snapshot의 모든 node를 색인합니다. world bounds가 비유한(transform 오버플로)인 node는
    /// 그릴 수 없으므로 색인하지 않고 [`SceneSpatialIndex::unindexed_count`]로 셉니다.
    pub fn build(snapshot: &SceneSnapshot) -> SceneSpatialIndex {
        let mut out = SceneSpatialIndex::default();
        let mut stack: Vec<&Arc<SceneNode>> = snapshot.roots().collect();
        while let Some(node) = stack.pop() {
            stack.extend(node.children());
            match node.world_render_bounds() {
                Some(bounds) => {
                    // z-order는 snapshot 안에서 유일하므로 중복 insert가 없습니다.
                    if out.index.insert(node.z(), bounds).is_ok() {
                        out.nodes.insert(node.z(), Arc::clone(node));
                    }
                }
                None => {
                    out.unindexed.insert(node.z());
                }
            }
        }
        out
    }

    /// 새 snapshot으로 인덱스를 갱신합니다(XI.2.5 증분 build의 하류, ADR-0084 결정 3).
    ///
    /// 같은 z-order에 같은 `Arc`가 있는 subtree는 내려가지 않습니다(증분 build가 공유한 subtree). 나머지 node는
    /// bounds를 다시 넣고, 새 snapshot에 없는 z-order는 지웁니다. 결과는 [`SceneSpatialIndex::build`]와 같습니다.
    /// 갱신한 node 수를 돌려줍니다.
    pub fn refresh(&mut self, snapshot: &SceneSnapshot) -> usize {
        let mut touched = 0usize;
        let mut seen_max = 0u64;
        let mut stack: Vec<&Arc<SceneNode>> = snapshot.roots().collect();
        while let Some(node) = stack.pop() {
            let z = node.z();
            seen_max = seen_max.max(node.last_z().0);
            if self.nodes.get(&z).is_some_and(|old| Arc::ptr_eq(old, node)) {
                continue;
            }
            touched += 1;
            stack.extend(node.children());
            let had = self.nodes.remove(&z).is_some();
            self.unindexed.remove(&z);
            match node.world_render_bounds() {
                Some(bounds) => {
                    let placed = if had {
                        self.index.update(z, bounds)
                    } else {
                        self.index.insert(z, bounds)
                    };
                    if placed.is_ok() {
                        self.nodes.insert(z, Arc::clone(node));
                    }
                }
                None => {
                    self.index.remove(z);
                    self.unindexed.insert(z);
                }
            }
        }
        // 새 snapshot에 없는 뒤쪽 z-order(삭제로 node 수가 줄어듦).
        let stale: Vec<ZOrder> = self
            .nodes
            .keys()
            .chain(self.unindexed.iter())
            .copied()
            .filter(|z| z.0 > seen_max)
            .collect();
        for z in stale {
            self.nodes.remove(&z);
            self.index.remove(z);
            self.unindexed.remove(&z);
            touched += 1;
        }
        touched
    }

    /// 아래 공간 인덱스.
    pub const fn index(&self) -> &GridIndex<ZOrder> {
        &self.index
    }

    /// z-order key의 node.
    pub fn node(&self, z: ZOrder) -> Option<&Arc<SceneNode>> {
        self.nodes.get(&z)
    }

    /// 색인한 node 수.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// 색인한 node가 없으면 참입니다.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// world bounds가 비유한이라 색인하지 않은 node 수.
    pub fn unindexed_count(&self) -> usize {
        self.unindexed.len()
    }

    /// `area`와 교차하는 node의 z-order를 `out`에 덧붙입니다(순서 미정).
    pub fn query_rect(&self, area: &Aabb, out: &mut Vec<ZOrder>) {
        self.index.query_rect(area, out);
    }

    /// `point`를 포함하는 node의 z-order를 `out`에 덧붙입니다(순서 미정).
    pub fn query_point(&self, point: Vec2, out: &mut Vec<ZOrder>) {
        self.index.query_point(point, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{p, SplitMix64};

    fn aabb(a: f32, b: f32, c: f32, d: f32) -> Aabb {
        Aabb::new(a, b, c, d).expect("aabb")
    }

    fn random_bounds(rng: &mut SplitMix64) -> Aabb {
        let x = rng.range(-100_000.0, 100_000.0);
        let y = rng.range(-100_000.0, 100_000.0);
        let (w, h) = match rng.below(100) {
            // 대부분 작은 node.
            0..=79 => (rng.range(0.0, 100.0), rng.range(0.0, 100.0)),
            // 중간 크기.
            80..=94 => (rng.range(100.0, 5_000.0), rng.range(0.0, 5_000.0)),
            // 점(크기 0).
            95..=97 => (0.0, 0.0),
            // 가장 큰 level보다 큰 항목.
            _ => (rng.range(1.0e9, 2.0e9), rng.range(0.0, 10.0)),
        };
        aabb(x, y, x + w, y + h)
    }

    fn brute(live: &HashMap<u32, Aabb>, area: &Aabb) -> Vec<u32> {
        let mut keys: Vec<u32> = live
            .iter()
            .filter(|(_, bounds)| bounds.intersects(area))
            .map(|(key, _)| *key)
            .collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn random_200k_nodes_query_correctness() {
        // 검증 기준(XI.3.1): 20만 node random query correctness.
        let mut rng = SplitMix64::new(0x3_1);
        let mut index: GridIndex<u32> = GridIndex::new();
        let mut live: HashMap<u32, Aabb> = HashMap::new();
        for key in 0..200_000u32 {
            let bounds = random_bounds(&mut rng);
            index.insert(key, bounds).expect("insert");
            live.insert(key, bounds);
        }
        // 무작위 update(이동·크기 변경) 2만, remove 2만, 재insert 1만.
        for _ in 0..20_000 {
            let key = rng.below(200_000) as u32;
            let bounds = random_bounds(&mut rng);
            if live.contains_key(&key) {
                index.update(key, bounds).expect("update");
                live.insert(key, bounds);
            }
        }
        for _ in 0..20_000 {
            let key = rng.below(200_000) as u32;
            assert_eq!(index.remove(key), live.remove(&key));
        }
        for key in 200_000..210_000u32 {
            let bounds = random_bounds(&mut rng);
            index.insert(key, bounds).expect("insert");
            live.insert(key, bounds);
        }
        assert_eq!(index.len(), live.len());

        let mut out = Vec::new();
        for query in 0..150 {
            // 작은 사각형, 화면 크기, 거대(축소 보기) 사각형을 섞습니다.
            let x = rng.range(-120_000.0, 120_000.0);
            let y = rng.range(-120_000.0, 120_000.0);
            let size = match query % 3 {
                0 => rng.range(0.0, 50.0),
                1 => rng.range(500.0, 3_000.0),
                _ => rng.range(50_000.0, 400_000.0),
            };
            let area = aabb(x, y, x + size, y + size * 0.6);
            out.clear();
            index.query_rect(&area, &mut out);
            out.sort_unstable();
            assert_eq!(out, brute(&live, &area), "rect query {query}");
        }
        for query in 0..150 {
            let point = p(
                rng.range(-100_000.0, 100_000.0),
                rng.range(-100_000.0, 100_000.0),
            );
            out.clear();
            index.query_point(point, &mut out);
            out.sort_unstable();
            assert_eq!(
                out,
                brute(&live, &Aabb::point(point)),
                "point query {query}"
            );
        }
        // 실제 항목 위치(경계 위) 점 질의도 포함됩니다.
        for (key, bounds) in live.iter().take(200) {
            out.clear();
            index.query_point(p(bounds.max_x(), bounds.max_y()), &mut out);
            assert!(out.contains(key));
        }
    }

    #[test]
    fn insert_update_remove_contracts() {
        let mut index: GridIndex<u8> = GridIndex::with_base_cell(100.0);
        assert_eq!(index.base_cell(), 64.0);
        assert!(index.is_empty());
        index.insert(1, aabb(0.0, 0.0, 10.0, 10.0)).unwrap();
        assert_eq!(
            index.insert(1, aabb(0.0, 0.0, 1.0, 1.0)),
            Err(SpatialError::DuplicateKey)
        );
        assert_eq!(
            index.update(2, aabb(0.0, 0.0, 1.0, 1.0)),
            Err(SpatialError::MissingKey)
        );
        // 같은 cell 안 이동, 다른 level·cell로 이동, 거대 항목 전환.
        index.update(1, aabb(1.0, 1.0, 12.0, 12.0)).unwrap();
        index
            .update(1, aabb(5_000.0, 5_000.0, 9_000.0, 9_000.0))
            .unwrap();
        index.update(1, aabb(-1.0e10, 0.0, 1.0e10, 1.0)).unwrap();
        let mut out = Vec::new();
        index.query_point(p(0.0, 0.5), &mut out);
        assert_eq!(out, vec![1]);
        out.clear();
        index.query_point(p(6_000.0, 6_000.0), &mut out);
        assert!(out.is_empty());
        assert_eq!(index.bounds_of(1), Some(aabb(-1.0e10, 0.0, 1.0e10, 1.0)));
        assert_eq!(index.remove(1), Some(aabb(-1.0e10, 0.0, 1.0e10, 1.0)));
        assert_eq!(index.remove(1), None);
        assert!(index.is_empty());

        // 같은 cell 여러 항목 swap-remove 후에도 나머지 항목 slot이 맞습니다.
        for key in 0..10u8 {
            index
                .insert(key, aabb(key as f32, 0.0, key as f32 + 1.0, 1.0))
                .unwrap();
        }
        index.remove(0);
        index.remove(5);
        index.update(9, aabb(9.0, 0.0, 9.5, 0.5)).unwrap();
        out.clear();
        index.query_rect(&aabb(0.0, 0.0, 20.0, 1.0), &mut out);
        out.sort_unstable();
        assert_eq!(out, vec![1, 2, 3, 4, 6, 7, 8, 9]);
        for key in [1u8, 2, 3, 4, 6, 7, 8, 9] {
            assert!(index.remove(key).is_some(), "key {key}");
        }
        assert!(index.is_empty());
    }

    #[test]
    fn rounding_of_width_does_not_misplace_items() {
        // 실제 너비는 64를 조금 넘지만 f32 뺄셈은 64.0으로 반올림됩니다. level 선택이 너비가 아니라
        // cell 좌표로 정해지므로 cell 2 경계(128)의 점 질의가 항목을 찾아야 합니다.
        let below_64 = f32::from_bits(64.0f32.to_bits() - 1);
        assert_eq!(128.0f32 - below_64, 64.0);
        let mut index: GridIndex<u8> = GridIndex::new();
        index.insert(1, aabb(below_64, 0.0, 128.0, 1.0)).unwrap();
        let mut out = Vec::new();
        index.query_point(p(128.0, 0.5), &mut out);
        assert_eq!(out, vec![1]);
    }

    #[test]
    fn scene_index_matches_brute_force_world_bounds() {
        let snapshot = crate::fixtures::generate(crate::fixtures::FixtureConfig {
            seed: 0x31,
            roots: 4,
            children: 5,
            depth: 3,
        });
        let index = SceneSpatialIndex::build(&snapshot);
        assert_eq!(
            index.len() + index.unindexed_count(),
            snapshot.node_count() as usize
        );
        assert_eq!(index.unindexed_count(), 0);
        let mut rng = SplitMix64::new(9);
        let mut out = Vec::new();
        for _ in 0..200 {
            let point = p(rng.range(-500.0, 2_500.0), rng.range(-500.0, 2_500.0));
            out.clear();
            index.query_point(point, &mut out);
            out.sort_unstable();
            let mut expected: Vec<ZOrder> = snapshot
                .iter_preorder()
                .filter(|node| {
                    node.world_render_bounds()
                        .is_some_and(|bounds| bounds.contains_point(point))
                })
                .map(|node| node.z())
                .collect();
            expected.sort_unstable();
            assert_eq!(out, expected);
            for z in &out {
                assert_eq!(index.node(*z).map(|node| node.z()), Some(*z));
            }
        }
    }
}

//! 캔버스 이동·크기 조절의 스냅 엔진(XII.10.15, UX.3.8, ADR-0071 보완, ADR-0105 결정 3).
//!
//! 좌표는 모두 대상 부모의 local 좌표(CSS px)입니다. 후보는 제스처 시작 때 한 번 모으고([`collect`],
//! [`collect_in`]) 드래그 update마다 다시 쓰는 순수 함수 [`snap`]이 맞춥니다. 셸은 화면 허용 오차(pt)와 부모 local
//! 1px의 화면 배율만 넘기고 상자를 고르지 않습니다.
//!
//! # 후보와 우선순위
//!
//! 축마다 허용 오차 안의 후보 가운데 다음 순서로 앞선 종류가 이기고, 같은 종류면 가까운 쪽, 그다음 작은 좌표가
//! 이깁니다(결정론).
//!
//! 1. 가이드(문서 가이드 선, 호출자가 [`SnapLines::guides`]로 넘김)
//! 2. 레이아웃 그리드 선(Frame 열·행 경계, [`SnapLines::grid`])
//! 3. 부모 가장자리·중심, 그다음 편집 범위 형제의 가장자리·중심
//! 4. 형제 사이와 같은 간격(이동)
//! 5. 형제와 같은 크기(크기 조절, 정렬선이 아닌 [`SnapResult::size_match`])
//! 6. 픽셀 그리드(정수 부모 px)
//!
//! 흐름 컨테이너 자식에는 객체 스냅(3–5)을 적용하지 않습니다([`CollectOptions::flow_child`]).
//!
//! # 성능
//!
//! 제스처 시작 때 가장자리·중심 좌표와 크기를 축마다 정렬한 표로, 형제 상자를 공간 인덱스(`GridIndex`)로 만듭니다.
//! update 한 번은 정렬 표의 이분 탐색과 상자 주변 띠 질의만 하므로 후보 수에 거의 비례하지 않습니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{Transform2D, Vec2};
use stellar_scene::bounds::Aabb;
use stellar_scene::spatial::{GridIndex, SpatialIndex};

use crate::app::gesture_math::{Modifiers, ResizeHandle, MIN_SIZE};
use crate::canvas::CanvasScene;

/// 화면 허용 오차(pt). XII.10.2 값(ADR-0104)이 정해지면 그 값을 씁니다(지금은 잠정값).
pub const DEFAULT_TOLERANCE_PT: f32 = 6.0;
/// 형제 후보 상한. 넘으면 거부하지 않고 시작 상자에 가까운 형제만 남깁니다. 정렬 표·공간 인덱스로 update 비용이
/// 후보 수에 거의 비례하지 않으므로 처음 값(4096)보다 크게 둡니다(ADR-0105 결정 3).
pub const MAX_SIBLINGS: usize = 65_536;

const RANK_GUIDE: u8 = 0;
const RANK_GRID: u8 = 1;
const RANK_PARENT: u8 = 2;
const RANK_SIBLING: u8 = 3;
const RANK_SPACING: u8 = 4;
const RANK_SIZE: u8 = 5;
const RANK_PIXEL: u8 = 6;

/// 후보의 종류. 순서가 우선순위입니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandidateKind {
    /// 문서 가이드 선.
    Guide,
    /// 레이아웃 그리드 선.
    GridLine,
    /// 대상 부모 자신(local 상자).
    Parent,
    /// 대상 부모의 직계 자식 중 움직이지 않는 것.
    Sibling,
}

impl CandidateKind {
    fn rank(self) -> u8 {
        match self {
            CandidateKind::Guide => RANK_GUIDE,
            CandidateKind::GridLine => RANK_GRID,
            CandidateKind::Parent => RANK_PARENT,
            CandidateKind::Sibling => RANK_SIBLING,
        }
    }

    fn of_rank(rank: u8) -> Option<CandidateKind> {
        Some(match rank {
            RANK_GUIDE => CandidateKind::Guide,
            RANK_GRID => CandidateKind::GridLine,
            RANK_PARENT => CandidateKind::Parent,
            RANK_SIBLING => CandidateKind::Sibling,
            _ => return None,
        })
    }
}

/// 부모 local 좌표의 후보 상자(부모·형제).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapCandidate {
    /// x, y, width, height.
    pub rect: [f32; 4],
    /// 역할([`CandidateKind::Parent`] 또는 [`CandidateKind::Sibling`]).
    pub kind: CandidateKind,
}

/// 상자가 아닌 선 후보(부모 local 좌표). `[0]`은 세로선의 x, `[1]`은 가로선의 y입니다.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapLines {
    /// 문서 가이드(XII.10.18·XII.10.19가 저장 위치를 정하면 호출자가 넘깁니다).
    pub guides: [Vec<f32>; 2],
    /// 레이아웃 그리드 열·행 경계(XII.10.20).
    pub grid: [Vec<f32>; 2],
}

/// 제스처 시작 때 한 번 준비하는 후보. 드래그 중에는 정렬·수집을 반복하지 않습니다.
#[derive(Clone, Debug, Default)]
pub struct SnapCandidates {
    /// 부모·형제 상자. 부모가 먼저입니다.
    pub boxes: Vec<SnapCandidate>,
    /// 서로 겹치는 이웃 형제 사이의 양수 간격(x축, y축). 오름차순이고 같은 값은 한 번만 둡니다.
    pub gaps: [Vec<f32>; 2],
    /// 선 후보.
    pub lines: SnapLines,
    /// 축마다 (좌표, 우선순위) 정렬 표. 같은 좌표·우선순위는 한 번만 둡니다.
    anchors: [Vec<(f32, u8)>; 2],
    /// 축마다 형제 크기 정렬 표(같은 값 한 번).
    sizes: [Vec<f32>; 2],
    /// 형제 상자 공간 인덱스(값은 `boxes` 번호).
    index: GridIndex<u32>,
    /// 형제 상자 전체 경계(띠 질의 범위).
    extent: Option<[f32; 4]>,
}

fn finite_box(r: &[f32; 4]) -> bool {
    r.iter().all(|v| v.is_finite()) && r[2] >= 0.0 && r[3] >= 0.0
}

fn aabb(r: [f32; 4]) -> Option<Aabb> {
    Aabb::new(r[0], r[1], r[0] + r[2], r[1] + r[3])
}

fn sorted_distinct(mut values: Vec<f32>) -> Vec<f32> {
    values.retain(|v| v.is_finite());
    values.sort_by(f32::total_cmp);
    values.dedup_by(|a, b| a.to_bits() == b.to_bits());
    values
}

impl SnapCandidates {
    /// 상자 후보만으로 준비합니다. 형제가 [`MAX_SIBLINGS`]를 넘으면 `focus` 상자 중심에 가까운 것만 남깁니다.
    pub fn new(boxes: Vec<SnapCandidate>, focus: [f32; 4]) -> SnapCandidates {
        SnapCandidates::with_lines(boxes, SnapLines::default(), focus)
    }

    /// 상자와 선 후보로 준비합니다.
    pub fn with_lines(
        mut boxes: Vec<SnapCandidate>,
        lines: SnapLines,
        focus: [f32; 4],
    ) -> SnapCandidates {
        boxes.retain(|c| {
            finite_box(&c.rect) && matches!(c.kind, CandidateKind::Parent | CandidateKind::Sibling)
        });
        boxes.sort_by_key(|c| c.kind);
        let siblings = boxes
            .iter()
            .filter(|c| c.kind == CandidateKind::Sibling)
            .count();
        if siblings > MAX_SIBLINGS {
            let center = |r: [f32; 4]| [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0];
            let [fx, fy] = center(focus);
            let split = boxes.len() - siblings;
            boxes[split..].sort_by(|a, b| {
                let distance = |c: &SnapCandidate| {
                    let [x, y] = center(c.rect);
                    (x - fx).hypot(y - fy)
                };
                distance(a).total_cmp(&distance(b))
            });
            boxes.truncate(split + MAX_SIBLINGS);
        }
        let lines = SnapLines {
            guides: lines.guides.map(sorted_distinct),
            grid: lines.grid.map(sorted_distinct),
        };
        let mut anchors: [Vec<(f32, u8)>; 2] = [Vec::new(), Vec::new()];
        let mut sizes: [Vec<f32>; 2] = [Vec::new(), Vec::new()];
        for (axis, out) in anchors.iter_mut().enumerate() {
            out.extend(lines.guides[axis].iter().map(|&v| (v, RANK_GUIDE)));
            out.extend(lines.grid[axis].iter().map(|&v| (v, RANK_GRID)));
            for c in &boxes {
                let (start, size) = (c.rect[axis], c.rect[axis + 2]);
                for v in [start, start + size / 2.0, start + size] {
                    out.push((v, c.kind.rank()));
                }
                if c.kind == CandidateKind::Sibling {
                    sizes[axis].push(size);
                }
            }
            out.retain(|(v, _)| v.is_finite());
            out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            out.dedup_by(|a, b| a.0.to_bits() == b.0.to_bits() && a.1 == b.1);
        }
        let sizes = sizes.map(sorted_distinct);
        let mut index = GridIndex::new();
        let mut extent: Option<[f32; 4]> = None;
        for (number, c) in boxes.iter().enumerate() {
            if c.kind != CandidateKind::Sibling {
                continue;
            }
            if let (Some(bounds), Ok(key)) = (aabb(c.rect), u32::try_from(number)) {
                let _ = index.insert(key, bounds);
                extent = Some(extent.map_or(c.rect, |e| union(e, c.rect)));
            }
        }
        let mut candidates = SnapCandidates {
            boxes,
            gaps: [Vec::new(), Vec::new()],
            lines,
            anchors,
            sizes,
            index,
            extent,
        };
        candidates.gaps = [candidates.neighbour_gaps(0), candidates.neighbour_gaps(1)];
        candidates
    }

    /// 형제 상자 번호(오름차순).
    fn siblings(&self) -> impl Iterator<Item = (usize, &SnapCandidate)> {
        self.boxes
            .iter()
            .enumerate()
            .filter(|(_, c)| c.kind == CandidateKind::Sibling)
    }

    /// `rect`와 `axis` 반대 축에서 겹치는 형제를 띠 질의로 찾습니다(번호 오름차순, 결정론).
    fn band(&self, rect: [f32; 4], axis: usize, reach: f32) -> Vec<usize> {
        let Some(extent) = self.extent else {
            return Vec::new();
        };
        let other = 1 - axis;
        let mut area = [0.0f32; 4];
        area[axis] = (rect[axis] - reach).max(extent[axis]);
        let far = (rect[axis] + rect[axis + 2] + reach).min(extent[axis] + extent[axis + 2]);
        if far < area[axis] {
            return Vec::new();
        }
        area[axis + 2] = far - area[axis];
        area[other] = rect[other];
        area[other + 2] = rect[other + 2];
        let Some(query) = aabb(area) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        self.index.query_rect(&query, &mut found);
        let mut found: Vec<usize> = found
            .into_iter()
            .map(|k| k as usize)
            .filter(|&i| {
                let c = self.boxes[i].rect;
                rect[other] < c[other] + c[other + 2] && c[other] < rect[other] + rect[other + 2]
            })
            .collect();
        found.sort_unstable();
        found
    }

    /// 축에서 겹치는 이웃 형제 사이 양수 간격(제스처 시작 때 한 번).
    fn neighbour_gaps(&self, axis: usize) -> Vec<f32> {
        let other = 1 - axis;
        let mut ordered: Vec<[f32; 4]> = self.siblings().map(|(_, c)| c.rect).collect();
        ordered.sort_by(|a, b| {
            a[axis]
                .total_cmp(&b[axis])
                .then(a[other].total_cmp(&b[other]))
        });
        let mut gaps = Vec::new();
        for pair in ordered.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if a[other] >= b[other] + b[other + 2] || b[other] >= a[other] + a[other + 2] {
                continue;
            }
            let gap = b[axis] - a[axis] - a[axis + 2];
            if gap > 0.0 {
                gaps.push(gap);
            }
        }
        sorted_distinct(gaps)
    }
}

/// 스냅할 제스처.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SnapMode {
    /// 상자 전체 이동(가장자리·중심 셋 모두 후보와 맞춤).
    Move,
    /// 크기 핸들. 수식키 제약을 지킵니다.
    Resize {
        /// 잡은 핸들.
        handle: ResizeHandle,
        /// Shift(비율 유지)·Option(중심 기준).
        modifiers: Modifiers,
    },
}

/// 켜진 스냅 종류(앱 설정, ADR-0071). 가이드·레이아웃 그리드 선은 넘긴 선이 있으면 늘 씁니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapSettings {
    /// 부모·형제 객체 스냅.
    pub objects: bool,
    /// 정수 px 스냅.
    pub pixels: bool,
}

/// 스냅 결과. 모든 값은 부모 local CSS px이며 문서에 기록하지 않는 표시용 값입니다.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SnapResult {
    /// 조정한 x, y, width, height.
    pub rect: [f32; 4],
    /// 가이드·그리드·부모·형제와 일치한 정렬선(x 세로선, y 가로선).
    pub guides: [Option<f32>; 2],
    /// 정렬선을 만든 후보 종류.
    pub guide_kind: [Option<CandidateKind>; 2],
    /// 크기 조절에서 형제와 같아진 너비·높이.
    pub size_match: [Option<f32>; 2],
    /// 같은 간격 스냅으로 맞춘 간격(x, y).
    pub spacing: [Option<f32>; 2],
    /// 조정 뒤 가장 가까운 형제와의 간격(x, y).
    pub distance: [Option<f32>; 2],
}

/// 화면 허용 오차(pt)를 부모 local px로 바꿉니다. `scale`은 부모 local 1px의 화면 pt입니다(줌과 부모 변환의 곱).
/// 그래서 줌 25%와 400%에서 화면에서 같은 거리 안이면 똑같이 맞습니다.
pub fn tolerance(points: f32, scale: f32) -> Option<f32> {
    (points.is_finite() && points >= 0.0 && scale.is_finite() && scale > 0.0)
        .then_some(points / scale)
}

/// 한 축에서 찾은 가장 좋은 맞춤.
#[derive(Clone, Copy)]
struct Hit {
    delta: f32,
    rank: u8,
    guide: Option<f32>,
    size: Option<f32>,
    spacing: Option<f32>,
}

impl Hit {
    fn better(self, other: Option<Hit>) -> bool {
        other.is_none_or(|old| {
            self.rank < old.rank
                || self.rank == old.rank && self.delta.abs() < old.delta.abs()
                || self.rank == old.rank
                    && self.delta.abs() == old.delta.abs()
                    && self.delta < old.delta
        })
    }
}

/// `rect`(제스처가 정한 상자)를 후보에 맞춥니다. `tolerance`는 부모 local px입니다. 비유한 입력은 `None`입니다.
///
/// 크기 조절의 수식키 제약: Option(중심 기준)은 옮긴 변의 반대 변도 같은 거리만큼 옮겨 중심을 지키고, Shift(비율
/// 유지)는 한 축을 맞추면 다른 축 비율이 깨지므로 스냅하지 않습니다(2026-11-23 결정).
pub fn snap(
    rect: [f32; 4],
    mode: SnapMode,
    tolerance: f32,
    settings: SnapSettings,
    candidates: &SnapCandidates,
) -> Option<SnapResult> {
    if !(finite_box(&rect) && tolerance.is_finite() && tolerance >= 0.0) {
        return None;
    }
    let mut output = SnapResult {
        rect,
        ..SnapResult::default()
    };
    if let SnapMode::Resize { modifiers, .. } = mode {
        if modifiers.shift {
            output.distance = distances(output.rect, candidates);
            return Some(output);
        }
    }
    for axis in 0..2 {
        let (moves_min, moves_max, centered) = match mode {
            SnapMode::Move => (true, true, false),
            SnapMode::Resize { handle, modifiers } => {
                let (min, max) = handle.moves(axis);
                (min, max, modifiers.option)
            }
        };
        if !moves_min && !moves_max {
            continue;
        }
        let start = rect[axis];
        let size = rect[axis + 2];
        // 이동은 가장자리·중심 셋, 크기 조절은 옮기는 변 하나를 맞춥니다.
        let sources: &[f32] = match mode {
            SnapMode::Move => &[start, start + size / 2.0, start + size],
            SnapMode::Resize { .. } if moves_min => &[start],
            SnapMode::Resize { .. } => &[start + size],
        };
        let max_rank = if settings.objects {
            RANK_SIBLING
        } else {
            RANK_GRID
        };
        let mut best = anchor_hit(&candidates.anchors[axis], sources, tolerance, max_rank);
        if settings.objects {
            let later = match mode {
                SnapMode::Move => spacing_hit(rect, axis, tolerance, candidates),
                SnapMode::Resize { .. } => size_hit(
                    &candidates.sizes[axis],
                    size,
                    moves_min,
                    centered,
                    tolerance,
                ),
            };
            if let Some(hit) = later {
                if hit.better(best) {
                    best = Some(hit);
                }
            }
        }
        if best.is_none() && settings.pixels {
            let value = sources[0];
            let delta = value.round() - value;
            if delta.abs() <= tolerance {
                best = Some(Hit {
                    delta,
                    rank: RANK_PIXEL,
                    guide: None,
                    size: None,
                    spacing: None,
                });
            }
        }
        let Some(hit) = best else {
            continue;
        };
        match mode {
            SnapMode::Move => output.rect[axis] += hit.delta,
            SnapMode::Resize { .. } => {
                let (mut lo, mut hi) = (start, start + size);
                if moves_min {
                    lo += hit.delta;
                    if centered {
                        hi -= hit.delta;
                    }
                } else {
                    hi += hit.delta;
                    if centered {
                        lo -= hit.delta;
                    }
                }
                if hi - lo < MIN_SIZE {
                    continue;
                }
                output.rect[axis] = lo;
                output.rect[axis + 2] = hi - lo;
            }
        }
        output.guides[axis] = hit.guide;
        output.guide_kind[axis] = hit.guide.and(CandidateKind::of_rank(hit.rank));
        output.size_match[axis] = hit.size;
        output.spacing[axis] = hit.spacing;
    }
    output.distance = distances(output.rect, candidates);
    Some(output)
}

/// 정렬 표에서 `sources` 각각의 허용 오차 창을 이분 탐색합니다. `max_rank`보다 뒤 종류는 건너뜁니다.
fn anchor_hit(anchors: &[(f32, u8)], sources: &[f32], tolerance: f32, max_rank: u8) -> Option<Hit> {
    let mut best: Option<Hit> = None;
    for &source in sources {
        let low = source - tolerance;
        let first = anchors.partition_point(|(v, _)| *v < low);
        for &(value, rank) in &anchors[first..] {
            if value > source + tolerance {
                break;
            }
            if rank > max_rank {
                continue;
            }
            let hit = Hit {
                delta: value - source,
                rank,
                guide: Some(value),
                size: None,
                spacing: None,
            };
            if hit.delta.abs() <= tolerance && hit.better(best) {
                best = Some(hit);
            }
        }
    }
    best
}

/// 크기 조절에서 형제와 같은 크기. 중심 기준이면 한 변의 이동이 크기를 두 배로 바꿉니다.
fn size_hit(
    sizes: &[f32],
    size: f32,
    moves_min: bool,
    centered: bool,
    tolerance: f32,
) -> Option<Hit> {
    let factor = if centered { 2.0 } else { 1.0 };
    let reach = tolerance * factor;
    let first = sizes.partition_point(|v| *v < size - reach);
    let mut best: Option<Hit> = None;
    for &candidate in &sizes[first..] {
        if candidate > size + reach {
            break;
        }
        let change = (candidate - size) / factor;
        let hit = Hit {
            delta: if moves_min { -change } else { change },
            rank: RANK_SIZE,
            guide: None,
            size: Some(candidate),
            spacing: None,
        };
        if change.abs() <= tolerance && hit.better(best) {
            best = Some(hit);
        }
    }
    best
}

/// 이동 중인 상자와 가장 가까운 겹치는 형제의 간격을 형제 사이 간격 중 하나와 맞춥니다.
fn spacing_hit(
    rect: [f32; 4],
    axis: usize,
    tolerance: f32,
    candidates: &SnapCandidates,
) -> Option<Hit> {
    let largest = *candidates.gaps[axis].last()?;
    let (current, direction) = candidates
        .band(rect, axis, largest + tolerance)
        .into_iter()
        .filter_map(|i| {
            let c = candidates.boxes[i].rect;
            let after = rect[axis] - c[axis] - c[axis + 2];
            let before = c[axis] - rect[axis] - rect[axis + 2];
            if after >= 0.0 {
                Some((after, 1.0f32))
            } else if before >= 0.0 {
                Some((before, -1.0f32))
            } else {
                None
            }
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)))?;
    let gaps = &candidates.gaps[axis];
    let first = gaps.partition_point(|g| *g < current - tolerance);
    let mut best: Option<Hit> = None;
    for &gap in &gaps[first..] {
        if gap > current + tolerance {
            break;
        }
        let hit = Hit {
            delta: direction * (gap - current),
            rank: RANK_SPACING,
            guide: None,
            size: None,
            spacing: Some(gap),
        };
        if hit.delta.abs() <= tolerance && hit.better(best) {
            best = Some(hit);
        }
    }
    best
}

/// 조정 뒤 가장 가까운 형제와의 수평·수직 간격(겹치는 축에서만).
fn distances(rect: [f32; 4], candidates: &SnapCandidates) -> [Option<f32>; 2] {
    let mut out = [None, None];
    let Some(extent) = candidates.extent else {
        return out;
    };
    for (axis, slot) in out.iter_mut().enumerate() {
        // 상자에서 형제 전체 경계의 먼 쪽 끝까지(상자가 경계 밖에 있어도 모두 덮음).
        let reach = (rect[axis] - extent[axis])
            .max(extent[axis] + extent[axis + 2] - rect[axis] - rect[axis + 2])
            .max(0.0);
        for i in candidates.band(rect, axis, reach) {
            let c = candidates.boxes[i].rect;
            let gap = if c[axis] >= rect[axis] + rect[axis + 2] {
                c[axis] - (rect[axis] + rect[axis + 2])
            } else if rect[axis] >= c[axis] + c[axis + 2] {
                rect[axis] - (c[axis] + c[axis + 2])
            } else {
                0.0
            };
            if gap > 0.0 && slot.is_none_or(|old: f32| gap < old) {
                *slot = Some(gap);
            }
        }
    }
    out
}

/// 후보 수집 선택 사항.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollectOptions {
    /// 가시 영역(world x, y, width, height). 있으면 world 경계가 이 영역과 겹치는 형제만 후보입니다.
    pub visible: Option<[f32; 4]>,
    /// 움직이는 node가 흐름 컨테이너 자식이면 참. 객체(부모·형제) 후보를 만들지 않습니다.
    pub flow_child: bool,
    /// 선 후보(부모 local).
    pub lines: SnapLines,
}

/// `moving` 노드들의 부모 local 좌표계에서 부모 상자와 움직이지 않는 직계 형제 상자를 모읍니다([`collect_in`]의
/// 기본 선택 사항 판).
pub(crate) fn collect(scene: &CanvasScene, moving: &[StableId]) -> Option<SnapCandidates> {
    collect_in(scene, moving, &CollectOptions::default())
}

/// `moving` 노드들의 부모 local 좌표계에서 후보를 모읍니다.
///
/// 모든 `moving`이 한 부모를 공유해야 합니다(아니면 `None`, 스냅하지 않음). 형제 상자는 회전·기울임을 반영한 부모
/// local 경계 상자입니다. 같은 노드의 인스턴스 투영 행은 합칩니다. `focus`(움직이는 상자 경계)는 형제 상한을 넘을
/// 때의 기준입니다.
pub fn collect_in(
    scene: &CanvasScene,
    moving: &[StableId],
    options: &CollectOptions,
) -> Option<SnapCandidates> {
    let rows = &scene.frames;
    let index_of = |id: StableId| rows.iter().position(|row| row.id == id);
    let first = index_of(*moving.first()?)?;
    let parent = rows[first].parent?;
    for id in moving {
        if rows[index_of(*id)?].parent != Some(parent) {
            return None;
        }
    }
    let parent_row = rows.get(parent as usize)?;
    let to_parent = parent_row.world.inverse()?;
    let bounds_in =
        |transform: &Transform2D, rect: stellar_core_types::value::Rect| -> Option<[f32; 4]> {
            let (x, y) = (rect.origin().x(), rect.origin().y());
            let (w, h) = (rect.size().width(), rect.size().height());
            let mut min = [f32::INFINITY; 2];
            let mut max = [f32::NEG_INFINITY; 2];
            for (px, py) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
                let p = transform.apply(Vec2::new(px, py).ok()?).ok()?;
                min = [min[0].min(p.x()), min[1].min(p.y())];
                max = [max[0].max(p.x()), max[1].max(p.y())];
            }
            Some([min[0], min[1], max[0] - min[0], max[1] - min[1]])
        };
    let parent_rect = parent_row.geometry.rect;
    let mut boxes = Vec::new();
    if !options.flow_child {
        boxes.push(SnapCandidate {
            rect: [
                parent_rect.origin().x(),
                parent_rect.origin().y(),
                parent_rect.size().width(),
                parent_rect.size().height(),
            ],
            kind: CandidateKind::Parent,
        });
    }
    let visible = options.visible.and_then(aabb);
    let mut merged: std::collections::BTreeMap<StableId, [f32; 4]> = Default::default();
    let mut focus: Option<[f32; 4]> = None;
    for row in rows.iter().filter(|row| row.parent == Some(parent)) {
        let local = row.world.then(&to_parent).ok();
        let Some(rect) = local.and_then(|local| bounds_in(&local, row.geometry.rect)) else {
            continue;
        };
        if moving.contains(&row.id) {
            focus = Some(focus.map_or(rect, |f| union(f, rect)));
            continue;
        }
        if options.flow_child {
            continue;
        }
        if let Some(visible) = &visible {
            let world = bounds_in(&row.world, row.geometry.rect).and_then(aabb);
            if !world.is_some_and(|w| w.intersects(visible)) {
                continue;
            }
        }
        merged
            .entry(row.id)
            .and_modify(|existing| *existing = union(*existing, rect))
            .or_insert(rect);
    }
    boxes.extend(merged.into_values().map(|rect| SnapCandidate {
        rect,
        kind: CandidateKind::Sibling,
    }));
    Some(SnapCandidates::with_lines(
        boxes,
        options.lines.clone(),
        focus.unwrap_or([0.0; 4]),
    ))
}

fn union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let x0 = a[0].min(b[0]);
    let y0 = a[1].min(b[1]);
    let x1 = (a[0] + a[2]).max(b[0] + b[2]);
    let y1 = (a[1] + a[3]).max(b[1] + b[3]);
    [x0, y0, x1 - x0, y1 - y0]
}

#[cfg(test)]
mod tests;

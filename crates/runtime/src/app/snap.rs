//! 캔버스 이동·크기 조절의 스냅 계산(UX.3.8, ADR-0071).
//!
//! 좌표는 모두 대상 부모의 local 좌표(CSS px)입니다. 후보(부모 상자, 직계 형제 상자)는 코어가 게시한 캔버스 scene에서
//! [`collect`]로 모읍니다. 셸은 화면 허용 오차(pt)와 부모 local 1px의 화면 배율만 넘기고 상자를 고르지 않습니다.
//!
//! 우선순위는 축마다 부모 가장자리·중심 → 형제 가장자리·중심 → 형제 같은 간격(이동) → 정수 px입니다. 크기 조절에서
//! 형제와 같은 너비·높이는 정렬선이 아닌 별도 결과([`SnapResult::size_match`])로 냅니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{Transform2D, Vec2};

use super::gesture_math::{Modifiers, ResizeHandle};
use crate::canvas::CanvasScene;

/// 화면 허용 오차(pt). XII.10.2 값이 정해질 때 ADR로 확정합니다(지금은 잠정값).
pub const DEFAULT_TOLERANCE_PT: f32 = 6.0;
/// 형제 후보 상한. 넘으면 거부하지 않고 시작 상자에 가까운 형제만 남깁니다.
pub const MAX_SIBLINGS: usize = 4096;

/// 후보 상자의 역할.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CandidateKind {
    /// 대상 부모 자신(local 상자).
    Parent,
    /// 대상 부모의 직계 자식 중 움직이지 않는 것.
    Sibling,
}

/// 부모 local 좌표의 후보 상자.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapCandidate {
    /// x, y, width, height.
    pub rect: [f32; 4],
    /// 역할.
    pub kind: CandidateKind,
}

/// 제스처 시작 때 한 번 준비하는 후보. 드래그 중에는 정렬·수집을 반복하지 않습니다.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapCandidates {
    /// 부모·형제 상자. 부모가 먼저입니다.
    pub boxes: Vec<SnapCandidate>,
    /// 서로 겹치는 형제 사이의 양수 간격(x축, y축). 같은 값은 한 번만 둡니다.
    pub gaps: [Vec<f32>; 2],
}

impl SnapCandidates {
    /// 후보를 준비합니다. 형제가 [`MAX_SIBLINGS`]를 넘으면 `focus` 상자 중심에 가까운 것만 남깁니다.
    pub fn new(mut boxes: Vec<SnapCandidate>, focus: [f32; 4]) -> SnapCandidates {
        boxes.retain(|c| {
            c.rect.iter().all(|v| v.is_finite()) && c.rect[2] >= 0.0 && c.rect[3] >= 0.0
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
        let mut gaps = [Vec::new(), Vec::new()];
        for (axis, out) in gaps.iter_mut().enumerate() {
            let other = 1 - axis;
            let mut ordered: Vec<[f32; 4]> = boxes
                .iter()
                .filter(|c| c.kind == CandidateKind::Sibling)
                .map(|c| c.rect)
                .collect();
            ordered.sort_by(|a, b| a[axis].total_cmp(&b[axis]));
            let mut seen = std::collections::BTreeSet::new();
            for pair in ordered.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                if a[other] >= b[other] + b[other + 2] || b[other] >= a[other] + a[other + 2] {
                    continue;
                }
                let gap = b[axis] - a[axis] - a[axis + 2];
                if gap > 0.0 && seen.insert(gap.to_bits()) {
                    out.push(gap);
                }
            }
        }
        SnapCandidates { boxes, gaps }
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

/// 켜진 스냅 종류(앱 설정, ADR-0071).
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
    /// 부모·형제와 일치한 정렬선(x 세로선, y 가로선).
    pub guides: [Option<f32>; 2],
    /// 크기 조절에서 형제와 같아진 너비·높이.
    pub size_match: [Option<f32>; 2],
    /// 같은 간격 스냅으로 맞춘 간격(x, y).
    pub spacing: [Option<f32>; 2],
    /// 조정 뒤 가장 가까운 형제와의 간격(x, y).
    pub distance: [Option<f32>; 2],
}

/// 화면 허용 오차(pt)를 부모 local px로 바꿉니다. `scale`은 부모 local 1px의 화면 pt입니다.
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
            self.rank < old.rank || self.rank == old.rank && self.delta.abs() < old.delta.abs()
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
    if !(rect.iter().all(|v| v.is_finite())
        && rect[2] >= 0.0
        && rect[3] >= 0.0
        && tolerance.is_finite()
        && tolerance >= 0.0)
    {
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
        let sources: Vec<f32> = match mode {
            SnapMode::Move => vec![start, start + size / 2.0, start + size],
            SnapMode::Resize { .. } if moves_min => vec![start],
            SnapMode::Resize { .. } => vec![start + size],
        };
        let mut best: Option<Hit> = None;
        if settings.objects {
            for candidate in &candidates.boxes {
                let rank = match candidate.kind {
                    CandidateKind::Parent => 0,
                    CandidateKind::Sibling => 1,
                };
                let (c_start, c_size) = (candidate.rect[axis], candidate.rect[axis + 2]);
                for anchor in [c_start, c_start + c_size / 2.0, c_start + c_size] {
                    for source in &sources {
                        let hit = Hit {
                            delta: anchor - source,
                            rank,
                            guide: Some(anchor),
                            size: None,
                            spacing: None,
                        };
                        if hit.delta.abs() <= tolerance && hit.better(best) {
                            best = Some(hit);
                        }
                    }
                }
                if matches!(mode, SnapMode::Resize { .. })
                    && candidate.kind == CandidateKind::Sibling
                {
                    // 중심 기준이면 한 변의 이동이 크기를 두 배로 바꿉니다.
                    let factor = if centered { 2.0 } else { 1.0 };
                    let change = (c_size - size) / factor;
                    let hit = Hit {
                        delta: if moves_min { -change } else { change },
                        rank: 2,
                        guide: None,
                        size: Some(c_size),
                        spacing: None,
                    };
                    if change.abs() <= tolerance && hit.better(best) {
                        best = Some(hit);
                    }
                }
            }
            if best.is_none() && mode == SnapMode::Move {
                best = spacing_hit(rect, axis, tolerance, candidates);
            }
        }
        if best.is_none() && settings.pixels {
            let value = sources[0];
            let delta = value.round() - value;
            if delta.abs() <= tolerance {
                best = Some(Hit {
                    delta,
                    rank: 4,
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
                if hi - lo < super::gesture_math::MIN_SIZE {
                    continue;
                }
                output.rect[axis] = lo;
                output.rect[axis + 2] = hi - lo;
            }
        }
        output.guides[axis] = hit.guide;
        output.size_match[axis] = hit.size;
        output.spacing[axis] = hit.spacing;
    }
    output.distance = distances(output.rect, candidates);
    Some(output)
}

/// 이동 중인 상자와 가장 가까운 겹치는 형제의 간격을 형제 사이 간격 중 하나와 맞춥니다.
fn spacing_hit(
    rect: [f32; 4],
    axis: usize,
    tolerance: f32,
    candidates: &SnapCandidates,
) -> Option<Hit> {
    let other = 1 - axis;
    let (current, direction) = candidates
        .boxes
        .iter()
        .filter(|c| c.kind == CandidateKind::Sibling)
        .filter(|c| {
            rect[other] < c.rect[other] + c.rect[other + 2]
                && c.rect[other] < rect[other] + rect[other + 2]
        })
        .filter_map(|c| {
            let after = rect[axis] - c.rect[axis] - c.rect[axis + 2];
            let before = c.rect[axis] - rect[axis] - rect[axis + 2];
            if after >= 0.0 {
                Some((after, 1.0))
            } else if before >= 0.0 {
                Some((before, -1.0))
            } else {
                None
            }
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))?;
    let mut best: Option<Hit> = None;
    for gap in &candidates.gaps[axis] {
        let hit = Hit {
            delta: direction * (gap - current),
            rank: 3,
            guide: None,
            size: None,
            spacing: Some(*gap),
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
    for candidate in candidates
        .boxes
        .iter()
        .filter(|c| c.kind == CandidateKind::Sibling)
    {
        for (axis, slot) in out.iter_mut().enumerate() {
            let other = 1 - axis;
            let c = candidate.rect;
            if !(rect[other] < c[other] + c[other + 2] && c[other] < rect[other] + rect[other + 2])
            {
                continue;
            }
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

/// `moving` 노드들의 부모 local 좌표계에서 부모 상자와 움직이지 않는 직계 형제 상자를 모읍니다.
///
/// 모든 `moving`이 한 부모를 공유해야 합니다(아니면 `None`, 스냅하지 않음). 형제 상자는 회전·기울임을 반영한 부모
/// local 경계 상자입니다. 같은 노드의 인스턴스 투영 행은 합칩니다. `focus`는 형제 상한을 넘을 때의 기준입니다.
pub(crate) fn collect(scene: &CanvasScene, moving: &[StableId]) -> Option<SnapCandidates> {
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
    let local_box =
        |world: &Transform2D, rect: stellar_core_types::value::Rect| -> Option<[f32; 4]> {
            let local = world.then(&to_parent).ok()?;
            let (x, y) = (rect.origin().x(), rect.origin().y());
            let (w, h) = (rect.size().width(), rect.size().height());
            let mut min = [f32::INFINITY; 2];
            let mut max = [f32::NEG_INFINITY; 2];
            for (px, py) in [(x, y), (x + w, y), (x, y + h), (x + w, y + h)] {
                let p = local.apply(Vec2::new(px, py).ok()?).ok()?;
                min = [min[0].min(p.x()), min[1].min(p.y())];
                max = [max[0].max(p.x()), max[1].max(p.y())];
            }
            Some([min[0], min[1], max[0] - min[0], max[1] - min[1]])
        };
    let parent_rect = parent_row.geometry.rect;
    let mut boxes = vec![SnapCandidate {
        rect: [
            parent_rect.origin().x(),
            parent_rect.origin().y(),
            parent_rect.size().width(),
            parent_rect.size().height(),
        ],
        kind: CandidateKind::Parent,
    }];
    let mut merged: std::collections::BTreeMap<StableId, [f32; 4]> = Default::default();
    let mut focus: Option<[f32; 4]> = None;
    for row in rows.iter().filter(|row| row.parent == Some(parent)) {
        let Some(rect) = local_box(&row.world, row.geometry.rect) else {
            continue;
        };
        if moving.contains(&row.id) {
            focus = Some(focus.map_or(rect, |f| union(f, rect)));
            continue;
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
    Some(SnapCandidates::new(boxes, focus.unwrap_or([0.0; 4])))
}

fn union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let x0 = a[0].min(b[0]);
    let y0 = a[1].min(b[1]);
    let x1 = (a[0] + a[2]).max(b[0] + b[2]);
    let y1 = (a[1] + a[3]).max(b[1] + b[3]);
    [x0, y0, x1 - x0, y1 - y0]
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: SnapSettings = SnapSettings {
        objects: true,
        pixels: true,
    };
    const OBJECTS: SnapSettings = SnapSettings {
        objects: true,
        pixels: false,
    };

    fn parent() -> SnapCandidate {
        SnapCandidate {
            rect: [0.0, 0.0, 400.0, 300.0],
            kind: CandidateKind::Parent,
        }
    }

    fn sibling(rect: [f32; 4]) -> SnapCandidate {
        SnapCandidate {
            rect,
            kind: CandidateKind::Sibling,
        }
    }

    fn candidates(siblings: &[[f32; 4]]) -> SnapCandidates {
        let mut boxes = vec![parent()];
        boxes.extend(siblings.iter().map(|r| sibling(*r)));
        SnapCandidates::new(boxes, [0.0; 4])
    }

    #[test]
    fn move_prefers_parent_then_sibling_edges_and_centers() {
        let set = candidates(&[[200.0, 50.0, 50.0, 50.0]]);
        // 부모 왼쪽 가장자리(0)에 3px 안.
        let r = snap([3.0, 120.0, 40.0, 40.0], SnapMode::Move, 6.0, OBJECTS, &set).unwrap();
        assert_eq!(r.rect[0], 0.0);
        assert_eq!(r.guides[0], Some(0.0));
        // 형제 위 가장자리(50)에 맞춥니다.
        let r = snap(
            [100.0, 52.0, 40.0, 40.0],
            SnapMode::Move,
            6.0,
            OBJECTS,
            &set,
        )
        .unwrap();
        assert_eq!((r.rect[1], r.guides[1]), (50.0, Some(50.0)));
        // 부모 중심(200)과 형제 가장자리(200)가 같은 거리면 부모가 먼저입니다.
        let r = snap(
            [178.0, 120.0, 40.0, 40.0],
            SnapMode::Move,
            6.0,
            OBJECTS,
            &set,
        )
        .unwrap();
        assert_eq!(r.rect[0], 180.0);
        // 허용 오차 밖이면 그대로입니다.
        let r = snap(
            [20.0, 120.0, 40.0, 40.0],
            SnapMode::Move,
            6.0,
            OBJECTS,
            &set,
        )
        .unwrap();
        assert_eq!(r.rect, [20.0, 120.0, 40.0, 40.0]);
        assert_eq!(r.guides, [None, None]);
    }

    #[test]
    fn move_matches_equal_spacing_and_reports_distance() {
        // 형제 두 개 사이 간격 30. 오른쪽 형제와 28 떨어져 있으면 30으로 맞춥니다.
        let set = candidates(&[[10.0, 100.0, 40.0, 40.0], [80.0, 100.0, 40.0, 40.0]]);
        assert_eq!(set.gaps[0], vec![30.0]);
        let r = snap(
            [148.0, 100.0, 40.0, 40.0],
            SnapMode::Move,
            3.0,
            OBJECTS,
            &set,
        )
        .unwrap();
        assert_eq!(r.rect[0], 150.0);
        assert_eq!(r.spacing[0], Some(30.0));
        assert_eq!(r.guides[0], None);
        assert_eq!(r.distance[0], Some(30.0));
    }

    #[test]
    fn pixel_snap_rounds_to_integer_parent_px_only_without_object_hit() {
        let set = candidates(&[]);
        let r = snap(
            [100.4, 200.6, 10.0, 10.0],
            SnapMode::Move,
            1.0,
            SnapSettings {
                objects: false,
                pixels: true,
            },
            &set,
        )
        .unwrap();
        assert_eq!(&r.rect[..2], &[100.0, 201.0]);
        assert_eq!(r.guides, [None, None]);
        let r = snap([0.4, 0.0, 10.0, 10.0], SnapMode::Move, 1.0, ON, &set).unwrap();
        assert_eq!((r.rect[0], r.guides[0]), (0.0, Some(0.0)));
    }

    fn resize(handle: ResizeHandle, shift: bool, option: bool) -> SnapMode {
        SnapMode::Resize {
            handle,
            modifiers: Modifiers { shift, option },
        }
    }

    #[test]
    fn resize_size_match_is_not_an_alignment_guide() {
        let set = candidates(&[[300.0, 10.0, 60.0, 30.0]]);
        // 오른쪽 변을 끌어 너비 58 → 형제와 같은 60.
        let r = snap(
            [100.0, 100.0, 58.0, 20.0],
            resize(ResizeHandle::Right, false, false),
            3.0,
            OBJECTS,
            &set,
        )
        .unwrap();
        assert_eq!(r.rect, [100.0, 100.0, 60.0, 20.0]);
        assert_eq!(r.size_match[0], Some(60.0));
        assert_eq!(r.guides, [None, None]);
    }

    #[test]
    fn resize_keeps_option_center_and_skips_shift_ratio() {
        let set = candidates(&[]);
        // 중심 기준: 오른쪽 변을 부모 오른쪽(400)에 맞추면 왼쪽 변도 같은 거리만큼 움직여 중심(300)을 지킵니다.
        let r = snap(
            [202.0, 100.0, 196.0, 20.0],
            resize(ResizeHandle::Right, false, true),
            3.0,
            OBJECTS,
            &set,
        )
        .unwrap();
        assert_eq!(r.rect, [200.0, 100.0, 200.0, 20.0]);
        assert_eq!(r.rect[0] + r.rect[2] / 2.0, 300.0);
        // 비율 유지는 스냅하지 않습니다.
        let rect = [202.0, 100.0, 196.0, 98.0];
        let r = snap(
            rect,
            resize(ResizeHandle::BottomRight, true, false),
            3.0,
            ON,
            &set,
        )
        .unwrap();
        assert_eq!(r.rect, rect);
        // 왼쪽 위 핸들은 옮기는 변(왼쪽·위)만 맞춥니다.
        let r = snap(
            [2.0, 1.0, 98.0, 99.0],
            resize(ResizeHandle::TopLeft, false, false),
            3.0,
            OBJECTS,
            &set,
        )
        .unwrap();
        assert_eq!(r.rect, [0.0, 0.0, 100.0, 100.0]);
    }

    #[test]
    fn too_many_siblings_are_limited_to_the_nearest_instead_of_rejected() {
        let mut boxes = vec![parent()];
        for index in 0..(MAX_SIBLINGS + 100) {
            boxes.push(sibling([index as f32 * 10.0, 0.0, 5.0, 5.0]));
        }
        let set = SnapCandidates::new(boxes, [0.0, 0.0, 5.0, 5.0]);
        assert_eq!(set.boxes.len(), 1 + MAX_SIBLINGS);
        assert_eq!(set.boxes[0].kind, CandidateKind::Parent);
        // 가장 먼 형제(x = 40,990)는 빠지고 가까운 형제는 남습니다.
        assert!(set.boxes.iter().any(|c| c.rect[0] == 10.0));
        assert!(!set
            .boxes
            .iter()
            .any(|c| c.rect[0] == ((MAX_SIBLINGS + 99) as f32) * 10.0));
    }

    #[test]
    fn invalid_input_is_rejected_and_tolerance_converts_screen_points() {
        let set = candidates(&[]);
        assert!(snap([f32::NAN, 0.0, 1.0, 1.0], SnapMode::Move, 6.0, ON, &set).is_none());
        assert!(snap([0.0, 0.0, -1.0, 1.0], SnapMode::Move, 6.0, ON, &set).is_none());
        assert!(snap([0.0, 0.0, 1.0, 1.0], SnapMode::Move, f32::NAN, ON, &set).is_none());
        assert_eq!(tolerance(6.0, 2.0), Some(3.0));
        assert_eq!(tolerance(6.0, 0.0), None);
    }

    #[test]
    fn collect_reads_parent_local_boxes_from_the_scene_and_excludes_moving_nodes() {
        use crate::canvas::CanvasCache;
        use crate::window_session::test_fixtures::{
            identity, sample_document, saved_project_with, sid, window,
        };
        use crate::window_session::ProjectWindowSession;
        use stellar_core_types::value::Size2;
        use stellar_document::frame::FramePayload;
        use stellar_document::records::NodePayload;
        let placed = |w: f32, h: f32, transform: Transform2D| {
            NodePayload::Frame(FramePayload {
                size: Size2::new(w, h).unwrap(),
                transform,
                ..FramePayload::default()
            })
        };
        let at = |x: f32, y: f32| Transform2D::new(1., 0., 0., 1., x, y).unwrap();
        let mut model = sample_document();
        // root 10(400×300) 아래 11(10,10 100×100)과 12(200,50 50×50, 90° 회전).
        model
            .set_payload(sid(10), placed(400., 300., at(0., 0.)))
            .unwrap();
        model
            .set_payload(sid(11), placed(100., 100., at(10., 10.)))
            .unwrap();
        let rotated = Transform2D::new(0., 1., -1., 0., 250., 50.).unwrap();
        model
            .set_payload(sid(12), placed(50., 20., rotated))
            .unwrap();
        let dir = saved_project_with("snap-collect", &[&model]);
        let session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        let scene = CanvasCache::default()
            .current(&session, None, &mut Default::default())
            .unwrap();
        let set = collect(&scene, &[sid(11)]).unwrap();
        assert_eq!(set.boxes[0].kind, CandidateKind::Parent);
        assert_eq!(set.boxes[0].rect, [0.0, 0.0, 400.0, 300.0]);
        // 회전한 형제는 부모 local 경계 상자(x 230~250, y 50~100)입니다. 움직이는 11은 후보가 아닙니다.
        assert_eq!(set.boxes.len(), 2);
        let [x, y, w, h] = set.boxes[1].rect;
        assert!(
            (x - 230.0).abs() < 1e-3 && (y - 50.0).abs() < 1e-3,
            "{x} {y}"
        );
        assert!(
            (w - 20.0).abs() < 1e-3 && (h - 50.0).abs() < 1e-3,
            "{w} {h}"
        );
        // root는 부모가 없어 스냅 후보를 만들지 않습니다.
        assert!(collect(&scene, &[sid(10)]).is_none());
    }

    #[test]
    #[ignore = "release에서 형제 4096개 후보의 스냅 한 번 시간 측정(0.75ms 예산)"]
    fn measure_snap_with_max_siblings() {
        let mut boxes = vec![parent()];
        for index in 0..MAX_SIBLINGS {
            let (col, row) = ((index % 64) as f32, (index / 64) as f32);
            boxes.push(sibling([col * 30.0, row * 30.0, 20.0, 20.0]));
        }
        let set = SnapCandidates::new(boxes, [0.0; 4]);
        let mut times = Vec::new();
        for step in 0..101 {
            let started = std::time::Instant::now();
            let r = snap(
                [step as f32 * 7.3, 333.3, 25.0, 25.0],
                SnapMode::Move,
                3.0,
                ON,
                &set,
            );
            times.push(started.elapsed());
            assert!(r.is_some());
        }
        times.sort();
        eprintln!(
            "snap 4096 형제: 중앙값 {:?}, 최대 {:?}",
            times[50], times[100]
        );
    }
}

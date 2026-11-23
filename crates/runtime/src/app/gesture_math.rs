//! 캔버스 핸들·생성 도구의 포인터 해석. 미리보기와 확정이 같은 결과를 사용합니다.

/// Shift 비율·각도 제한.
pub const SHIFT: u32 = 1;
/// Option 중심·개별 반경.
pub const OPTION: u32 = 2;
/// 생성 도구의 클릭(기본 100px).
pub const CLICK: u32 = 4;

/// 코어가 해석하는 포인터 제스처. 좌표는 대상 부모 또는 선택 항목의 local 공간입니다.
#[derive(Clone, Copy)]
pub struct GestureInput {
    /// 1 크기, 2 Line 끝점, 3 반경, 4 생성, 5 Line 생성.
    pub kind: u32,
    /// 크기·반경 핸들 번호 또는 Line 끝점 번호.
    pub handle: i32,
    /// Shift·Option·click bit.
    pub flags: u32,
    /// 시작 상자(x, y, width, height). 생성일 때 x·y는 누른 점.
    pub rect: [f32; 4],
    /// 시작·끝점(x0, y0, x1, y1).
    pub line: [f32; 4],
    /// 모서리 반경(tl, tr, br, bl).
    pub radii: [f32; 4],
    /// 현재 포인터 local 좌표.
    pub point: [f32; 2],
}

/// 미리보기와 저장 요청에 공유되는 상자·끝점·반경.
#[derive(Clone, Copy)]
pub struct GestureResult {
    /// 해석한 상자.
    pub rect: [f32; 4],
    /// 해석한 선 또는 생성 두 점.
    pub line: [f32; 4],
    /// 해석한 네 반경.
    pub radii: [f32; 4],
}

/// 포인터 제스처를 결정적 결과로 해석합니다. 잘못된 핸들·비유한 값은 거부합니다.
pub fn preview(input: GestureInput) -> Option<GestureResult> {
    if input.flags & !(SHIFT | OPTION | CLICK) != 0
        || !input
            .rect
            .iter()
            .chain(input.line.iter())
            .chain(input.radii.iter())
            .chain(input.point.iter())
            .all(|v| v.is_finite())
    {
        return None;
    }
    let mut result = GestureResult {
        rect: input.rect,
        line: input.line,
        radii: input.radii,
    };
    let [x, y, width, height] = input.rect;
    let [px, py] = input.point;
    match input.kind {
        1 if (0..8).contains(&input.handle) && width > 0.0 && height > 0.0 => {
            let h = input.handle;
            let min_x = matches!(h, 0 | 3 | 7);
            let max_x = matches!(h, 1 | 2 | 5);
            let min_y = matches!(h, 0 | 1 | 4);
            let max_y = matches!(h, 2 | 3 | 6);
            let centered = input.flags & OPTION != 0;
            let axis = |lo: f32, size: f32, pointer: f32, moves_min: bool, moves_max: bool| {
                if !moves_min && !moves_max {
                    return None;
                }
                let anchor = if centered {
                    lo + size / 2.0
                } else if moves_min {
                    lo + size
                } else {
                    lo
                };
                Some((anchor, pointer - anchor))
            };
            let horizontal = axis(x, width, px, min_x, max_x);
            let vertical = axis(y, height, py, min_y, max_y);
            let reference_x = if centered { width / 2.0 } else { width };
            let reference_y = if centered { height / 2.0 } else { height };
            let scale = (input.flags & SHIFT != 0).then(|| {
                (horizontal.map_or(0.0, |(_, extent)| extent.abs() / reference_x))
                    .max(vertical.map_or(0.0, |(_, extent)| extent.abs() / reference_y))
            });
            let range = |axis: Option<(f32, f32)>, lo: f32, size: f32, reference: f32| {
                if let Some((anchor, mut extent)) = axis {
                    if let Some(scale) = scale {
                        extent = extent.signum() * reference * scale;
                    }
                    let (start, end) = if centered {
                        (anchor - extent.abs(), anchor + extent.abs())
                    } else {
                        (anchor.min(anchor + extent), anchor.max(anchor + extent))
                    };
                    (start, end.max(start + 1.0))
                } else if let Some(scale) = scale {
                    let half = (size * scale).max(1.0) / 2.0;
                    (lo + size / 2.0 - half, lo + size / 2.0 + half)
                } else {
                    (lo, lo + size)
                }
            };
            let (x0, x1) = range(horizontal, x, width, reference_x);
            let (y0, y1) = range(vertical, y, height, reference_y);
            result.rect = [x0, y0, x1 - x0, y1 - y0];
        }
        2 if input.handle == 0 || input.handle == 1 => {
            let end = input.handle == 1;
            let other = if end {
                [input.line[0], input.line[1]]
            } else {
                [input.line[2], input.line[3]]
            };
            let point = if input.flags & SHIFT != 0 {
                angle45(other, [px, py])
            } else {
                [px, py]
            };
            if end {
                result.line[2..4].copy_from_slice(&point);
            } else {
                result.line[0..2].copy_from_slice(&point);
            }
        }
        3 if (-1..4).contains(&input.handle) && width > 0.0 && height > 0.0 => {
            let h = input.handle;
            let limit = width.min(height) / 2.0;
            let value = if h < 0 {
                py - y
            } else {
                let (cx, cy, sx, sy) = match h {
                    0 => (x, y, 1.0, 1.0),
                    1 => (x + width, y, -1.0, 1.0),
                    2 => (x + width, y + height, -1.0, -1.0),
                    _ => (x, y + height, 1.0, -1.0),
                };
                ((px - cx) * sx + (py - cy) * sy) / 2.0
            }
            .clamp(0.0, limit)
            .round();
            for (index, radius) in result.radii.iter_mut().enumerate() {
                if h < 0 || input.flags & OPTION == 0 || index == h as usize {
                    *radius = value;
                }
            }
        }
        4 | 5 => {
            let line = input.kind == 5;
            let centered = input.flags & OPTION != 0;
            let mut dx = px - x;
            let mut dy = py - y;
            if input.flags & CLICK != 0 {
                if line {
                    return None;
                }
                dx = if centered { 50.0 } else { 100.0 };
                dy = dx;
            } else if input.flags & SHIFT != 0 {
                if line {
                    [dx, dy] = angle45([0.0, 0.0], [dx, dy]);
                } else {
                    let extent = dx.abs().max(dy.abs());
                    dx = dx.signum() * extent;
                    dy = dy.signum() * extent;
                }
            }
            let start = if centered { [x - dx, y - dy] } else { [x, y] };
            let end = [x + dx, y + dy];
            result.line = [start[0], start[1], end[0], end[1]];
            result.rect = [
                start[0].min(end[0]),
                start[1].min(end[1]),
                dx.abs() * if centered { 2.0 } else { 1.0 },
                dy.abs() * if centered { 2.0 } else { 1.0 },
            ];
        }
        _ => return None,
    }
    result
        .rect
        .iter()
        .chain(result.line.iter())
        .chain(result.radii.iter())
        .all(|v| v.is_finite())
        .then_some(result)
}

fn angle45(origin: [f32; 2], point: [f32; 2]) -> [f32; 2] {
    let dx = point[0] - origin[0];
    let dy = point[1] - origin[1];
    let octant = (dy.atan2(dx) / std::f32::consts::FRAC_PI_4).round() as i32;
    let angle = octant as f32 * std::f32::consts::FRAC_PI_4;
    let length = dx.hypot(dy);
    let mut x = angle.cos();
    let mut y = angle.sin();
    if octant % 2 == 0 {
        x = x.round();
        y = y.round();
    }
    [origin[0] + length * x, origin[1] + length * y]
}

/// 제스처 시작 때 모은 부모(1)·형제(2)의 같은 좌표계 상자.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SnapCandidate {
    /// x, y, width, height.
    pub rect: [f32; 4],
    /// 1 부모, 2 형제, 3 수평 간격, 4 수직 간격.
    pub kind: u32,
}

/// 객체 스냅 토글.
pub const SNAP_OBJECTS: u32 = 1;
/// 논리 픽셀 정수 스냅 토글.
pub const SNAP_PIXELS: u32 = 2;

/// 스냅 결과. guide flags bit0 x, bit1 y, bit2 x 간격, bit3 y 간격.
#[derive(Clone, Copy)]
pub struct SnapResult {
    /// 조정된 x, y, width, height.
    pub rect: [f32; 4],
    /// 일치한 x 기준선.
    pub guide_x: f32,
    /// 일치한 y 기준선.
    pub guide_y: f32,
    /// 가장 가까운 형제와의 수평 간격(px).
    pub distance_x: f32,
    /// 가장 가까운 형제와의 수직 간격(px).
    pub distance_y: f32,
    /// 표시할 가이드 bit.
    pub flags: u32,
}

/// 제스처 시작 때 형제 사이의 양수 간격을 한 번 추출합니다. 드래그 중에는 정렬을 반복하지 않습니다.
pub fn snap_intervals(candidates: &[SnapCandidate]) -> Vec<SnapCandidate> {
    let siblings: Vec<_> = candidates.iter().filter(|c| c.kind == 2).copied().collect();
    let mut gaps = Vec::new();
    for axis in 0..2 {
        let mut seen = std::collections::BTreeSet::new();
        let other = 1 - axis;
        let mut ordered = siblings.clone();
        ordered.sort_by(|a, b| a.rect[axis].total_cmp(&b.rect[axis]));
        for pair in ordered.windows(2) {
            let a = pair[0].rect;
            let b = pair[1].rect;
            if a[other] >= b[other] + b[other + 2] || b[other] >= a[other] + a[other + 2] {
                continue;
            }
            let gap = b[axis] - a[axis] - a[axis + 2];
            if gap > 0.0 && seen.insert(gap.to_bits()) {
                gaps.push(SnapCandidate {
                    rect: [gap, 0.0, 0.0, 0.0],
                    kind: axis as u32 + 3,
                });
            }
        }
    }
    gaps
}

/// `mode` 0 이동, 1 크기; `handle`은 크기 핸들 번호. 허용 오차는 화면 6pt를 `scale`로 나눕니다.
pub fn snap(
    rect: [f32; 4],
    mode: u32,
    handle: i32,
    scale: f32,
    pixel_origin: [f32; 2],
    pixel_step: f32,
    flags: u32,
    candidates: &[SnapCandidate],
) -> Option<SnapResult> {
    if !(scale.is_finite() && scale > 0.0 && pixel_step.is_finite() && pixel_step > 0.0)
        || pixel_origin.iter().any(|v| !v.is_finite())
        || flags & !(SNAP_OBJECTS | SNAP_PIXELS) != 0
        || rect.iter().any(|v| !v.is_finite())
        || rect[2] < 0.0
        || rect[3] < 0.0
        || candidates.len() > 4096
        || candidates.iter().any(|c| {
            !(1..=4).contains(&c.kind)
                || c.rect.iter().any(|v| !v.is_finite())
                || c.rect[2] < 0.0
                || c.rect[3] < 0.0
        })
    {
        return None;
    }
    if mode > 1 || mode == 1 && !(0..8).contains(&handle) {
        return None;
    }
    let tolerance = 6.0 / scale;
    let mut output = SnapResult {
        rect,
        guide_x: 0.0,
        guide_y: 0.0,
        distance_x: 0.0,
        distance_y: 0.0,
        flags: 0,
    };
    for axis in 0..2 {
        let moves_min = mode == 1
            && if axis == 0 {
                matches!(handle, 0 | 3 | 7)
            } else {
                matches!(handle, 0 | 1 | 4)
            };
        let moves_max = mode == 1
            && if axis == 0 {
                matches!(handle, 1 | 2 | 5)
            } else {
                matches!(handle, 2 | 3 | 6)
            };
        if mode == 1 && !moves_min && !moves_max {
            continue;
        }
        let start = rect[axis];
        let size = rect[axis + 2];
        let moving = if mode == 0 {
            [start, start + size / 2.0, start + size]
        } else if moves_min {
            [start, start, start]
        } else {
            [start + size, start + size, start + size]
        };
        let mut best: Option<(f32, f32, u32, f32)> = None;
        if flags & SNAP_OBJECTS != 0 {
            for kind in [1, 2] {
                for candidate in candidates.iter().filter(|c| c.kind == kind) {
                    let c_start = candidate.rect[axis];
                    let c_size = candidate.rect[axis + 2];
                    let anchors = [c_start, c_start + c_size / 2.0, c_start + c_size];
                    for (source_index, source) in moving.iter().enumerate() {
                        if mode == 1 && source_index > 0 {
                            break;
                        }
                        for anchor in anchors {
                            let delta = anchor - source;
                            if delta.abs() <= tolerance
                                && best.is_none_or(|(old, _, old_kind, _)| {
                                    kind < old_kind || kind == old_kind && delta.abs() < old.abs()
                                })
                            {
                                best = Some((delta, anchor, kind, c_size));
                            }
                        }
                    }
                    // 크기 조절 중에는 형제와 같은 너비·높이도 후보입니다.
                    if mode == 1 && kind == 2 {
                        let delta = if moves_min {
                            size - c_size
                        } else {
                            c_size - size
                        };
                        if delta.abs() <= tolerance
                            && best.is_none_or(|(old, _, old_kind, _)| {
                                kind < old_kind || kind == old_kind && delta.abs() < old.abs()
                            })
                        {
                            best = Some((delta, moving[0] + delta, kind, c_size));
                        }
                    }
                }
            }
        }
        if best.is_none() && mode == 0 && flags & SNAP_OBJECTS != 0 {
            let other = 1 - axis;
            let nearest = candidates
                .iter()
                .filter(|c| c.kind == 2)
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
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((current, direction)) = nearest {
                for gap in candidates
                    .iter()
                    .filter(|c| c.kind == if axis == 0 { 3 } else { 4 })
                {
                    let delta = direction * (gap.rect[0] - current);
                    if delta.abs() <= tolerance
                        && best.is_none_or(|(old, _, _, _)| delta.abs() < old.abs())
                    {
                        best = Some((delta, moving[0] + delta, 3, gap.rect[0]));
                    }
                }
            }
        }
        if best.is_none() && flags & SNAP_PIXELS != 0 {
            let value = moving[0];
            let snapped = ((value - pixel_origin[axis]) / pixel_step).round() * pixel_step
                + pixel_origin[axis];
            let delta = snapped - value;
            if delta.abs() <= tolerance {
                best = Some((delta, snapped, 3, size));
            }
        }
        if let Some((delta, guide, kind, _)) = best {
            if mode == 0 || moves_min {
                output.rect[axis] += delta;
            }
            if mode == 1 {
                output.rect[axis + 2] += if moves_min { -delta } else { delta };
                if output.rect[axis + 2] < 1.0 {
                    output.rect[axis + 2] = 1.0;
                }
            }
            if kind < 3 {
                if axis == 0 {
                    output.guide_x = guide;
                    output.flags |= 1;
                } else {
                    output.guide_y = guide;
                    output.flags |= 2;
                }
            }
        }
    }
    // 정렬 후 가장 가까운 형제 간격. 결과에만 넣고 문서에는 기록하지 않습니다.
    let [x, y, w, h] = output.rect;
    for candidate in candidates.iter().filter(|c| c.kind == 2) {
        let [cx, cy, cw, ch] = candidate.rect;
        if y < cy + ch && cy < y + h {
            let gap = if cx >= x + w {
                cx - (x + w)
            } else if x >= cx + cw {
                x - (cx + cw)
            } else {
                0.0
            };
            if gap > 0.0 && (output.flags & 4 == 0 || gap < output.distance_x) {
                output.distance_x = gap;
                output.flags |= 4;
            }
        }
        if x < cx + cw && cx < x + w {
            let gap = if cy >= y + h {
                cy - (y + h)
            } else if y >= cy + ch {
                y - (cy + ch)
            } else {
                0.0
            };
            if gap > 0.0 && (output.flags & 8 == 0 || gap < output.distance_y) {
                output.distance_y = gap;
                output.flags |= 8;
            }
        }
    }
    Some(output)
}

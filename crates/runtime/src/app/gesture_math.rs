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

/// macOS 화살표 키를 부모 공간의 1px 또는 Shift 10px 이동으로 해석합니다.
pub fn arrow_nudge(key_code: u16, shift: bool) -> Option<[f32; 2]> {
    let step = if shift { 10.0 } else { 1.0 };
    match key_code {
        123 => Some([-step, 0.0]),
        124 => Some([step, 0.0]),
        125 => Some([0.0, step]),
        126 => Some([0.0, -step]),
        _ => None,
    }
}

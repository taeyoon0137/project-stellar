//! 캔버스 핸들·생성 도구의 포인터 해석. 미리보기와 확정이 같은 결과를 사용합니다.
//!
//! 입력은 타입 있는 [`Gesture`]와 [`Modifiers`]입니다. C ABI(`stellar_canvas_gesture_preview`)의 kind·handle·flag
//! 숫자는 FFI 층이 [`Gesture::from_abi`]로 한 번 바꾸고, 이 모듈 안에서는 숫자 표를 쓰지 않습니다.

/// 생성 도구를 클릭만 했을 때 만드는 상자 한 변(논리 px). Option(중심)이면 클릭한 점이 중심입니다.
pub const CLICK_SIZE: f32 = 100.0;
/// 크기 핸들 결과의 최소 변(논리 px).
pub const MIN_SIZE: f32 = 1.0;

/// 크기 핸들 8종. 번호는 C ABI 순서(모서리 TL·TR·BR·BL, 변 T·R·B·L)입니다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeHandle {
    /// 왼쪽 위 모서리.
    TopLeft,
    /// 오른쪽 위 모서리.
    TopRight,
    /// 오른쪽 아래 모서리.
    BottomRight,
    /// 왼쪽 아래 모서리.
    BottomLeft,
    /// 위 변.
    Top,
    /// 오른쪽 변.
    Right,
    /// 아래 변.
    Bottom,
    /// 왼쪽 변.
    Left,
}

impl ResizeHandle {
    /// 8종 전체(C ABI 번호 순서).
    pub const ALL: [ResizeHandle; 8] = [
        ResizeHandle::TopLeft,
        ResizeHandle::TopRight,
        ResizeHandle::BottomRight,
        ResizeHandle::BottomLeft,
        ResizeHandle::Top,
        ResizeHandle::Right,
        ResizeHandle::Bottom,
        ResizeHandle::Left,
    ];

    /// C ABI 번호(0~7)에서 바꿉니다.
    pub fn from_index(index: i32) -> Option<ResizeHandle> {
        usize::try_from(index)
            .ok()
            .and_then(|index| Self::ALL.get(index).copied())
    }

    /// 축(0 x, 1 y)에서 이 핸들이 옮기는 변: (작은 쪽, 큰 쪽).
    pub fn moves(self, axis: usize) -> (bool, bool) {
        use ResizeHandle as H;
        if axis == 0 {
            (
                matches!(self, H::TopLeft | H::BottomLeft | H::Left),
                matches!(self, H::TopRight | H::BottomRight | H::Right),
            )
        } else {
            (
                matches!(self, H::TopLeft | H::TopRight | H::Top),
                matches!(self, H::BottomRight | H::BottomLeft | H::Bottom),
            )
        }
    }
}

/// Line의 두 끝점.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnd {
    /// 시작점(x0, y0).
    Start,
    /// 끝점(x1, y1).
    End,
}

/// 반경 핸들의 모서리.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    /// 왼쪽 위.
    TopLeft,
    /// 오른쪽 위.
    TopRight,
    /// 오른쪽 아래.
    BottomRight,
    /// 왼쪽 아래.
    BottomLeft,
}

impl Corner {
    const ALL: [Corner; 4] = [
        Corner::TopLeft,
        Corner::TopRight,
        Corner::BottomRight,
        Corner::BottomLeft,
    ];

    fn index(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).expect("corner")
    }
}

/// 제스처에 함께 누른 수식키.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// 크기는 비율 유지, Line·생성 Line은 45° 단위, 생성 상자는 정사각형.
    pub shift: bool,
    /// 크기·생성은 중심 기준, 반경은 한 모서리만.
    pub option: bool,
}

/// 코어가 해석하는 포인터 제스처. 좌표는 대상 부모 또는 선택 항목의 local 공간입니다.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    /// 크기 핸들. `rect`는 누른 시점의 상자(x, y, width, height, 양수 크기).
    Resize {
        /// 잡은 핸들.
        handle: ResizeHandle,
        /// 시작 상자.
        rect: [f32; 4],
    },
    /// Line 끝점 하나를 옮깁니다. `line`은 (x0, y0, x1, y1)입니다.
    LineEnd {
        /// 옮기는 끝점.
        end: LineEnd,
        /// 시작 선.
        line: [f32; 4],
    },
    /// 모서리 반경. `corner`가 없으면 값 하나(Polygon)를 위 변에서 잽니다.
    Radius {
        /// 잡은 모서리.
        corner: Option<Corner>,
        /// 대상 상자.
        rect: [f32; 4],
        /// 시작 반경(tl, tr, br, bl).
        radii: [f32; 4],
    },
    /// 생성 도구. `origin`은 누른 점, `click`은 끌지 않고 놓은 경우입니다.
    Create {
        /// Line 생성이면 true.
        line: bool,
        /// 누른 점.
        origin: [f32; 2],
        /// 끌지 않고 놓음.
        click: bool,
    },
}

/// 미리보기와 저장 요청에 공유되는 결과. 제스처 종류마다 바꾸는 값만 담습니다.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GestureOutput {
    /// 크기 핸들의 새 상자(x, y, width, height).
    Rect([f32; 4]),
    /// Line 끝점을 옮긴 선(x0, y0, x1, y1).
    Line([f32; 4]),
    /// 새 네 반경(tl, tr, br, bl).
    Radii([f32; 4]),
    /// 생성 상자와 두 점(Line 생성은 두 점이 선입니다).
    Created {
        /// 두 점을 감싸는 상자.
        rect: [f32; 4],
        /// 시작점과 끝점.
        line: [f32; 4],
    },
}

impl GestureOutput {
    fn finite(&self) -> bool {
        let values: &[f32] = match self {
            GestureOutput::Rect(v) | GestureOutput::Line(v) | GestureOutput::Radii(v) => v,
            GestureOutput::Created { rect, line } => {
                return rect.iter().chain(line).all(|v| v.is_finite())
            }
        };
        values.iter().all(|v| v.is_finite())
    }
}

/// C ABI flag: Shift.
pub const ABI_SHIFT: u32 = 1;
/// C ABI flag: Option.
pub const ABI_OPTION: u32 = 2;
/// C ABI flag: 생성 도구의 클릭.
pub const ABI_CLICK: u32 = 4;

impl Gesture {
    /// `StellarGesturePreview` 필드에서 바꿉니다. kind 1 크기, 2 Line 끝점, 3 반경, 4 생성, 5 Line 생성입니다.
    /// 알 수 없는 kind·handle·flag는 `None`입니다.
    pub fn from_abi(
        kind: u32,
        handle: i32,
        flags: u32,
        rect: [f32; 4],
        line: [f32; 4],
        radii: [f32; 4],
    ) -> Option<(Gesture, Modifiers)> {
        if flags & !(ABI_SHIFT | ABI_OPTION | ABI_CLICK) != 0 {
            return None;
        }
        let modifiers = Modifiers {
            shift: flags & ABI_SHIFT != 0,
            option: flags & ABI_OPTION != 0,
        };
        let click = flags & ABI_CLICK != 0;
        if click && !matches!(kind, 4 | 5) {
            return None;
        }
        let gesture = match kind {
            1 => Gesture::Resize {
                handle: ResizeHandle::from_index(handle)?,
                rect,
            },
            2 => Gesture::LineEnd {
                end: match handle {
                    0 => LineEnd::Start,
                    1 => LineEnd::End,
                    _ => return None,
                },
                line,
            },
            3 => Gesture::Radius {
                corner: match handle {
                    -1 => None,
                    0..=3 => Some(Corner::ALL[handle as usize]),
                    _ => return None,
                },
                rect,
                radii,
            },
            4 | 5 => Gesture::Create {
                line: kind == 5,
                origin: [rect[0], rect[1]],
                click,
            },
            _ => return None,
        };
        Some((gesture, modifiers))
    }
}

/// 포인터 제스처를 결정적 결과로 해석합니다. 비유한 값, 크기 0 이하 상자, 클릭 Line 생성은 거부합니다.
///
/// - 크기: 반대 변(Option이면 중심)을 고정하고 포인터까지 늘립니다. Shift는 두 축 중 큰 배율로 비율을 지키고, 한 축만
///   움직이는 변 핸들도 다른 축을 중심 기준으로 같은 배율로 키웁니다. 결과 변은 [`MIN_SIZE`] 이상입니다.
/// - Line 끝점·Line 생성: Shift는 반대 끝점 기준 45° 단위로 맞춥니다. 축 방향(0°·90°…)은 cos·sin을 반올림해
///   좌표가 정확히 축 위에 놓입니다(f32 cos(π/2)의 오차 제거, 2026-11-23 결정).
/// - 반경: 모서리에서 상자 안쪽 대각선 방향 거리의 절반, 값 하나는 위 변에서의 거리입니다. 0과 짧은 변의 절반 사이로
///   자르고 정수로 반올림합니다. Option은 잡은 모서리만 바꿉니다.
/// - 생성: 누른 점에서 포인터까지. 클릭은 [`CLICK_SIZE`] 정사각형, Shift는 정사각형(Line은 45°), Option은 중심 기준입니다.
pub fn preview(gesture: Gesture, modifiers: Modifiers, pointer: [f32; 2]) -> Option<GestureOutput> {
    let inputs: &[f32] = match &gesture {
        Gesture::Resize { rect, .. } => rect,
        Gesture::LineEnd { line, .. } => line,
        Gesture::Radius { rect, radii, .. } => {
            if !radii.iter().all(|v| v.is_finite()) {
                return None;
            }
            rect
        }
        Gesture::Create { origin, .. } => origin,
    };
    if !inputs.iter().chain(pointer.iter()).all(|v| v.is_finite()) {
        return None;
    }
    let [px, py] = pointer;
    let result = match gesture {
        Gesture::Resize { handle, rect } => {
            let [x, y, width, height] = rect;
            if !(width > 0.0 && height > 0.0) {
                return None;
            }
            let centered = modifiers.option;
            let axis = |lo: f32, size: f32, pointer: f32, (moves_min, moves_max): (bool, bool)| {
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
            let horizontal = axis(x, width, px, handle.moves(0));
            let vertical = axis(y, height, py, handle.moves(1));
            let reference_x = if centered { width / 2.0 } else { width };
            let reference_y = if centered { height / 2.0 } else { height };
            let scale = modifiers.shift.then(|| {
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
                    (start, end.max(start + MIN_SIZE))
                } else if let Some(scale) = scale {
                    let half = (size * scale).max(MIN_SIZE) / 2.0;
                    (lo + size / 2.0 - half, lo + size / 2.0 + half)
                } else {
                    (lo, lo + size)
                }
            };
            let (x0, x1) = range(horizontal, x, width, reference_x);
            let (y0, y1) = range(vertical, y, height, reference_y);
            GestureOutput::Rect([x0, y0, x1 - x0, y1 - y0])
        }
        Gesture::LineEnd { end, line } => {
            let other = match end {
                LineEnd::End => [line[0], line[1]],
                LineEnd::Start => [line[2], line[3]],
            };
            let point = if modifiers.shift {
                angle45(other, pointer)
            } else {
                pointer
            };
            let mut moved = line;
            match end {
                LineEnd::End => moved[2..4].copy_from_slice(&point),
                LineEnd::Start => moved[0..2].copy_from_slice(&point),
            }
            GestureOutput::Line(moved)
        }
        Gesture::Radius {
            corner,
            rect,
            radii,
        } => {
            let [x, y, width, height] = rect;
            if !(width > 0.0 && height > 0.0) {
                return None;
            }
            let limit = width.min(height) / 2.0;
            let value = match corner {
                None => py - y,
                Some(corner) => {
                    let (cx, cy, sx, sy) = match corner {
                        Corner::TopLeft => (x, y, 1.0, 1.0),
                        Corner::TopRight => (x + width, y, -1.0, 1.0),
                        Corner::BottomRight => (x + width, y + height, -1.0, -1.0),
                        Corner::BottomLeft => (x, y + height, 1.0, -1.0),
                    };
                    ((px - cx) * sx + (py - cy) * sy) / 2.0
                }
            }
            .clamp(0.0, limit)
            .round();
            let only = corner.filter(|_| modifiers.option).map(Corner::index);
            let mut next = radii;
            for (index, radius) in next.iter_mut().enumerate() {
                if only.is_none_or(|only| only == index) {
                    *radius = value;
                }
            }
            GestureOutput::Radii(next)
        }
        Gesture::Create {
            line,
            origin,
            click,
        } => {
            let [x, y] = origin;
            let centered = modifiers.option;
            let mut dx = px - x;
            let mut dy = py - y;
            if click {
                if line {
                    return None;
                }
                dx = if centered {
                    CLICK_SIZE / 2.0
                } else {
                    CLICK_SIZE
                };
                dy = dx;
            } else if modifiers.shift {
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
            let factor = if centered { 2.0 } else { 1.0 };
            GestureOutput::Created {
                rect: [
                    start[0].min(end[0]),
                    start[1].min(end[1]),
                    dx.abs() * factor,
                    dy.abs() * factor,
                ],
                line: [start[0], start[1], end[0], end[1]],
            }
        }
    };
    result.finite().then_some(result)
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

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers {
        shift: false,
        option: false,
    };
    const SHIFT: Modifiers = Modifiers {
        shift: true,
        option: false,
    };
    const OPTION: Modifiers = Modifiers {
        shift: false,
        option: true,
    };
    /// x 10, y 20, 100×50.
    const BOX: [f32; 4] = [10.0, 20.0, 100.0, 50.0];

    fn resize(handle: ResizeHandle, modifiers: Modifiers, pointer: [f32; 2]) -> [f32; 4] {
        match preview(Gesture::Resize { handle, rect: BOX }, modifiers, pointer) {
            Some(GestureOutput::Rect(rect)) => rect,
            other => panic!("{handle:?}: {other:?}"),
        }
    }

    #[test]
    fn each_resize_handle_moves_only_its_edges() {
        use ResizeHandle as H;
        // 포인터를 상자 바깥 (-10, -10)/(+10, +10)으로 끕니다.
        let cases = [
            (H::TopLeft, [0.0, 10.0], [0.0, 10.0, 110.0, 60.0]),
            (H::TopRight, [120.0, 10.0], [10.0, 10.0, 110.0, 60.0]),
            (H::BottomRight, [120.0, 80.0], [10.0, 20.0, 110.0, 60.0]),
            (H::BottomLeft, [0.0, 80.0], [0.0, 20.0, 110.0, 60.0]),
            (H::Top, [999.0, 10.0], [10.0, 10.0, 100.0, 60.0]),
            (H::Right, [120.0, 999.0], [10.0, 20.0, 110.0, 50.0]),
            (H::Bottom, [999.0, 80.0], [10.0, 20.0, 100.0, 60.0]),
            (H::Left, [0.0, 999.0], [0.0, 20.0, 110.0, 50.0]),
        ];
        for (handle, pointer, expected) in cases {
            assert_eq!(resize(handle, NONE, pointer), expected, "{handle:?}");
        }
        // 반대 변을 넘어 끌면 뒤집힌 상자가 아니라 고정 변 반대쪽 상자입니다.
        assert_eq!(resize(H::Right, NONE, [0.0, 0.0]), [0.0, 20.0, 10.0, 50.0]);
    }

    #[test]
    fn resize_keeps_a_minimum_of_one_pixel() {
        assert_eq!(
            resize(ResizeHandle::BottomRight, NONE, [10.0, 20.0]),
            [10.0, 20.0, 1.0, 1.0]
        );
        assert_eq!(
            resize(ResizeHandle::Left, NONE, [110.0, 0.0]),
            [110.0, 20.0, 1.0, 50.0]
        );
    }

    #[test]
    fn shift_keeps_aspect_ratio_and_option_resizes_from_the_center() {
        // 오른쪽 아래를 가로 2배로 끌면 세로도 2배입니다.
        assert_eq!(
            resize(ResizeHandle::BottomRight, SHIFT, [210.0, 30.0]),
            [10.0, 20.0, 200.0, 100.0]
        );
        // 변 핸들은 다른 축을 중심 기준으로 같은 배율로 키웁니다.
        assert_eq!(
            resize(ResizeHandle::Right, SHIFT, [210.0, 0.0]),
            [10.0, -5.0, 200.0, 100.0]
        );
        // Option: 중심(60, 45)을 고정하고 양쪽으로 같은 만큼 늘립니다.
        assert_eq!(
            resize(ResizeHandle::Right, OPTION, [120.0, 0.0]),
            [0.0, 20.0, 120.0, 50.0]
        );
        // Shift+Option: 중심 고정과 비율 유지.
        let both = Modifiers {
            shift: true,
            option: true,
        };
        assert_eq!(
            resize(ResizeHandle::BottomRight, both, [160.0, 0.0]),
            [-40.0, -5.0, 200.0, 100.0]
        );
    }

    fn line_end(end: LineEnd, modifiers: Modifiers, pointer: [f32; 2]) -> [f32; 4] {
        match preview(
            Gesture::LineEnd {
                end,
                line: [0.0, 0.0, 10.0, 0.0],
            },
            modifiers,
            pointer,
        ) {
            Some(GestureOutput::Line(line)) => line,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn line_end_snaps_to_45_degrees_with_exact_axis_values() {
        assert_eq!(
            line_end(LineEnd::End, NONE, [7.0, 3.0]),
            [0.0, 0.0, 7.0, 3.0]
        );
        // 거의 수직이면 정확히 x = 0 위에 놓입니다(cos 반올림).
        let vertical = line_end(LineEnd::End, SHIFT, [0.5, 20.0]);
        assert_eq!(vertical[2], 0.0);
        assert!((vertical[3] - 0.5f32.hypot(20.0)).abs() < 1e-4);
        // 45°: 두 성분이 같습니다.
        let diagonal = line_end(LineEnd::End, SHIFT, [10.0, 9.0]);
        assert!((diagonal[2] - diagonal[3]).abs() < 1e-4, "{diagonal:?}");
        // 시작점은 끝점 기준입니다.
        assert_eq!(
            line_end(LineEnd::Start, SHIFT, [10.5, -30.0])[..2],
            [10.0, -(0.5f32.hypot(30.0))]
        );
    }

    fn radius(corner: Option<Corner>, modifiers: Modifiers, pointer: [f32; 2]) -> [f32; 4] {
        match preview(
            Gesture::Radius {
                corner,
                rect: BOX,
                radii: [1.0, 2.0, 3.0, 4.0],
            },
            modifiers,
            pointer,
        ) {
            Some(GestureOutput::Radii(radii)) => radii,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn radius_is_clamped_rounded_and_option_changes_one_corner() {
        // 왼쪽 위에서 안쪽으로 (8, 9) → (8+9)/2 = 8.5 → 9(반올림), 네 모서리.
        assert_eq!(radius(Some(Corner::TopLeft), NONE, [18.0, 29.0]), [9.0; 4]);
        // 짧은 변(50)의 절반 25에서 자르고, 바깥쪽은 0입니다.
        assert_eq!(
            radius(Some(Corner::BottomRight), NONE, [0.0, 0.0]),
            [25.0; 4]
        );
        assert_eq!(radius(Some(Corner::TopRight), NONE, [200.0, 0.0]), [0.0; 4]);
        // Option은 잡은 모서리만 바꿉니다.
        assert_eq!(
            radius(Some(Corner::BottomLeft), OPTION, [16.0, 64.0]),
            [1.0, 2.0, 3.0, 6.0]
        );
        // 값 하나(Polygon)는 위 변에서의 거리이고 Option과 무관하게 네 값이 같습니다.
        assert_eq!(radius(None, OPTION, [0.0, 32.0]), [12.0; 4]);
    }

    fn create(
        line: bool,
        click: bool,
        modifiers: Modifiers,
        pointer: [f32; 2],
    ) -> Option<GestureOutput> {
        preview(
            Gesture::Create {
                line,
                origin: [50.0, 50.0],
                click,
            },
            modifiers,
            pointer,
        )
    }

    #[test]
    fn create_click_shift_and_option() {
        assert_eq!(
            create(false, true, NONE, [0.0, 0.0]),
            Some(GestureOutput::Created {
                rect: [50.0, 50.0, CLICK_SIZE, CLICK_SIZE],
                line: [50.0, 50.0, 150.0, 150.0],
            })
        );
        // Option 클릭은 누른 점이 중심입니다.
        assert_eq!(
            create(false, true, OPTION, [0.0, 0.0]),
            Some(GestureOutput::Created {
                rect: [0.0, 0.0, CLICK_SIZE, CLICK_SIZE],
                line: [0.0, 0.0, 100.0, 100.0],
            })
        );
        // 끌기: 왼쪽 위로 끌어도 양수 상자이고, Shift는 정사각형입니다.
        assert_eq!(
            create(false, false, SHIFT, [20.0, 40.0]),
            Some(GestureOutput::Created {
                rect: [20.0, 20.0, 30.0, 30.0],
                line: [50.0, 50.0, 20.0, 20.0],
            })
        );
        // Line은 클릭으로 만들지 않습니다.
        assert_eq!(create(true, true, NONE, [0.0, 0.0]), None);
        // Line 생성 Shift는 45° 단위입니다.
        let Some(GestureOutput::Created { line, .. }) = create(true, false, SHIFT, [90.0, 51.0])
        else {
            panic!("line");
        };
        assert_eq!(line[3], 50.0);
    }

    #[test]
    fn abi_conversion_rejects_unknown_values_and_non_finite_input() {
        let zero = [0.0; 4];
        assert!(Gesture::from_abi(1, 8, 0, BOX, zero, zero).is_none());
        assert!(Gesture::from_abi(2, 2, 0, BOX, zero, zero).is_none());
        assert!(Gesture::from_abi(3, 4, 0, BOX, zero, zero).is_none());
        assert!(Gesture::from_abi(6, 0, 0, BOX, zero, zero).is_none());
        assert!(Gesture::from_abi(1, 0, 8, BOX, zero, zero).is_none());
        // 클릭 flag는 생성 도구에만 있습니다.
        assert!(Gesture::from_abi(1, 0, ABI_CLICK, BOX, zero, zero).is_none());
        let (gesture, modifiers) =
            Gesture::from_abi(3, -1, ABI_SHIFT | ABI_OPTION, BOX, zero, zero).unwrap();
        assert_eq!(
            gesture,
            Gesture::Radius {
                corner: None,
                rect: BOX,
                radii: zero
            }
        );
        assert!(modifiers.shift && modifiers.option);
        assert_eq!(
            preview(
                Gesture::Resize {
                    handle: ResizeHandle::Top,
                    rect: [0.0, 0.0, 0.0, 10.0]
                },
                NONE,
                [0.0, 0.0]
            ),
            None
        );
        assert_eq!(
            preview(
                Gesture::Resize {
                    handle: ResizeHandle::Top,
                    rect: BOX
                },
                NONE,
                [f32::NAN, 0.0]
            ),
            None
        );
    }
}

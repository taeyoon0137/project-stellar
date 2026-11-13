//! 레이아웃 좌표·크기 (X.1.1).
//!
//! 레이아웃 결과의 모든 좌표는 logical subpixel 정수 [`LayoutUnit`](X.1.5)입니다. 따라서 결과는
//! 항상 유한하며, core-types의 finite 기하([`Rect`])로 변환해도 NaN·±무한대가 생기지 않습니다.

use std::fmt;

use stellar_core_types::value::{Rect, Size2, Vec2};

use crate::rounding::{LayoutUnit, LayoutUnitError};

/// 레이아웃 축.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Axis {
    /// 가로(x, width).
    Horizontal,
    /// 세로(y, height).
    Vertical,
}

impl Axis {
    /// 두 축(가로, 세로 순서).
    pub const BOTH: [Axis; 2] = [Axis::Horizontal, Axis::Vertical];

    /// 반대 축.
    pub const fn cross(self) -> Axis {
        match self {
            Axis::Horizontal => Axis::Vertical,
            Axis::Vertical => Axis::Horizontal,
        }
    }

    /// 배열 index(가로 0, 세로 1).
    pub const fn index(self) -> usize {
        match self {
            Axis::Horizontal => 0,
            Axis::Vertical => 1,
        }
    }
}

impl fmt::Display for Axis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Axis::Horizontal => f.write_str("가로"),
            Axis::Vertical => f.write_str("세로"),
        }
    }
}

/// 비음수 레이아웃 크기 (X.1.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct LayoutSize {
    width: LayoutUnit,
    height: LayoutUnit,
}

impl LayoutSize {
    /// 0 크기.
    pub const ZERO: LayoutSize = LayoutSize {
        width: LayoutUnit::ZERO,
        height: LayoutUnit::ZERO,
    };

    /// 비음수 크기를 만듭니다. 음수 성분은 `None`.
    pub fn new(width: LayoutUnit, height: LayoutUnit) -> Option<LayoutSize> {
        if width < LayoutUnit::ZERO || height < LayoutUnit::ZERO {
            return None;
        }
        Some(LayoutSize { width, height })
    }

    /// 너비.
    pub const fn width(&self) -> LayoutUnit {
        self.width
    }

    /// 높이.
    pub const fn height(&self) -> LayoutUnit {
        self.height
    }

    /// 축 성분.
    pub const fn get(&self, axis: Axis) -> LayoutUnit {
        match axis {
            Axis::Horizontal => self.width,
            Axis::Vertical => self.height,
        }
    }
}

/// 축 정렬 레이아웃 사각형(origin + 비음수 size) (X.1.1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct LayoutRect {
    x: LayoutUnit,
    y: LayoutUnit,
    width: LayoutUnit,
    height: LayoutUnit,
}

impl LayoutRect {
    /// 원점의 0 크기 사각형.
    pub const ZERO: LayoutRect = LayoutRect {
        x: LayoutUnit::ZERO,
        y: LayoutUnit::ZERO,
        width: LayoutUnit::ZERO,
        height: LayoutUnit::ZERO,
    };

    /// 사각형을 만듭니다. 너비·높이가 음수면 `None`.
    pub fn new(
        x: LayoutUnit,
        y: LayoutUnit,
        width: LayoutUnit,
        height: LayoutUnit,
    ) -> Option<LayoutRect> {
        let size = LayoutSize::new(width, height)?;
        Some(LayoutRect::from_origin_size(x, y, size))
    }

    /// origin과 비음수 size로 만듭니다.
    pub const fn from_origin_size(x: LayoutUnit, y: LayoutUnit, size: LayoutSize) -> LayoutRect {
        LayoutRect {
            x,
            y,
            width: size.width,
            height: size.height,
        }
    }

    /// 원점에 놓인 `size` 크기 사각형(node 자신의 local bounds).
    pub const fn at_origin(size: LayoutSize) -> LayoutRect {
        LayoutRect::from_origin_size(LayoutUnit::ZERO, LayoutUnit::ZERO, size)
    }

    /// x.
    pub const fn x(&self) -> LayoutUnit {
        self.x
    }

    /// y.
    pub const fn y(&self) -> LayoutUnit {
        self.y
    }

    /// 너비.
    pub const fn width(&self) -> LayoutUnit {
        self.width
    }

    /// 높이.
    pub const fn height(&self) -> LayoutUnit {
        self.height
    }

    /// 크기.
    pub const fn size(&self) -> LayoutSize {
        LayoutSize {
            width: self.width,
            height: self.height,
        }
    }

    /// 축 방향 시작 좌표.
    pub const fn start(&self, axis: Axis) -> LayoutUnit {
        match axis {
            Axis::Horizontal => self.x,
            Axis::Vertical => self.y,
        }
    }

    /// 오른쪽 끝(x + width). 범위를 넘으면 오류입니다.
    pub fn right(&self) -> Result<LayoutUnit, LayoutUnitError> {
        self.x.checked_add(self.width)
    }

    /// 아래쪽 끝(y + height). 범위를 넘으면 오류입니다.
    pub fn bottom(&self) -> Result<LayoutUnit, LayoutUnitError> {
        self.y.checked_add(self.height)
    }

    /// `(dx, dy)`만큼 옮긴 사각형.
    pub fn translated(
        &self,
        dx: LayoutUnit,
        dy: LayoutUnit,
    ) -> Result<LayoutRect, LayoutUnitError> {
        Ok(LayoutRect {
            x: self.x.checked_add(dx)?,
            y: self.y.checked_add(dy)?,
            width: self.width,
            height: self.height,
        })
    }

    /// 두 사각형을 모두 포함하는 최소 사각형.
    pub fn union(&self, other: &LayoutRect) -> Result<LayoutRect, LayoutUnitError> {
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = self.right()?.max(other.right()?);
        let bottom = self.bottom()?.max(other.bottom()?);
        Ok(LayoutRect {
            x: left,
            y: top,
            width: right.checked_sub(left)?,
            height: bottom.checked_sub(top)?,
        })
    }

    /// `other`가 이 사각형 안에 완전히 들어가는지 여부.
    pub fn contains_rect(&self, other: &LayoutRect) -> bool {
        match (self.right(), self.bottom(), other.right(), other.bottom()) {
            (Ok(right), Ok(bottom), Ok(other_right), Ok(other_bottom)) => {
                self.x <= other.x
                    && self.y <= other.y
                    && other_right <= right
                    && other_bottom <= bottom
            }
            _ => false,
        }
    }

    /// core-types의 finite [`Rect`]로 변환합니다(logical px f32). 결과는 항상 유한합니다.
    pub fn to_rect(&self) -> Rect {
        let origin = Vec2::new(self.x.to_px_f32().get(), self.y.to_px_f32().get())
            .expect("LayoutUnit의 px 값은 항상 유한");
        let size = Size2::new(self.width.to_px_f32().get(), self.height.to_px_f32().get())
            .expect("LayoutRect 크기는 유한·비음수");
        Rect::new(origin, size)
    }
}

impl fmt::Display for LayoutRect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {}) {}×{}", self.x, self.y, self.width, self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(px: i32) -> LayoutUnit {
        LayoutUnit::from_px_int(px).unwrap()
    }

    #[test]
    fn size_and_rect_reject_negative() {
        assert!(LayoutSize::new(unit(-1), unit(0)).is_none());
        assert!(LayoutRect::new(unit(0), unit(0), unit(1), unit(-1)).is_none());
        let rect = LayoutRect::new(unit(-5), unit(3), unit(10), unit(4)).unwrap();
        assert_eq!(rect.right().unwrap(), unit(5));
        assert_eq!(rect.bottom().unwrap(), unit(7));
        assert_eq!(rect.size().get(Axis::Vertical), unit(4));
        assert_eq!(rect.start(Axis::Horizontal), unit(-5));
    }

    #[test]
    fn union_translate_and_contains() {
        let a = LayoutRect::new(unit(0), unit(0), unit(10), unit(10)).unwrap();
        let b = LayoutRect::new(unit(5), unit(-2), unit(10), unit(4)).unwrap();
        let u = a.union(&b).unwrap();
        assert_eq!(
            u,
            LayoutRect::new(unit(0), unit(-2), unit(15), unit(12)).unwrap()
        );
        assert!(u.contains_rect(&a) && u.contains_rect(&b));
        assert!(!a.contains_rect(&b));
        assert_eq!(
            a.translated(unit(2), unit(3)).unwrap(),
            LayoutRect::new(unit(2), unit(3), unit(10), unit(10)).unwrap()
        );
        assert!(LayoutRect::new(LayoutUnit::MAX, unit(0), unit(1), unit(0))
            .unwrap()
            .right()
            .is_err());
    }

    #[test]
    fn to_rect_is_finite_px() {
        let rect = LayoutRect::new(
            LayoutUnit::from_raw(21),
            LayoutUnit::MIN,
            LayoutUnit::MAX,
            LayoutUnit::from_raw(64),
        )
        .unwrap();
        let converted = rect.to_rect();
        assert_eq!(converted.origin().x(), 0.328125);
        assert!(converted.origin().y().is_finite());
        assert!(converted.size().width().is_finite());
        assert_eq!(converted.size().height(), 1.0);
        assert_eq!(Axis::Horizontal.cross(), Axis::Vertical);
        assert_eq!(Axis::Vertical.index(), 1);
        assert!(rect.to_string().contains("×"));
    }
}

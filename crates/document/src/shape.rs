//! Shape payload (VI.5).
//!
//! 파라메트릭 도형은 **하나의 node kind**([`NodeKind::Shape`](crate::node_kind::NodeKind))
//! 안에서 [`ShapeKind`]로 구분합니다 — 도형 종류마다 node kind를 늘리지 않습니다.

use std::fmt;

use stellar_core_types::value::Size2;

use crate::frame::CornerRadius;

/// Shape payload 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShapeError {
    /// 알 수 없는 shape kind tag.
    UnknownKind {
        /// 발견된 tag.
        tag: u8,
    },
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for ShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShapeError::UnknownKind { tag } => write!(f, "알 수 없는 shape kind tag: {tag}"),
            ShapeError::Malformed { reason } => write!(f, "Shape binary 형식 위반: {reason}"),
        }
    }
}

impl std::error::Error for ShapeError {}

/// 도형 종류. 숫자 tag는 저장 포맷의 일부입니다(golden test로 고정).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ShapeKind {
    /// 사각형(VI.5.2).
    Rectangle = 1,
    /// 타원(VI.5.3).
    Ellipse = 2,
    /// 직선(VI.5.4).
    Line = 3,
    /// 정다각형(VI.5.5).
    Polygon = 4,
}

impl ShapeKind {
    /// 정의된 모든 종류.
    pub const ALL: [ShapeKind; 4] = [
        ShapeKind::Rectangle,
        ShapeKind::Ellipse,
        ShapeKind::Line,
        ShapeKind::Polygon,
    ];

    /// 안정 numeric tag.
    pub fn tag(self) -> u8 {
        self as u8
    }

    /// tag에서 복원합니다.
    pub fn from_tag(tag: u8) -> Result<ShapeKind, ShapeError> {
        ShapeKind::ALL
            .into_iter()
            .find(|k| k.tag() == tag)
            .ok_or(ShapeError::UnknownKind { tag })
    }

    /// 진단용 이름.
    pub fn name(self) -> &'static str {
        match self {
            ShapeKind::Rectangle => "rectangle",
            ShapeKind::Ellipse => "ellipse",
            ShapeKind::Line => "line",
            ShapeKind::Polygon => "polygon",
        }
    }
}

/// Rectangle 파라미터 (VI.5.2).
///
/// 저장값은 사용자가 지정한 **원본 radius**를 그대로 유지합니다. bounds를 넘는
/// radius는 평가 단계([`RectangleParams::effective_corner_radius`])에서만
/// clamp합니다 — 크기를 다시 키우면 원래 둥글기가 돌아옵니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RectangleParams {
    /// 크기(비음수).
    pub size: Size2,
    /// 개별 corner radius(원본, 비음수).
    pub corner_radius: CornerRadius,
}

impl RectangleParams {
    /// 렌더·path 평가에 쓰는 실제 radius `(top_left, top_right, bottom_right,
    /// bottom_left)`.
    ///
    /// 각 변에서 인접한 두 radius의 합이 변 길이를 넘으면, 모든 radius를 같은
    /// 비율 `f = min(변 길이 / 인접 radius 합)`으로 줄입니다(형태 비율 보존).
    pub fn effective_corner_radius(&self) -> (f32, f32, f32, f32) {
        let (tl, tr, br, bl) = self.corner_radius.components();
        let w = self.size.width();
        let h = self.size.height();
        let mut factor: f32 = 1.0;
        for (side, sum) in [(w, tl + tr), (h, tr + br), (w, br + bl), (h, bl + tl)] {
            if sum > side {
                factor = factor.min(if sum > 0.0 { side / sum } else { 1.0 });
            }
        }
        (tl * factor, tr * factor, br * factor, bl * factor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_variants_round_trip() {
        // 검증 기준(VI.5.1): kind variant round-trip.
        for kind in ShapeKind::ALL {
            assert_eq!(ShapeKind::from_tag(kind.tag()).unwrap(), kind);
        }
    }

    #[test]
    fn golden_tags_are_stable() {
        let golden = [
            (ShapeKind::Rectangle, 1, "rectangle"),
            (ShapeKind::Ellipse, 2, "ellipse"),
            (ShapeKind::Line, 3, "line"),
            (ShapeKind::Polygon, 4, "polygon"),
        ];
        for (kind, tag, name) in golden {
            assert_eq!(kind.tag(), tag);
            assert_eq!(kind.name(), name);
        }
    }

    #[test]
    fn unknown_tags_are_rejected() {
        for tag in [0u8, 5, 255] {
            assert_eq!(
                ShapeKind::from_tag(tag).unwrap_err(),
                ShapeError::UnknownKind { tag }
            );
        }
    }

    #[test]
    fn shapes_share_one_node_kind() {
        // 도형 종류를 node kind로 늘리지 않습니다.
        use crate::node_kind::NodeKind;
        for name in ["rectangle", "ellipse", "line", "polygon"] {
            assert!(NodeKind::ALL.iter().all(|k| k.name() != name));
        }
    }

    #[test]
    fn oversize_radius_reference_fixture() {
        // 검증 기준(VI.5.2): oversize radius reference fixture 통과.
        // 100×50, 모든 radius 40 → 세로 변 합 80 > 50 → f = 0.625 → 25.
        let params = RectangleParams {
            size: Size2::new(100.0, 50.0).unwrap(),
            corner_radius: CornerRadius::new(40.0, 40.0, 40.0, 40.0).unwrap(),
        };
        assert_eq!(params.effective_corner_radius(), (25.0, 25.0, 25.0, 25.0));
        // 저장값은 원본 유지.
        assert_eq!(params.corner_radius.components(), (40.0, 40.0, 40.0, 40.0));

        // 비대칭: 200×100, tl=150, 나머지 0 → 위 변 150 ≤ 200, 왼 변 150 > 100 → f=2/3.
        let params = RectangleParams {
            size: Size2::new(200.0, 100.0).unwrap(),
            corner_radius: CornerRadius::new(150.0, 0.0, 0.0, 0.0).unwrap(),
        };
        let (tl, tr, br, bl) = params.effective_corner_radius();
        assert!((tl - 100.0).abs() < 1e-4, "tl={tl}");
        assert_eq!((tr, br, bl), (0.0, 0.0, 0.0));
    }

    #[test]
    fn fitting_radius_is_unchanged_and_zero_size_collapses() {
        let params = RectangleParams {
            size: Size2::new(100.0, 100.0).unwrap(),
            corner_radius: CornerRadius::new(10.0, 20.0, 30.0, 40.0).unwrap(),
        };
        assert_eq!(params.effective_corner_radius(), (10.0, 20.0, 30.0, 40.0));
        // 크기 0이면 실효 radius 0(원본은 유지).
        let params = RectangleParams {
            size: Size2::ZERO,
            corner_radius: CornerRadius::new(8.0, 8.0, 8.0, 8.0).unwrap(),
        };
        assert_eq!(params.effective_corner_radius(), (0.0, 0.0, 0.0, 0.0));
        assert_eq!(params.corner_radius.components(), (8.0, 8.0, 8.0, 8.0));
    }
}

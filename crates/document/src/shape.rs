//! Shape payload (VI.5).
//!
//! 파라메트릭 도형은 **하나의 node kind**([`NodeKind::Shape`](crate::node_kind::NodeKind))
//! 안에서 [`ShapeKind`]로 구분합니다 — 도형 종류마다 node kind를 늘리지 않습니다.

use std::fmt;

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
}

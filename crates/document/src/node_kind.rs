//! Node 종류 (VI.2.1).
//!
//! scene tree에 존재할 수 있는 node 종류입니다. 숫자 값은 저장 포맷의 일부라
//! golden test로 고정합니다.
//!
//! **ComponentDefinition은 scene node가 아닙니다** — 컴포넌트 정의는 문서
//! resource(ComponentLibrary chunk, V.6.1)로 유지되고, scene에는
//! [`NodeKind::ComponentInstance`]만 배치됩니다.

/// scene node 종류.
///
/// 숫자 값은 안정적으로 유지되어야 합니다(golden test로 고정).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum NodeKind {
    /// layout containing block과 좌표계를 만드는 컨테이너.
    Frame = 1,
    /// 렌더·layout 필드가 없는 그룹 marker.
    Fragment = 2,
    /// 파라메트릭 도형(Rectangle/Ellipse/Line/Polygon, VI.5).
    Shape = 3,
    /// 자유 곡선 경로(VI.6).
    Path = 4,
    /// 텍스트(VI.7).
    Text = 5,
    /// 이미지(VI.8).
    Image = 6,
    /// 컴포넌트 인스턴스(정의는 resource).
    ComponentInstance = 7,
}

impl NodeKind {
    /// 정의된 모든 종류.
    pub const ALL: [NodeKind; 7] = [
        NodeKind::Frame,
        NodeKind::Fragment,
        NodeKind::Shape,
        NodeKind::Path,
        NodeKind::Text,
        NodeKind::Image,
        NodeKind::ComponentInstance,
    ];

    /// 안정적인 숫자 값.
    pub fn as_u16(self) -> u16 {
        self as u16
    }

    /// 숫자 값에서 복원합니다. 알 수 없는 값은 `None`.
    pub fn from_u16(value: u16) -> Option<NodeKind> {
        NodeKind::ALL.into_iter().find(|k| k.as_u16() == value)
    }

    /// 진단용 이름.
    pub fn name(self) -> &'static str {
        match self {
            NodeKind::Frame => "frame",
            NodeKind::Fragment => "fragment",
            NodeKind::Shape => "shape",
            NodeKind::Path => "path",
            NodeKind::Text => "text",
            NodeKind::Image => "image",
            NodeKind::ComponentInstance => "component-instance",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_numeric_kinds_are_stable() {
        // 검증 기준(VI.2.1): numeric kind golden test.
        // 숫자 값은 저장 포맷의 일부 — 변경은 스키마 마이그레이션과 함께만 허용.
        let golden: [(NodeKind, u16, &str); 7] = [
            (NodeKind::Frame, 1, "frame"),
            (NodeKind::Fragment, 2, "fragment"),
            (NodeKind::Shape, 3, "shape"),
            (NodeKind::Path, 4, "path"),
            (NodeKind::Text, 5, "text"),
            (NodeKind::Image, 6, "image"),
            (NodeKind::ComponentInstance, 7, "component-instance"),
        ];
        assert_eq!(NodeKind::ALL.len(), golden.len());
        for (kind, value, name) in golden {
            assert_eq!(kind.as_u16(), value, "{name}");
            assert_eq!(kind.name(), name);
            assert_eq!(NodeKind::from_u16(value), Some(kind));
        }
    }

    #[test]
    fn unknown_numeric_values_are_rejected() {
        assert_eq!(NodeKind::from_u16(0), None);
        assert_eq!(NodeKind::from_u16(8), None);
        assert_eq!(NodeKind::from_u16(u16::MAX), None);
    }

    #[test]
    fn component_definition_is_not_a_scene_node() {
        // ComponentDefinition은 resource로 유지 — scene node kind에 없어야 함.
        for kind in NodeKind::ALL {
            assert_ne!(kind.name(), "component-definition");
        }
    }
}

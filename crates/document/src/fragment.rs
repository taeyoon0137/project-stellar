//! Fragment payload (VI.4).
//!
//! Fragment는 자식을 묶는 **zero-size marker**입니다. 렌더·layout 필드가 없고
//! 좌표계도 만들지 않습니다(VI.3.2). 공통 node column(VI.2.2: id, kind, parent,
//! order, visible, locked, 표시 이름) 외에 어떤 필드도 저장하지 않습니다.

use std::fmt;

/// Fragment payload 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FragmentError {
    /// payload bytes가 비어 있지 않습니다(Fragment는 필드가 없음).
    UnexpectedPayload {
        /// 발견된 byte 수.
        len: usize,
    },
    /// Fragment에는 설정할 수 없는 시각·layout 속성입니다(VI.4.2).
    ForbiddenProperty {
        /// 시도한 속성 종류.
        class: VisualPropertyClass,
    },
}

/// 렌더·layout에 영향을 주는 속성 분류 (VI.4.2).
///
/// Fragment는 이 중 어떤 것도 가질 수 없습니다 — 필요하면 Frame으로 변환합니다
/// (VI.4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VisualPropertyClass {
    /// local transform.
    Transform,
    /// 불투명도.
    Opacity,
    /// blend mode.
    Blend,
    /// mask.
    Mask,
    /// filter(blur·shadow 등).
    Filter,
    /// fill(배경·채우기).
    Fill,
    /// border·stroke.
    Border,
    /// padding(layout inset).
    Padding,
}

impl VisualPropertyClass {
    /// 정의된 모든 분류.
    pub const ALL: [VisualPropertyClass; 8] = [
        VisualPropertyClass::Transform,
        VisualPropertyClass::Opacity,
        VisualPropertyClass::Blend,
        VisualPropertyClass::Mask,
        VisualPropertyClass::Filter,
        VisualPropertyClass::Fill,
        VisualPropertyClass::Border,
        VisualPropertyClass::Padding,
    ];

    /// 진단용 이름.
    pub fn name(self) -> &'static str {
        match self {
            VisualPropertyClass::Transform => "transform",
            VisualPropertyClass::Opacity => "opacity",
            VisualPropertyClass::Blend => "blend",
            VisualPropertyClass::Mask => "mask",
            VisualPropertyClass::Filter => "filter",
            VisualPropertyClass::Fill => "fill",
            VisualPropertyClass::Border => "border",
            VisualPropertyClass::Padding => "padding",
        }
    }
}

/// node 종류에 시각 속성을 설정할 수 있는지 검증합니다(VI.4.2).
///
/// Fragment는 모든 [`VisualPropertyClass`]를 거부합니다. 다른 종류의 세부 허용
/// 규칙은 각 종류의 property registry(VI.3.5 등)가 담당하며 여기서는 통과시킵니다.
pub fn validate_visual_property(
    kind: crate::node_kind::NodeKind,
    class: VisualPropertyClass,
) -> Result<(), FragmentError> {
    if kind == crate::node_kind::NodeKind::Fragment {
        return Err(FragmentError::ForbiddenProperty { class });
    }
    Ok(())
}

impl fmt::Display for FragmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FragmentError::UnexpectedPayload { len } => {
                write!(f, "Fragment payload는 비어 있어야 합니다({len} bytes 발견)")
            }
            FragmentError::ForbiddenProperty { class } => write!(
                f,
                "Fragment에는 {}를 설정할 수 없습니다. Frame으로 변환한 뒤 설정하세요",
                class.name()
            ),
        }
    }
}

impl std::error::Error for FragmentError {}

/// Fragment node payload: 필드 없는 marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FragmentPayload;

impl FragmentPayload {
    /// canonical bytes: 항상 빈 bytes(공통 metadata 외 추가 필드 없음).
    pub fn encode(&self) -> Vec<u8> {
        Vec::new()
    }

    /// bytes에서 복원합니다. 비어 있지 않으면 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<FragmentPayload, FragmentError> {
        if bytes.is_empty() {
            Ok(FragmentPayload)
        } else {
            Err(FragmentError::UnexpectedPayload { len: bytes.len() })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialized_payload_has_no_fields() {
        // 검증 기준(VI.4.1): payload serialized bytes가 metadata 외 추가 필드 없음.
        let bytes = FragmentPayload.encode();
        assert!(bytes.is_empty());
        assert_eq!(FragmentPayload::decode(&bytes).unwrap(), FragmentPayload);
        // 타입 자체도 zero-size.
        assert_eq!(std::mem::size_of::<FragmentPayload>(), 0);
    }

    #[test]
    fn non_empty_payload_is_rejected() {
        assert_eq!(
            FragmentPayload::decode(&[0x00]).unwrap_err(),
            FragmentError::UnexpectedPayload { len: 1 }
        );
    }

    #[test]
    fn every_forbidden_property_fails_on_fragment() {
        // 검증 기준(VI.4.2): 금지 속성 fixture 전부 실패.
        use crate::node_kind::NodeKind;
        for class in VisualPropertyClass::ALL {
            let err = validate_visual_property(NodeKind::Fragment, class).unwrap_err();
            assert_eq!(err, FragmentError::ForbiddenProperty { class });
            assert!(err.to_string().contains(class.name()));
        }
        assert_eq!(VisualPropertyClass::ALL.len(), 8);
    }

    #[test]
    fn other_kinds_pass_through_fragment_rule() {
        use crate::node_kind::NodeKind;
        for kind in NodeKind::ALL {
            if kind == NodeKind::Fragment {
                continue;
            }
            for class in VisualPropertyClass::ALL {
                assert!(
                    validate_visual_property(kind, class).is_ok(),
                    "{}",
                    kind.name()
                );
            }
        }
    }
}

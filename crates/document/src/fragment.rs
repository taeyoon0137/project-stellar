//! Fragment payload (VI.4).
//!
//! Fragment는 자식을 묶는 **zero-size marker**입니다. 렌더·layout 필드가 없고
//! 좌표계도 만들지 않습니다(VI.3.2). 공통 node column(VI.2.2: id, kind, parent,
//! order, visible, locked, 표시 이름) 외에 어떤 필드도 저장하지 않습니다.

use std::fmt;

use stellar_core_types::ids::RuntimeEntityId;
use stellar_core_types::value::{Rect, Size2, Vec2};

use crate::node_columns::NodeColumns;
use crate::node_kind::NodeKind;
use crate::tree_ops::children_in_order;

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
    kind: NodeKind,
    class: VisualPropertyClass,
) -> Result<(), FragmentError> {
    if kind == NodeKind::Fragment {
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

// --- visual bounds (VI.4.3) ---

fn union_rect(a: Rect, b: Rect) -> Rect {
    let min_x = a.origin().x().min(b.origin().x());
    let min_y = a.origin().y().min(b.origin().y());
    let max_x = (a.origin().x() + a.size().width()).max(b.origin().x() + b.size().width());
    let max_y = (a.origin().y() + a.size().height()).max(b.origin().y() + b.size().height());
    // 입력이 finite이면 합집합 경계도 finite(오버플로 시 원래 a를 유지).
    match (
        Vec2::new(min_x, min_y),
        Size2::new(max_x - min_x, max_y - min_y),
    ) {
        (Ok(origin), Ok(size)) => Rect::new(origin, size),
        _ => a,
    }
}

/// Fragment의 visual bounds: **가시 자식 bounds의 합집합**(read-only helper).
///
/// - Fragment는 좌표계를 만들지 않으므로 자식 bounds는 containing Frame
///   좌표계에서 받습니다(`leaf_bounds`가 제공).
/// - 숨김(`visible == false`) 자식은 제외합니다.
/// - 중첩 Fragment는 재귀적으로 그 가시 자식들의 합집합을 사용합니다.
/// - 가시 자식이 없으면(빈 Fragment) `None`입니다.
pub fn fragment_visual_bounds(
    columns: &NodeColumns,
    fragment: RuntimeEntityId,
    leaf_bounds: &dyn Fn(RuntimeEntityId) -> Option<Rect>,
) -> Option<Rect> {
    let mut union: Option<Rect> = None;
    for child in children_in_order(columns, fragment) {
        let Some(fields) = columns.get(child) else {
            continue;
        };
        if !fields.visible {
            continue;
        }
        let bounds = if fields.kind == NodeKind::Fragment {
            fragment_visual_bounds(columns, child, leaf_bounds)
        } else {
            leaf_bounds(child)
        };
        if let Some(rect) = bounds {
            union = Some(match union {
                None => rect,
                Some(current) => union_rect(current, rect),
            });
        }
    }
    union
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

    mod bounds {
        use super::super::*;
        use crate::node_columns::NodeFields;
        use std::collections::HashMap;
        use stellar_core_types::ids::StableId;
        use stellar_core_types::order_key::OrderKey;

        fn insert(
            columns: &mut NodeColumns,
            seed: u64,
            kind: NodeKind,
            parent: Option<RuntimeEntityId>,
            visible: bool,
        ) -> RuntimeEntityId {
            columns.insert(NodeFields {
                stable_id: StableId::new_v7(seed, 0, seed),
                kind,
                parent,
                order_key: OrderKey::from_bytes(vec![(seed % 250) as u8 + 1]).unwrap(),
                visible,
                locked: false,
                display_name: String::new(),
            })
        }

        fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
            Rect::new(Vec2::new(x, y).unwrap(), Size2::new(w, h).unwrap())
        }

        #[test]
        fn empty_fragment_has_no_bounds() {
            // 검증 기준(VI.4.3): 빈 fragment.
            let mut columns = NodeColumns::new();
            let frame = insert(&mut columns, 1, NodeKind::Frame, None, true);
            let fragment = insert(&mut columns, 2, NodeKind::Fragment, Some(frame), true);
            assert_eq!(fragment_visual_bounds(&columns, fragment, &|_| None), None);
        }

        #[test]
        fn hidden_children_are_excluded() {
            // 검증 기준(VI.4.3): 숨김 자식.
            let mut columns = NodeColumns::new();
            let frame = insert(&mut columns, 1, NodeKind::Frame, None, true);
            let fragment = insert(&mut columns, 2, NodeKind::Fragment, Some(frame), true);
            let a = insert(&mut columns, 3, NodeKind::Shape, Some(fragment), true);
            let hidden = insert(&mut columns, 4, NodeKind::Shape, Some(fragment), false);
            let map: HashMap<RuntimeEntityId, Rect> = [
                (a, rect(10.0, 10.0, 20.0, 20.0)),
                (hidden, rect(500.0, 500.0, 10.0, 10.0)),
            ]
            .into_iter()
            .collect();
            let lookup = |id: RuntimeEntityId| map.get(&id).copied();
            assert_eq!(
                fragment_visual_bounds(&columns, fragment, &lookup),
                Some(rect(10.0, 10.0, 20.0, 20.0))
            );
        }

        #[test]
        fn nested_fragments_union_recursively() {
            // 검증 기준(VI.4.3): nested fragment.
            let mut columns = NodeColumns::new();
            let frame = insert(&mut columns, 1, NodeKind::Frame, None, true);
            let outer = insert(&mut columns, 2, NodeKind::Fragment, Some(frame), true);
            let a = insert(&mut columns, 3, NodeKind::Text, Some(outer), true);
            let inner = insert(&mut columns, 4, NodeKind::Fragment, Some(outer), true);
            let b = insert(&mut columns, 5, NodeKind::Image, Some(inner), true);
            let empty_inner = insert(&mut columns, 6, NodeKind::Fragment, Some(outer), true);
            let _ = empty_inner;
            let map: HashMap<RuntimeEntityId, Rect> = [
                (a, rect(0.0, 0.0, 10.0, 10.0)),
                (b, rect(50.0, -20.0, 30.0, 40.0)),
            ]
            .into_iter()
            .collect();
            let lookup = |id: RuntimeEntityId| map.get(&id).copied();
            // x: 0..80, y: -20..20.
            assert_eq!(
                fragment_visual_bounds(&columns, outer, &lookup),
                Some(rect(0.0, -20.0, 80.0, 40.0))
            );
            // 숨김 inner fragment면 그 자손도 제외.
            columns.set_visible(inner, false);
            assert_eq!(
                fragment_visual_bounds(&columns, outer, &lookup),
                Some(rect(0.0, 0.0, 10.0, 10.0))
            );
        }
    }
}

//! node 종류 capability validator (VI.3.2).
//!
//! **Frame만** 자식의 layout containing block과 좌표계를 만들 수 있습니다.
//! Fragment는 그룹 marker일 뿐 좌표계를 만들지 않으므로(VI.4.1) containing
//! block으로 사용하면 오류입니다. Shape·Path·Text·Image·ComponentInstance도
//! 마찬가지입니다.

use std::fmt;

use stellar_core_types::ids::RuntimeEntityId;

use crate::node_columns::NodeColumns;
use crate::node_kind::NodeKind;

/// capability 위반.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityError {
    /// containing block이 될 수 없는 종류입니다.
    NotAContainingBlock {
        /// 해당 종류.
        kind: NodeKind,
    },
    /// 핸들이 유효하지 않습니다.
    StaleHandle,
}

impl fmt::Display for CapabilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CapabilityError::NotAContainingBlock { kind } => {
                write!(
                    f,
                    "{}는 layout containing block이 될 수 없습니다(Frame 전용)",
                    kind.name()
                )
            }
            CapabilityError::StaleHandle => f.write_str("유효하지 않은 핸들"),
        }
    }
}

impl std::error::Error for CapabilityError {}

/// 이 종류가 containing block(좌표계 생성)이 될 수 있는지.
pub fn can_be_containing_block(kind: NodeKind) -> bool {
    kind == NodeKind::Frame
}

/// containing block으로 사용하려는 종류를 검증합니다. Frame 외에는 오류입니다.
pub fn validate_containing_block_kind(kind: NodeKind) -> Result<(), CapabilityError> {
    if can_be_containing_block(kind) {
        Ok(())
    } else {
        Err(CapabilityError::NotAContainingBlock { kind })
    }
}

/// node의 layout containing block을 해석합니다: **가장 가까운 Frame 조상**.
///
/// Fragment 등 좌표계를 만들지 않는 조상은 건너뜁니다. Frame 조상이 없으면
/// `Ok(None)`(root가 Frame이 아닌 비정상 tree — 검증은 VI.10.1).
pub fn resolve_containing_block(
    columns: &NodeColumns,
    node: RuntimeEntityId,
) -> Result<Option<RuntimeEntityId>, CapabilityError> {
    let fields = columns.get(node).ok_or(CapabilityError::StaleHandle)?;
    let mut current = fields.parent;
    while let Some(ancestor) = current {
        let ancestor_fields = columns.get(ancestor).ok_or(CapabilityError::StaleHandle)?;
        if can_be_containing_block(ancestor_fields.kind) {
            return Ok(Some(ancestor));
        }
        current = ancestor_fields.parent;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_columns::NodeFields;
    use stellar_core_types::ids::StableId;
    use stellar_core_types::order_key::OrderKey;

    fn fields(seed: u64, kind: NodeKind, parent: Option<RuntimeEntityId>) -> NodeFields {
        NodeFields {
            stable_id: StableId::new_v7(seed, 0, seed),
            kind,
            parent,
            order_key: OrderKey::between(None, None),
            visible: true,
            locked: false,
            display_name: String::new(),
        }
    }

    #[test]
    fn only_frame_can_be_containing_block() {
        // 검증 기준(VI.3.2): Fragment가 containing block으로 사용되면 오류.
        assert!(validate_containing_block_kind(NodeKind::Frame).is_ok());
        for kind in [
            NodeKind::Fragment,
            NodeKind::Shape,
            NodeKind::Path,
            NodeKind::Text,
            NodeKind::Image,
            NodeKind::ComponentInstance,
        ] {
            assert_eq!(
                validate_containing_block_kind(kind).unwrap_err(),
                CapabilityError::NotAContainingBlock { kind },
                "{}",
                kind.name()
            );
        }
    }

    #[test]
    fn resolve_skips_fragment_to_nearest_frame() {
        // frame_root ← fragment ← shape: shape의 containing block은 fragment가
        // 아니라 frame_root여야 합니다.
        let mut columns = NodeColumns::new();
        let frame_root = columns.insert(fields(1, NodeKind::Frame, None));
        let fragment = columns.insert(fields(2, NodeKind::Fragment, Some(frame_root)));
        let shape = columns.insert(fields(3, NodeKind::Shape, Some(fragment)));

        assert_eq!(
            resolve_containing_block(&columns, shape).unwrap(),
            Some(frame_root)
        );
        // fragment 자신의 containing block도 frame_root.
        assert_eq!(
            resolve_containing_block(&columns, fragment).unwrap(),
            Some(frame_root)
        );
    }

    #[test]
    fn nested_frames_use_nearest_frame() {
        let mut columns = NodeColumns::new();
        let outer = columns.insert(fields(1, NodeKind::Frame, None));
        let inner = columns.insert(fields(2, NodeKind::Frame, Some(outer)));
        let text = columns.insert(fields(3, NodeKind::Text, Some(inner)));
        assert_eq!(
            resolve_containing_block(&columns, text).unwrap(),
            Some(inner)
        );
        assert_eq!(
            resolve_containing_block(&columns, inner).unwrap(),
            Some(outer)
        );
        // root frame 위에는 없음.
        assert_eq!(resolve_containing_block(&columns, outer).unwrap(), None);
    }

    #[test]
    fn stale_handle_is_rejected() {
        let mut columns = NodeColumns::new();
        let node = columns.insert(fields(1, NodeKind::Frame, None));
        columns.remove(node);
        assert_eq!(
            resolve_containing_block(&columns, node).unwrap_err(),
            CapabilityError::StaleHandle
        );
    }
}

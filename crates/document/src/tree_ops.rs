//! parent-child 삽입·이동 validator (VI.2.3)와 정렬 자식 조회 (VI.2.4).
//!
//! reparent가 tree 불변식을 깨지 않는지 적용 전에 검증합니다:
//!
//! - 자기 자신을 부모로 지정 금지.
//! - 자기 자손을 부모로 지정 금지(cycle 차단).
//! - 다른 View의 tree로 무단 이동 금지 — 같은 root 아래에서만 이동할 수
//!   있습니다(View 간 이동은 별도 명시 command의 몫).
//! - View root(부모 없는 node) 자체의 reparent 금지(VI.1.2의 단일 root 보장).

use std::fmt;

use stellar_core_types::ids::RuntimeEntityId;

use crate::node_columns::NodeColumns;

/// reparent 거부 사유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReparentError {
    /// 핸들이 유효하지 않습니다.
    StaleHandle {
        /// 문제 핸들.
        id: RuntimeEntityId,
    },
    /// 자기 자신을 부모로 지정했습니다.
    SelfParent,
    /// 자기 자손을 부모로 지정했습니다(cycle).
    DescendantParent,
    /// 다른 View의 tree로 이동하려 합니다.
    CrossViewMove,
    /// View root는 reparent할 수 없습니다.
    ViewRootMove,
}

impl fmt::Display for ReparentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReparentError::StaleHandle { id } => write!(f, "유효하지 않은 핸들: {id:?}"),
            ReparentError::SelfParent => f.write_str("자기 자신을 부모로 지정할 수 없습니다"),
            ReparentError::DescendantParent => {
                f.write_str("자기 자손을 부모로 지정할 수 없습니다(cycle)")
            }
            ReparentError::CrossViewMove => f.write_str("다른 View의 tree로는 이동할 수 없습니다"),
            ReparentError::ViewRootMove => f.write_str("View root는 이동할 수 없습니다"),
        }
    }
}

impl std::error::Error for ReparentError {}

/// parent chain을 따라 tree root를 찾습니다.
fn tree_root(columns: &NodeColumns, id: RuntimeEntityId) -> Option<RuntimeEntityId> {
    let mut current = id;
    loop {
        let fields = columns.get(current)?;
        match fields.parent {
            Some(parent) => current = parent,
            None => return Some(current),
        }
    }
}

/// `ancestor`가 `id`의 조상(또는 자기 자신)인지 확인합니다.
fn is_ancestor_or_self(
    columns: &NodeColumns,
    ancestor: RuntimeEntityId,
    id: RuntimeEntityId,
) -> bool {
    let mut current = Some(id);
    while let Some(node) = current {
        if node == ancestor {
            return true;
        }
        current = columns.get(node).and_then(|f| f.parent);
    }
    false
}

/// reparent를 검증합니다. 통과해야만 적용할 수 있습니다.
pub fn validate_reparent(
    columns: &NodeColumns,
    node: RuntimeEntityId,
    new_parent: RuntimeEntityId,
) -> Result<(), ReparentError> {
    let node_fields = columns
        .get(node)
        .ok_or(ReparentError::StaleHandle { id: node })?;
    columns
        .get(new_parent)
        .ok_or(ReparentError::StaleHandle { id: new_parent })?;

    if node_fields.parent.is_none() {
        return Err(ReparentError::ViewRootMove);
    }
    if new_parent == node {
        return Err(ReparentError::SelfParent);
    }
    // 새 부모가 자기 자손이면 cycle.
    if is_ancestor_or_self(columns, node, new_parent) {
        return Err(ReparentError::DescendantParent);
    }
    // 다른 View(다른 root의 tree)로의 무단 이동 금지.
    let node_root = tree_root(columns, node).ok_or(ReparentError::StaleHandle { id: node })?;
    let parent_root =
        tree_root(columns, new_parent).ok_or(ReparentError::StaleHandle { id: new_parent })?;
    if node_root != parent_root {
        return Err(ReparentError::CrossViewMove);
    }
    Ok(())
}

/// 부모의 자식들을 **OrderKey 순서**로 반환합니다(VI.2.4).
///
/// arena 삽입 순서(slot 순서)에 의존하지 않습니다 — 같은 tree를 어떤 순서로
/// 삽입해도 결과가 같습니다. OrderKey가 같으면 StableId로 tie-break해
/// 결정론을 보장합니다(OrderKey 유일성 검증은 VI.10.1).
pub fn children_in_order(columns: &NodeColumns, parent: RuntimeEntityId) -> Vec<RuntimeEntityId> {
    let mut children: Vec<_> = columns
        .iter_live()
        .filter_map(|id| {
            let fields = columns.get(id)?;
            (fields.parent == Some(parent)).then_some((fields.order_key, fields.stable_id, id))
        })
        .collect();
    children.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    children.into_iter().map(|(_, _, id)| id).collect()
}

/// 검증 후 reparent를 적용합니다.
pub fn reparent(
    columns: &mut NodeColumns,
    node: RuntimeEntityId,
    new_parent: RuntimeEntityId,
) -> Result<(), ReparentError> {
    validate_reparent(columns, node, new_parent)?;
    let applied = columns.set_parent(node, Some(new_parent));
    debug_assert!(applied, "검증 통과 후 적용은 성공해야 함");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_columns::NodeFields;
    use crate::node_kind::NodeKind;
    use stellar_core_types::ids::StableId;
    use stellar_core_types::order_key::OrderKey;

    fn fields(seed: u64, parent: Option<RuntimeEntityId>) -> NodeFields {
        NodeFields {
            stable_id: StableId::new_v7(seed, 0, seed),
            kind: NodeKind::Frame,
            parent,
            order_key: OrderKey::between(None, None),
            visible: true,
            locked: false,
            display_name: String::new(),
        }
    }

    /// root ← a ← b ← c 체인과 root ← d 형제 구조.
    struct Tree {
        columns: NodeColumns,
        root: RuntimeEntityId,
        a: RuntimeEntityId,
        b: RuntimeEntityId,
        c: RuntimeEntityId,
        d: RuntimeEntityId,
    }

    fn tree() -> Tree {
        let mut columns = NodeColumns::new();
        let root = columns.insert(fields(1, None));
        let a = columns.insert(fields(2, Some(root)));
        let b = columns.insert(fields(3, Some(a)));
        let c = columns.insert(fields(4, Some(b)));
        let d = columns.insert(fields(5, Some(root)));
        Tree {
            columns,
            root,
            a,
            b,
            c,
            d,
        }
    }

    #[test]
    fn self_parent_is_rejected() {
        let t = tree();
        assert_eq!(
            validate_reparent(&t.columns, t.a, t.a).unwrap_err(),
            ReparentError::SelfParent
        );
    }

    #[test]
    fn descendant_parent_is_rejected() {
        // 검증 기준(VI.2.3): cycle fixture 실패.
        let t = tree();
        // a의 자손 b·c를 a의 부모로 지정 → cycle.
        assert_eq!(
            validate_reparent(&t.columns, t.a, t.b).unwrap_err(),
            ReparentError::DescendantParent
        );
        assert_eq!(
            validate_reparent(&t.columns, t.a, t.c).unwrap_err(),
            ReparentError::DescendantParent
        );
    }

    #[test]
    fn cross_view_move_is_rejected() {
        // 검증 기준(VI.2.3): cross-view fixture 실패.
        let mut t = tree();
        // 별도 View의 root와 자식.
        let other_root = t.columns.insert(fields(100, None));
        let other_child = t.columns.insert(fields(101, Some(other_root)));

        assert_eq!(
            validate_reparent(&t.columns, t.c, other_child).unwrap_err(),
            ReparentError::CrossViewMove
        );
        assert_eq!(
            validate_reparent(&t.columns, t.c, other_root).unwrap_err(),
            ReparentError::CrossViewMove
        );
    }

    #[test]
    fn view_root_cannot_be_reparented() {
        let t = tree();
        assert_eq!(
            validate_reparent(&t.columns, t.root, t.a).unwrap_err(),
            ReparentError::ViewRootMove
        );
    }

    #[test]
    fn valid_reparent_within_view_applies() {
        let mut t = tree();
        // c를 b 아래에서 d 아래로 이동(같은 View).
        reparent(&mut t.columns, t.c, t.d).unwrap();
        assert_eq!(t.columns.get(t.c).unwrap().parent, Some(t.d));
        // 이동 후에도 다시 유효한 이동 가능.
        reparent(&mut t.columns, t.c, t.a).unwrap();
        assert_eq!(t.columns.get(t.c).unwrap().parent, Some(t.a));
    }

    #[test]
    fn child_order_is_independent_of_insertion_order() {
        // 검증 기준(VI.2.4): 삽입 순서가 다른 동일 tree fixture의 child order 동일.
        use crate::node_columns::NodeColumns;

        // 자식 3개의 (seed, order key bytes) — 의미상 순서는 k1 < k2 < k3.
        let spec: [(u64, u8); 3] = [(10, 0x20), (11, 0x40), (12, 0x60)];

        // 서로 다른 삽입 순서 6가지 전부.
        let permutations: [[usize; 3]; 6] = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];
        let mut reference: Option<Vec<StableId>> = None;
        for perm in permutations {
            let mut columns = NodeColumns::new();
            let root = columns.insert(fields(1, None));
            for index in perm {
                let (seed, key_byte) = spec[index];
                let mut f = fields(seed, Some(root));
                f.order_key = OrderKey::from_bytes(vec![key_byte]).unwrap();
                columns.insert(f);
            }
            let order: Vec<StableId> = children_in_order(&columns, root)
                .into_iter()
                .map(|id| columns.get(id).unwrap().stable_id)
                .collect();
            match &reference {
                None => reference = Some(order),
                Some(expected) => assert_eq!(&order, expected, "삽입 순서 {perm:?}"),
            }
        }
        // 의미 순서(k1<k2<k3)와 일치.
        assert_eq!(
            reference.unwrap(),
            vec![
                StableId::new_v7(10, 0, 10),
                StableId::new_v7(11, 0, 11),
                StableId::new_v7(12, 0, 12),
            ]
        );
    }

    #[test]
    fn children_query_excludes_other_parents_and_removed() {
        let mut t = tree();
        // root의 자식은 a, d.
        let children = children_in_order(&t.columns, t.root);
        assert_eq!(children.len(), 2);
        assert!(children.contains(&t.a) && children.contains(&t.d));
        // d 제거 후엔 a만.
        t.columns.remove(t.d);
        assert_eq!(children_in_order(&t.columns, t.root), vec![t.a]);
        // 자식 없는 leaf.
        assert!(children_in_order(&t.columns, t.c).is_empty());
    }

    #[test]
    fn stale_handles_are_rejected() {
        let mut t = tree();
        let removed = t.d;
        t.columns.remove(removed);
        assert!(matches!(
            validate_reparent(&t.columns, t.c, removed).unwrap_err(),
            ReparentError::StaleHandle { .. }
        ));
        assert!(matches!(
            validate_reparent(&t.columns, removed, t.a).unwrap_err(),
            ReparentError::StaleHandle { .. }
        ));
    }
}

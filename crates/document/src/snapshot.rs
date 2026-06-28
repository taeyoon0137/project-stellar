//! 문서 소유자와 읽기 전용 스냅샷 view (III.6.3).
//!
//! [`Document`]가 노드 저장소([`NodeColumns`])를 **소유**하고, 외부에는 읽기 전용
//! [`DocumentSnapshotView`]만 제공합니다. 가변 arena 참조(`&mut NodeColumns`)나 가변 arena
//! 접근자를 공개 API로 노출하지 않으므로, 외부 crate는 스냅샷을 통해 **읽기만** 할 수 있습니다.
//! 변경은 `&mut Document` 메서드를 통해서만 가능합니다(borrow checker가 강제).

use stellar_core_types::ids::RuntimeEntityId;

use crate::node_columns::{NodeColumns, NodeFields};

/// 노드 저장소를 소유하는 문서. 외부에는 읽기 전용 스냅샷만 제공합니다.
#[derive(Debug, Default)]
pub struct Document {
    columns: NodeColumns,
}

impl Document {
    /// 빈 문서를 만듭니다.
    pub fn new() -> Document {
        Document {
            columns: NodeColumns::new(),
        }
    }

    /// 노드를 추가하고 핸들을 반환합니다(`&mut self` 필요).
    pub fn insert_node(&mut self, fields: NodeFields) -> RuntimeEntityId {
        self.columns.insert(fields)
    }

    /// 노드를 제거합니다(`&mut self` 필요). stale 핸들이면 `false`.
    pub fn remove_node(&mut self, id: RuntimeEntityId) -> bool {
        self.columns.remove(id)
    }

    /// 부모를 갱신합니다(`&mut self` 필요). stale 핸들이면 `false`.
    pub fn set_parent(&mut self, id: RuntimeEntityId, parent: Option<RuntimeEntityId>) -> bool {
        self.columns.set_parent(id, parent)
    }

    /// 읽기 전용 스냅샷 view를 반환합니다(`&self`).
    ///
    /// 읽기 전용 소비자(렌더러 등 외부 crate)는 이 view로만 접근합니다. 가변 arena 참조는
    /// 노출하지 않습니다.
    pub fn snapshot(&self) -> DocumentSnapshotView<'_> {
        DocumentSnapshotView {
            columns: &self.columns,
        }
    }
}

/// 문서의 읽기 전용 스냅샷 view (III.6.3).
///
/// 공유 참조(`&NodeColumns`)만 보유하며 어떤 변경 API도 제공하지 않습니다. 따라서 외부
/// crate는 이 view로 노드를 변경할 수 없습니다.
///
/// # 외부 직접 변경은 컴파일 실패
///
/// 읽기 전용 view에는 변경 메서드가 없습니다.
///
/// ```compile_fail
/// use stellar_document::snapshot::Document;
/// let doc = Document::new();
/// let view = doc.snapshot();
/// // view에는 mutation API가 없어 컴파일 실패.
/// view.remove_node(todo!());
/// ```
///
/// 공유 참조(`&Document`)만 가진 소비자는 변경 메서드를 호출할 수 없습니다.
///
/// ```compile_fail
/// use stellar_document::snapshot::Document;
/// use stellar_core_types::ids::RuntimeEntityId;
/// fn read_only_consumer(doc: &Document, id: RuntimeEntityId) {
///     // remove_node는 &mut self를 요구하므로 &Document로는 컴파일 실패.
///     doc.remove_node(id);
/// }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct DocumentSnapshotView<'a> {
    columns: &'a NodeColumns,
}

impl DocumentSnapshotView<'_> {
    /// 핸들의 공통 필드 스냅샷을 읽습니다. stale 핸들이면 `None`.
    pub fn get(&self, id: RuntimeEntityId) -> Option<NodeFields> {
        self.columns.get(id)
    }

    /// 핸들이 살아 있는지 여부.
    pub fn contains(&self, id: RuntimeEntityId) -> bool {
        self.columns.contains(id)
    }

    /// 살아 있는 노드 수.
    pub fn len(&self) -> usize {
        self.columns.len()
    }

    /// 비어 있는지 여부.
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::ids::StableId;
    use stellar_core_types::order_key::OrderKey;

    fn fields(seed: u64) -> NodeFields {
        NodeFields {
            stable_id: StableId::new_v7(seed, 0, seed),
            parent: None,
            order_key: OrderKey::between(None, None),
        }
    }

    #[test]
    fn snapshot_reads_document_state() {
        let mut doc = Document::new();
        let a = doc.insert_node(fields(1));
        let b = doc.insert_node(fields(2));

        let view = doc.snapshot();
        assert_eq!(view.len(), 2);
        assert!(!view.is_empty());
        assert!(view.contains(a));
        assert_eq!(view.get(b).unwrap().stable_id, StableId::new_v7(2, 0, 2));
    }

    #[test]
    fn snapshot_reflects_only_state_at_borrow() {
        let mut doc = Document::new();
        let a = doc.insert_node(fields(1));
        {
            let view = doc.snapshot();
            assert_eq!(view.len(), 1);
            assert!(view.contains(a));
        }
        // view 반납 후 변경 가능(&mut self).
        assert!(doc.remove_node(a));
        let view = doc.snapshot();
        assert!(view.is_empty());
        assert!(!view.contains(a));
    }

    #[test]
    fn snapshot_view_is_copy_and_cheap() {
        let mut doc = Document::new();
        doc.insert_node(fields(1));
        let view = doc.snapshot();
        let copied = view; // Copy.
        assert_eq!(view.len(), copied.len());
    }
}

//! command 검증용 문서 조회 helper (VII.2).
//!
//! [`DocumentModel`]의 공개 조회 API만으로 부모·깊이·자식 index·사용 중 id를 계산합니다.
//! 자식 조회를 반복하면 node 수에 비례한 scan이 반복되므로, subtree 순회는 한 번 만든
//! [`ChildIndex`]를 씁니다.

use std::collections::BTreeMap;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
#[cfg(test)]
use stellar_document::frame::Clipping;
use stellar_document::model::DocumentModel;
use stellar_document::node_kind::NodeKind;
use stellar_document::records::NodePayload;
#[cfg(test)]
use stellar_document::reference_index::ReferenceIndex;

/// node의 부모. 바깥 `None`은 node 없음, 안쪽 `None`은 View root.
pub(crate) fn parent_of(model: &DocumentModel, id: StableId) -> Option<Option<StableId>> {
    let handle = model.runtime_id(id)?;
    let fields = model.columns().get(handle)?;
    Some(match fields.parent {
        None => None,
        Some(parent) => Some(model.stable_id(parent)?),
    })
}

/// node의 OrderKey.
pub(crate) fn order_key_of(model: &DocumentModel, id: StableId) -> Option<OrderKey> {
    let handle = model.runtime_id(id)?;
    model.columns().get(handle).map(|fields| fields.order_key)
}

/// View root depth 0 기준 깊이.
pub(crate) fn depth_of(model: &DocumentModel, id: StableId) -> Option<u32> {
    let mut depth = 0u32;
    let mut current = id;
    loop {
        match parent_of(model, current)? {
            None => return Some(depth),
            Some(parent) => {
                depth = depth.checked_add(1)?;
                current = parent;
            }
        }
    }
}

/// node 종류.
pub(crate) fn kind_of(model: &DocumentModel, id: StableId) -> Option<NodeKind> {
    model.payload(id).map(NodePayload::kind)
}

/// 모든 live node id(StableId 순).
pub(crate) fn node_ids(model: &DocumentModel) -> Vec<StableId> {
    let mut ids: Vec<StableId> = model
        .columns()
        .iter_live()
        .filter_map(|handle| model.stable_id(handle))
        .collect();
    ids.sort_unstable();
    ids
}

/// 부모 → 자식(OrderKey, StableId 순) index. 한 번 만들고 여러 번 조회합니다.
#[derive(Debug, Default)]
pub(crate) struct ChildIndex {
    children: BTreeMap<StableId, Vec<StableId>>,
}

impl ChildIndex {
    /// 모델 전체를 한 번 훑어 index를 만듭니다.
    pub(crate) fn build(model: &DocumentModel) -> ChildIndex {
        let mut grouped: BTreeMap<StableId, Vec<(OrderKey, StableId)>> = BTreeMap::new();
        for handle in model.columns().iter_live() {
            let Some(fields) = model.columns().get(handle) else {
                continue;
            };
            let Some(parent) = fields.parent.and_then(|p| model.stable_id(p)) else {
                continue;
            };
            grouped
                .entry(parent)
                .or_default()
                .push((fields.order_key, fields.stable_id));
        }
        let children = grouped
            .into_iter()
            .map(|(parent, mut kids)| {
                kids.sort();
                (parent, kids.into_iter().map(|(_, id)| id).collect())
            })
            .collect();
        ChildIndex { children }
    }

    /// 자식(순서대로).
    pub(crate) fn children(&self, parent: StableId) -> &[StableId] {
        self.children.get(&parent).map_or(&[], Vec::as_slice)
    }

    /// `root`부터 pre-order(형제는 OrderKey 순) subtree.
    pub(crate) fn subtree(&self, root: StableId) -> Vec<StableId> {
        let mut out = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            out.push(id);
            stack.extend(self.children(id).iter().rev().copied());
        }
        out
    }
}

/// 문서의 node 간 참조 index(VI.2.5): Frame clip 대상과 modifier 참조(VI.9.4, 비활성 포함).
/// 간선은 참조를 가진 node → 대상 node입니다. NIL 참조는 제외합니다.
#[cfg(test)]
pub(crate) fn reference_index(model: &DocumentModel) -> ReferenceIndex {
    let mut index = ReferenceIndex::new();
    for id in node_ids(model) {
        if let Some(NodePayload::Frame(frame)) = model.payload(id) {
            if let Clipping::Path(target) = frame.clipping {
                if target.id() != StableId::NIL {
                    index.add_reference(id, target.id());
                }
            }
        }
        if let Some(stack) = model.modifier_stack(id) {
            for entry in stack.entries() {
                for target in entry.payload.referenced_nodes() {
                    index.add_reference(id, target);
                }
            }
        }
    }
    index
}

/// path payload의 point·handle id.
pub(crate) fn path_element_ids(payload: &NodePayload) -> Vec<StableId> {
    let NodePayload::Path(path) = payload else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for ids in path.ids() {
        if let Some(point) = ids.point {
            out.push(point.0);
        }
        if let Some((h1, h2)) = ids.handles {
            out.extend([h1.0, h2.0]);
        }
    }
    out
}

/// 문서 안 영속 id의 종류(참조 대상 검사용).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdOwner {
    /// 문서·페이지·View 정의.
    Definition,
    /// node.
    Node(NodeKind),
    /// modifier.
    Modifier,
    /// path point·handle.
    PathElement,
}

/// 문서 안 모든 영속 id와 그 종류(문서 validator VI.10.1의 id 공간과 같음).
pub(crate) fn id_owners(model: &DocumentModel) -> BTreeMap<StableId, IdOwner> {
    let mut out = BTreeMap::new();
    let definition = model.definition();
    out.insert(definition.id, IdOwner::Definition);
    for page in &definition.pages {
        out.insert(page.id, IdOwner::Definition);
        for view in &page.views {
            out.insert(view.id, IdOwner::Definition);
        }
    }
    for id in node_ids(model) {
        let Some(payload) = model.payload(id) else {
            continue;
        };
        out.insert(id, IdOwner::Node(payload.kind()));
        for element in path_element_ids(payload) {
            out.insert(element, IdOwner::PathElement);
        }
        if let Some(stack) = model.modifier_stack(id) {
            for entry in stack.entries() {
                out.insert(entry.id.stable_id(), IdOwner::Modifier);
            }
        }
    }
    out
}

//! command 검증용 문서 조회 helper (VII.2, XVI.4.7).
//!
//! [`DocumentModel`]의 공개 조회 API만으로 부모·깊이·자식·사용 중 id를 조회합니다. 자식·inbound 참조·영속 id
//! 공간은 [`DocumentModel`]이 편집과 함께 증분 유지하는 index(부모별 형제 집합, [`DocumentModel::references`],
//! 영속 id 소유 수)를 그대로 읽으므로, command 검증이 호출마다 문서 전체를 훑지 않습니다(ADR-0108 결정 1).
//! 문서 전체에서 같은 결과를 다시 만드는 함수(`ChildIndex::rebuild`, `reference_index`, `id_owners`)는 증분
//! index와 비교하는 test oracle로만 남깁니다.

#[cfg(test)]
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

/// 모든 live node id(StableId 순). 문서 전체를 훑으므로(O(N)) command 검증 경로에서는 쓰지 않습니다.
pub(crate) fn node_ids(model: &DocumentModel) -> Vec<StableId> {
    let mut ids: Vec<StableId> = model
        .columns()
        .iter_live()
        .filter_map(|handle| model.stable_id(handle))
        .collect();
    ids.sort_unstable();
    ids
}

/// 부모 → 자식(OrderKey, StableId 순) 조회(XVI.4.7).
///
/// [`DocumentModel`]이 편집과 함께 증분 유지하는 부모별 형제 index(`children_in_order`)를 읽는 view입니다.
/// 만들 때 문서를 훑지 않으므로(O(1)) command 검증에서 호출마다 만들어도 비용이 문서 크기에 비례하지 않습니다.
/// 같은 표를 문서 전체에서 다시 만드는 `ChildIndex::rebuild`는 test oracle입니다.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ChildIndex<'a> {
    model: &'a DocumentModel,
}

impl<'a> ChildIndex<'a> {
    /// 모델의 증분 자식 index view.
    pub(crate) fn build(model: &'a DocumentModel) -> ChildIndex<'a> {
        ChildIndex { model }
    }

    /// 자식(순서대로).
    pub(crate) fn children(&self, parent: StableId) -> Vec<StableId> {
        self.model.children_in_order(parent)
    }

    /// `root`부터 pre-order(형제는 OrderKey 순) subtree.
    pub(crate) fn subtree(&self, root: StableId) -> Vec<StableId> {
        let mut out = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            out.push(id);
            stack.extend(self.children(id).into_iter().rev());
        }
        out
    }

    /// 모델 전체를 한 번 훑어 부모 → 자식 표를 다시 만듭니다(O(N), test oracle).
    #[cfg(test)]
    pub(crate) fn rebuild(model: &DocumentModel) -> BTreeMap<StableId, Vec<StableId>> {
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
        grouped
            .into_iter()
            .map(|(parent, mut kids)| {
                kids.sort();
                (parent, kids.into_iter().map(|(_, id)| id).collect())
            })
            .collect()
    }
}

/// 문서의 참조 index(VI.2.5)를 문서 전체에서 다시 만듭니다: Frame clip 대상, modifier 참조(VI.9.4, 비활성 포함),
/// 인스턴스가 참조하는 컴포넌트 정의. 간선은 참조를 가진 node → 대상입니다. NIL clip 참조는 제외합니다.
///
/// command 검증은 모델이 증분 유지하는 [`DocumentModel::references`]를 쓰고, 이 함수는 두 index가 같은지 확인하는
/// test oracle입니다.
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
        if let Some(NodePayload::ComponentInstance(instance)) = model.payload(id) {
            index.add_reference(id, instance.component().id());
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
    path.element_ids().collect()
}

/// 문서 영속 id 공간에서 id를 쓰는 쪽(참조 대상 종류 검사용, XVI.4.7).
///
/// command 검증은 "node이고 종류가 무엇인가"와 "node가 아닌 영속 id(문서·페이지·View·typography·modifier·path
/// element)인가"만 구분하므로, node가 아닌 쪽은 한 값으로 묶습니다. 그래서 모델이 증분 유지하는 영속 id 소유
/// 수만으로 O(log N)에 답할 수 있습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdUse {
    /// node.
    Node(NodeKind),
    /// node가 아닌 영속 id(문서·페이지·View·typography resource·modifier·path point·handle).
    Other,
}

/// `id`를 쓰는 쪽. 문서에 없는 id면 `None`입니다(문서 validator VI.10.1의 id 공간과 같음).
///
/// node id면 그 종류를 돌려줍니다. 같은 id를 node와 다른 영속 id가 함께 쓰는 문서(validator가 이미 거부하는
/// `DuplicateId`)에서는 node를 우선합니다.
pub(crate) fn id_use(model: &DocumentModel, id: StableId) -> Option<IdUse> {
    if let Some(payload) = model.payload(id) {
        return Some(IdUse::Node(payload.kind()));
    }
    model.contains_persistent_id(id).then_some(IdUse::Other)
}

/// 문서 안 영속 id의 종류(test oracle의 분류).
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdOwner {
    /// 문서·페이지·View 정의.
    Definition,
    /// 문서 typography resource.
    Typography,
    /// node.
    Node(NodeKind),
    /// modifier.
    Modifier,
    /// path point·handle.
    PathElement,
}

#[cfg(test)]
impl IdOwner {
    /// command 검증이 보는 구분.
    pub(crate) fn id_use(self) -> IdUse {
        match self {
            IdOwner::Node(kind) => IdUse::Node(kind),
            _ => IdUse::Other,
        }
    }
}

/// 문서 안 모든 영속 id와 그 종류를 문서 전체에서 다시 만듭니다(O(N), test oracle). 같은 id를 둘 이상이 쓰면
/// node를 우선합니다([`id_use`]와 같은 규칙).
#[cfg(test)]
pub(crate) fn id_owners(model: &DocumentModel) -> BTreeMap<StableId, IdOwner> {
    let mut out = BTreeMap::new();
    let definition = model.definition();
    out.insert(definition.id, IdOwner::Definition);
    for id in definition.text_styles.typography.keys() {
        out.insert(*id, IdOwner::Typography);
    }
    for page in &definition.pages {
        out.insert(page.id, IdOwner::Definition);
        for view in &page.views {
            out.insert(view.id, IdOwner::Definition);
        }
    }
    let ids = node_ids(model);
    for &id in &ids {
        let Some(payload) = model.payload(id) else {
            continue;
        };
        for element in path_element_ids(payload) {
            out.insert(element, IdOwner::PathElement);
        }
        if let Some(stack) = model.modifier_stack(id) {
            for entry in stack.entries() {
                out.insert(entry.id.stable_id(), IdOwner::Modifier);
            }
        }
    }
    for id in ids {
        if let Some(payload) = model.payload(id) {
            out.insert(id, IdOwner::Node(payload.kind()));
        }
    }
    out
}

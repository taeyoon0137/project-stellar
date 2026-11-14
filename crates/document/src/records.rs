//! 영속 문서 레코드 (VI.10).
//!
//! runtime 핸들([`RuntimeEntityId`](stellar_core_types::ids::RuntimeEntityId)) 없이
//! [`StableId`]만으로 표현한 문서의 영속 의미 모델입니다. 부모도 StableId로 가리킵니다.
//!
//! - 문서 validator(VI.10.1)의 입력입니다.
//! - chunk serializer(VI.10.2)와 loader(VI.10.3)가 이 형태로 주고받습니다.
//! - modifier stack은 node payload와 **분리**되어 별도 목록으로 저장됩니다(VI.9.1).
//!
//! node 종류는 payload variant에서 유도하므로 kind와 payload가 어긋난 레코드는 표현할 수
//! 없습니다. 컴포넌트 인스턴스는 정의와 희소 override만 보관합니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;

use crate::component::instance::ComponentInstancePayload;
use crate::definitions::{DocumentDefinition, ViewDefinition};
use crate::fragment::FragmentPayload;
use crate::frame::FramePayload;
use crate::image::ImagePayload;
use crate::modifier::ModifierStack;
use crate::node_kind::NodeKind;
use crate::path::IdentifiedPath;
use crate::shape::ShapeGeometry;
use crate::text::TextPayload;

/// node 종류별 payload.
#[derive(Debug, Clone, PartialEq)]
pub enum NodePayload {
    /// Frame(VI.3).
    Frame(FramePayload),
    /// Fragment(VI.4) — 필드 없음.
    Fragment(FragmentPayload),
    /// 파라메트릭 도형(VI.5).
    Shape(ShapeGeometry),
    /// point·handle id가 붙은 path(VI.6).
    Path(IdentifiedPath),
    /// 텍스트(VI.7).
    Text(TextPayload),
    /// 이미지(VI.8).
    Image(ImagePayload),
    /// 컴포넌트 정의에 대한 참조와 희소 override(IX.2.1).
    ComponentInstance(ComponentInstancePayload),
}

impl NodePayload {
    /// payload가 나타내는 node 종류.
    pub fn kind(&self) -> NodeKind {
        match self {
            NodePayload::Frame(_) => NodeKind::Frame,
            NodePayload::Fragment(_) => NodeKind::Fragment,
            NodePayload::Shape(_) => NodeKind::Shape,
            NodePayload::Path(_) => NodeKind::Path,
            NodePayload::Text(_) => NodeKind::Text,
            NodePayload::Image(_) => NodeKind::Image,
            NodePayload::ComponentInstance(_) => NodeKind::ComponentInstance,
        }
    }
}

/// node 하나의 영속 레코드: 공통 필드(VI.2.2) + payload.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeRecord {
    /// 안정 식별자.
    pub id: StableId,
    /// 부모 node StableId. `None`이면 View root.
    pub parent: Option<StableId>,
    /// 형제 순서 키.
    pub order_key: OrderKey,
    /// 표시 여부.
    pub visible: bool,
    /// 잠금 여부.
    pub locked: bool,
    /// 표시 이름(식별에 사용 금지).
    pub display_name: String,
    /// 종류별 payload.
    pub payload: NodePayload,
}

impl NodeRecord {
    /// node 종류(payload에서 유도).
    pub fn kind(&self) -> NodeKind {
        self.payload.kind()
    }
}

/// 한 문서의 영속 레코드 전체.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentRecords {
    /// 문서·페이지·View 정의(VI.1).
    pub definition: DocumentDefinition,
    /// 모든 View의 node 레코드.
    pub nodes: Vec<NodeRecord>,
    /// node별 modifier stack(빈 stack은 저장하지 않음).
    pub modifier_stacks: Vec<ModifierStack>,
}

impl DocumentRecords {
    /// node가 없는 레코드.
    pub fn new(definition: DocumentDefinition) -> DocumentRecords {
        DocumentRecords {
            definition,
            nodes: Vec::new(),
            modifier_stacks: Vec::new(),
        }
    }

    /// 정의 순서(페이지 → View)로 모든 View를 순회합니다.
    pub fn views(&self) -> impl Iterator<Item = &ViewDefinition> {
        self.definition.pages.iter().flat_map(|p| p.views.iter())
    }

    /// canonical 순서로 정렬합니다: node는 StableId 오름차순, stack은 소유 node
    /// StableId 오름차순(안정 정렬). 같은 의미의 레코드는 정렬 후 `==`로 비교할 수 있습니다.
    pub fn sort_canonical(&mut self) {
        self.nodes.sort_by_key(|n| n.id);
        self.modifier_stacks.sort_by_key(|s| s.node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::{PageDefinition, ViewDefinition};

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn record(id: u64, parent: Option<u64>, payload: NodePayload) -> NodeRecord {
        NodeRecord {
            id: sid(id),
            parent: parent.map(sid),
            order_key: OrderKey::between(None, None),
            visible: true,
            locked: false,
            display_name: String::new(),
            payload,
        }
    }

    #[test]
    fn kind_is_derived_from_payload() {
        let frame = record(1, None, NodePayload::Frame(FramePayload::default()));
        assert_eq!(frame.kind(), NodeKind::Frame);
        let fragment = record(2, Some(1), NodePayload::Fragment(FragmentPayload));
        assert_eq!(fragment.kind(), NodeKind::Fragment);
        assert_eq!(
            NodePayload::Text(TextPayload::empty(0)).kind(),
            NodeKind::Text
        );
    }

    #[test]
    fn canonical_sort_orders_nodes_and_stacks() {
        let mut definition = DocumentDefinition::new(sid(100), String::new());
        let mut page = PageDefinition::new(sid(101), String::new());
        page.add_view(ViewDefinition::new(sid(102), String::new(), sid(1)))
            .unwrap();
        definition.add_page(page).unwrap();
        let mut records = DocumentRecords::new(definition);
        records
            .nodes
            .push(record(3, Some(1), NodePayload::Fragment(FragmentPayload)));
        records
            .nodes
            .push(record(1, None, NodePayload::Frame(FramePayload::default())));
        records.modifier_stacks.push(ModifierStack::new(sid(3)));
        records.modifier_stacks.push(ModifierStack::new(sid(1)));
        records.sort_canonical();
        assert_eq!(records.nodes[0].id, sid(1));
        assert_eq!(records.modifier_stacks[0].node, sid(1));
        assert_eq!(records.views().count(), 1);
    }
}

//! ComponentDefinition resource (IX.1.1).
//!
//! 컴포넌트 정의는 scene node가 아니라 문서 resource(ComponentLibrary chunk, V.6.1)입니다
//! (VI.2.1). 정의는 자신이 **소유한** node tree([`DefinitionTree`])의 root Frame을 가리키고,
//! 공개 Property 목록, Slot 목록, local token scope를 갖습니다.
//!
//! - root는 이 컴포넌트 정의 tree 안의 **parent 없는 Frame**이어야 합니다. 다른 컴포넌트의
//!   node, View(scene) node처럼 컴포넌트 밖 node를 가리키면 거부합니다.
//! - 한 node는 정확히 하나의 컴포넌트 정의에만 속합니다([`ComponentLibrary`]가 강제).
//! - Property·Slot 목록은 interface 선언(StableId 목록)입니다. 상세 record는 각각
//!   ComponentProperty(IX.1.2)·ComponentSlot(IX.3.1)이 정의합니다.
//! - local token scope는 이 컴포넌트 안에서만 보이는 token 참조 집합입니다. token 정의 자체는
//!   token 계층(Phase VIII)이 소유하며 여기서는 참조(`TokenRef`)만 보관합니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{ComponentRef, NodeRef, TokenRef};

use super::codec::{
    capacity_hint, put_bool, put_bytes, put_count, put_id, put_kind, put_opt_id, put_order_key,
    put_str, Malformed, Reader,
};
use super::slot::{ComponentSlot, SlotError};
use super::slot_override::SlotOverrideError;
use crate::node_kind::NodeKind;

/// Component Property의 안정 식별자. node·slot 식별자와 타입이 섞이지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ComponentPropertyId(StableId);

impl ComponentPropertyId {
    /// `StableId`로부터 만듭니다.
    pub const fn new(id: StableId) -> ComponentPropertyId {
        ComponentPropertyId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }
}

/// Component Slot의 안정 식별자. node·property 식별자와 타입이 섞이지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ComponentSlotId(StableId);

impl ComponentSlotId {
    /// `StableId`로부터 만듭니다.
    pub const fn new(id: StableId) -> ComponentSlotId {
        ComponentSlotId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }
}

/// 컴포넌트 정의·정의 tree·library 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentDefinitionError {
    /// root가 이 컴포넌트 정의 tree 밖 node를 가리킵니다.
    RootOutsideComponent {
        /// 컴포넌트 id.
        component: StableId,
        /// root가 가리킨 node.
        root: StableId,
    },
    /// root node가 Frame이 아닙니다.
    RootNotFrame {
        /// root node.
        root: StableId,
        /// 실제 종류.
        kind: NodeKind,
    },
    /// root가 정의 tree의 최상위(parent 없는) node가 아닙니다.
    RootNotTreeTop {
        /// root node.
        root: StableId,
    },
    /// 정의 tree의 소유 컴포넌트가 정의와 다릅니다.
    TreeOwnerMismatch {
        /// 정의 id.
        definition: StableId,
        /// tree가 속한 컴포넌트 id.
        tree_owner: StableId,
    },
    /// 같은 id가 이미 있습니다(node·property·slot·component).
    DuplicateId {
        /// 중복 id.
        id: StableId,
    },
    /// 대상 id를 찾을 수 없습니다.
    NotFound {
        /// 요청 id.
        id: StableId,
    },
    /// 정의 tree node의 부모가 tree에 없습니다.
    ParentNotInTree {
        /// node.
        node: StableId,
        /// 지정된 부모.
        parent: StableId,
    },
    /// 정의 tree에 parent 없는 node가 둘 이상입니다.
    MultipleTreeRoots {
        /// 두 번째 root 후보.
        node: StableId,
    },
    /// 같은 부모 아래 OrderKey가 중복됩니다(순서 모호).
    DuplicateOrderKey {
        /// 부모 node.
        parent: StableId,
    },
    /// node가 이미 다른 컴포넌트 정의에 속합니다.
    NodeOwnedByOtherComponent {
        /// node.
        node: StableId,
        /// 기존 소유 컴포넌트.
        owner: StableId,
    },
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
    /// slot record가 잘못됐습니다(IX.3.1, library 디코딩 경로).
    Slot(SlotError),
    /// 정의 tree 안 nested 인스턴스의 slot override가 투영 규칙을 어깁니다(IX.3.2, library
    /// 디코딩 경로).
    SlotOverride(SlotOverrideError),
}

impl From<SlotError> for ComponentDefinitionError {
    fn from(err: SlotError) -> ComponentDefinitionError {
        ComponentDefinitionError::Slot(err)
    }
}

impl fmt::Display for ComponentDefinitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComponentDefinitionError::RootOutsideComponent { component, root } => write!(
                f,
                "컴포넌트 {component}의 root {root}가 정의 tree 밖 node를 가리킵니다"
            ),
            ComponentDefinitionError::RootNotFrame { root, kind } => {
                write!(f, "root {root}는 Frame이어야 합니다({})", kind.name())
            }
            ComponentDefinitionError::RootNotTreeTop { root } => {
                write!(f, "root {root}가 정의 tree의 최상위 node가 아닙니다")
            }
            ComponentDefinitionError::TreeOwnerMismatch {
                definition,
                tree_owner,
            } => write!(
                f,
                "정의 {definition}에 다른 컴포넌트({tree_owner})의 tree를 연결할 수 없습니다"
            ),
            ComponentDefinitionError::DuplicateId { id } => write!(f, "중복 StableId: {id}"),
            ComponentDefinitionError::NotFound { id } => write!(f, "찾을 수 없는 StableId: {id}"),
            ComponentDefinitionError::ParentNotInTree { node, parent } => {
                write!(f, "node {node}의 부모 {parent}가 정의 tree에 없습니다")
            }
            ComponentDefinitionError::MultipleTreeRoots { node } => {
                write!(f, "정의 tree root가 둘 이상입니다(추가 root {node})")
            }
            ComponentDefinitionError::DuplicateOrderKey { parent } => {
                write!(f, "부모 {parent} 아래 OrderKey가 중복됩니다")
            }
            ComponentDefinitionError::NodeOwnedByOtherComponent { node, owner } => {
                write!(f, "node {node}는 이미 컴포넌트 {owner}에 속합니다")
            }
            ComponentDefinitionError::Malformed { reason } => {
                write!(f, "컴포넌트 정의 binary 형식 위반: {reason}")
            }
            ComponentDefinitionError::Slot(err) => write!(f, "slot 오류: {err}"),
            ComponentDefinitionError::SlotOverride(err) => {
                write!(f, "nested 인스턴스 slot override 오류: {err}")
            }
        }
    }
}

impl std::error::Error for ComponentDefinitionError {}

impl From<Malformed> for ComponentDefinitionError {
    fn from(err: Malformed) -> ComponentDefinitionError {
        ComponentDefinitionError::Malformed { reason: err.0 }
    }
}

/// 컴포넌트 안에서만 보이는 token 참조 집합(local token scope).
///
/// 항상 `TokenRef` 오름차순·중복 없음으로 유지되어 canonical 직렬화가 입력 순서와
/// 무관합니다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocalTokenScope {
    tokens: BTreeSet<TokenRef>,
}

impl LocalTokenScope {
    /// 빈 scope.
    pub fn new() -> LocalTokenScope {
        LocalTokenScope::default()
    }

    /// token을 scope에 추가합니다. 이미 있으면 `false`.
    pub fn insert(&mut self, token: TokenRef) -> bool {
        self.tokens.insert(token)
    }

    /// token을 scope에서 제거합니다. 없으면 `false`.
    pub fn remove(&mut self, token: TokenRef) -> bool {
        self.tokens.remove(&token)
    }

    /// token이 scope에 있는지.
    pub fn contains(&self, token: TokenRef) -> bool {
        self.tokens.contains(&token)
    }

    /// 오름차순 token 목록.
    pub fn iter(&self) -> impl Iterator<Item = TokenRef> + '_ {
        self.tokens.iter().copied()
    }

    /// token 수.
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
}

/// 컴포넌트 정의 tree의 node 한 개.
///
/// 공통 필드는 scene node column(VI.2.2)과 같고, 부모는 런타임 핸들이 아니라 `StableId`로
/// 가리킵니다(정의 tree는 resource라 런타임 arena와 독립). `payload`는 종류별 canonical
/// payload bytes(Frame·Text·ComponentInstance 등)이며 이 계층은 해석하지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionNode {
    /// 안정 식별자.
    pub id: StableId,
    /// node 종류.
    pub kind: NodeKind,
    /// 부모 node(`None`이면 정의 tree root).
    pub parent: Option<StableId>,
    /// 형제 순서 키.
    pub order_key: OrderKey,
    /// 표시 여부.
    pub visible: bool,
    /// 잠금 여부.
    pub locked: bool,
    /// 표시 이름(식별에 사용 금지).
    pub display_name: String,
    /// 종류별 canonical payload bytes.
    pub payload: Vec<u8>,
}

/// 한 컴포넌트 정의가 소유한 node tree.
///
/// 삽입은 부모가 먼저 있어야 하므로(root 제외) cycle이 생길 수 없고, parent 없는 node는
/// 하나뿐입니다. 자식 목록은 OrderKey(동일 시 StableId) 순으로 유지합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionTree {
    component: ComponentRef,
    nodes: BTreeMap<StableId, DefinitionNode>,
    children: BTreeMap<StableId, Vec<StableId>>,
    root: Option<StableId>,
}

impl DefinitionTree {
    /// 컴포넌트의 빈 정의 tree.
    pub fn new(component: ComponentRef) -> DefinitionTree {
        DefinitionTree {
            component,
            nodes: BTreeMap::new(),
            children: BTreeMap::new(),
            root: None,
        }
    }

    /// 소유 컴포넌트.
    pub fn component(&self) -> ComponentRef {
        self.component
    }

    /// node를 추가합니다. 중복 id, 없는 부모, 두 번째 root, 형제 OrderKey 중복은 거부합니다.
    pub fn insert(&mut self, node: DefinitionNode) -> Result<(), ComponentDefinitionError> {
        if self.nodes.contains_key(&node.id) {
            return Err(ComponentDefinitionError::DuplicateId { id: node.id });
        }
        match node.parent {
            None => {
                if self.root.is_some() {
                    return Err(ComponentDefinitionError::MultipleTreeRoots { node: node.id });
                }
                self.root = Some(node.id);
            }
            Some(parent) => {
                if !self.nodes.contains_key(&parent) {
                    return Err(ComponentDefinitionError::ParentNotInTree {
                        node: node.id,
                        parent,
                    });
                }
                let nodes = &self.nodes;
                let siblings = self.children.get(&parent).map(Vec::as_slice).unwrap_or(&[]);
                if siblings
                    .iter()
                    .any(|s| nodes.get(s).is_some_and(|n| n.order_key == node.order_key))
                {
                    return Err(ComponentDefinitionError::DuplicateOrderKey { parent });
                }
                let at = siblings.partition_point(|s| {
                    nodes
                        .get(s)
                        .is_some_and(|n| (&n.order_key, n.id) < (&node.order_key, node.id))
                });
                self.children.entry(parent).or_default().insert(at, node.id);
            }
        }
        self.nodes.insert(node.id, node);
        Ok(())
    }

    /// id로 node를 찾습니다.
    pub fn node(&self, id: StableId) -> Option<&DefinitionNode> {
        self.nodes.get(&id)
    }

    /// node가 이 정의 tree에 속하는지.
    pub fn contains(&self, id: StableId) -> bool {
        self.nodes.contains_key(&id)
    }

    /// 자식 목록(OrderKey 순). 자식이 없거나 node가 없으면 빈 목록.
    pub fn children(&self, id: StableId) -> &[StableId] {
        self.children.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// parent 없는 최상위 node.
    pub fn tree_root(&self) -> Option<StableId> {
        self.root
    }

    /// node 수.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// StableId 오름차순 node 순회.
    pub fn iter(&self) -> impl Iterator<Item = &DefinitionNode> + '_ {
        self.nodes.values()
    }

    /// root부터 자식 순서대로 전위 순회한 node id 목록.
    pub fn preorder(&self) -> Vec<StableId> {
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut stack: Vec<StableId> = self.root.into_iter().collect();
        while let Some(id) = stack.pop() {
            out.push(id);
            for child in self.children(id).iter().rev() {
                stack.push(*child);
            }
        }
        out
    }

    fn encode_into(&self, buf: &mut Vec<u8>) {
        put_id(buf, self.component.id());
        put_count(buf, self.nodes.len());
        // 전위 순서: 디코딩 시 부모가 항상 먼저 나오고, 같은 tree는 항상 같은 bytes.
        for id in self.preorder() {
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            put_id(buf, node.id);
            put_kind(buf, node.kind);
            put_opt_id(buf, node.parent);
            put_order_key(buf, &node.order_key);
            put_bool(buf, node.visible);
            put_bool(buf, node.locked);
            put_str(buf, &node.display_name);
            put_bytes(buf, &node.payload);
        }
    }

    fn decode_from(reader: &mut Reader<'_>) -> Result<DefinitionTree, ComponentDefinitionError> {
        let component = ComponentRef::new(reader.id()?);
        let count = reader.count()?;
        let mut tree = DefinitionTree::new(component);
        for _ in 0..count {
            let node = DefinitionNode {
                id: reader.id()?,
                kind: reader.kind()?,
                parent: reader.opt_id()?,
                order_key: reader.order_key()?,
                visible: reader.bool()?,
                locked: reader.bool()?,
                display_name: reader.string()?,
                payload: reader.bytes()?.to_vec(),
            };
            tree.insert(node)?;
        }
        Ok(tree)
    }

    /// canonical binary bytes(전위 순서)로 인코딩합니다.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode_into(&mut buf);
        buf
    }

    /// binary bytes에서 디코딩합니다. 전위 순서가 아닌(비canonical) 입력도 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<DefinitionTree, ComponentDefinitionError> {
        let mut reader = Reader::new(bytes);
        let tree = DefinitionTree::decode_from(&mut reader)?;
        reader.expect_end()?;
        if tree.encode() != bytes {
            return Err(ComponentDefinitionError::Malformed {
                reason: "정의 tree node 순서가 canonical 전위 순서가 아님",
            });
        }
        Ok(tree)
    }
}

/// 컴포넌트 정의 resource.
///
/// 표시 이름은 식별에 사용하지 않습니다(VI.1.3과 같은 정책). Property·Slot 목록은 선언
/// 순서를 유지하며 중복을 허용하지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentDefinition {
    /// 고유 StableId.
    pub id: StableId,
    display_name: String,
    /// 정의 tree의 root Frame.
    pub root: NodeRef,
    properties: Vec<ComponentPropertyId>,
    slots: Vec<ComponentSlotId>,
    local_tokens: LocalTokenScope,
}

impl ComponentDefinition {
    /// 비어 있는 interface(Property·Slot·token 없음)의 정의를 만듭니다.
    pub fn new(id: StableId, display_name: String, root: NodeRef) -> ComponentDefinition {
        ComponentDefinition {
            id,
            display_name,
            root,
            properties: Vec::new(),
            slots: Vec::new(),
            local_tokens: LocalTokenScope::new(),
        }
    }

    /// 이 정의를 가리키는 참조.
    pub fn reference(&self) -> ComponentRef {
        ComponentRef::new(self.id)
    }

    /// 표시 이름(식별에 사용 금지).
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// 표시 이름을 바꿉니다. id·root·참조에는 영향이 없습니다.
    pub fn rename(&mut self, display_name: String) {
        self.display_name = display_name;
    }

    /// 공개 Property 목록(선언 순서).
    pub fn properties(&self) -> &[ComponentPropertyId] {
        &self.properties
    }

    /// Property를 목록 끝에 선언합니다. 중복은 거부합니다.
    pub fn add_property(
        &mut self,
        property: ComponentPropertyId,
    ) -> Result<(), ComponentDefinitionError> {
        if self.properties.contains(&property) {
            return Err(ComponentDefinitionError::DuplicateId {
                id: property.stable_id(),
            });
        }
        self.properties.push(property);
        Ok(())
    }

    /// Property 선언을 제거합니다. 없으면 오류입니다.
    pub fn remove_property(
        &mut self,
        property: ComponentPropertyId,
    ) -> Result<(), ComponentDefinitionError> {
        let index = self.properties.iter().position(|p| *p == property).ok_or(
            ComponentDefinitionError::NotFound {
                id: property.stable_id(),
            },
        )?;
        self.properties.remove(index);
        Ok(())
    }

    /// Slot 목록(선언 순서).
    pub fn slots(&self) -> &[ComponentSlotId] {
        &self.slots
    }

    /// Slot을 목록 끝에 선언합니다. 중복은 거부합니다.
    pub fn add_slot(&mut self, slot: ComponentSlotId) -> Result<(), ComponentDefinitionError> {
        if self.slots.contains(&slot) {
            return Err(ComponentDefinitionError::DuplicateId {
                id: slot.stable_id(),
            });
        }
        self.slots.push(slot);
        Ok(())
    }

    /// Slot 선언을 제거합니다. 없으면 오류입니다.
    pub fn remove_slot(&mut self, slot: ComponentSlotId) -> Result<(), ComponentDefinitionError> {
        let index = self.slots.iter().position(|s| *s == slot).ok_or(
            ComponentDefinitionError::NotFound {
                id: slot.stable_id(),
            },
        )?;
        self.slots.remove(index);
        Ok(())
    }

    /// local token scope.
    pub fn local_tokens(&self) -> &LocalTokenScope {
        &self.local_tokens
    }

    /// local token scope(가변).
    pub fn local_tokens_mut(&mut self) -> &mut LocalTokenScope {
        &mut self.local_tokens
    }

    /// 정의를 자신의 정의 tree에 대해 검증합니다.
    ///
    /// - tree가 이 정의 소유여야 합니다.
    /// - root가 tree 안에 있어야 합니다(컴포넌트 밖 node 거부).
    /// - root는 Frame이고 tree 최상위(parent 없음) node여야 합니다.
    pub fn validate(&self, tree: &DefinitionTree) -> Result<(), ComponentDefinitionError> {
        if tree.component().id() != self.id {
            return Err(ComponentDefinitionError::TreeOwnerMismatch {
                definition: self.id,
                tree_owner: tree.component().id(),
            });
        }
        let root = self.root.id();
        let node = tree
            .node(root)
            .ok_or(ComponentDefinitionError::RootOutsideComponent {
                component: self.id,
                root,
            })?;
        if node.kind != NodeKind::Frame {
            return Err(ComponentDefinitionError::RootNotFrame {
                root,
                kind: node.kind,
            });
        }
        if node.parent.is_some() {
            return Err(ComponentDefinitionError::RootNotTreeTop { root });
        }
        Ok(())
    }

    fn encode_into(&self, buf: &mut Vec<u8>) {
        put_id(buf, self.id);
        put_str(buf, &self.display_name);
        put_id(buf, self.root.id());
        put_count(buf, self.properties.len());
        for property in &self.properties {
            put_id(buf, property.stable_id());
        }
        put_count(buf, self.slots.len());
        for slot in &self.slots {
            put_id(buf, slot.stable_id());
        }
        put_count(buf, self.local_tokens.len());
        for token in self.local_tokens.iter() {
            put_id(buf, token.id());
        }
    }

    fn decode_from(
        reader: &mut Reader<'_>,
    ) -> Result<ComponentDefinition, ComponentDefinitionError> {
        let id = reader.id()?;
        let display_name = reader.string()?;
        let root = NodeRef::new(reader.id()?);
        let mut definition = ComponentDefinition::new(id, display_name, root);
        let property_count = reader.count()?;
        definition.properties.reserve(capacity_hint(property_count));
        for _ in 0..property_count {
            definition.add_property(ComponentPropertyId::new(reader.id()?))?;
        }
        let slot_count = reader.count()?;
        definition.slots.reserve(capacity_hint(slot_count));
        for _ in 0..slot_count {
            definition.add_slot(ComponentSlotId::new(reader.id()?))?;
        }
        let token_count = reader.count()?;
        let mut previous: Option<TokenRef> = None;
        for _ in 0..token_count {
            let token = TokenRef::new(reader.id()?);
            if previous.is_some_and(|p| p >= token) {
                return Err(ComponentDefinitionError::Malformed {
                    reason: "local token scope가 오름차순이 아님",
                });
            }
            previous = Some(token);
            definition.local_tokens.insert(token);
        }
        Ok(definition)
    }

    /// canonical binary bytes로 인코딩합니다.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode_into(&mut buf);
        buf
    }

    /// binary bytes에서 디코딩합니다. 중복·정렬 위반·trailing bytes는 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<ComponentDefinition, ComponentDefinitionError> {
        let mut reader = Reader::new(bytes);
        let definition = ComponentDefinition::decode_from(&mut reader)?;
        reader.expect_end()?;
        Ok(definition)
    }
}

/// library에 등록된 컴포넌트 한 개: 정의 + 정의 tree + Slot record(IX.3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentEntry {
    definition: ComponentDefinition,
    tree: DefinitionTree,
    slots: BTreeMap<ComponentSlotId, ComponentSlot>,
}

impl ComponentEntry {
    /// 정의.
    pub fn definition(&self) -> &ComponentDefinition {
        &self.definition
    }

    /// 정의 tree.
    pub fn tree(&self) -> &DefinitionTree {
        &self.tree
    }

    /// slot record.
    pub fn slot(&self, slot: ComponentSlotId) -> Option<&ComponentSlot> {
        self.slots.get(&slot)
    }

    /// slot record(slot id 오름차순).
    pub fn slots(&self) -> impl Iterator<Item = &ComponentSlot> + '_ {
        self.slots.values()
    }

    /// host node에 붙은 slot.
    pub fn slot_for_host(&self, host: StableId) -> Option<&ComponentSlot> {
        self.slots.values().find(|slot| slot.host.id() == host)
    }

    pub(super) fn insert_slot(&mut self, slot: ComponentSlot) {
        self.slots.insert(slot.id, slot);
    }
}

/// 문서의 컴포넌트 정의 모음(ComponentLibrary chunk resource).
///
/// 등록 시 정의를 tree에 대해 검증하고, 한 node가 두 컴포넌트에 동시에 속하지 않게
/// 합니다. 순회·직렬화는 컴포넌트 StableId 오름차순입니다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComponentLibrary {
    entries: BTreeMap<StableId, ComponentEntry>,
    node_owner: BTreeMap<StableId, StableId>,
}

impl ComponentLibrary {
    /// 빈 library.
    pub fn new() -> ComponentLibrary {
        ComponentLibrary::default()
    }

    /// 정의와 정의 tree를 등록합니다.
    ///
    /// 정의 검증([`ComponentDefinition::validate`]) 실패, 중복 컴포넌트 id, 다른 컴포넌트가
    /// 이미 소유한 node를 포함한 tree는 거부하며 이때 library는 변경되지 않습니다.
    pub fn insert(
        &mut self,
        definition: ComponentDefinition,
        tree: DefinitionTree,
    ) -> Result<(), ComponentDefinitionError> {
        if self.entries.contains_key(&definition.id) || self.node_owner.contains_key(&definition.id)
        {
            return Err(ComponentDefinitionError::DuplicateId { id: definition.id });
        }
        definition.validate(&tree)?;
        if let Some((node, owner)) = tree
            .iter()
            .find_map(|n| self.node_owner.get(&n.id).map(|owner| (n.id, *owner)))
        {
            return Err(ComponentDefinitionError::NodeOwnedByOtherComponent { node, owner });
        }
        // node id는 컴포넌트 id와도 겹칠 수 없습니다(전역 유일성).
        if let Some(clash) = tree
            .iter()
            .map(|n| n.id)
            .find(|id| *id == definition.id || self.entries.contains_key(id))
        {
            return Err(ComponentDefinitionError::DuplicateId { id: clash });
        }
        for node in tree.iter() {
            self.node_owner.insert(node.id, definition.id);
        }
        self.entries.insert(
            definition.id,
            ComponentEntry {
                definition,
                tree,
                slots: BTreeMap::new(),
            },
        );
        Ok(())
    }

    /// 컴포넌트를 찾습니다.
    pub fn get(&self, component: ComponentRef) -> Option<&ComponentEntry> {
        self.entries.get(&component.id())
    }

    pub(super) fn entry_mut(&mut self, component: ComponentRef) -> Option<&mut ComponentEntry> {
        self.entries.get_mut(&component.id())
    }

    /// node를 소유한 컴포넌트.
    pub fn owner_of(&self, node: StableId) -> Option<ComponentRef> {
        self.node_owner.get(&node).copied().map(ComponentRef::new)
    }

    /// 컴포넌트를 제거하고 반환합니다. 없으면 오류입니다.
    pub fn remove(
        &mut self,
        component: ComponentRef,
    ) -> Result<ComponentEntry, ComponentDefinitionError> {
        let entry = self
            .entries
            .remove(&component.id())
            .ok_or(ComponentDefinitionError::NotFound { id: component.id() })?;
        for node in entry.tree.iter() {
            self.node_owner.remove(&node.id);
        }
        Ok(entry)
    }

    /// 등록된 컴포넌트 수.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 컴포넌트 StableId 오름차순 순회.
    pub fn iter(&self) -> impl Iterator<Item = &ComponentEntry> + '_ {
        self.entries.values()
    }

    /// canonical binary bytes로 인코딩합니다(컴포넌트 StableId 오름차순).
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        put_count(&mut buf, self.entries.len());
        for entry in self.entries.values() {
            entry.definition.encode_into(&mut buf);
            entry.tree.encode_into(&mut buf);
            put_count(&mut buf, entry.slots.len());
            for slot in entry.slots.values() {
                slot.encode_into(&mut buf);
            }
        }
        buf
    }

    /// binary bytes에서 디코딩하고 등록 규칙으로 재검증합니다. 비canonical 입력은 거부합니다.
    ///
    /// 모든 컴포넌트가 로드된 뒤 선언 Slot마다 record가 있는지([`ComponentLibrary::validate_slots`])와
    /// 정의 tree 안 nested 인스턴스의 slot override 투영 규칙(IX.3.2)을 검사합니다. nested
    /// 인스턴스가 이 library에 없는 컴포넌트를 참조하면(외부·미해결 참조, III.2.4) 그 인스턴스의
    /// 투영 검사는 건너뜁니다 — 참조 해석 계층이 진단합니다.
    pub fn decode(bytes: &[u8]) -> Result<ComponentLibrary, ComponentDefinitionError> {
        let mut reader = Reader::new(bytes);
        let count = reader.count()?;
        let mut library = ComponentLibrary::new();
        for _ in 0..count {
            let definition = ComponentDefinition::decode_from(&mut reader)?;
            let component = definition.reference();
            let tree = DefinitionTree::decode_from(&mut reader)?;
            library.insert(definition, tree)?;
            let slot_count = reader.count()?;
            for _ in 0..slot_count {
                let slot = ComponentSlot::decode_from(&mut reader)?;
                library.add_slot(component, slot)?;
            }
        }
        reader.expect_end()?;
        if library.encode() != bytes {
            return Err(ComponentDefinitionError::Malformed {
                reason: "component library가 canonical 순서가 아님",
            });
        }
        library.validate_loaded_slots()?;
        Ok(library)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn node(id: u64, kind: NodeKind, parent: Option<u64>, key: u8) -> DefinitionNode {
        DefinitionNode {
            id: stable(id),
            kind,
            parent: parent.map(stable),
            order_key: OrderKey::from_bytes(vec![key]).unwrap(),
            visible: true,
            locked: false,
            display_name: format!("node-{id}"),
            payload: vec![id as u8],
        }
    }

    /// 컴포넌트 `component`의 정의 tree: root(Frame) ← label(Text), icon(Shape).
    fn tree(component: u64, base: u64) -> DefinitionTree {
        let mut tree = DefinitionTree::new(ComponentRef::new(stable(component)));
        tree.insert(node(base, NodeKind::Frame, None, 0x80))
            .unwrap();
        tree.insert(node(base + 1, NodeKind::Text, Some(base), 0x40))
            .unwrap();
        tree.insert(node(base + 2, NodeKind::Shape, Some(base), 0x20))
            .unwrap();
        tree
    }

    fn definition(component: u64, root: u64) -> ComponentDefinition {
        ComponentDefinition::new(
            stable(component),
            "버튼".to_string(),
            NodeRef::new(stable(root)),
        )
    }

    #[test]
    fn root_outside_component_is_rejected() {
        // 검증 기준(IX.1.1): root가 component 밖 node를 가리키면 거부.
        let tree_a = tree(1, 100);
        let tree_b = tree(2, 200);

        // 다른 컴포넌트(B)의 node를 root로 지정.
        let def = definition(1, 200);
        assert_eq!(
            def.validate(&tree_a).unwrap_err(),
            ComponentDefinitionError::RootOutsideComponent {
                component: stable(1),
                root: stable(200),
            }
        );
        // scene(View) node처럼 어떤 정의 tree에도 없는 node를 root로 지정.
        let def = definition(1, 9_999);
        assert!(matches!(
            def.validate(&tree_a).unwrap_err(),
            ComponentDefinitionError::RootOutsideComponent { .. }
        ));
        // library 등록 경로도 같은 규칙으로 거부하고 상태를 바꾸지 않습니다.
        let mut library = ComponentLibrary::new();
        library.insert(definition(2, 200), tree_b).unwrap();
        assert!(matches!(
            library.insert(definition(1, 200), tree_a.clone()),
            Err(ComponentDefinitionError::RootOutsideComponent { .. })
        ));
        assert_eq!(library.len(), 1);
        assert_eq!(library.owner_of(stable(100)), None);

        // 올바른 root는 통과.
        library.insert(definition(1, 100), tree_a).unwrap();
        assert_eq!(
            library.owner_of(stable(101)),
            Some(ComponentRef::new(stable(1)))
        );
    }

    #[test]
    fn root_must_be_top_level_frame_of_own_tree() {
        let tree_a = tree(1, 100);
        // tree 중간 node(Text)를 root로 지정 → Frame 아님.
        assert_eq!(
            definition(1, 101).validate(&tree_a).unwrap_err(),
            ComponentDefinitionError::RootNotFrame {
                root: stable(101),
                kind: NodeKind::Text,
            }
        );
        // tree 중간 Frame을 root로 지정 → 최상위 아님.
        let mut nested = tree(1, 100);
        nested
            .insert(node(103, NodeKind::Frame, Some(100), 0x60))
            .unwrap();
        assert_eq!(
            definition(1, 103).validate(&nested).unwrap_err(),
            ComponentDefinitionError::RootNotTreeTop { root: stable(103) }
        );
        // 다른 컴포넌트 소유 tree를 연결.
        assert_eq!(
            definition(1, 200).validate(&tree(2, 200)).unwrap_err(),
            ComponentDefinitionError::TreeOwnerMismatch {
                definition: stable(1),
                tree_owner: stable(2),
            }
        );
        assert!(definition(1, 100).validate(&tree_a).is_ok());
    }

    #[test]
    fn node_belongs_to_exactly_one_component() {
        let mut library = ComponentLibrary::new();
        library.insert(definition(1, 100), tree(1, 100)).unwrap();
        // 같은 node id(101)를 포함한 다른 컴포넌트 tree는 거부.
        let mut other = DefinitionTree::new(ComponentRef::new(stable(2)));
        other
            .insert(node(200, NodeKind::Frame, None, 0x80))
            .unwrap();
        other
            .insert(node(101, NodeKind::Text, Some(200), 0x40))
            .unwrap();
        assert_eq!(
            library.insert(definition(2, 200), other).unwrap_err(),
            ComponentDefinitionError::NodeOwnedByOtherComponent {
                node: stable(101),
                owner: stable(1),
            }
        );
        // 중복 컴포넌트 id.
        assert_eq!(
            library
                .insert(definition(1, 300), tree(1, 300))
                .unwrap_err(),
            ComponentDefinitionError::DuplicateId { id: stable(1) }
        );
        // 제거 후에는 소유 관계도 해제.
        library.remove(ComponentRef::new(stable(1))).unwrap();
        assert_eq!(library.owner_of(stable(101)), None);
        assert!(library.is_empty());
    }

    #[test]
    fn tree_insert_rules() {
        let mut tree = DefinitionTree::new(ComponentRef::new(stable(1)));
        tree.insert(node(100, NodeKind::Frame, None, 0x80)).unwrap();
        // 두 번째 root.
        assert_eq!(
            tree.insert(node(150, NodeKind::Frame, None, 0x80))
                .unwrap_err(),
            ComponentDefinitionError::MultipleTreeRoots { node: stable(150) }
        );
        // 없는 부모.
        assert_eq!(
            tree.insert(node(101, NodeKind::Text, Some(999), 0x40))
                .unwrap_err(),
            ComponentDefinitionError::ParentNotInTree {
                node: stable(101),
                parent: stable(999),
            }
        );
        // 중복 id.
        assert!(matches!(
            tree.insert(node(100, NodeKind::Text, Some(100), 0x40)),
            Err(ComponentDefinitionError::DuplicateId { .. })
        ));
        // 형제 OrderKey 중복.
        tree.insert(node(101, NodeKind::Text, Some(100), 0x40))
            .unwrap();
        assert_eq!(
            tree.insert(node(102, NodeKind::Text, Some(100), 0x40))
                .unwrap_err(),
            ComponentDefinitionError::DuplicateOrderKey {
                parent: stable(100)
            }
        );
        // 자식은 OrderKey 순(삽입 순서 무관).
        tree.insert(node(103, NodeKind::Shape, Some(100), 0x20))
            .unwrap();
        assert_eq!(tree.children(stable(100)), &[stable(103), stable(101)]);
        assert_eq!(tree.preorder(), vec![stable(100), stable(103), stable(101)]);
    }

    #[test]
    fn interface_lists_reject_duplicates() {
        let mut def = definition(1, 100);
        let p1 = ComponentPropertyId::new(stable(500));
        let p2 = ComponentPropertyId::new(stable(501));
        def.add_property(p2).unwrap();
        def.add_property(p1).unwrap();
        assert!(matches!(
            def.add_property(p1),
            Err(ComponentDefinitionError::DuplicateId { .. })
        ));
        // 선언 순서 유지.
        assert_eq!(def.properties(), &[p2, p1]);
        def.remove_property(p2).unwrap();
        assert!(def.remove_property(p2).is_err());

        let s1 = ComponentSlotId::new(stable(600));
        def.add_slot(s1).unwrap();
        assert!(def.add_slot(s1).is_err());
        assert_eq!(def.slots(), &[s1]);
        def.remove_slot(s1).unwrap();
        assert!(def.slots().is_empty());

        let t1 = TokenRef::new(stable(701));
        let t0 = TokenRef::new(stable(700));
        assert!(def.local_tokens_mut().insert(t1));
        assert!(def.local_tokens_mut().insert(t0));
        assert!(!def.local_tokens_mut().insert(t0));
        assert_eq!(def.local_tokens().iter().collect::<Vec<_>>(), vec![t0, t1]);
        assert!(def.local_tokens().contains(t1));
    }

    #[test]
    fn definition_and_library_round_trip() {
        let mut def = definition(1, 100);
        def.add_property(ComponentPropertyId::new(stable(501)))
            .unwrap();
        def.add_property(ComponentPropertyId::new(stable(500)))
            .unwrap();
        def.add_slot(ComponentSlotId::new(stable(600))).unwrap();
        def.local_tokens_mut().insert(TokenRef::new(stable(701)));
        def.local_tokens_mut().insert(TokenRef::new(stable(700)));

        let bytes = def.encode();
        let decoded = ComponentDefinition::decode(&bytes).unwrap();
        assert_eq!(decoded, def);
        assert_eq!(decoded.encode(), bytes);

        let tree_a = tree(1, 100);
        let tree_bytes = tree_a.encode();
        assert_eq!(DefinitionTree::decode(&tree_bytes).unwrap(), tree_a);

        let mut library = ComponentLibrary::new();
        library.insert(def, tree_a).unwrap();
        library.insert(definition(2, 200), tree(2, 200)).unwrap();
        // 선언 slot(600)에 record가 없는 library는 디코딩에서 거부됩니다(validate_slots).
        assert_eq!(
            ComponentLibrary::decode(&library.encode()).unwrap_err(),
            ComponentDefinitionError::Slot(SlotError::MissingSlotRecord {
                slot: ComponentSlotId::new(stable(600)),
            })
        );
        library
            .add_slot(
                ComponentRef::new(stable(1)),
                ComponentSlot::new(
                    ComponentSlotId::new(stable(600)),
                    NodeRef::new(stable(100)),
                    [NodeKind::Text, NodeKind::Shape],
                    crate::component::slot::SlotCardinality::unbounded(),
                    vec![NodeRef::new(stable(102)), NodeRef::new(stable(101))],
                ),
            )
            .unwrap();
        let lib_bytes = library.encode();
        let lib_decoded = ComponentLibrary::decode(&lib_bytes).unwrap();
        assert_eq!(lib_decoded, library);
        assert_eq!(lib_decoded.encode(), lib_bytes);
    }

    #[test]
    fn malformed_bytes_are_rejected() {
        let mut def = definition(1, 100);
        def.add_property(ComponentPropertyId::new(stable(500)))
            .unwrap();
        def.local_tokens_mut().insert(TokenRef::new(stable(700)));
        let bytes = def.encode();
        for cut in [0, 15, 16, bytes.len() - 1] {
            assert!(
                ComponentDefinition::decode(&bytes[..cut]).is_err(),
                "절단 {cut}"
            );
        }
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(ComponentDefinition::decode(&extended).is_err());

        // 중복 property id가 든 bytes.
        let mut dup = definition(1, 100);
        dup.add_property(ComponentPropertyId::new(stable(500)))
            .unwrap();
        dup.add_property(ComponentPropertyId::new(stable(501)))
            .unwrap();
        let mut dup_bytes = dup.encode();
        // 두 번째 property id(501)를 첫 번째(500)로 덮어씀.
        let first = 16 + 4 + "버튼".len() + 16 + 4;
        let (head, tail) = dup_bytes.split_at_mut(first + 16);
        tail[..16].copy_from_slice(&head[first..first + 16]);
        assert!(matches!(
            ComponentDefinition::decode(&dup_bytes),
            Err(ComponentDefinitionError::DuplicateId { .. })
        ));

        // 전위 순서가 아닌 tree bytes(자식이 부모보다 먼저)는 거부.
        let tree_a = tree(1, 100);
        let mut reordered = Vec::new();
        put_id(&mut reordered, stable(1));
        put_count(&mut reordered, 2);
        for n in [
            node(101, NodeKind::Text, Some(100), 0x40),
            node(100, NodeKind::Frame, None, 0x80),
        ] {
            put_id(&mut reordered, n.id);
            put_kind(&mut reordered, n.kind);
            put_opt_id(&mut reordered, n.parent);
            put_order_key(&mut reordered, &n.order_key);
            put_bool(&mut reordered, n.visible);
            put_bool(&mut reordered, n.locked);
            put_str(&mut reordered, &n.display_name);
            put_bytes(&mut reordered, &n.payload);
        }
        assert!(DefinitionTree::decode(&reordered).is_err());
        assert!(DefinitionTree::decode(&tree_a.encode()).is_ok());
    }

    #[test]
    fn rename_keeps_identity() {
        let mut def = definition(1, 100);
        def.rename("새 이름".to_string());
        assert_eq!(def.display_name(), "새 이름");
        assert_eq!(def.reference(), ComponentRef::new(stable(1)));
        assert!(def.validate(&tree(1, 100)).is_ok());
    }
}

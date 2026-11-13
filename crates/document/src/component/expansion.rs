//! Instance expansion view (IX.2.2).
//!
//! 인스턴스의 하위 구조는 정의 tree를 **복제하지 않고** 읽기 전용 가상(virtual) subtree로
//! 제공합니다. 가상 node는 `(인스턴스 chain, 정의 node StableId)` 쌍으로 식별되고, node 내용은
//! [`ComponentLibrary`]에 저장된 [`DefinitionNode`]를 그대로 참조합니다. 따라서 인스턴스가
//! 5만 개여도 정의 node는 한 벌뿐입니다.
//!
//! - nested ComponentInstance(정의 tree 안의 인스턴스)는 그 정의의 가상 subtree로 다시
//!   펼칩니다. 인스턴스 chain에 같은 컴포넌트가 다시 나오면 재귀 정의로 거부합니다.
//! - instance override는 [`InstanceExpansion::property_override`]로 합성해 읽습니다. 값을
//!   node 속성에 반영하는 binding은 Phase VIII의 몫입니다.
//! - expansion은 공유 참조만 보유하며 변경 API가 없습니다(읽기 전용).

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fmt;

use stellar_core_types::ids::{RuntimeEntityId, StableId};
use stellar_core_types::property::PropertyPath;
use stellar_core_types::reference::{ComponentRef, UnresolvedRef};
use stellar_core_types::typed_value::TypedValue;

use super::definition::{ComponentEntry, ComponentLibrary, DefinitionNode};
use super::instance::{keys, ComponentInstancePayload, InstancePayloadError, OverrideKey};
use crate::node_columns::{NodeColumns, NodeFields};
use crate::node_kind::NodeKind;

/// 인스턴스 중첩 깊이 상한(문서 depth ≤ 64 정책과 같은 값).
pub const MAX_INSTANCE_NESTING: usize = 64;

/// scene node별 canonical payload bytes(종류별 codec이 해석).
pub type ScenePayloads = BTreeMap<StableId, Vec<u8>>;

/// expansion 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpansionError {
    /// scene에 node가 없습니다.
    NodeNotFound {
        /// 요청 id.
        id: StableId,
    },
    /// node가 ComponentInstance가 아닙니다.
    NotAnInstance {
        /// node.
        id: StableId,
        /// 실제 종류.
        kind: NodeKind,
    },
    /// 인스턴스 payload가 없거나 해석할 수 없습니다.
    Payload {
        /// 인스턴스 node.
        node: StableId,
        /// 원인.
        error: InstancePayloadError,
    },
    /// 참조한 컴포넌트 정의가 library에 없습니다(미해결 참조, III.2.4).
    UnresolvedComponent(UnresolvedRef),
    /// 인스턴스 chain에 같은 컴포넌트가 다시 나옵니다(재귀 정의).
    RecursiveComponent {
        /// 반복된 컴포넌트.
        component: StableId,
    },
    /// 인스턴스 중첩이 상한을 넘습니다.
    NestingTooDeep {
        /// 시도한 깊이.
        depth: usize,
    },
    /// id가 이 expansion에 속하지 않습니다.
    NotInExpansion {
        /// 요청 id.
        id: ExpandedNodeId,
    },
    /// 인스턴스 소유 node가 어떤 slot에도 투영되지 않았습니다(IX.3.2).
    NotProjected {
        /// node.
        node: StableId,
    },
}

impl fmt::Display for ExpansionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExpansionError::NodeNotFound { id } => write!(f, "scene에 node {id}가 없습니다"),
            ExpansionError::NotAnInstance { id, kind } => write!(
                f,
                "node {id}는 ComponentInstance가 아닙니다({})",
                kind.name()
            ),
            ExpansionError::Payload { node, error } => {
                write!(f, "인스턴스 {node} payload 오류: {error}")
            }
            ExpansionError::UnresolvedComponent(unresolved) => write!(
                f,
                "컴포넌트 {}를 찾을 수 없습니다",
                unresolved.original_id()
            ),
            ExpansionError::RecursiveComponent { component } => {
                write!(f, "컴포넌트 {component}가 자기 자신을 중첩합니다")
            }
            ExpansionError::NestingTooDeep { depth } => write!(
                f,
                "인스턴스 중첩 깊이 {depth}가 상한 {MAX_INSTANCE_NESTING}을 넘습니다"
            ),
            ExpansionError::NotInExpansion { id } => {
                write!(f, "{id:?}는 이 expansion에 속하지 않습니다")
            }
            ExpansionError::NotProjected { node } => {
                write!(
                    f,
                    "인스턴스 소유 node {node}가 어떤 slot에도 투영되지 않았습니다"
                )
            }
        }
    }
}

impl std::error::Error for ExpansionError {}

/// scene node를 StableId로 조회하는 읽기 전용 index.
///
/// [`NodeColumns`]는 런타임 핸들 기반이라, expansion이 쓸 수 있게 StableId ↔ 핸들과
/// 정렬된 자식 목록(OrderKey, 동일 시 StableId — VI.2.4와 같은 규칙)을 한 번 색인합니다.
#[derive(Debug)]
pub struct SceneIndex<'a> {
    columns: &'a NodeColumns,
    payloads: &'a ScenePayloads,
    by_stable: HashMap<StableId, RuntimeEntityId>,
    stable_by_runtime: HashMap<RuntimeEntityId, StableId>,
    children: HashMap<StableId, Vec<StableId>>,
}

impl<'a> SceneIndex<'a> {
    /// scene column과 payload로 index를 만듭니다.
    pub fn build(columns: &'a NodeColumns, payloads: &'a ScenePayloads) -> SceneIndex<'a> {
        let mut by_stable = HashMap::with_capacity(columns.len());
        let mut stable_by_runtime = HashMap::with_capacity(columns.len());
        let mut records = Vec::with_capacity(columns.len());
        for handle in columns.iter_live() {
            if let Some(fields) = columns.get(handle) {
                by_stable.insert(fields.stable_id, handle);
                stable_by_runtime.insert(handle, fields.stable_id);
                records.push(fields);
            }
        }
        let mut children: HashMap<StableId, Vec<(NodeFields, StableId)>> = HashMap::new();
        for fields in records {
            if let Some(parent) = fields.parent.and_then(|p| stable_by_runtime.get(&p)) {
                let id = fields.stable_id;
                children.entry(*parent).or_default().push((fields, id));
            }
        }
        let children = children
            .into_iter()
            .map(|(parent, mut list)| {
                list.sort_by(|a, b| (&a.0.order_key, a.1).cmp(&(&b.0.order_key, b.1)));
                (parent, list.into_iter().map(|(_, id)| id).collect())
            })
            .collect();
        SceneIndex {
            columns,
            payloads,
            by_stable,
            stable_by_runtime,
            children,
        }
    }

    /// StableId의 런타임 핸들.
    pub fn runtime_id(&self, id: StableId) -> Option<RuntimeEntityId> {
        self.by_stable.get(&id).copied()
    }

    /// 공통 필드.
    pub fn fields(&self, id: StableId) -> Option<NodeFields> {
        self.columns.get(self.runtime_id(id)?)
    }

    /// 부모 StableId(root면 `None`).
    pub fn parent(&self, id: StableId) -> Option<StableId> {
        let parent = self.fields(id)?.parent?;
        self.stable_by_runtime.get(&parent).copied()
    }

    /// 자식(OrderKey 순).
    pub fn children(&self, id: StableId) -> &[StableId] {
        self.children.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// node의 payload bytes.
    pub fn payload(&self, id: StableId) -> Option<&'a [u8]> {
        self.payloads.get(&id).map(Vec::as_slice)
    }

    /// node 수.
    pub fn len(&self) -> usize {
        self.by_stable.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.by_stable.is_empty()
    }

    /// scene ComponentInstance node의 payload를 해석합니다.
    pub fn instance_payload(
        &self,
        id: StableId,
    ) -> Result<ComponentInstancePayload, ExpansionError> {
        let fields = self.fields(id).ok_or(ExpansionError::NodeNotFound { id })?;
        if fields.kind != NodeKind::ComponentInstance {
            return Err(ExpansionError::NotAnInstance {
                id,
                kind: fields.kind,
            });
        }
        let bytes = self.payload(id).ok_or(ExpansionError::Payload {
            node: id,
            error: InstancePayloadError::Malformed {
                reason: "인스턴스 payload 없음",
            },
        })?;
        ComponentInstancePayload::decode(bytes)
            .map_err(|error| ExpansionError::Payload { node: id, error })
    }
}

/// expansion 안 node 식별자.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExpandedNodeId {
    /// 정의 tree node의 가상 투영.
    ///
    /// `instances`는 scene 인스턴스부터 이 node를 소유한 정의의 인스턴스까지의 chain입니다
    /// (첫 원소는 scene ComponentInstance, 이후는 정의 tree 안 nested instance node).
    Virtual {
        /// 인스턴스 chain(바깥 → 안쪽).
        instances: Vec<StableId>,
        /// 정의 tree node.
        source: StableId,
    },
    /// 인스턴스가 소유한 실제 scene node(slot 투영, IX.3.2).
    ///
    /// scene에서 부모는 인스턴스(또는 그 소유 node) 그대로이고, expansion에서만 slot host
    /// 아래에 투영됩니다.
    Owned(StableId),
}

impl ExpandedNodeId {
    /// 가상 node id를 만듭니다.
    pub fn virtual_node(instances: Vec<StableId>, source: StableId) -> ExpandedNodeId {
        ExpandedNodeId::Virtual { instances, source }
    }
}

/// expansion node의 내용 출처.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NodeSource<'a> {
    /// 정의 node 참조(복제 없음).
    Definition(&'a DefinitionNode),
    /// 인스턴스 소유 scene node.
    Owned {
        fields: NodeFields,
        payload: Option<&'a [u8]>,
    },
}

/// expansion 안 node 한 개(읽기 전용).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedNode<'a> {
    id: ExpandedNodeId,
    source: NodeSource<'a>,
}

impl<'a> ExpandedNode<'a> {
    /// node id.
    pub fn id(&self) -> &ExpandedNodeId {
        &self.id
    }

    /// 원본 정의 node — 복제본이 아니라 library 저장소의 참조입니다. 인스턴스 소유 node면
    /// `None`.
    pub fn definition_node(&self) -> Option<&'a DefinitionNode> {
        match &self.source {
            NodeSource::Definition(node) => Some(node),
            NodeSource::Owned { .. } => None,
        }
    }

    /// 인스턴스 소유 scene node면 그 공통 필드.
    pub fn owned_fields(&self) -> Option<&NodeFields> {
        match &self.source {
            NodeSource::Definition(_) => None,
            NodeSource::Owned { fields, .. } => Some(fields),
        }
    }

    /// node 종류.
    pub fn kind(&self) -> NodeKind {
        match &self.source {
            NodeSource::Definition(node) => node.kind,
            NodeSource::Owned { fields, .. } => fields.kind,
        }
    }

    /// 표시 여부.
    pub fn visible(&self) -> bool {
        match &self.source {
            NodeSource::Definition(node) => node.visible,
            NodeSource::Owned { fields, .. } => fields.visible,
        }
    }

    /// 잠금 여부.
    pub fn locked(&self) -> bool {
        match &self.source {
            NodeSource::Definition(node) => node.locked,
            NodeSource::Owned { fields, .. } => fields.locked,
        }
    }

    /// 표시 이름.
    pub fn display_name(&self) -> &str {
        match &self.source {
            NodeSource::Definition(node) => &node.display_name,
            NodeSource::Owned { fields, .. } => &fields.display_name,
        }
    }

    /// 종류별 payload bytes(scene node에 payload가 없으면 `None`).
    pub fn payload(&self) -> Option<&'a [u8]> {
        match &self.source {
            NodeSource::Definition(node) => Some(&node.payload),
            NodeSource::Owned { payload, .. } => *payload,
        }
    }
}

/// 한 scene 인스턴스의 읽기 전용 가상 subtree.
///
/// 범위는 인스턴스의 시각 subtree 전체입니다: 정의 tree의 가상 투영, slot에 투영된 인스턴스
/// 소유 node(`Owned`), 그 소유 node가 다시 ComponentInstance이면 그 정의의 가상 투영까지.
#[derive(Debug, Clone)]
pub struct InstanceExpansion<'a> {
    library: &'a ComponentLibrary,
    scene: &'a SceneIndex<'a>,
    instance: StableId,
    payload: ComponentInstancePayload,
    entry: &'a ComponentEntry,
}

fn unresolved(location: StableId, component: ComponentRef) -> ExpansionError {
    ExpansionError::UnresolvedComponent(
        component.unresolved_at(PropertyPath::with_keys(location, vec![keys::COMPONENT_REF])),
    )
}

impl<'a> InstanceExpansion<'a> {
    /// scene 인스턴스의 expansion을 만듭니다.
    ///
    /// node가 ComponentInstance가 아니거나, payload가 잘못됐거나, 참조 정의가 library에
    /// 없으면 거부합니다(정의가 없으면 미해결 참조로 보고).
    pub fn new(
        library: &'a ComponentLibrary,
        scene: &'a SceneIndex<'a>,
        instance: StableId,
    ) -> Result<InstanceExpansion<'a>, ExpansionError> {
        let payload = scene.instance_payload(instance)?;
        let entry = library
            .get(payload.component())
            .ok_or_else(|| unresolved(instance, payload.component()))?;
        Ok(InstanceExpansion {
            library,
            scene,
            instance,
            payload,
            entry,
        })
    }

    /// scene 인스턴스 node.
    pub fn instance(&self) -> StableId {
        self.instance
    }

    /// scene 인스턴스 payload.
    pub fn payload(&self) -> &ComponentInstancePayload {
        &self.payload
    }

    /// 펼친 컴포넌트 정의.
    pub fn entry(&self) -> &'a ComponentEntry {
        self.entry
    }

    /// scene index.
    pub fn scene(&self) -> &'a SceneIndex<'a> {
        self.scene
    }

    /// library.
    pub fn library(&self) -> &'a ComponentLibrary {
        self.library
    }

    /// 가상 subtree root(정의 root Frame의 가상 투영).
    pub fn root(&self) -> ExpandedNodeId {
        ExpandedNodeId::virtual_node(vec![self.instance], self.entry.definition().root.id())
    }

    /// 이 인스턴스의 PropertyOverride 값(없으면 정의 default를 쓰라는 뜻).
    pub fn property_override(&self, key: &OverrideKey) -> Option<&TypedValue> {
        self.payload.property_override(key)
    }

    /// scene node가 이 인스턴스의 scene 후손(인스턴스 자신 제외)인지.
    fn is_scene_descendant(&self, id: StableId) -> bool {
        let mut current = self.scene.parent(id);
        let mut steps = 0usize;
        while let Some(parent) = current {
            if parent == self.instance {
                return true;
            }
            steps += 1;
            if steps > self.scene.len() {
                return false;
            }
            current = self.scene.parent(parent);
        }
        false
    }

    /// chain 첫 원소(scene 인스턴스)가 펼치는 컴포넌트.
    fn scene_entry(&self, instance: StableId) -> Result<&'a ComponentEntry, ExpansionError> {
        if instance == self.instance {
            return Ok(self.entry);
        }
        if !self.is_scene_descendant(instance) {
            return Err(self.not_in(&[instance], StableId::NIL));
        }
        let payload = self.scene.instance_payload(instance)?;
        self.library
            .get(payload.component())
            .ok_or_else(|| unresolved(instance, payload.component()))
    }

    /// scene 인스턴스 payload(이 expansion의 인스턴스면 차용).
    fn scene_payload(
        &self,
        instance: StableId,
    ) -> Result<Cow<'_, ComponentInstancePayload>, ExpansionError> {
        if instance == self.instance {
            Ok(Cow::Borrowed(&self.payload))
        } else {
            Ok(Cow::Owned(self.scene.instance_payload(instance)?))
        }
    }

    /// 인스턴스 chain을 따라 마지막 인스턴스가 펼치는 컴포넌트 정의를 찾습니다.
    pub(crate) fn resolve_chain(
        &self,
        instances: &[StableId],
    ) -> Result<&'a ComponentEntry, ExpansionError> {
        let (first, rest) = instances
            .split_first()
            .ok_or_else(|| self.not_in(instances, StableId::NIL))?;
        let mut entry = self.scene_entry(*first)?;
        for nested in rest {
            entry = self.nested_entry(entry, *nested)?;
        }
        Ok(entry)
    }

    /// chain 마지막 인스턴스의 payload.
    pub(crate) fn chain_payload(
        &self,
        instances: &[StableId],
    ) -> Result<Cow<'_, ComponentInstancePayload>, ExpansionError> {
        match instances.split_last() {
            Some((only, [])) => self.scene_payload(*only),
            Some((last, outer)) => {
                let owner = self.resolve_chain(outer)?;
                Ok(Cow::Owned(self.nested_payload(owner, *last)?))
            }
            None => Err(self.not_in(instances, StableId::NIL)),
        }
    }

    /// 정의 tree 안 nested instance node의 payload.
    pub(crate) fn nested_payload(
        &self,
        owner: &ComponentEntry,
        nested: StableId,
    ) -> Result<ComponentInstancePayload, ExpansionError> {
        let node = owner
            .tree()
            .node(nested)
            .ok_or(ExpansionError::NodeNotFound { id: nested })?;
        ComponentInstancePayload::decode_for(node.kind, &node.payload).map_err(
            |error| match error {
                InstancePayloadError::KindMismatch { kind } => {
                    ExpansionError::NotAnInstance { id: nested, kind }
                }
                error => ExpansionError::Payload {
                    node: nested,
                    error,
                },
            },
        )
    }

    fn nested_entry(
        &self,
        owner: &ComponentEntry,
        nested: StableId,
    ) -> Result<&'a ComponentEntry, ExpansionError> {
        let payload = self.nested_payload(owner, nested)?;
        self.library
            .get(payload.component())
            .ok_or_else(|| unresolved(nested, payload.component()))
    }

    fn not_in(&self, instances: &[StableId], source: StableId) -> ExpansionError {
        ExpansionError::NotInExpansion {
            id: ExpandedNodeId::virtual_node(instances.to_vec(), source),
        }
    }

    fn owned_fields(&self, id: StableId) -> Result<NodeFields, ExpansionError> {
        if !self.is_scene_descendant(id) {
            return Err(ExpansionError::NotInExpansion {
                id: ExpandedNodeId::Owned(id),
            });
        }
        self.scene
            .fields(id)
            .ok_or(ExpansionError::NodeNotFound { id })
    }

    /// node 내용을 읽습니다. 정의 node는 참조를 그대로 돌려줍니다(복제 없음).
    pub fn node(&self, id: &ExpandedNodeId) -> Result<ExpandedNode<'a>, ExpansionError> {
        match id {
            ExpandedNodeId::Virtual { instances, source } => {
                let entry = self.resolve_chain(instances)?;
                let definition = entry
                    .tree()
                    .node(*source)
                    .ok_or_else(|| self.not_in(instances, *source))?;
                Ok(ExpandedNode {
                    id: id.clone(),
                    source: NodeSource::Definition(definition),
                })
            }
            ExpandedNodeId::Owned(node) => {
                let fields = self.owned_fields(*node)?;
                Ok(ExpandedNode {
                    id: id.clone(),
                    source: NodeSource::Owned {
                        fields,
                        payload: self.scene.payload(*node),
                    },
                })
            }
        }
    }

    /// slot에 투영된 content id 목록. chain이 scene 인스턴스 하나면 소유 scene node, nested
    /// 인스턴스면 바깥 정의의 node입니다.
    fn projected(instances: &[StableId], content: &[StableId]) -> Vec<ExpandedNodeId> {
        match instances.split_last() {
            Some((_, [])) | None => content.iter().map(|c| ExpandedNodeId::Owned(*c)).collect(),
            Some((_, outer)) => content
                .iter()
                .map(|c| ExpandedNodeId::virtual_node(outer.to_vec(), *c))
                .collect(),
        }
    }

    /// 자식 목록(시각 순서).
    ///
    /// - nested ComponentInstance의 자식은 그 정의 root의 가상 투영 하나입니다.
    /// - slot host는 인스턴스가 slot을 override했으면 투영된 content, 아니면 default content
    ///   (정의 tree 자식)입니다.
    /// - 재귀 정의와 중첩 상한 초과는 거부합니다.
    pub fn children(&self, id: &ExpandedNodeId) -> Result<Vec<ExpandedNodeId>, ExpansionError> {
        match id {
            ExpandedNodeId::Virtual { instances, source } => {
                let entry = self.resolve_chain(instances)?;
                let node = entry
                    .tree()
                    .node(*source)
                    .ok_or_else(|| self.not_in(instances, *source))?;
                if node.kind == NodeKind::ComponentInstance {
                    let nested = self.nested_entry(entry, *source)?;
                    self.check_nesting(instances, nested)?;
                    let mut chain = instances.clone();
                    chain.push(*source);
                    return Ok(vec![ExpandedNodeId::virtual_node(
                        chain,
                        nested.definition().root.id(),
                    )]);
                }
                if let Some(slot) = entry.slot_for_host(*source) {
                    let payload = self.chain_payload(instances)?;
                    if let Some(content) = payload.slot_override(slot.id) {
                        return Ok(Self::projected(instances, content.content()));
                    }
                }
                Ok(entry
                    .tree()
                    .children(*source)
                    .iter()
                    .map(|child| ExpandedNodeId::virtual_node(instances.clone(), *child))
                    .collect())
            }
            ExpandedNodeId::Owned(node) => {
                let fields = self.owned_fields(*node)?;
                if fields.kind == NodeKind::ComponentInstance {
                    let entry = self.scene_entry(*node)?;
                    return Ok(vec![ExpandedNodeId::virtual_node(
                        vec![*node],
                        entry.definition().root.id(),
                    )]);
                }
                Ok(self
                    .scene
                    .children(*node)
                    .iter()
                    .map(|child| ExpandedNodeId::Owned(*child))
                    .collect())
            }
        }
    }

    /// nested 인스턴스로 내려가기 전 재귀·깊이 검사.
    fn check_nesting(
        &self,
        instances: &[StableId],
        nested: &ComponentEntry,
    ) -> Result<(), ExpansionError> {
        let depth = instances.len() + 1;
        if depth > MAX_INSTANCE_NESTING {
            return Err(ExpansionError::NestingTooDeep { depth });
        }
        let nested_id = nested.definition().id;
        for prefix in 1..=instances.len() {
            if self.resolve_chain(&instances[..prefix])?.definition().id == nested_id {
                return Err(ExpansionError::RecursiveComponent {
                    component: nested_id,
                });
            }
        }
        Ok(())
    }

    /// `content`가 들어 있는 slot의 host(인스턴스 payload 기준). 없으면 투영되지 않은 node.
    fn projection_host(
        entry: &ComponentEntry,
        payload: &ComponentInstancePayload,
        content: StableId,
    ) -> Result<StableId, ExpansionError> {
        payload
            .slot_overrides()
            .find(|(_, over)| over.position(content).is_some())
            .and_then(|(slot, _)| entry.slot(slot))
            .map(|slot| slot.host.id())
            .ok_or(ExpansionError::NotProjected { node: content })
    }

    /// 부모 id. 가상 root의 부모는 scene 인스턴스 자신이라 `None`입니다.
    ///
    /// slot에 투영된 node의 부모는 (scene·정의 tree의 실제 부모가 아니라) 투영 위치인 slot
    /// host입니다.
    pub fn parent(&self, id: &ExpandedNodeId) -> Result<Option<ExpandedNodeId>, ExpansionError> {
        match id {
            ExpandedNodeId::Virtual { instances, source } => {
                let entry = self.resolve_chain(instances)?;
                let node = entry
                    .tree()
                    .node(*source)
                    .ok_or_else(|| self.not_in(instances, *source))?;
                match node.parent {
                    Some(parent) => {
                        let parent_node = entry
                            .tree()
                            .node(parent)
                            .ok_or_else(|| self.not_in(instances, parent))?;
                        if parent_node.kind == NodeKind::ComponentInstance {
                            // nested 인스턴스의 slot content → 그 정의의 slot host에 투영.
                            let payload = self.nested_payload(entry, parent)?;
                            let nested = self.nested_entry(entry, parent)?;
                            let host = Self::projection_host(nested, &payload, *source)?;
                            let mut chain = instances.clone();
                            chain.push(parent);
                            return Ok(Some(ExpandedNodeId::virtual_node(chain, host)));
                        }
                        Ok(Some(ExpandedNodeId::virtual_node(
                            instances.clone(),
                            parent,
                        )))
                    }
                    None => match instances.split_last() {
                        Some((nested, outer)) if !outer.is_empty() => {
                            Ok(Some(ExpandedNodeId::virtual_node(outer.to_vec(), *nested)))
                        }
                        Some((only, _)) if *only != self.instance => {
                            Ok(Some(ExpandedNodeId::Owned(*only)))
                        }
                        _ => Ok(None),
                    },
                }
            }
            ExpandedNodeId::Owned(node) => {
                self.owned_fields(*node)?;
                let parent = self
                    .scene
                    .parent(*node)
                    .ok_or(ExpansionError::NodeNotFound { id: *node })?;
                let parent_kind = self
                    .scene
                    .fields(parent)
                    .map(|f| f.kind)
                    .ok_or(ExpansionError::NodeNotFound { id: parent })?;
                if parent_kind == NodeKind::ComponentInstance {
                    let entry = self.scene_entry(parent)?;
                    let payload = self.scene_payload(parent)?;
                    let host = Self::projection_host(entry, &payload, *node)?;
                    return Ok(Some(ExpandedNodeId::virtual_node(vec![parent], host)));
                }
                Ok(Some(ExpandedNodeId::Owned(parent)))
            }
        }
    }

    /// 가상 subtree 전체를 root부터 전위 순회한 id 목록.
    pub fn walk(&self) -> Result<Vec<ExpandedNodeId>, ExpansionError> {
        let mut out = Vec::new();
        let mut stack = vec![self.root()];
        while let Some(id) = stack.pop() {
            let children = self.children(&id)?;
            out.push(id);
            stack.extend(children.into_iter().rev());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::definition::{ComponentDefinition, ComponentPropertyId, DefinitionTree};
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::reference::NodeRef;

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn key(byte: u8) -> OrderKey {
        OrderKey::from_bytes(vec![byte]).unwrap()
    }

    fn def_node(
        id: u64,
        kind: NodeKind,
        parent: Option<u64>,
        order: u8,
        payload: Vec<u8>,
    ) -> DefinitionNode {
        DefinitionNode {
            id: stable(id),
            kind,
            parent: parent.map(stable),
            order_key: key(order),
            visible: true,
            locked: false,
            display_name: format!("def-{id}"),
            payload,
        }
    }

    fn instance_bytes(component: u64) -> Vec<u8> {
        ComponentInstancePayload::new(ComponentRef::new(stable(component))).encode()
    }

    /// B(2): root 200 ← 201(Text).
    /// A(1): root 100 ← 102(Shape, key 0x20), 101(ComponentInstance of B, key 0x40) ← 103(Text).
    fn library() -> ComponentLibrary {
        let mut library = ComponentLibrary::new();
        let mut tree_b = DefinitionTree::new(ComponentRef::new(stable(2)));
        tree_b
            .insert(def_node(200, NodeKind::Frame, None, 0x80, vec![]))
            .unwrap();
        tree_b
            .insert(def_node(201, NodeKind::Text, Some(200), 0x40, vec![]))
            .unwrap();
        library
            .insert(
                ComponentDefinition::new(stable(2), "B".into(), NodeRef::new(stable(200))),
                tree_b,
            )
            .unwrap();

        let mut tree_a = DefinitionTree::new(ComponentRef::new(stable(1)));
        tree_a
            .insert(def_node(100, NodeKind::Frame, None, 0x80, vec![1]))
            .unwrap();
        tree_a
            .insert(def_node(
                101,
                NodeKind::ComponentInstance,
                Some(100),
                0x40,
                instance_bytes(2),
            ))
            .unwrap();
        tree_a
            .insert(def_node(102, NodeKind::Shape, Some(100), 0x20, vec![2]))
            .unwrap();
        tree_a
            .insert(def_node(103, NodeKind::Text, Some(100), 0x60, vec![3]))
            .unwrap();
        library
            .insert(
                ComponentDefinition::new(stable(1), "A".into(), NodeRef::new(stable(100))),
                tree_a,
            )
            .unwrap();
        library
    }

    fn scene_fields(id: u64, kind: NodeKind, parent: Option<RuntimeEntityId>) -> NodeFields {
        NodeFields {
            stable_id: stable(id),
            kind,
            parent,
            order_key: OrderKey::between(None, None),
            visible: true,
            locked: false,
            display_name: String::new(),
        }
    }

    /// scene: root 1000 ← instance 1001(A).
    fn scene(payload: ComponentInstancePayload) -> (NodeColumns, ScenePayloads) {
        let mut columns = NodeColumns::new();
        let root = columns.insert(scene_fields(1000, NodeKind::Frame, None));
        columns.insert(scene_fields(1001, NodeKind::ComponentInstance, Some(root)));
        let mut payloads = ScenePayloads::new();
        payloads.insert(stable(1001), payload.encode());
        (columns, payloads)
    }

    fn sources(expansion: &InstanceExpansion<'_>) -> Vec<(usize, StableId)> {
        expansion
            .walk()
            .unwrap()
            .into_iter()
            .map(|id| match id {
                ExpandedNodeId::Virtual { instances, source } => (instances.len(), source),
                // IX.3.2에서 추가된 variant: chain 길이 0으로 표시(이 fixture에는 slot이 없음).
                ExpandedNodeId::Owned(node) => (0, node),
            })
            .collect()
    }

    #[test]
    fn virtual_subtree_mirrors_definition_and_nested_instance() {
        let library = library();
        let (columns, payloads) =
            scene(ComponentInstancePayload::new(ComponentRef::new(stable(1))));
        let index = SceneIndex::build(&columns, &payloads);
        let expansion = InstanceExpansion::new(&library, &index, stable(1001)).unwrap();
        // 전위 순회: A root → 102(0x20) → 101(0x40, nested B) → B root → 201 → 103(0x60).
        assert_eq!(
            sources(&expansion),
            vec![
                (1, stable(100)),
                (1, stable(102)),
                (1, stable(101)),
                (2, stable(200)),
                (2, stable(201)),
                (1, stable(103)),
            ]
        );
        // node 내용은 정의 node 그대로.
        let root = expansion.node(&expansion.root()).unwrap();
        assert_eq!(root.kind(), NodeKind::Frame);
        assert_eq!(root.payload(), Some(&[1u8][..]));
        assert!(std::ptr::eq(
            root.definition_node().unwrap(),
            library
                .get(ComponentRef::new(stable(1)))
                .unwrap()
                .tree()
                .node(stable(100))
                .unwrap()
        ));
        // scene과 library는 변하지 않음(물리 복제 없음).
        assert_eq!(columns.len(), 2);
        assert_eq!(
            library
                .iter()
                .map(|entry| entry.tree().len())
                .sum::<usize>(),
            6
        );
    }

    #[test]
    fn parent_links_cross_instance_boundaries() {
        let library = library();
        let (columns, payloads) =
            scene(ComponentInstancePayload::new(ComponentRef::new(stable(1))));
        let index = SceneIndex::build(&columns, &payloads);
        let expansion = InstanceExpansion::new(&library, &index, stable(1001)).unwrap();
        let b_text = ExpandedNodeId::virtual_node(vec![stable(1001), stable(101)], stable(201));
        let b_root = ExpandedNodeId::virtual_node(vec![stable(1001), stable(101)], stable(200));
        let nested = ExpandedNodeId::virtual_node(vec![stable(1001)], stable(101));
        assert_eq!(expansion.parent(&b_text).unwrap(), Some(b_root.clone()));
        assert_eq!(expansion.parent(&b_root).unwrap(), Some(nested.clone()));
        assert_eq!(expansion.parent(&nested).unwrap(), Some(expansion.root()));
        assert_eq!(expansion.parent(&expansion.root()).unwrap(), None);
        // 다른 인스턴스 chain의 id는 거부.
        let foreign = ExpandedNodeId::virtual_node(vec![stable(9_999)], stable(100));
        assert!(matches!(
            expansion.node(&foreign),
            Err(ExpansionError::NotInExpansion { .. })
        ));
        let missing = ExpandedNodeId::virtual_node(vec![stable(1001)], stable(201));
        assert!(matches!(
            expansion.node(&missing),
            Err(ExpansionError::NotInExpansion { .. })
        ));
    }

    #[test]
    fn property_overrides_are_composed_from_instance() {
        let library = library();
        let mut payload = ComponentInstancePayload::new(ComponentRef::new(stable(1)));
        let key = OverrideKey::direct(ComponentPropertyId::new(stable(500)));
        payload.set_property_override(key.clone(), TypedValue::String("확인".into()));
        let (columns, payloads) = scene(payload);
        let index = SceneIndex::build(&columns, &payloads);
        let expansion = InstanceExpansion::new(&library, &index, stable(1001)).unwrap();
        assert_eq!(
            expansion.property_override(&key),
            Some(&TypedValue::String("확인".into()))
        );
        let other = OverrideKey::direct(ComponentPropertyId::new(stable(501)));
        assert_eq!(expansion.property_override(&other), None);
    }

    #[test]
    fn invalid_instances_are_rejected() {
        let library = library();
        // 정의가 없는 컴포넌트 → 미해결 참조(위치: instance의 COMPONENT_REF).
        let (columns, payloads) =
            scene(ComponentInstancePayload::new(ComponentRef::new(stable(77))));
        let index = SceneIndex::build(&columns, &payloads);
        match InstanceExpansion::new(&library, &index, stable(1001)).unwrap_err() {
            ExpansionError::UnresolvedComponent(unresolved) => {
                assert_eq!(unresolved.original_id(), stable(77));
                assert_eq!(unresolved.location().root(), stable(1001));
                assert_eq!(unresolved.location().keys(), &[keys::COMPONENT_REF]);
            }
            other => panic!("예상과 다른 오류: {other:?}"),
        }
        // ComponentInstance가 아닌 node.
        assert!(matches!(
            InstanceExpansion::new(&library, &index, stable(1000)),
            Err(ExpansionError::NotAnInstance { .. })
        ));
        // 없는 node.
        assert!(matches!(
            InstanceExpansion::new(&library, &index, stable(5)),
            Err(ExpansionError::NodeNotFound { .. })
        ));
        // payload 손상.
        let mut broken = payloads.clone();
        broken.insert(stable(1001), vec![1, 2, 3]);
        let index = SceneIndex::build(&columns, &broken);
        assert!(matches!(
            InstanceExpansion::new(&library, &index, stable(1001)),
            Err(ExpansionError::Payload { .. })
        ));
    }

    #[test]
    fn recursive_component_is_rejected() {
        // C(3): root 300 ← 301(instance of C) — 자기 자신을 중첩.
        let mut library = ComponentLibrary::new();
        let mut tree = DefinitionTree::new(ComponentRef::new(stable(3)));
        tree.insert(def_node(300, NodeKind::Frame, None, 0x80, vec![]))
            .unwrap();
        tree.insert(def_node(
            301,
            NodeKind::ComponentInstance,
            Some(300),
            0x40,
            instance_bytes(3),
        ))
        .unwrap();
        library
            .insert(
                ComponentDefinition::new(stable(3), "C".into(), NodeRef::new(stable(300))),
                tree,
            )
            .unwrap();
        let (columns, payloads) =
            scene(ComponentInstancePayload::new(ComponentRef::new(stable(3))));
        let index = SceneIndex::build(&columns, &payloads);
        let expansion = InstanceExpansion::new(&library, &index, stable(1001)).unwrap();
        assert_eq!(
            expansion.walk().unwrap_err(),
            ExpansionError::RecursiveComponent {
                component: stable(3)
            }
        );
    }
}

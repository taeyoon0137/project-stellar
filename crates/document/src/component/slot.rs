//! ComponentSlot (IX.3.1).
//!
//! Slot은 컴포넌트 정의가 **명시적으로 공개한** 콘텐츠 삽입 위치입니다(기술 아키텍처 15.2).
//! 공개되지 않은 위치에는 인스턴스가 자식을 추가할 수 없습니다.
//!
//! - host는 정의 tree 안의 Frame입니다. 정의 밖 node(다른 컴포넌트·scene node)를 가리키면
//!   거부합니다.
//! - host는 slot 전용 컨테이너입니다: host의 정의 tree 자식이 곧 default content이며,
//!   `default_content`는 그 자식들을 시각 순서대로 정확히 나열해야 합니다. 인스턴스가 slot을
//!   override하면 default content 전체가 인스턴스 소유 node로 대체됩니다(IX.3.2).
//! - allowed node kinds와 cardinality(최소·최대 개수)는 default content와 override 양쪽에
//!   적용됩니다.
//! - 한 host에는 slot이 하나뿐입니다(투영 위치 모호성 방지).

use std::collections::BTreeSet;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::{ComponentRef, NodeRef};

use super::codec::{
    capacity_hint, put_count, put_id, put_kind, put_u32, put_u8, Malformed, Reader,
};
use super::definition::{ComponentLibrary, ComponentSlotId, DefinitionTree};
use crate::node_kind::NodeKind;

/// Slot 정의·검증 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotError {
    /// host가 정의 tree 밖 node입니다.
    HostOutsideDefinition {
        /// slot.
        slot: ComponentSlotId,
        /// host로 지정된 node.
        host: StableId,
    },
    /// host가 Frame이 아닙니다.
    HostNotFrame {
        /// slot.
        slot: ComponentSlotId,
        /// host node.
        host: StableId,
        /// 실제 종류.
        kind: NodeKind,
    },
    /// 허용 node 종류가 비어 있습니다.
    EmptyAllowedKinds {
        /// slot.
        slot: ComponentSlotId,
    },
    /// 최소 개수가 최대 개수보다 큽니다.
    InvalidCardinality {
        /// 최소.
        min: u32,
        /// 최대.
        max: u32,
    },
    /// default content가 host의 자식이 아닙니다(정의 밖 포함).
    DefaultContentOutsideHost {
        /// slot.
        slot: ComponentSlotId,
        /// node.
        node: StableId,
    },
    /// host의 자식이 default content에 선언되지 않았습니다.
    UndeclaredHostChild {
        /// slot.
        slot: ComponentSlotId,
        /// 선언되지 않은 자식.
        node: StableId,
    },
    /// default content 순서가 host 자식 순서와 다릅니다.
    DefaultContentOrder {
        /// slot.
        slot: ComponentSlotId,
    },
    /// 허용되지 않은 node 종류입니다.
    DisallowedKind {
        /// slot.
        slot: ComponentSlotId,
        /// node.
        node: StableId,
        /// 종류.
        kind: NodeKind,
    },
    /// content가 중복됩니다.
    DuplicateContent {
        /// slot.
        slot: ComponentSlotId,
        /// node.
        node: StableId,
    },
    /// content 개수가 cardinality를 벗어납니다.
    CardinalityViolation {
        /// slot.
        slot: ComponentSlotId,
        /// 실제 개수.
        count: usize,
    },
    /// 같은 host에 slot이 둘 이상입니다.
    DuplicateSlotHost {
        /// host node.
        host: StableId,
    },
    /// 정의 Slot 목록에 선언되지 않은 slot입니다.
    SlotNotDeclared {
        /// slot.
        slot: ComponentSlotId,
    },
    /// 선언된 slot에 record가 없습니다.
    MissingSlotRecord {
        /// slot.
        slot: ComponentSlotId,
    },
    /// slot record가 이미 있습니다.
    DuplicateSlot {
        /// slot.
        slot: ComponentSlotId,
    },
    /// 컴포넌트가 library에 없습니다.
    ComponentNotFound {
        /// 컴포넌트.
        component: StableId,
    },
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for SlotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SlotError::HostOutsideDefinition { slot, host } => write!(
                f,
                "slot {}의 host {host}가 정의 밖 node입니다",
                slot.stable_id()
            ),
            SlotError::HostNotFrame { slot, host, kind } => write!(
                f,
                "slot {}의 host {host}는 Frame이어야 합니다({})",
                slot.stable_id(),
                kind.name()
            ),
            SlotError::EmptyAllowedKinds { slot } => {
                write!(f, "slot {}의 허용 종류가 비어 있습니다", slot.stable_id())
            }
            SlotError::InvalidCardinality { min, max } => {
                write!(f, "slot 최소 개수 {min}가 최대 {max}보다 큽니다")
            }
            SlotError::DefaultContentOutsideHost { slot, node } => write!(
                f,
                "slot {}의 default content {node}가 host의 자식이 아닙니다",
                slot.stable_id()
            ),
            SlotError::UndeclaredHostChild { slot, node } => write!(
                f,
                "slot {} host의 자식 {node}가 default content에 없습니다",
                slot.stable_id()
            ),
            SlotError::DefaultContentOrder { slot } => write!(
                f,
                "slot {}의 default content 순서가 host 자식 순서와 다릅니다",
                slot.stable_id()
            ),
            SlotError::DisallowedKind { slot, node, kind } => write!(
                f,
                "slot {}에 {} node {node}를 넣을 수 없습니다",
                slot.stable_id(),
                kind.name()
            ),
            SlotError::DuplicateContent { slot, node } => {
                write!(f, "slot {}에 node {node}가 중복됩니다", slot.stable_id())
            }
            SlotError::CardinalityViolation { slot, count } => write!(
                f,
                "slot {}의 content {count}개가 허용 개수를 벗어납니다",
                slot.stable_id()
            ),
            SlotError::DuplicateSlotHost { host } => {
                write!(f, "host {host}에 slot이 둘 이상입니다")
            }
            SlotError::SlotNotDeclared { slot } => {
                write!(f, "slot {}은 정의에 선언되지 않았습니다", slot.stable_id())
            }
            SlotError::MissingSlotRecord { slot } => {
                write!(f, "선언된 slot {}의 정의가 없습니다", slot.stable_id())
            }
            SlotError::DuplicateSlot { slot } => {
                write!(f, "slot {}이 이미 정의되어 있습니다", slot.stable_id())
            }
            SlotError::ComponentNotFound { component } => {
                write!(f, "컴포넌트 {component}가 library에 없습니다")
            }
            SlotError::Malformed { reason } => write!(f, "slot binary 형식 위반: {reason}"),
        }
    }
}

impl std::error::Error for SlotError {}

impl From<Malformed> for SlotError {
    fn from(err: Malformed) -> SlotError {
        SlotError::Malformed { reason: err.0 }
    }
}

/// slot content 개수 범위. `max`가 `None`이면 상한 없음.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotCardinality {
    min: u32,
    max: Option<u32>,
}

impl SlotCardinality {
    /// 범위를 만듭니다. `min > max`이면 거부합니다.
    pub fn new(min: u32, max: Option<u32>) -> Result<SlotCardinality, SlotError> {
        if let Some(max) = max {
            if min > max {
                return Err(SlotError::InvalidCardinality { min, max });
            }
        }
        Ok(SlotCardinality { min, max })
    }

    /// 0개 이상, 상한 없음.
    pub const fn unbounded() -> SlotCardinality {
        SlotCardinality { min: 0, max: None }
    }

    /// 0개 이상 `max`개 이하.
    pub const fn at_most(max: u32) -> SlotCardinality {
        SlotCardinality {
            min: 0,
            max: Some(max),
        }
    }

    /// 최소 개수.
    pub const fn min(&self) -> u32 {
        self.min
    }

    /// 최대 개수(없으면 상한 없음).
    pub const fn max(&self) -> Option<u32> {
        self.max
    }

    /// 개수가 범위 안인지.
    pub fn allows(&self, count: usize) -> bool {
        count >= self.min as usize && self.max.is_none_or(|max| count <= max as usize)
    }

    /// 한 개 더 넣으면 상한을 넘는지.
    pub fn is_full(&self, count: usize) -> bool {
        self.max.is_some_and(|max| count >= max as usize)
    }
}

/// 컴포넌트 정의의 Slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentSlot {
    /// 고유 id.
    pub id: ComponentSlotId,
    /// slot 전용 host Frame(정의 tree 안).
    pub host: NodeRef,
    allowed_kinds: BTreeSet<NodeKind>,
    /// content 개수 범위.
    pub cardinality: SlotCardinality,
    default_content: Vec<NodeRef>,
}

impl ComponentSlot {
    /// slot을 만듭니다. 정의 tree에 대한 검증은 [`ComponentSlot::validate`]가 합니다.
    pub fn new(
        id: ComponentSlotId,
        host: NodeRef,
        allowed_kinds: impl IntoIterator<Item = NodeKind>,
        cardinality: SlotCardinality,
        default_content: Vec<NodeRef>,
    ) -> ComponentSlot {
        ComponentSlot {
            id,
            host,
            allowed_kinds: allowed_kinds.into_iter().collect(),
            cardinality,
            default_content,
        }
    }

    /// 허용 node 종류(숫자 값 오름차순).
    pub fn allowed_kinds(&self) -> impl Iterator<Item = NodeKind> + '_ {
        self.allowed_kinds.iter().copied()
    }

    /// 종류가 허용되는지.
    pub fn allows_kind(&self, kind: NodeKind) -> bool {
        self.allowed_kinds.contains(&kind)
    }

    /// default content(시각 순서).
    pub fn default_content(&self) -> &[NodeRef] {
        &self.default_content
    }

    /// content 목록(종류 포함)이 종류·중복·개수 규칙을 지키는지 검사합니다.
    ///
    /// default content와 인스턴스 override(IX.3.2)가 같은 규칙을 씁니다.
    pub fn check_content(
        &self,
        content: impl IntoIterator<Item = (StableId, NodeKind)>,
    ) -> Result<(), SlotError> {
        let mut seen = BTreeSet::new();
        for (node, kind) in content {
            if !seen.insert(node) {
                return Err(SlotError::DuplicateContent {
                    slot: self.id,
                    node,
                });
            }
            if !self.allows_kind(kind) {
                return Err(SlotError::DisallowedKind {
                    slot: self.id,
                    node,
                    kind,
                });
            }
        }
        if !self.cardinality.allows(seen.len()) {
            return Err(SlotError::CardinalityViolation {
                slot: self.id,
                count: seen.len(),
            });
        }
        Ok(())
    }

    /// slot을 소유 컴포넌트의 정의 tree에 대해 검증합니다.
    ///
    /// host가 정의 밖이면 `HostOutsideDefinition`, Frame이 아니면 `HostNotFrame`입니다.
    /// default content는 host 자식 전체를 시각 순서대로 나열해야 하고 종류·개수 규칙을
    /// 지켜야 합니다.
    pub fn validate(&self, tree: &DefinitionTree) -> Result<(), SlotError> {
        let host_id = self.host.id();
        let host = tree.node(host_id).ok_or(SlotError::HostOutsideDefinition {
            slot: self.id,
            host: host_id,
        })?;
        if host.kind != NodeKind::Frame {
            return Err(SlotError::HostNotFrame {
                slot: self.id,
                host: host_id,
                kind: host.kind,
            });
        }
        if self.allowed_kinds.is_empty() {
            return Err(SlotError::EmptyAllowedKinds { slot: self.id });
        }
        let children = tree.children(host_id);
        let mut content = Vec::with_capacity(self.default_content.len());
        for node in &self.default_content {
            let definition = tree
                .node(node.id())
                .filter(|n| n.parent == Some(host_id))
                .ok_or(SlotError::DefaultContentOutsideHost {
                    slot: self.id,
                    node: node.id(),
                })?;
            content.push((node.id(), definition.kind));
        }
        self.check_content(content)?;
        if let Some(child) = children
            .iter()
            .find(|child| !self.default_content.contains(&NodeRef::new(**child)))
        {
            return Err(SlotError::UndeclaredHostChild {
                slot: self.id,
                node: *child,
            });
        }
        if !self
            .default_content
            .iter()
            .map(NodeRef::id)
            .eq(children.iter().copied())
        {
            return Err(SlotError::DefaultContentOrder { slot: self.id });
        }
        Ok(())
    }

    pub(super) fn encode_into(&self, buf: &mut Vec<u8>) {
        put_id(buf, self.id.stable_id());
        put_id(buf, self.host.id());
        put_count(buf, self.allowed_kinds.len());
        for kind in &self.allowed_kinds {
            put_kind(buf, *kind);
        }
        put_u32(buf, self.cardinality.min);
        match self.cardinality.max {
            Some(max) => {
                put_u8(buf, 1);
                put_u32(buf, max);
            }
            None => put_u8(buf, 0),
        }
        put_count(buf, self.default_content.len());
        for node in &self.default_content {
            put_id(buf, node.id());
        }
    }

    pub(super) fn decode_from(reader: &mut Reader<'_>) -> Result<ComponentSlot, SlotError> {
        let id = ComponentSlotId::new(reader.id()?);
        let host = NodeRef::new(reader.id()?);
        let kind_count = reader.count()?;
        let mut allowed_kinds = BTreeSet::new();
        let mut previous: Option<NodeKind> = None;
        for _ in 0..kind_count {
            let kind = reader.kind()?;
            if previous.is_some_and(|p| p >= kind) {
                return Err(SlotError::Malformed {
                    reason: "허용 종류가 오름차순이 아님",
                });
            }
            previous = Some(kind);
            allowed_kinds.insert(kind);
        }
        let min = reader.u32()?;
        let max = match reader.u8()? {
            0 => None,
            1 => Some(reader.u32()?),
            _ => {
                return Err(SlotError::Malformed {
                    reason: "cardinality max flag",
                })
            }
        };
        let cardinality = SlotCardinality::new(min, max)?;
        let content_count = reader.count()?;
        let mut default_content = Vec::with_capacity(capacity_hint(content_count));
        for _ in 0..content_count {
            default_content.push(NodeRef::new(reader.id()?));
        }
        Ok(ComponentSlot {
            id,
            host,
            allowed_kinds,
            cardinality,
            default_content,
        })
    }

    /// canonical binary bytes로 인코딩합니다.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode_into(&mut buf);
        buf
    }

    /// binary bytes에서 디코딩합니다(정의 tree 검증은 별도).
    pub fn decode(bytes: &[u8]) -> Result<ComponentSlot, SlotError> {
        let mut reader = Reader::new(bytes);
        let slot = ComponentSlot::decode_from(&mut reader)?;
        reader.expect_end()?;
        Ok(slot)
    }
}

impl ComponentLibrary {
    /// 컴포넌트에 slot record를 추가합니다.
    ///
    /// slot은 정의 Slot 목록에 선언돼 있어야 하고, 정의 tree에 대해 유효해야 하며, 다른
    /// slot과 host를 공유할 수 없습니다. 실패 시 library는 변경되지 않습니다.
    pub fn add_slot(
        &mut self,
        component: ComponentRef,
        slot: ComponentSlot,
    ) -> Result<(), SlotError> {
        let entry = self
            .entry_mut(component)
            .ok_or(SlotError::ComponentNotFound {
                component: component.id(),
            })?;
        if !entry.definition().slots().contains(&slot.id) {
            return Err(SlotError::SlotNotDeclared { slot: slot.id });
        }
        if entry.slot(slot.id).is_some() {
            return Err(SlotError::DuplicateSlot { slot: slot.id });
        }
        slot.validate(entry.tree())?;
        if entry.slot_for_host(slot.host.id()).is_some() {
            return Err(SlotError::DuplicateSlotHost {
                host: slot.host.id(),
            });
        }
        entry.insert_slot(slot);
        Ok(())
    }

    /// 컴포넌트의 선언 Slot이 모두 record를 갖는지 검사합니다.
    pub fn validate_slots(&self, component: ComponentRef) -> Result<(), SlotError> {
        let entry = self.get(component).ok_or(SlotError::ComponentNotFound {
            component: component.id(),
        })?;
        for slot in entry.definition().slots() {
            if entry.slot(*slot).is_none() {
                return Err(SlotError::MissingSlotRecord { slot: *slot });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::definition::{ComponentDefinition, DefinitionNode};
    use stellar_core_types::order_key::OrderKey;

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn slot_id(ts: u64) -> ComponentSlotId {
        ComponentSlotId::new(stable(ts))
    }

    fn node(id: u64, kind: NodeKind, parent: Option<u64>, key: u8) -> DefinitionNode {
        DefinitionNode {
            id: stable(id),
            kind,
            parent: parent.map(stable),
            order_key: OrderKey::from_bytes(vec![key]).unwrap(),
            visible: true,
            locked: false,
            display_name: String::new(),
            payload: Vec::new(),
        }
    }

    /// Card(1): 100(Frame) ← 101(Text 제목), 102(Frame body host) ← 103(Text), 104(Image).
    /// Other(2): 200(Frame) ← 201(Frame).
    fn library() -> ComponentLibrary {
        let mut library = ComponentLibrary::new();
        let mut tree = DefinitionTree::new(ComponentRef::new(stable(1)));
        tree.insert(node(100, NodeKind::Frame, None, 0x80)).unwrap();
        tree.insert(node(101, NodeKind::Text, Some(100), 0x20))
            .unwrap();
        tree.insert(node(102, NodeKind::Frame, Some(100), 0x40))
            .unwrap();
        tree.insert(node(103, NodeKind::Text, Some(102), 0x20))
            .unwrap();
        tree.insert(node(104, NodeKind::Image, Some(102), 0x40))
            .unwrap();
        let mut definition =
            ComponentDefinition::new(stable(1), "Card".into(), NodeRef::new(stable(100)));
        definition.add_slot(slot_id(600)).unwrap();
        definition.add_slot(slot_id(601)).unwrap();
        library.insert(definition, tree).unwrap();

        let mut other = DefinitionTree::new(ComponentRef::new(stable(2)));
        other
            .insert(node(200, NodeKind::Frame, None, 0x80))
            .unwrap();
        other
            .insert(node(201, NodeKind::Frame, Some(200), 0x40))
            .unwrap();
        library
            .insert(
                ComponentDefinition::new(stable(2), "Other".into(), NodeRef::new(stable(200))),
                other,
            )
            .unwrap();
        library
    }

    fn body_slot(host: u64, content: &[u64]) -> ComponentSlot {
        ComponentSlot::new(
            slot_id(600),
            NodeRef::new(stable(host)),
            [NodeKind::Text, NodeKind::Image, NodeKind::Frame],
            SlotCardinality::new(1, Some(3)).unwrap(),
            content.iter().map(|c| NodeRef::new(stable(*c))).collect(),
        )
    }

    fn card() -> ComponentRef {
        ComponentRef::new(stable(1))
    }

    #[test]
    fn host_outside_definition_is_rejected() {
        // 검증 기준(IX.3.1): host가 definition 밖이면 거부.
        let mut library = library();
        // 다른 컴포넌트(Other)의 Frame을 host로 지정.
        let foreign = body_slot(201, &[]);
        assert_eq!(
            library.add_slot(card(), foreign).unwrap_err(),
            SlotError::HostOutsideDefinition {
                slot: slot_id(600),
                host: stable(201),
            }
        );
        // 어떤 정의에도 없는(scene) node를 host로 지정.
        assert!(matches!(
            library.add_slot(card(), body_slot(9_999, &[])),
            Err(SlotError::HostOutsideDefinition { .. })
        ));
        // 거부 후 library 불변.
        assert!(library.get(card()).unwrap().slot(slot_id(600)).is_none());

        // 정의 안 host는 통과.
        library
            .add_slot(card(), body_slot(102, &[103, 104]))
            .unwrap();
        let entry = library.get(card()).unwrap();
        assert_eq!(
            entry.slot(slot_id(600)).unwrap().host,
            NodeRef::new(stable(102))
        );
        assert_eq!(
            entry.slot_for_host(stable(102)).map(|s| s.id),
            Some(slot_id(600))
        );
    }

    #[test]
    fn host_and_default_content_rules() {
        let library = library();
        let tree = library.get(card()).unwrap().tree();
        // Frame이 아닌 host.
        assert!(matches!(
            body_slot(101, &[]).validate(tree),
            Err(SlotError::HostNotFrame { .. })
        ));
        // host 자식이 아닌 default content.
        assert_eq!(
            body_slot(102, &[103, 101]).validate(tree).unwrap_err(),
            SlotError::DefaultContentOutsideHost {
                slot: slot_id(600),
                node: stable(101),
            }
        );
        // 선언되지 않은 host 자식.
        assert_eq!(
            body_slot(102, &[103]).validate(tree).unwrap_err(),
            SlotError::UndeclaredHostChild {
                slot: slot_id(600),
                node: stable(104),
            }
        );
        // 순서 불일치.
        assert_eq!(
            body_slot(102, &[104, 103]).validate(tree).unwrap_err(),
            SlotError::DefaultContentOrder { slot: slot_id(600) }
        );
        // 허용되지 않은 종류.
        let text_only = ComponentSlot::new(
            slot_id(600),
            NodeRef::new(stable(102)),
            [NodeKind::Text],
            SlotCardinality::unbounded(),
            vec![NodeRef::new(stable(103)), NodeRef::new(stable(104))],
        );
        assert!(matches!(
            text_only.validate(tree),
            Err(SlotError::DisallowedKind {
                kind: NodeKind::Image,
                ..
            })
        ));
        // cardinality 위반(최대 1개인데 default 2개).
        let single = ComponentSlot::new(
            slot_id(600),
            NodeRef::new(stable(102)),
            [NodeKind::Text, NodeKind::Image],
            SlotCardinality::at_most(1),
            vec![NodeRef::new(stable(103)), NodeRef::new(stable(104))],
        );
        assert_eq!(
            single.validate(tree).unwrap_err(),
            SlotError::CardinalityViolation {
                slot: slot_id(600),
                count: 2,
            }
        );
        // 허용 종류 비어 있음.
        let none = ComponentSlot::new(
            slot_id(600),
            NodeRef::new(stable(102)),
            [],
            SlotCardinality::unbounded(),
            vec![NodeRef::new(stable(103)), NodeRef::new(stable(104))],
        );
        assert!(matches!(
            none.validate(tree),
            Err(SlotError::EmptyAllowedKinds { .. })
        ));
        // 잘못된 cardinality.
        assert_eq!(
            SlotCardinality::new(3, Some(1)).unwrap_err(),
            SlotError::InvalidCardinality { min: 3, max: 1 }
        );
    }

    #[test]
    fn library_slot_registration_rules() {
        let mut library = library();
        // 선언되지 않은 slot id.
        let undeclared = ComponentSlot::new(
            slot_id(699),
            NodeRef::new(stable(102)),
            [NodeKind::Text, NodeKind::Image],
            SlotCardinality::unbounded(),
            vec![NodeRef::new(stable(103)), NodeRef::new(stable(104))],
        );
        assert_eq!(
            library.add_slot(card(), undeclared).unwrap_err(),
            SlotError::SlotNotDeclared { slot: slot_id(699) }
        );
        library
            .add_slot(card(), body_slot(102, &[103, 104]))
            .unwrap();
        // 같은 slot 재등록.
        assert!(matches!(
            library.add_slot(card(), body_slot(102, &[103, 104])),
            Err(SlotError::DuplicateSlot { .. })
        ));
        // 같은 host에 두 번째 slot.
        let second = ComponentSlot::new(
            slot_id(601),
            NodeRef::new(stable(102)),
            [NodeKind::Text, NodeKind::Image],
            SlotCardinality::unbounded(),
            vec![NodeRef::new(stable(103)), NodeRef::new(stable(104))],
        );
        assert_eq!(
            library.add_slot(card(), second).unwrap_err(),
            SlotError::DuplicateSlotHost { host: stable(102) }
        );
        // 선언된 601에 record가 없음.
        assert_eq!(
            library.validate_slots(card()).unwrap_err(),
            SlotError::MissingSlotRecord { slot: slot_id(601) }
        );
        // 없는 컴포넌트.
        assert!(matches!(
            library.add_slot(ComponentRef::new(stable(77)), body_slot(102, &[])),
            Err(SlotError::ComponentNotFound { .. })
        ));
    }

    #[test]
    fn slot_and_library_round_trip() {
        let slot = body_slot(102, &[103, 104]);
        let bytes = slot.encode();
        assert_eq!(ComponentSlot::decode(&bytes).unwrap(), slot);
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(ComponentSlot::decode(&extended).is_err());
        assert!(ComponentSlot::decode(&bytes[..bytes.len() - 1]).is_err());

        let mut library = library();
        library.add_slot(card(), slot).unwrap();
        let lib_bytes = library.encode();
        let decoded = ComponentLibrary::decode(&lib_bytes).unwrap();
        assert_eq!(decoded, library);
        assert_eq!(decoded.encode(), lib_bytes);
    }

    #[test]
    fn cardinality_helpers() {
        let range = SlotCardinality::new(1, Some(2)).unwrap();
        assert!(!range.allows(0));
        assert!(range.allows(2));
        assert!(!range.allows(3));
        assert!(range.is_full(2));
        assert!(!range.is_full(1));
        assert!(SlotCardinality::unbounded().allows(10_000));
        assert!(!SlotCardinality::unbounded().is_full(10_000));
    }
}

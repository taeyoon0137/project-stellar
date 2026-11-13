//! VariantOverrideSet (IX.7.3).
//!
//! View Variant 하나가 base View 위에 덧씌우는 **희소(sparse) patch 묶음**입니다(기술
//! 아키텍처 16장). 모든 구조 override는 StableId 기반 patch입니다.
//!
//! - **property patch**: `PropertyPath` → `TypedValue`. 지정한 속성만 덮어씁니다.
//! - **구조 patch**: node별 하나(`StableId` → [`StructurePatch`]): 추가·삭제·이동·교체.
//! - **slot change**: (인스턴스, slot) → [`SlotChange`]. 인스턴스 slot override를 설정하거나
//!   default로 되돌립니다. nested 컴포넌트 내부는 Property·Drilling·Slot으로만 바꿉니다.
//!
//! [`VariantOverrideSet::validate_references`]가 base View 구조(scene)와 컴포넌트 library에
//! 대해 모든 참조를 검사합니다. 조건(VariantCondition, IX.7.2)과 적용 순서(IX.7.4~IX.7.6),
//! ComponentInstance 캡슐화 연결(IX.7.7)은 별도 작업입니다.
//!
//! slot change가 컴포넌트 Slot 모델에 의존하므로 component 모듈에 둡니다.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::{PathSegment, PropertyKey, PropertyPath};
use stellar_core_types::typed_value::{TypedValue, ValueDecodeError};

use super::codec::{
    capacity_hint, put_bool, put_bytes, put_count, put_id, put_kind, put_order_key, put_str,
    put_u32, put_u8, Malformed, Reader,
};
use super::definition::{ComponentLibrary, ComponentSlotId};
use super::expansion::SceneIndex;
use super::instance::{ComponentInstancePayload, InstancePayloadError, SlotOverride};
use crate::node_kind::NodeKind;

/// variant가 새로 만드는 node 내용(위치 제외).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantNode {
    /// 새 StableId.
    pub id: StableId,
    /// 종류.
    pub kind: NodeKind,
    /// 표시 여부.
    pub visible: bool,
    /// 잠금 여부.
    pub locked: bool,
    /// 표시 이름.
    pub display_name: String,
    /// 종류별 payload bytes.
    pub payload: Vec<u8>,
}

/// node 하나에 대한 구조 patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StructurePatch {
    /// 새 node를 `parent` 아래 `order_key` 위치에 추가합니다(map key = 새 node id).
    Add {
        /// 새 node.
        node: VariantNode,
        /// 부모(base node 또는 같은 set이 추가한 node).
        parent: StableId,
        /// 형제 순서 키.
        order_key: OrderKey,
    },
    /// base node(와 subtree)를 숨깁니다.
    Delete,
    /// base node를 새 부모·순서로 옮깁니다.
    Move {
        /// 새 부모.
        parent: StableId,
        /// 새 순서 키.
        order_key: OrderKey,
    },
    /// base node(와 subtree)를 같은 위치의 새 node로 교체합니다.
    Replace {
        /// 교체 node(새 id).
        node: VariantNode,
    },
}

/// slot 하나에 대한 변경.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotChange {
    /// slot override를 이 content로 설정합니다.
    Set(SlotOverride),
    /// slot override를 지워 default content로 되돌립니다.
    Reset,
}

const PATCH_ADD: u8 = 1;
const PATCH_DELETE: u8 = 2;
const PATCH_MOVE: u8 = 3;
const PATCH_REPLACE: u8 = 4;
const SLOT_SET: u8 = 1;
const SLOT_RESET: u8 = 2;
const TAIL_NONE: u8 = 0;
const TAIL_INDEX: u8 = 1;
const TAIL_KEY: u8 = 2;

/// VariantOverrideSet 오류(직렬화·참조 검증).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariantOverrideError {
    /// Add patch의 map key와 새 node id가 다릅니다.
    KeyMismatch {
        /// map key.
        key: StableId,
    },
    /// 새 node id가 base 또는 다른 patch와 겹칩니다.
    DuplicateNodeId {
        /// id.
        id: StableId,
    },
    /// patch 대상이 base View에 없습니다.
    UnknownTarget {
        /// 대상.
        target: StableId,
    },
    /// View root는 삭제·이동·교체할 수 없습니다.
    RootStructureChange {
        /// root.
        root: StableId,
    },
    /// 부모가 최종 구조에 없습니다(삭제·교체됐거나 존재하지 않음).
    DanglingParent {
        /// node.
        node: StableId,
        /// 부모.
        parent: StableId,
    },
    /// 자식을 가질 수 없는 종류를 부모로 지정했습니다.
    InvalidParentKind {
        /// 부모.
        parent: StableId,
        /// 종류.
        kind: NodeKind,
    },
    /// 추가·이동 결과 부모 관계에 cycle이 생깁니다.
    Cycle {
        /// cycle에 걸린 node.
        node: StableId,
    },
    /// property patch 대상이 최종 구조에 없습니다.
    DanglingPatch {
        /// patch 경로 root.
        target: StableId,
    },
    /// slot change 대상이 ComponentInstance가 아닙니다(또는 최종 구조에 없음).
    NotAnInstance {
        /// 대상.
        instance: StableId,
    },
    /// 인스턴스 컴포넌트에 없는 slot입니다.
    UnknownSlot {
        /// 인스턴스.
        instance: StableId,
        /// slot.
        slot: ComponentSlotId,
    },
    /// slot content가 최종 구조에서 그 인스턴스 소유(부모 = 인스턴스)가 아닙니다.
    SlotContentNotOwned {
        /// 인스턴스.
        instance: StableId,
        /// content.
        node: StableId,
    },
    /// 인스턴스 payload를 해석할 수 없습니다.
    Payload(InstancePayloadError),
    /// property 값 bytes가 잘못됐습니다.
    Value(ValueDecodeError),
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for VariantOverrideError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VariantOverrideError::KeyMismatch { key } => {
                write!(f, "Add patch key {key}와 새 node id가 다릅니다")
            }
            VariantOverrideError::DuplicateNodeId { id } => {
                write!(f, "새 node id {id}가 이미 쓰입니다")
            }
            VariantOverrideError::UnknownTarget { target } => {
                write!(f, "patch 대상 {target}가 base View에 없습니다")
            }
            VariantOverrideError::RootStructureChange { root } => {
                write!(f, "View root {root}는 삭제·이동·교체할 수 없습니다")
            }
            VariantOverrideError::DanglingParent { node, parent } => {
                write!(f, "node {node}의 부모 {parent}가 최종 구조에 없습니다")
            }
            VariantOverrideError::InvalidParentKind { parent, kind } => {
                write!(f, "{} node {parent}는 자식을 가질 수 없습니다", kind.name())
            }
            VariantOverrideError::Cycle { node } => {
                write!(f, "node {node}의 부모 관계에 cycle이 생깁니다")
            }
            VariantOverrideError::DanglingPatch { target } => {
                write!(f, "property patch 대상 {target}가 최종 구조에 없습니다")
            }
            VariantOverrideError::NotAnInstance { instance } => {
                write!(
                    f,
                    "slot change 대상 {instance}가 ComponentInstance가 아닙니다"
                )
            }
            VariantOverrideError::UnknownSlot { instance, slot } => write!(
                f,
                "인스턴스 {instance}의 컴포넌트에 slot {}이 없습니다",
                slot.stable_id()
            ),
            VariantOverrideError::SlotContentNotOwned { instance, node } => write!(
                f,
                "slot content {node}가 인스턴스 {instance} 소유가 아닙니다"
            ),
            VariantOverrideError::Payload(err) => write!(f, "{err}"),
            VariantOverrideError::Value(err) => write!(f, "property 값 디코드 실패: {err}"),
            VariantOverrideError::Malformed { reason } => {
                write!(f, "VariantOverrideSet binary 형식 위반: {reason}")
            }
        }
    }
}

impl std::error::Error for VariantOverrideError {}

impl From<Malformed> for VariantOverrideError {
    fn from(err: Malformed) -> VariantOverrideError {
        VariantOverrideError::Malformed { reason: err.0 }
    }
}

/// View Variant 하나의 희소 patch 묶음.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VariantOverrideSet {
    property_patches: BTreeMap<PropertyPath, TypedValue>,
    structure: BTreeMap<StableId, StructurePatch>,
    slot_changes: BTreeMap<(StableId, ComponentSlotId), SlotChange>,
}

/// 최종 구조에서 node 정보: (종류, 부모).
type FinalNode = (NodeKind, Option<StableId>);

impl VariantOverrideSet {
    /// 빈 set.
    pub fn new() -> VariantOverrideSet {
        VariantOverrideSet::default()
    }

    /// patch가 하나도 없는지.
    pub fn is_empty(&self) -> bool {
        self.property_patches.is_empty()
            && self.structure.is_empty()
            && self.slot_changes.is_empty()
    }

    /// property patch를 설정하고 이전 값을 반환합니다.
    pub fn set_property(&mut self, path: PropertyPath, value: TypedValue) -> Option<TypedValue> {
        self.property_patches.insert(path, value)
    }

    /// property patch를 지웁니다(base·일반 variant 값 상속으로 복귀).
    pub fn remove_property(&mut self, path: &PropertyPath) -> Option<TypedValue> {
        self.property_patches.remove(path)
    }

    /// property patch(경로 오름차순).
    pub fn property_patches(&self) -> impl Iterator<Item = (&PropertyPath, &TypedValue)> + '_ {
        self.property_patches.iter()
    }

    /// node에 구조 patch를 설정하고 이전 patch를 반환합니다. Add는 key가 새 node id입니다.
    pub fn set_structure(
        &mut self,
        node: StableId,
        patch: StructurePatch,
    ) -> Option<StructurePatch> {
        self.structure.insert(node, patch)
    }

    /// 구조 patch를 지웁니다.
    pub fn remove_structure(&mut self, node: StableId) -> Option<StructurePatch> {
        self.structure.remove(&node)
    }

    /// 구조 patch(node id 오름차순).
    pub fn structure_patches(&self) -> impl Iterator<Item = (StableId, &StructurePatch)> + '_ {
        self.structure.iter().map(|(id, patch)| (*id, patch))
    }

    /// slot change를 설정하고 이전 값을 반환합니다.
    pub fn set_slot(
        &mut self,
        instance: StableId,
        slot: ComponentSlotId,
        change: SlotChange,
    ) -> Option<SlotChange> {
        self.slot_changes.insert((instance, slot), change)
    }

    /// slot change를 지웁니다.
    pub fn remove_slot(&mut self, instance: StableId, slot: ComponentSlotId) -> Option<SlotChange> {
        self.slot_changes.remove(&(instance, slot))
    }

    /// slot change((인스턴스, slot) 오름차순).
    pub fn slot_changes(
        &self,
    ) -> impl Iterator<Item = (StableId, ComponentSlotId, &SlotChange)> + '_ {
        self.slot_changes
            .iter()
            .map(|((instance, slot), change)| (*instance, *slot, change))
    }

    // --- 참조 검증 ---

    /// base View 구조와 library에 대해 모든 참조를 검사합니다.
    ///
    /// base node는 `view_root` subtree 안에 있어야 합니다. 검사 순서는 구조 patch(node id
    /// 순) → property patch(경로 순) → slot change 순이며 첫 위반을 반환합니다.
    pub fn validate_references(
        &self,
        base: &SceneIndex<'_>,
        view_root: StableId,
        library: &ComponentLibrary,
    ) -> Result<(), VariantOverrideError> {
        let in_view = |id: StableId| -> bool {
            let mut current = Some(id);
            let mut steps = 0usize;
            while let Some(node) = current {
                if node == view_root {
                    return true;
                }
                steps += 1;
                if steps > base.len() {
                    return false;
                }
                current = base.parent(node);
            }
            false
        };
        let base_node = |id: StableId| -> Option<FinalNode> {
            if !in_view(id) {
                return None;
            }
            base.fields(id).map(|f| (f.kind, base.parent(id)))
        };

        // 1) 구조 patch 대상·새 id 검사, 최종 부모 관계 구성.
        let mut added: HashMap<StableId, FinalNode> = HashMap::new();
        let mut moved: HashMap<StableId, StableId> = HashMap::new();
        let mut removed: BTreeSet<StableId> = BTreeSet::new();
        let mut new_ids: BTreeSet<StableId> = BTreeSet::new();
        for (key, patch) in &self.structure {
            match patch {
                StructurePatch::Add {
                    node,
                    parent,
                    order_key: _,
                } => {
                    if node.id != *key {
                        return Err(VariantOverrideError::KeyMismatch { key: *key });
                    }
                    if base.fields(node.id).is_some() || !new_ids.insert(node.id) {
                        return Err(VariantOverrideError::DuplicateNodeId { id: node.id });
                    }
                    added.insert(node.id, (node.kind, Some(*parent)));
                }
                StructurePatch::Delete
                | StructurePatch::Move { .. }
                | StructurePatch::Replace { .. } => {
                    let (_, base_parent) = base_node(*key)
                        .ok_or(VariantOverrideError::UnknownTarget { target: *key })?;
                    if *key == view_root {
                        return Err(VariantOverrideError::RootStructureChange { root: view_root });
                    }
                    match patch {
                        StructurePatch::Move { parent, .. } => {
                            moved.insert(*key, *parent);
                        }
                        StructurePatch::Replace { node } => {
                            if base.fields(node.id).is_some() || !new_ids.insert(node.id) {
                                return Err(VariantOverrideError::DuplicateNodeId { id: node.id });
                            }
                            removed.insert(*key);
                            added.insert(node.id, (node.kind, base_parent));
                        }
                        _ => {
                            removed.insert(*key);
                        }
                    }
                }
            }
        }

        // 최종 구조 조회: 추가 node → 이동 → base 순.
        let final_node = |id: StableId| -> Option<FinalNode> {
            if let Some(node) = added.get(&id) {
                return Some(*node);
            }
            let (kind, parent) = base_node(id)?;
            Some((kind, moved.get(&id).copied().or(parent)))
        };
        let limit = base.len() + added.len() + 1;
        // node가 최종 구조에서 살아 있는지(제거된 조상 없이 root에 닿는지). cycle이면 Err.
        let live = |id: StableId| -> Result<bool, VariantOverrideError> {
            let mut current = Some(id);
            let mut steps = 0usize;
            while let Some(node) = current {
                if removed.contains(&node) {
                    return Ok(false);
                }
                if node == view_root {
                    return Ok(true);
                }
                steps += 1;
                if steps > limit {
                    return Err(VariantOverrideError::Cycle { node: id });
                }
                match final_node(node) {
                    Some((_, parent)) => current = parent,
                    None => return Ok(false),
                }
            }
            Ok(false)
        };

        // 2) Add·Move의 부모 검사.
        for (key, patch) in &self.structure {
            let parent = match patch {
                StructurePatch::Add { parent, .. } | StructurePatch::Move { parent, .. } => *parent,
                _ => continue,
            };
            if !live(parent)? {
                return Err(VariantOverrideError::DanglingParent { node: *key, parent });
            }
            let (kind, _) = final_node(parent)
                .ok_or(VariantOverrideError::DanglingParent { node: *key, parent })?;
            if !matches!(
                kind,
                NodeKind::Frame | NodeKind::Fragment | NodeKind::ComponentInstance
            ) {
                return Err(VariantOverrideError::InvalidParentKind { parent, kind });
            }
            // 이동·추가 node 자신이 최종 구조에서 root에 닿아야 합니다(cycle 아님).
            live(*key)?;
        }

        // 3) property patch 대상.
        for path in self.property_patches.keys() {
            if !live(path.root())? {
                return Err(VariantOverrideError::DanglingPatch {
                    target: path.root(),
                });
            }
        }

        // 4) slot change.
        for ((instance, slot), change) in &self.slot_changes {
            let kind = if live(*instance)? {
                final_node(*instance).map(|(kind, _)| kind)
            } else {
                None
            };
            if kind != Some(NodeKind::ComponentInstance) {
                return Err(VariantOverrideError::NotAnInstance {
                    instance: *instance,
                });
            }
            let payload_bytes = self
                .added_payload(*instance)
                .or_else(|| base.payload(*instance))
                .unwrap_or(&[]);
            let payload = ComponentInstancePayload::decode(payload_bytes)
                .map_err(VariantOverrideError::Payload)?;
            let declared = library
                .get(payload.component())
                .is_some_and(|entry| entry.slot(*slot).is_some());
            if !declared {
                return Err(VariantOverrideError::UnknownSlot {
                    instance: *instance,
                    slot: *slot,
                });
            }
            if let SlotChange::Set(content) = change {
                for node in content.content() {
                    let owned = live(*node)?
                        && final_node(*node).is_some_and(|(_, parent)| parent == Some(*instance));
                    if !owned {
                        return Err(VariantOverrideError::SlotContentNotOwned {
                            instance: *instance,
                            node: *node,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// 이 set이 추가·교체한 node의 payload.
    fn added_payload(&self, id: StableId) -> Option<&[u8]> {
        self.structure.iter().find_map(|(_, patch)| match patch {
            StructurePatch::Add { node, .. } | StructurePatch::Replace { node }
                if node.id == id =>
            {
                Some(node.payload.as_slice())
            }
            _ => None,
        })
    }

    // --- 직렬화 ---

    /// canonical binary bytes로 인코딩합니다(각 map은 key 오름차순).
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        put_count(&mut buf, self.property_patches.len());
        for (path, value) in &self.property_patches {
            put_path(&mut buf, path);
            put_bytes(&mut buf, &value.to_bytes());
        }
        put_count(&mut buf, self.structure.len());
        for (id, patch) in &self.structure {
            put_id(&mut buf, *id);
            match patch {
                StructurePatch::Add {
                    node,
                    parent,
                    order_key,
                } => {
                    put_u8(&mut buf, PATCH_ADD);
                    put_node(&mut buf, node);
                    put_id(&mut buf, *parent);
                    put_order_key(&mut buf, order_key);
                }
                StructurePatch::Delete => put_u8(&mut buf, PATCH_DELETE),
                StructurePatch::Move { parent, order_key } => {
                    put_u8(&mut buf, PATCH_MOVE);
                    put_id(&mut buf, *parent);
                    put_order_key(&mut buf, order_key);
                }
                StructurePatch::Replace { node } => {
                    put_u8(&mut buf, PATCH_REPLACE);
                    put_node(&mut buf, node);
                }
            }
        }
        put_count(&mut buf, self.slot_changes.len());
        for ((instance, slot), change) in &self.slot_changes {
            put_id(&mut buf, *instance);
            put_id(&mut buf, slot.stable_id());
            match change {
                SlotChange::Set(content) => {
                    put_u8(&mut buf, SLOT_SET);
                    put_count(&mut buf, content.len());
                    for node in content.content() {
                        put_id(&mut buf, *node);
                    }
                }
                SlotChange::Reset => put_u8(&mut buf, SLOT_RESET),
            }
        }
        buf
    }

    /// binary bytes에서 디코딩합니다. 오름차순 위반·알 수 없는 tag·trailing bytes는 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<VariantOverrideSet, VariantOverrideError> {
        let mut reader = Reader::new(bytes);
        let mut set = VariantOverrideSet::new();

        let count = reader.count()?;
        let mut previous: Option<PropertyPath> = None;
        for _ in 0..count {
            let path = take_path(&mut reader)?;
            if previous.as_ref().is_some_and(|p| *p >= path) {
                return Err(VariantOverrideError::Malformed {
                    reason: "property patch 경로가 오름차순이 아님",
                });
            }
            let value =
                TypedValue::from_bytes(reader.bytes()?).map_err(VariantOverrideError::Value)?;
            previous = Some(path.clone());
            set.property_patches.insert(path, value);
        }

        let count = reader.count()?;
        let mut previous: Option<StableId> = None;
        for _ in 0..count {
            let id = reader.id()?;
            if previous.is_some_and(|p| p >= id) {
                return Err(VariantOverrideError::Malformed {
                    reason: "구조 patch node가 오름차순이 아님",
                });
            }
            previous = Some(id);
            let patch = match reader.u8()? {
                PATCH_ADD => StructurePatch::Add {
                    node: take_node(&mut reader)?,
                    parent: reader.id()?,
                    order_key: reader.order_key()?,
                },
                PATCH_DELETE => StructurePatch::Delete,
                PATCH_MOVE => StructurePatch::Move {
                    parent: reader.id()?,
                    order_key: reader.order_key()?,
                },
                PATCH_REPLACE => StructurePatch::Replace {
                    node: take_node(&mut reader)?,
                },
                _ => {
                    return Err(VariantOverrideError::Malformed {
                        reason: "알 수 없는 구조 patch tag",
                    })
                }
            };
            set.structure.insert(id, patch);
        }

        let count = reader.count()?;
        let mut previous: Option<(StableId, ComponentSlotId)> = None;
        for _ in 0..count {
            let key = (reader.id()?, ComponentSlotId::new(reader.id()?));
            if previous.is_some_and(|p| p >= key) {
                return Err(VariantOverrideError::Malformed {
                    reason: "slot change가 오름차순이 아님",
                });
            }
            previous = Some(key);
            let change = match reader.u8()? {
                SLOT_SET => {
                    let len = reader.count()?;
                    let mut content = Vec::with_capacity(capacity_hint(len));
                    for _ in 0..len {
                        content.push(reader.id()?);
                    }
                    SlotChange::Set(
                        SlotOverride::new(content).map_err(VariantOverrideError::Payload)?,
                    )
                }
                SLOT_RESET => SlotChange::Reset,
                _ => {
                    return Err(VariantOverrideError::Malformed {
                        reason: "알 수 없는 slot change tag",
                    })
                }
            };
            set.slot_changes.insert(key, change);
        }
        reader.expect_end()?;
        Ok(set)
    }
}

fn put_path(buf: &mut Vec<u8>, path: &PropertyPath) {
    put_id(buf, path.root());
    put_count(buf, path.keys().len());
    for key in path.keys() {
        put_u32(buf, key.0);
    }
    match path.tail() {
        None => put_u8(buf, TAIL_NONE),
        Some(PathSegment::Index(index)) => {
            put_u8(buf, TAIL_INDEX);
            put_u32(buf, *index);
        }
        Some(PathSegment::Key(key)) => {
            put_u8(buf, TAIL_KEY);
            put_str(buf, key);
        }
    }
}

fn take_path(reader: &mut Reader<'_>) -> Result<PropertyPath, VariantOverrideError> {
    let root = reader.id()?;
    let count = reader.count()?;
    let mut keys = Vec::with_capacity(capacity_hint(count));
    for _ in 0..count {
        keys.push(PropertyKey(reader.u32()?));
    }
    let path = PropertyPath::with_keys(root, keys);
    Ok(match reader.u8()? {
        TAIL_NONE => path,
        TAIL_INDEX => path.with_index(reader.u32()?),
        TAIL_KEY => path.with_key_segment(&reader.string()?),
        _ => {
            return Err(VariantOverrideError::Malformed {
                reason: "알 수 없는 PropertyPath tail tag",
            })
        }
    })
}

fn put_node(buf: &mut Vec<u8>, node: &VariantNode) {
    put_id(buf, node.id);
    put_kind(buf, node.kind);
    put_bool(buf, node.visible);
    put_bool(buf, node.locked);
    put_str(buf, &node.display_name);
    put_bytes(buf, &node.payload);
}

fn take_node(reader: &mut Reader<'_>) -> Result<VariantNode, VariantOverrideError> {
    Ok(VariantNode {
        id: reader.id()?,
        kind: reader.kind()?,
        visible: reader.bool()?,
        locked: reader.bool()?,
        display_name: reader.string()?,
        payload: reader.bytes()?.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::definition::{ComponentDefinition, DefinitionNode, DefinitionTree};
    use crate::component::expansion::ScenePayloads;
    use crate::component::slot::{ComponentSlot, SlotCardinality};
    use crate::frame::keys as frame_keys;
    use crate::node_columns::{NodeColumns, NodeFields};
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::ids::RuntimeEntityId;
    use stellar_core_types::reference::{ComponentRef, NodeRef};
    use stellar_core_types::value::FiniteF32;

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn key(byte: u8) -> OrderKey {
        OrderKey::from_bytes(vec![byte]).unwrap()
    }

    fn slot(ts: u64) -> ComponentSlotId {
        ComponentSlotId::new(stable(ts))
    }

    fn card() -> ComponentRef {
        ComponentRef::new(stable(1))
    }

    /// Card(1): 100 ← 102(body host, slot 600: Text) — default 없음.
    fn library() -> ComponentLibrary {
        let mut tree = DefinitionTree::new(card());
        for (id, parent, order) in [(100, None, 0x80u8), (102, Some(100), 0x40)] {
            tree.insert(DefinitionNode {
                id: stable(id),
                kind: NodeKind::Frame,
                parent: parent.map(stable),
                order_key: key(order),
                visible: true,
                locked: false,
                display_name: String::new(),
                payload: Vec::new(),
            })
            .unwrap();
        }
        let mut definition =
            ComponentDefinition::new(stable(1), "Card".into(), NodeRef::new(stable(100)));
        definition.add_slot(slot(600)).unwrap();
        let mut library = ComponentLibrary::new();
        library.insert(definition, tree).unwrap();
        library
            .add_slot(
                card(),
                ComponentSlot::new(
                    slot(600),
                    NodeRef::new(stable(102)),
                    [NodeKind::Text],
                    SlotCardinality::unbounded(),
                    vec![],
                ),
            )
            .unwrap();
        library
    }

    /// base View: 10(root Frame) ← 11(Frame) ← 12(Text), 13(Shape); 10 ← 14(Card 인스턴스)
    /// ← 15(Text, slot content). View 밖: 90(다른 View root) ← 91.
    fn base() -> (NodeColumns, ScenePayloads) {
        let mut columns = NodeColumns::new();
        let mk = |id: u64, kind, parent: Option<RuntimeEntityId>, order: u8| NodeFields {
            stable_id: stable(id),
            kind,
            parent,
            order_key: key(order),
            visible: true,
            locked: false,
            display_name: String::new(),
        };
        let root = columns.insert(mk(10, NodeKind::Frame, None, 0x80));
        let group = columns.insert(mk(11, NodeKind::Frame, Some(root), 0x20));
        columns.insert(mk(12, NodeKind::Text, Some(group), 0x20));
        columns.insert(mk(13, NodeKind::Shape, Some(group), 0x40));
        let instance = columns.insert(mk(14, NodeKind::ComponentInstance, Some(root), 0x40));
        columns.insert(mk(15, NodeKind::Text, Some(instance), 0x40));
        let other = columns.insert(mk(90, NodeKind::Frame, None, 0x80));
        columns.insert(mk(91, NodeKind::Frame, Some(other), 0x40));
        let mut payload = ComponentInstancePayload::new(card());
        payload.set_slot_override(slot(600), SlotOverride::new(vec![stable(15)]).unwrap());
        let mut payloads = ScenePayloads::new();
        payloads.insert(stable(14), payload.encode());
        (columns, payloads)
    }

    fn new_node(id: u64, kind: NodeKind) -> VariantNode {
        VariantNode {
            id: stable(id),
            kind,
            visible: true,
            locked: false,
            display_name: format!("variant-{id}"),
            payload: vec![id as u8],
        }
    }

    /// 모든 patch 종류를 담은 유효한 set.
    fn sample() -> VariantOverrideSet {
        let mut set = VariantOverrideSet::new();
        set.set_property(
            PropertyPath::with_keys(stable(30), vec![frame_keys::BACKGROUND]),
            TypedValue::Color(Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).unwrap()),
        );
        set.set_property(
            PropertyPath::with_keys(stable(20), vec![PropertyKey(0x0501)]).with_index(2),
            TypedValue::Number(FiniteF32::ONE),
        );
        set.set_property(
            PropertyPath::new(stable(21)).with_key_segment("label"),
            TypedValue::String("오류".into()),
        );
        // 추가: 20(Frame) ← 22(Text), 그리고 인스턴스 소유 21(Text).
        set.set_structure(
            stable(20),
            StructurePatch::Add {
                node: new_node(20, NodeKind::Frame),
                parent: stable(10),
                order_key: key(0x70),
            },
        );
        set.set_structure(
            stable(22),
            StructurePatch::Add {
                node: new_node(22, NodeKind::Text),
                parent: stable(20),
                order_key: key(0x40),
            },
        );
        set.set_structure(
            stable(21),
            StructurePatch::Add {
                node: new_node(21, NodeKind::Text),
                parent: stable(14),
                order_key: key(0x60),
            },
        );
        set.set_structure(stable(13), StructurePatch::Delete);
        set.set_structure(
            stable(12),
            StructurePatch::Move {
                parent: stable(20),
                order_key: key(0x20),
            },
        );
        set.set_structure(
            stable(11),
            StructurePatch::Replace {
                node: new_node(30, NodeKind::Frame),
            },
        );
        set.set_slot(
            stable(14),
            slot(600),
            SlotChange::Set(SlotOverride::new(vec![stable(21), stable(15)]).unwrap()),
        );
        set
    }

    #[test]
    fn variant_override_set_round_trips() {
        // 검증 기준(IX.7.3): round-trip.
        let set = sample();
        let bytes = set.encode();
        let decoded = VariantOverrideSet::decode(&bytes).unwrap();
        assert_eq!(decoded, set);
        assert_eq!(decoded.encode(), bytes, "재인코딩 byte-identical");

        // 설정 순서와 무관.
        let mut reversed = VariantOverrideSet::new();
        let mut patches: Vec<_> = set
            .structure_patches()
            .map(|(i, p)| (i, p.clone()))
            .collect();
        patches.reverse();
        for (id, patch) in patches {
            reversed.set_structure(id, patch);
        }
        let mut props: Vec<_> = set
            .property_patches()
            .map(|(p, v)| (p.clone(), v.clone()))
            .collect();
        props.reverse();
        for (path, value) in props {
            reversed.set_property(path, value);
        }
        for (instance, slot, change) in set.slot_changes() {
            reversed.set_slot(instance, slot, change.clone());
        }
        assert_eq!(reversed.encode(), bytes);

        // Reset slot change와 빈 set.
        let mut reset = VariantOverrideSet::new();
        assert!(reset.is_empty());
        assert_eq!(reset.encode(), vec![0; 12]);
        reset.set_slot(stable(14), slot(600), SlotChange::Reset);
        assert_eq!(VariantOverrideSet::decode(&reset.encode()).unwrap(), reset);
    }

    #[test]
    fn malformed_bytes_are_rejected() {
        let bytes = sample().encode();
        for cut in [0, 4, 20, bytes.len() - 1] {
            assert!(
                VariantOverrideSet::decode(&bytes[..cut]).is_err(),
                "절단 {cut}"
            );
        }
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(VariantOverrideSet::decode(&extended).is_err());

        // 알 수 없는 구조 patch tag.
        let mut bad = Vec::new();
        put_count(&mut bad, 0);
        put_count(&mut bad, 1);
        put_id(&mut bad, stable(13));
        put_u8(&mut bad, 9);
        put_count(&mut bad, 0);
        assert!(matches!(
            VariantOverrideSet::decode(&bad),
            Err(VariantOverrideError::Malformed { .. })
        ));
        // 구조 patch 내림차순.
        let mut unordered = Vec::new();
        put_count(&mut unordered, 0);
        put_count(&mut unordered, 2);
        for id in [13, 12] {
            put_id(&mut unordered, stable(id));
            put_u8(&mut unordered, PATCH_DELETE);
        }
        put_count(&mut unordered, 0);
        assert!(VariantOverrideSet::decode(&unordered).is_err());
    }

    #[test]
    fn valid_references_pass() {
        // 검증 기준(IX.7.3): reference validation.
        let library = library();
        let (columns, payloads) = base();
        let index = SceneIndex::build(&columns, &payloads);
        sample()
            .validate_references(&index, stable(10), &library)
            .unwrap();
    }

    #[test]
    fn dangling_and_invalid_references_are_rejected() {
        let library = library();
        let (columns, payloads) = base();
        let index = SceneIndex::build(&columns, &payloads);
        let check =
            |set: &VariantOverrideSet| set.validate_references(&index, stable(10), &library);

        // 없는 node·다른 View node 대상.
        for target in [99, 91] {
            let mut set = VariantOverrideSet::new();
            set.set_structure(stable(target), StructurePatch::Delete);
            assert_eq!(
                check(&set).unwrap_err(),
                VariantOverrideError::UnknownTarget {
                    target: stable(target)
                }
            );
        }
        // root 구조 변경.
        let mut set = VariantOverrideSet::new();
        set.set_structure(stable(10), StructurePatch::Delete);
        assert_eq!(
            check(&set).unwrap_err(),
            VariantOverrideError::RootStructureChange { root: stable(10) }
        );
        // 기존 id로 추가, key 불일치.
        let mut set = VariantOverrideSet::new();
        set.set_structure(
            stable(12),
            StructurePatch::Add {
                node: new_node(12, NodeKind::Text),
                parent: stable(10),
                order_key: key(0x70),
            },
        );
        assert_eq!(
            check(&set).unwrap_err(),
            VariantOverrideError::DuplicateNodeId { id: stable(12) }
        );
        let mut set = VariantOverrideSet::new();
        set.set_structure(
            stable(40),
            StructurePatch::Add {
                node: new_node(41, NodeKind::Text),
                parent: stable(10),
                order_key: key(0x70),
            },
        );
        assert!(matches!(
            check(&set).unwrap_err(),
            VariantOverrideError::KeyMismatch { .. }
        ));
        // 삭제된 node 아래로 추가·이동, 삭제된 subtree의 property patch.
        let mut set = VariantOverrideSet::new();
        set.set_structure(stable(11), StructurePatch::Delete);
        set.set_structure(
            stable(40),
            StructurePatch::Add {
                node: new_node(40, NodeKind::Text),
                parent: stable(11),
                order_key: key(0x70),
            },
        );
        assert_eq!(
            check(&set).unwrap_err(),
            VariantOverrideError::DanglingParent {
                node: stable(40),
                parent: stable(11),
            }
        );
        let mut set = VariantOverrideSet::new();
        set.set_structure(stable(11), StructurePatch::Delete);
        set.set_property(
            PropertyPath::with_keys(stable(12), vec![PropertyKey(0x0501)]),
            TypedValue::Boolean(true),
        );
        assert_eq!(
            check(&set).unwrap_err(),
            VariantOverrideError::DanglingPatch { target: stable(12) }
        );
        // 삭제된 subtree에서 이동해 나온 node의 patch는 유효.
        set.set_structure(
            stable(12),
            StructurePatch::Move {
                parent: stable(10),
                order_key: key(0x10),
            },
        );
        check(&set).unwrap();
        // 자식을 가질 수 없는 부모, 자기 자손 아래로 이동(cycle).
        let mut set = VariantOverrideSet::new();
        set.set_structure(
            stable(12),
            StructurePatch::Move {
                parent: stable(13),
                order_key: key(0x10),
            },
        );
        assert_eq!(
            check(&set).unwrap_err(),
            VariantOverrideError::InvalidParentKind {
                parent: stable(13),
                kind: NodeKind::Shape,
            }
        );
        let mut set = VariantOverrideSet::new();
        set.set_structure(
            stable(11),
            StructurePatch::Move {
                parent: stable(40),
                order_key: key(0x10),
            },
        );
        set.set_structure(
            stable(40),
            StructurePatch::Add {
                node: new_node(40, NodeKind::Frame),
                parent: stable(11),
                order_key: key(0x10),
            },
        );
        assert!(matches!(
            check(&set).unwrap_err(),
            VariantOverrideError::Cycle { .. }
        ));
        // 교체된 node 대상 patch.
        let mut set = VariantOverrideSet::new();
        set.set_structure(
            stable(13),
            StructurePatch::Replace {
                node: new_node(31, NodeKind::Shape),
            },
        );
        set.set_property(PropertyPath::new(stable(13)), TypedValue::Boolean(false));
        assert_eq!(
            check(&set).unwrap_err(),
            VariantOverrideError::DanglingPatch { target: stable(13) }
        );
    }

    #[test]
    fn slot_change_references_are_validated() {
        let library = library();
        let (columns, payloads) = base();
        let index = SceneIndex::build(&columns, &payloads);
        let check =
            |set: &VariantOverrideSet| set.validate_references(&index, stable(10), &library);

        // ComponentInstance가 아닌 대상.
        let mut set = VariantOverrideSet::new();
        set.set_slot(stable(11), slot(600), SlotChange::Reset);
        assert_eq!(
            check(&set).unwrap_err(),
            VariantOverrideError::NotAnInstance {
                instance: stable(11)
            }
        );
        // 컴포넌트에 없는 slot.
        let mut set = VariantOverrideSet::new();
        set.set_slot(stable(14), slot(699), SlotChange::Reset);
        assert!(matches!(
            check(&set).unwrap_err(),
            VariantOverrideError::UnknownSlot { .. }
        ));
        // 인스턴스 소유가 아닌 content(부모가 11인 12).
        let mut set = VariantOverrideSet::new();
        set.set_slot(
            stable(14),
            slot(600),
            SlotChange::Set(SlotOverride::new(vec![stable(12)]).unwrap()),
        );
        assert_eq!(
            check(&set).unwrap_err(),
            VariantOverrideError::SlotContentNotOwned {
                instance: stable(14),
                node: stable(12),
            }
        );
        // 같은 set이 인스턴스 아래로 옮긴 node는 소유 content로 인정.
        set.set_structure(
            stable(12),
            StructurePatch::Move {
                parent: stable(14),
                order_key: key(0x10),
            },
        );
        check(&set).unwrap();
        // 같은 set이 추가한 인스턴스의 slot change.
        let mut set = VariantOverrideSet::new();
        set.set_structure(
            stable(50),
            StructurePatch::Add {
                node: VariantNode {
                    payload: ComponentInstancePayload::new(card()).encode(),
                    ..new_node(50, NodeKind::ComponentInstance)
                },
                parent: stable(10),
                order_key: key(0x70),
            },
        );
        set.set_slot(stable(50), slot(600), SlotChange::Reset);
        check(&set).unwrap();
    }
}

//! command 결과 change set (VII.1.4).
//!
//! command 실행 결과는 **무엇이 바뀌었는지**를 하류(layout dirty set X.7.1, scene dirty set
//! XI.2.1, 진단 증분 재검사 XXV.2.3, selection 정리 VII.3.4)에 알립니다.
//!
//! - changed nodes: 상태가 바뀐(삽입 포함, 삭제 제외) node와 node별 무효화 class.
//! - changed properties: 바뀐 속성의 [`PropertyPath`](root = node 또는 modifier StableId).
//! - invalidation classes: node별 무효화의 합집합([`InvalidationSet`], VI.9.7과 같은 class).
//! - inserted / removed nodes: 순(net) 효과 기준. 같은 실행 안에서 삽입 후 삭제된 node는 둘
//!   다에 없고, 삭제 후 다시 삽입된 node는 "바뀐 node"(구조 무효화)로 남습니다.
//! - new revision: 실행 후 문서 revision.
//!
//! 무효화 class는 속성 metadata registry(Frame VI.3.5, modifier VI.9.7, 공통 node 필드
//! [`node_keys`])에서 가져옵니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::revision::DocumentRevision;
use stellar_document::modifier_metadata::{InvalidationClass, InvalidationSet};

/// 모든 node 종류에 공통인 필드의 속성 key(`0x00xx` 대역).
///
/// 숫자 값은 저장·진단 경로의 일부입니다. Frame(`0x01xx`)·Image(`0x06xx`)·component
/// instance(`0x0Axx`)·modifier(`0x10xx`) 대역과 겹치지 않습니다.
pub mod node_keys {
    use stellar_core_types::property::PropertyKey;

    /// 표시 여부.
    pub const VISIBLE: PropertyKey = PropertyKey(0x0001);
    /// 잠금 여부.
    pub const LOCKED: PropertyKey = PropertyKey(0x0002);
    /// 표시 이름.
    pub const DISPLAY_NAME: PropertyKey = PropertyKey(0x0003);
    /// 부모 node.
    pub const PARENT: PropertyKey = PropertyKey(0x0004);
    /// 형제 순서 key.
    pub const ORDER_KEY: PropertyKey = PropertyKey(0x0005);
}

/// 구조 변경(삽입·삭제·이동·종류 변환)의 무효화: geometry + layout + paint.
pub const STRUCTURE_INVALIDATION: InvalidationSet = InvalidationSet::EMPTY
    .with(InvalidationClass::Geometry)
    .with(InvalidationClass::Layout)
    .with(InvalidationClass::Paint);

/// 공통 node 필드 key의 무효화 class. 공통 필드가 아니면 `None`.
///
/// - 표시 여부: 그리기·hit test·(자동 layout 참여 여부)가 바뀌므로 구조 변경과 같게 봅니다.
/// - 잠금·표시 이름: 편집 상호작용과 UI 표시에만 쓰이며 scene·layout 입력이 아닙니다.
/// - 부모·순서: 구조 변경.
pub fn node_field_invalidation(key: PropertyKey) -> Option<InvalidationSet> {
    match key {
        node_keys::VISIBLE | node_keys::PARENT | node_keys::ORDER_KEY => {
            Some(STRUCTURE_INVALIDATION)
        }
        node_keys::LOCKED | node_keys::DISPLAY_NAME => Some(InvalidationSet::EMPTY),
        _ => None,
    }
}

/// 실행 중 변경 기록기. command가 **의미 단위**로 기록합니다(primitive 구현 세부와 무관).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChangeRecorder {
    nodes: BTreeMap<StableId, InvalidationSet>,
    properties: BTreeSet<PropertyPath>,
    inserted: BTreeSet<StableId>,
    removed: BTreeSet<StableId>,
}

impl ChangeRecorder {
    /// 빈 기록기.
    pub fn new() -> ChangeRecorder {
        ChangeRecorder::default()
    }

    fn touch(&mut self, node: StableId, invalidation: InvalidationSet) {
        let entry = self.nodes.entry(node).or_default();
        *entry = entry.union(invalidation);
    }

    /// `node`가 소유한 속성 `path`가 바뀌었습니다.
    pub fn property(&mut self, node: StableId, path: PropertyPath, invalidation: InvalidationSet) {
        self.touch(node, invalidation);
        self.properties.insert(path);
    }

    /// node 자신의 속성 `key`가 바뀌었습니다(path = `(node, [key])`).
    pub fn node_property(
        &mut self,
        node: StableId,
        key: PropertyKey,
        invalidation: InvalidationSet,
    ) {
        self.property(node, PropertyPath::with_keys(node, vec![key]), invalidation);
    }

    /// node의 자식 구성이나 종류처럼 구조가 바뀌었습니다.
    pub fn structure(&mut self, node: StableId) {
        self.touch(node, STRUCTURE_INVALIDATION);
    }

    /// node가 삽입되었습니다.
    pub fn inserted(&mut self, node: StableId) {
        if self.removed.remove(&node) {
            // 삭제 후 재삽입: 존재는 유지, 내용이 바뀐 node.
            self.structure(node);
        } else {
            self.inserted.insert(node);
            self.structure(node);
        }
    }

    /// node가 삭제되었습니다.
    pub fn removed(&mut self, node: StableId) {
        self.nodes.remove(&node);
        self.properties.retain(|path| path.root() != node);
        if !self.inserted.remove(&node) {
            self.removed.insert(node);
        }
    }

    /// 다른 기록을 합칩니다(시간 순서: `self` 다음 `other`).
    pub fn merge(&mut self, other: ChangeRecorder) {
        for node in other.removed {
            self.removed(node);
        }
        for node in other.inserted {
            self.inserted(node);
        }
        for (node, invalidation) in other.nodes {
            self.touch(node, invalidation);
        }
        self.properties.extend(other.properties);
    }

    /// 기록이 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
            && self.properties.is_empty()
            && self.inserted.is_empty()
            && self.removed.is_empty()
    }

    /// 실행 후 revision과 함께 결과를 확정합니다.
    pub fn finish(self, new_revision: DocumentRevision) -> ChangeSet {
        ChangeSet {
            nodes: self.nodes,
            properties: self.properties,
            inserted: self.inserted,
            removed: self.removed,
            new_revision,
        }
    }
}

/// command(또는 transaction) 실행 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSet {
    nodes: BTreeMap<StableId, InvalidationSet>,
    properties: BTreeSet<PropertyPath>,
    inserted: BTreeSet<StableId>,
    removed: BTreeSet<StableId>,
    new_revision: DocumentRevision,
}

impl ChangeSet {
    /// 바뀐 node(StableId 순). 삽입 포함, 삭제 제외.
    pub fn changed_nodes(&self) -> Vec<StableId> {
        self.nodes.keys().copied().collect()
    }

    /// node 하나의 무효화 class(바뀌지 않았으면 `None`).
    pub fn node_invalidation(&self, node: StableId) -> Option<InvalidationSet> {
        self.nodes.get(&node).copied()
    }

    /// 바뀐 속성 경로(canonical 순).
    pub fn changed_properties(&self) -> Vec<PropertyPath> {
        self.properties.iter().cloned().collect()
    }

    /// 전체 무효화 class(node별 합집합).
    pub fn invalidation(&self) -> InvalidationSet {
        self.nodes
            .values()
            .fold(InvalidationSet::EMPTY, |acc, set| acc.union(*set))
    }

    /// 순 효과로 새로 생긴 node.
    pub fn inserted_nodes(&self) -> Vec<StableId> {
        self.inserted.iter().copied().collect()
    }

    /// 순 효과로 사라진 node.
    pub fn removed_nodes(&self) -> Vec<StableId> {
        self.removed.iter().copied().collect()
    }

    /// 실행 후 문서 revision.
    pub fn new_revision(&self) -> DocumentRevision {
        self.new_revision
    }

    /// 아무것도 바뀌지 않았는지.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
            && self.properties.is_empty()
            && self.inserted.is_empty()
            && self.removed.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_document::frame::keys as frame_keys;
    use stellar_document::frame_metadata::frame_property;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    #[test]
    fn common_field_keys_are_unique_and_outside_other_bands() {
        let keys = [
            node_keys::VISIBLE,
            node_keys::LOCKED,
            node_keys::DISPLAY_NAME,
            node_keys::PARENT,
            node_keys::ORDER_KEY,
        ];
        let unique: BTreeSet<u32> = keys.iter().map(|k| k.0).collect();
        assert_eq!(unique.len(), keys.len());
        for key in keys {
            assert_eq!(key.0 >> 8, 0, "0x00xx 대역");
            assert!(node_field_invalidation(key).is_some());
            assert!(frame_property(key).is_none(), "Frame 대역과 겹침");
        }
        assert_eq!(node_field_invalidation(frame_keys::SIZE), None);
    }

    #[test]
    fn single_property_change_set() {
        let mut recorder = ChangeRecorder::new();
        let invalidation =
            InvalidationSet::from(frame_property(frame_keys::SIZE).unwrap().invalidation);
        recorder.node_property(sid(5), frame_keys::SIZE, invalidation);
        let changes = recorder.finish(DocumentRevision(8));
        assert_eq!(changes.changed_nodes(), vec![sid(5)]);
        assert_eq!(
            changes.changed_properties(),
            vec![PropertyPath::with_keys(sid(5), vec![frame_keys::SIZE])]
        );
        assert_eq!(
            changes.invalidation().classes(),
            vec![InvalidationClass::Layout, InvalidationClass::Paint]
        );
        assert_eq!(changes.new_revision(), DocumentRevision(8));
        assert!(changes.inserted_nodes().is_empty() && changes.removed_nodes().is_empty());
    }

    #[test]
    fn insert_and_remove_are_net_effects() {
        let mut recorder = ChangeRecorder::new();
        recorder.inserted(sid(1));
        recorder.node_property(sid(1), node_keys::LOCKED, InvalidationSet::EMPTY);
        recorder.removed(sid(1));
        assert!(recorder.is_empty(), "삽입 후 삭제는 순 효과 없음");

        recorder.removed(sid(2));
        recorder.inserted(sid(2));
        let changes = recorder.clone().finish(DocumentRevision(1));
        assert_eq!(changes.changed_nodes(), vec![sid(2)]);
        assert!(changes.removed_nodes().is_empty() && changes.inserted_nodes().is_empty());
        assert_eq!(
            changes.node_invalidation(sid(2)),
            Some(STRUCTURE_INVALIDATION)
        );

        let mut later = ChangeRecorder::new();
        later.removed(sid(2));
        later.inserted(sid(3));
        recorder.merge(later);
        let merged = recorder.finish(DocumentRevision(2));
        assert_eq!(merged.removed_nodes(), vec![sid(2)]);
        assert_eq!(merged.inserted_nodes(), vec![sid(3)]);
        assert_eq!(merged.changed_nodes(), vec![sid(3)]);
    }
}

//! 런타임 문서 모델 (VI.10.2·VI.10.3).
//!
//! 한 문서의 영속 내용을 runtime 구조로 보관합니다:
//!
//! - 공통 node 필드: SoA [`NodeColumns`](III.6.2). 부모는 runtime 핸들입니다.
//! - StableId ↔ [`RuntimeEntityId`] 양방향 [`IdRegistry`](III.1.4).
//! - 종류별 payload와 modifier stack: StableId 키 `BTreeMap`(순회 결정론).
//!
//! **runtime 핸들은 이 모델 밖으로 영속되지 않습니다.** [`DocumentModel::to_records`]가 부모
//! 핸들을 StableId로 바꾼 영속 레코드(VI.10)를 만들고, 직렬화는 그 레코드만 사용합니다.
//! 핸들 값(slot·generation)은 삽입·삭제 이력에 따라 달라지지만 레코드는 같습니다.
//! 로드는 [`DocumentModel::from_records`]가 레코드를 검증한 뒤 핸들을 새로 할당하며 ID
//! registry와 부모·자식 순서를 재구축합니다(VI.10.3).

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::{IdRegistry, RuntimeEntityId, StableId};

use crate::definitions::DocumentDefinition;
use crate::frame::Clipping;
use crate::modifier::{validate_modifier_owner, ModifierError, ModifierStack};
use crate::node_columns::{NodeColumns, NodeFields};
use crate::records::{DocumentRecords, NodePayload, NodeRecord};
use crate::reference_index::ReferenceIndex;
use crate::tree_ops::children_in_order;
use crate::validate::{validate_document, DocumentDiagnostics, DocumentValidationError};

/// 런타임 모델 조작 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    /// NIL은 node id로 쓸 수 없습니다.
    NilNodeId,
    /// 같은 StableId node가 이미 있습니다.
    DuplicateNode {
        /// 중복 id.
        id: StableId,
    },
    /// 부모 node가 없습니다.
    MissingParent {
        /// 삽입하려던 node.
        id: StableId,
        /// 없는 부모.
        parent: StableId,
    },
    /// 대상 node가 없습니다.
    UnknownNode {
        /// 요청 id.
        id: StableId,
    },
    /// 자식이 있는 node는 leaf 삭제로 지울 수 없습니다.
    HasChildren {
        /// 대상 node.
        id: StableId,
    },
    /// 새 payload의 종류가 기존 node 종류와 다릅니다(종류 변경은 변환 command의 몫).
    PayloadKindChange {
        /// 대상 node.
        id: StableId,
    },
    /// modifier stack 제약 위반(예: Fragment modifier, VI.9.6).
    Modifier {
        /// 소유 node.
        owner: StableId,
        /// 원인.
        error: ModifierError,
    },
    /// 내부 불일치: 살아 있지 않은 부모 핸들이나 payload 누락(버그 신호).
    Inconsistent {
        /// 관련 node.
        id: StableId,
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::NilNodeId => f.write_str("NIL은 node id로 쓸 수 없습니다"),
            ModelError::DuplicateNode { id } => write!(f, "node {id}가 이미 있습니다"),
            ModelError::MissingParent { id, parent } => {
                write!(f, "node {id}의 부모 {parent}가 없습니다")
            }
            ModelError::UnknownNode { id } => write!(f, "node {id}가 없습니다"),
            ModelError::HasChildren { id } => write!(f, "node {id}에 자식이 있습니다"),
            ModelError::PayloadKindChange { id } => {
                write!(f, "node {id}의 종류를 payload 교체로 바꿀 수 없습니다")
            }
            ModelError::Modifier { owner, error } => {
                write!(f, "node {owner}의 modifier stack: {error}")
            }
            ModelError::Inconsistent { id, reason } => {
                write!(f, "모델 내부 불일치(node {id}): {reason}")
            }
        }
    }
}

impl std::error::Error for ModelError {}

/// 레코드 → 런타임 모델 로드 오류(VI.10.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadModelError {
    /// 레코드가 문서 불변식을 위반합니다(모델을 만들지 않음).
    Invalid(DocumentValidationError),
    /// 검증을 통과한 레코드를 모델에 넣지 못했습니다(버그 신호).
    Model(ModelError),
}

impl fmt::Display for LoadModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadModelError::Invalid(e) => write!(f, "{e}"),
            LoadModelError::Model(e) => write!(f, "문서 모델 재구축 실패: {e}"),
        }
    }
}

impl std::error::Error for LoadModelError {}

/// 한 문서의 런타임 모델.
#[derive(Debug)]
pub struct DocumentModel {
    definition: DocumentDefinition,
    columns: NodeColumns,
    ids: IdRegistry,
    payloads: BTreeMap<StableId, NodePayload>,
    modifiers: BTreeMap<StableId, ModifierStack>,
    references: ReferenceIndex,
}

impl DocumentModel {
    /// node가 없는 모델.
    pub fn new(definition: DocumentDefinition) -> DocumentModel {
        DocumentModel {
            definition,
            columns: NodeColumns::new(),
            ids: IdRegistry::new(),
            payloads: BTreeMap::new(),
            modifiers: BTreeMap::new(),
            references: ReferenceIndex::new(),
        }
    }

    /// 문서·페이지·View 정의.
    pub fn definition(&self) -> &DocumentDefinition {
        &self.definition
    }

    /// 문서 StableId.
    pub fn document_id(&self) -> StableId {
        self.definition.id
    }

    /// node 수.
    pub fn node_count(&self) -> usize {
        self.columns.len()
    }

    /// node가 있는지.
    pub fn contains(&self, id: StableId) -> bool {
        self.ids.runtime_for(id).is_some()
    }

    /// node의 runtime 핸들(런타임 전용, 영속 금지).
    pub fn runtime_id(&self, id: StableId) -> Option<RuntimeEntityId> {
        self.ids.runtime_for(id)
    }

    /// runtime 핸들의 StableId.
    pub fn stable_id(&self, handle: RuntimeEntityId) -> Option<StableId> {
        self.ids.stable_for(handle)
    }

    /// 읽기 전용 공통 field column(VI.2 helper 용).
    pub fn columns(&self) -> &NodeColumns {
        &self.columns
    }

    /// node payload.
    pub fn payload(&self, id: StableId) -> Option<&NodePayload> {
        self.payloads.get(&id)
    }

    /// node의 modifier stack(없으면 `None` — 빈 stack은 보관하지 않음).
    pub fn modifier_stack(&self, id: StableId) -> Option<&ModifierStack> {
        self.modifiers.get(&id)
    }

    /// Frame clip과 modifier가 참조하는 node. 미해결 대상과 비활성 modifier도 포함합니다.
    pub fn references(&self) -> &ReferenceIndex {
        &self.references
    }

    fn sync_references(&mut self, owner: StableId) {
        if let Some(stack) = self.modifiers.get(&owner) {
            stack.sync_references(&mut self.references);
        } else {
            self.references.remove_all_from(owner);
        }
        if let Some(NodePayload::Frame(frame)) = self.payloads.get(&owner) {
            if let Clipping::Path(target) = frame.clipping {
                if target.id() != StableId::NIL {
                    self.references.add_reference(owner, target.id());
                }
            }
        }
    }

    /// 부모의 자식 StableId를 OrderKey 순서로(VI.2.4).
    pub fn children_in_order(&self, parent: StableId) -> Vec<StableId> {
        let Some(handle) = self.ids.runtime_for(parent) else {
            return Vec::new();
        };
        children_in_order(&self.columns, handle)
            .into_iter()
            .filter_map(|child| self.ids.stable_for(child))
            .collect()
    }

    /// node를 추가합니다. 부모(있다면)가 먼저 있어야 하고 id는 유일해야 합니다.
    /// 문서 수준 불변식(root·순서·참조 등)은 저장 전 validator(VI.10.1)가 검사합니다.
    pub fn insert_node(&mut self, record: NodeRecord) -> Result<RuntimeEntityId, ModelError> {
        if record.id == StableId::NIL {
            return Err(ModelError::NilNodeId);
        }
        if self.contains(record.id) {
            return Err(ModelError::DuplicateNode { id: record.id });
        }
        let parent = match record.parent {
            None => None,
            Some(parent) => Some(self.ids.runtime_for(parent).ok_or(
                ModelError::MissingParent {
                    id: record.id,
                    parent,
                },
            )?),
        };
        let kind = record.kind();
        let handle = self.columns.insert(NodeFields {
            stable_id: record.id,
            kind,
            parent,
            order_key: record.order_key,
            visible: record.visible,
            locked: record.locked,
            display_name: record.display_name,
        });
        if self.ids.insert(record.id, handle).is_err() {
            self.columns.remove(handle);
            return Err(ModelError::DuplicateNode { id: record.id });
        }
        self.payloads.insert(record.id, record.payload);
        self.sync_references(record.id);
        Ok(handle)
    }

    /// 자식이 없는 node를 지우고 그 레코드를 반환합니다. modifier stack도 함께 지웁니다.
    /// 다른 node가 이 node를 참조하는지는 command 단계의 정책입니다(VI.2.5).
    ///
    /// 자식 확인은 column의 자식 수([`NodeColumns::child_count`], O(1))로 합니다 — 전체 column을
    /// 훑으면 subtree 삭제·되돌리기가 O(subtree × N)이 됩니다(posthoc CMD-DELETE-QUADRATIC).
    pub fn remove_leaf(&mut self, id: StableId) -> Result<NodeRecord, ModelError> {
        let record = self.record(id).ok_or(ModelError::UnknownNode { id })?;
        let has_children = self
            .ids
            .runtime_for(id)
            .and_then(|handle| self.columns.child_count(handle))
            .is_some_and(|count| count > 0);
        if has_children {
            return Err(ModelError::HasChildren { id });
        }
        let handle = self
            .ids
            .remove_by_stable(id)
            .ok_or(ModelError::UnknownNode { id })?;
        self.columns.remove(handle);
        self.payloads.remove(&id);
        self.modifiers.remove(&id);
        self.references.remove_all_from(id);
        Ok(record)
    }

    /// payload를 교체합니다. node 종류는 바꿀 수 없습니다.
    pub fn set_payload(&mut self, id: StableId, payload: NodePayload) -> Result<(), ModelError> {
        let current = self
            .payloads
            .get_mut(&id)
            .ok_or(ModelError::UnknownNode { id })?;
        if current.kind() != payload.kind() {
            return Err(ModelError::PayloadKindChange { id });
        }
        *current = payload;
        self.sync_references(id);
        Ok(())
    }

    /// 표시 이름을 바꿉니다.
    pub fn set_display_name(&mut self, id: StableId, name: String) -> Result<(), ModelError> {
        let handle = self
            .ids
            .runtime_for(id)
            .ok_or(ModelError::UnknownNode { id })?;
        if self.columns.set_display_name(handle, name) {
            Ok(())
        } else {
            Err(ModelError::Inconsistent {
                id,
                reason: "registry 핸들이 살아 있지 않음",
            })
        }
    }

    /// node의 modifier stack을 설정합니다. 소유 node가 있어야 하고 Fragment는 modifier를
    /// 가질 수 없습니다(VI.9.6). 빈 stack은 stack 제거와 같습니다.
    pub fn set_modifier_stack(&mut self, stack: ModifierStack) -> Result<(), ModelError> {
        let owner = stack.node;
        let kind = self
            .payloads
            .get(&owner)
            .map(NodePayload::kind)
            .ok_or(ModelError::UnknownNode { id: owner })?;
        for entry in stack.entries() {
            validate_modifier_owner(kind, entry.kind)
                .map_err(|error| ModelError::Modifier { owner, error })?;
        }
        if stack.entries().is_empty() {
            self.modifiers.remove(&owner);
        } else {
            self.modifiers.insert(owner, stack);
        }
        self.sync_references(owner);
        Ok(())
    }

    /// node 하나의 영속 레코드(부모는 StableId).
    pub fn record(&self, id: StableId) -> Option<NodeRecord> {
        let handle = self.ids.runtime_for(id)?;
        let fields = self.columns.get(handle)?;
        let parent = match fields.parent {
            None => None,
            Some(parent) => Some(self.ids.stable_for(parent)?),
        };
        Some(NodeRecord {
            id,
            parent,
            order_key: fields.order_key,
            visible: fields.visible,
            locked: fields.locked,
            display_name: fields.display_name,
            payload: self.payloads.get(&id)?.clone(),
        })
    }

    /// 영속 레코드에서 런타임 모델을 만듭니다(VI.10.3).
    ///
    /// 1. 전체 validator(VI.10.1)를 먼저 실행해 위반이 있으면 모델을 만들지 않습니다.
    /// 2. View 정의 순서로 각 root부터 pre-order(형제는 OrderKey, StableId 순)로 node를 넣어
    ///    부모가 항상 먼저 할당되게 합니다 — 새 runtime 핸들, ID registry, 부모·자식 순서가
    ///    입력 레코드 순서와 무관하게 결정론적으로 재구축됩니다.
    /// 3. modifier stack을 소유 node에 붙입니다.
    ///
    /// 유효한 문서의 미해결 참조 진단을 함께 반환합니다.
    pub fn from_records(
        records: DocumentRecords,
    ) -> Result<(DocumentModel, DocumentDiagnostics), LoadModelError> {
        let diagnostics = validate_document(&records).map_err(LoadModelError::Invalid)?;
        let DocumentRecords {
            definition,
            nodes,
            modifier_stacks,
        } = records;
        let roots: Vec<StableId> = definition
            .pages
            .iter()
            .flat_map(|p| p.views.iter().map(|v| v.root_frame))
            .collect();
        let mut model = DocumentModel::new(definition);

        let mut by_id: BTreeMap<StableId, NodeRecord> = BTreeMap::new();
        let mut children: BTreeMap<StableId, Vec<NodeRecord>> = BTreeMap::new();
        for node in nodes {
            match node.parent {
                Some(parent) => children.entry(parent).or_default().push(node),
                None => {
                    by_id.insert(node.id, node);
                }
            }
        }
        for kids in children.values_mut() {
            kids.sort_by(|a, b| (&a.order_key, a.id).cmp(&(&b.order_key, b.id)));
        }
        for root in roots {
            let Some(record) = by_id.remove(&root) else {
                continue;
            };
            let mut stack: Vec<NodeRecord> = vec![record];
            while let Some(record) = stack.pop() {
                let id = record.id;
                model.insert_node(record).map_err(LoadModelError::Model)?;
                if let Some(kids) = children.remove(&id) {
                    stack.extend(kids.into_iter().rev());
                }
            }
        }
        if let Some((id, _)) = by_id.into_iter().next() {
            return Err(LoadModelError::Model(ModelError::Inconsistent {
                id,
                reason: "View root가 아닌 부모 없는 node",
            }));
        }
        if let Some((parent, _)) = children.into_iter().next() {
            return Err(LoadModelError::Model(ModelError::Inconsistent {
                id: parent,
                reason: "root에서 닿지 않는 자식",
            }));
        }
        for stack in modifier_stacks {
            model
                .set_modifier_stack(stack)
                .map_err(LoadModelError::Model)?;
        }
        Ok((model, diagnostics))
    }

    /// 전체 영속 레코드를 만듭니다(VI.10.2). runtime 핸들은 StableId로 바뀌어 결과에
    /// 남지 않으며, 결과는 canonical 순서(node·stack 모두 StableId 순)입니다.
    pub fn to_records(&self) -> Result<DocumentRecords, ModelError> {
        let mut records = DocumentRecords::new(self.definition.clone());
        for handle in self.columns.iter_live() {
            let id = self
                .ids
                .stable_for(handle)
                .ok_or(ModelError::Inconsistent {
                    id: StableId::NIL,
                    reason: "registry에 없는 live 핸들",
                })?;
            let record = self.record(id).ok_or(ModelError::Inconsistent {
                id,
                reason: "부모 핸들 또는 payload가 없음",
            })?;
            records.nodes.push(record);
        }
        records.modifier_stacks = self.modifiers.values().cloned().collect();
        records.sort_canonical();
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::{PageDefinition, ViewDefinition};
    use crate::fragment::FragmentPayload;
    use crate::frame::FramePayload;
    use crate::modifier::{ModifierId, ModifierKind};
    use crate::node_kind::NodeKind;
    use crate::text::TextPayload;
    use stellar_core_types::order_key::OrderKey;

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn definition() -> DocumentDefinition {
        let mut definition = DocumentDefinition::new(sid(1), String::new());
        let mut page = PageDefinition::new(sid(2), String::new());
        page.add_view(ViewDefinition::new(sid(3), String::new(), sid(10)))
            .unwrap();
        definition.add_page(page).unwrap();
        definition
    }

    fn record(id: u64, parent: Option<u64>, key: u8, payload: NodePayload) -> NodeRecord {
        NodeRecord {
            id: sid(id),
            parent: parent.map(sid),
            order_key: OrderKey::from_bytes(vec![key]).unwrap(),
            visible: true,
            locked: false,
            display_name: format!("n{id}"),
            payload,
        }
    }

    fn frame() -> NodePayload {
        NodePayload::Frame(FramePayload::default())
    }

    #[test]
    fn reference_edges_follow_payload_stack_and_owner_lifetime() {
        use crate::modifier::ModifierPayload;
        use stellar_core_types::reference::NodeRef;

        let clip = |id| {
            NodePayload::Frame(FramePayload {
                clipping: Clipping::Path(NodeRef::new(id)),
                ..FramePayload::default()
            })
        };
        let owner = sid(11);
        let first = sid(50);
        let second = sid(51);
        let mut model = DocumentModel::new(definition());
        model.insert_node(record(10, None, 1, frame())).unwrap();
        model
            .insert_node(record(11, Some(10), 1, clip(first)))
            .unwrap();
        let mut stack = ModifierStack::new(owner);
        let modifier = ModifierId::new(sid(20));
        stack.push_back(modifier, ModifierKind::Mask).unwrap();
        stack
            .set_payload(
                modifier,
                ModifierPayload::Mask {
                    source: NodeRef::new(first),
                },
            )
            .unwrap();
        stack.set_enabled(modifier, false).unwrap();
        model.set_modifier_stack(stack).unwrap();
        assert_eq!(model.references().edge_count(), 1);
        model.set_payload(owner, clip(second)).unwrap();
        assert_eq!(model.references().outbound(owner), vec![first, second]);
        let (loaded, _) = DocumentModel::from_records(model.to_records().unwrap()).unwrap();
        assert_eq!(loaded.references(), model.references());
        model.set_modifier_stack(ModifierStack::new(owner)).unwrap();
        assert!(!model.references().has_inbound(first));
        assert_eq!(model.references().inbound(second), vec![owner]);
        assert!(model
            .set_payload(owner, NodePayload::Fragment(FragmentPayload))
            .is_err());
        assert_eq!(model.references().inbound(second), vec![owner]);
        model.remove_leaf(owner).unwrap();
        model
            .insert_node(record(11, Some(10), 1, clip(StableId::NIL)))
            .unwrap();
        assert_eq!(model.references().edge_count(), 0);
    }

    #[test]
    fn to_records_replaces_parent_handles_with_stable_ids() {
        let mut model = DocumentModel::new(definition());
        model.insert_node(record(10, None, 0x10, frame())).unwrap();
        model
            .insert_node(record(12, Some(10), 0x20, frame()))
            .unwrap();
        model
            .insert_node(record(
                11,
                Some(10),
                0x10,
                NodePayload::Text(TextPayload::empty(0)),
            ))
            .unwrap();
        let records = model.to_records().unwrap();
        let ids: Vec<StableId> = records.nodes.iter().map(|n| n.id).collect();
        assert_eq!(ids, vec![sid(10), sid(11), sid(12)], "canonical 순서");
        assert_eq!(records.nodes[0].parent, None);
        assert_eq!(records.nodes[1].parent, Some(sid(10)));
        assert_eq!(records.nodes[1].kind(), NodeKind::Text);
        assert_eq!(model.children_in_order(sid(10)), vec![sid(11), sid(12)]);
        assert_eq!(
            model.stable_id(model.runtime_id(sid(12)).unwrap()),
            Some(sid(12))
        );
    }

    #[test]
    fn runtime_history_does_not_change_records() {
        // 같은 의미 문서를 다른 삽입 순서·삭제 이력으로 만들면 runtime 핸들은 다르지만
        // 레코드는 같습니다.
        let mut a = DocumentModel::new(definition());
        a.insert_node(record(10, None, 0x10, frame())).unwrap();
        a.insert_node(record(11, Some(10), 0x10, frame())).unwrap();
        a.insert_node(record(12, Some(10), 0x20, frame())).unwrap();

        let mut b = DocumentModel::new(definition());
        for _ in 0..5 {
            b.insert_node(record(99, None, 0x10, frame())).unwrap();
            b.remove_leaf(sid(99)).unwrap();
        }
        b.insert_node(record(10, None, 0x10, frame())).unwrap();
        b.insert_node(record(12, Some(10), 0x20, frame())).unwrap();
        b.insert_node(record(11, Some(10), 0x10, frame())).unwrap();

        assert_ne!(a.runtime_id(sid(10)), b.runtime_id(sid(10)), "핸들은 다름");
        assert_eq!(a.to_records().unwrap(), b.to_records().unwrap());
    }

    #[test]
    fn insert_and_remove_errors() {
        let mut model = DocumentModel::new(definition());
        assert_eq!(
            model.insert_node(record(11, Some(10), 0x10, frame())),
            Err(ModelError::MissingParent {
                id: sid(11),
                parent: sid(10)
            })
        );
        model.insert_node(record(10, None, 0x10, frame())).unwrap();
        assert_eq!(
            model.insert_node(record(10, None, 0x20, frame())),
            Err(ModelError::DuplicateNode { id: sid(10) })
        );
        let mut nil = record(10, None, 0x10, frame());
        nil.id = StableId::NIL;
        assert_eq!(model.insert_node(nil), Err(ModelError::NilNodeId));
        model
            .insert_node(record(11, Some(10), 0x10, frame()))
            .unwrap();
        assert_eq!(
            model.remove_leaf(sid(10)),
            Err(ModelError::HasChildren { id: sid(10) })
        );
        assert_eq!(model.remove_leaf(sid(11)).unwrap().id, sid(11));
        assert_eq!(
            model.remove_leaf(sid(11)),
            Err(ModelError::UnknownNode { id: sid(11) })
        );
        assert_eq!(
            model.set_payload(sid(10), NodePayload::Fragment(FragmentPayload)),
            Err(ModelError::PayloadKindChange { id: sid(10) })
        );
        model.set_display_name(sid(10), "새 이름".into()).unwrap();
        assert_eq!(model.record(sid(10)).unwrap().display_name, "새 이름");
    }

    #[test]
    fn modifier_stack_rules() {
        let mut model = DocumentModel::new(definition());
        model.insert_node(record(10, None, 0x10, frame())).unwrap();
        model
            .insert_node(record(
                11,
                Some(10),
                0x10,
                NodePayload::Fragment(FragmentPayload),
            ))
            .unwrap();
        // Fragment modifier 거부(VI.9.6).
        let mut stack = ModifierStack::new(sid(11));
        stack
            .push_back(ModifierId::new(sid(50)), ModifierKind::Opacity)
            .unwrap();
        assert_eq!(
            model.set_modifier_stack(stack),
            Err(ModelError::Modifier {
                owner: sid(11),
                error: ModifierError::FragmentOwner {
                    kind: ModifierKind::Opacity
                }
            })
        );
        // Frame은 허용, 빈 stack은 제거.
        let mut stack = ModifierStack::new(sid(10));
        stack
            .push_back(ModifierId::new(sid(51)), ModifierKind::Blur)
            .unwrap();
        model.set_modifier_stack(stack).unwrap();
        assert!(model.modifier_stack(sid(10)).is_some());
        assert_eq!(model.to_records().unwrap().modifier_stacks.len(), 1);
        model
            .set_modifier_stack(ModifierStack::new(sid(10)))
            .unwrap();
        assert!(model.modifier_stack(sid(10)).is_none());
        assert_eq!(
            model.set_modifier_stack(ModifierStack::new(sid(77))),
            Err(ModelError::UnknownNode { id: sid(77) })
        );
    }

    #[test]
    fn from_records_rebuilds_registry_and_child_order() {
        // VI.10.3: ID registry와 parent order 재구축.
        let mut model = DocumentModel::new(definition());
        model.insert_node(record(10, None, 0x10, frame())).unwrap();
        model
            .insert_node(record(13, Some(10), 0x30, frame()))
            .unwrap();
        model
            .insert_node(record(11, Some(10), 0x10, frame()))
            .unwrap();
        model
            .insert_node(record(
                12,
                Some(13),
                0x20,
                NodePayload::Fragment(FragmentPayload),
            ))
            .unwrap();
        let mut stack = ModifierStack::new(sid(11));
        stack
            .push_back(ModifierId::new(sid(60)), ModifierKind::Opacity)
            .unwrap();
        model.set_modifier_stack(stack).unwrap();
        let records = model.to_records().unwrap();

        // 입력 레코드 순서를 뒤집어도 같은 결과.
        let mut reversed = records.clone();
        reversed.nodes.reverse();
        let (loaded, diagnostics) = DocumentModel::from_records(reversed).unwrap();
        assert!(diagnostics.unresolved.is_empty());
        assert_eq!(loaded.to_records().unwrap(), records);
        assert_eq!(loaded.node_count(), 4);
        for id in [10, 11, 12, 13] {
            let handle = loaded.runtime_id(sid(id)).expect("registry 재구축");
            assert_eq!(loaded.stable_id(handle), Some(sid(id)));
        }
        assert_eq!(loaded.children_in_order(sid(10)), vec![sid(11), sid(13)]);
        assert_eq!(loaded.children_in_order(sid(13)), vec![sid(12)]);
        assert!(loaded.modifier_stack(sid(11)).is_some());
    }

    #[test]
    fn from_records_rejects_invalid_documents() {
        let mut model = DocumentModel::new(definition());
        model.insert_node(record(10, None, 0x10, frame())).unwrap();
        model
            .insert_node(record(11, Some(10), 0x10, frame()))
            .unwrap();
        let mut records = model.to_records().unwrap();
        records.nodes[1].order_key = records.nodes[0].order_key.clone();
        records.nodes.push(record(12, Some(10), 0x10, frame()));
        match DocumentModel::from_records(records) {
            Err(LoadModelError::Invalid(err)) => assert_eq!(err.violations.len(), 1),
            other => panic!("검증 실패여야 함: {other:?}"),
        }
    }
}

//! 에이전트 승인 batch 기록: 역 transaction 보존과 값 지문 (XIX.6.1, XIX.6.7, ADR-0100).
//!
//! 승인된 에이전트 batch(CommandTransaction 하나)를 적용하면 [`AgentBatchRecord::capture`]가 다음을 남깁니다.
//!
//! - 정방향 transaction과 적용 결과의 역 transaction([`AppliedTransaction::inverse`]). 문서 사본을 만들지 않으므로
//!   기록 크기는 batch 크기에 비례합니다(full document clone 금지).
//! - 적용 결과 change set(VII.1.4)과 생성·삭제 node.
//! - 지문 key([`PropertyPath`])별 적용 직후 값 지문([`ValueFingerprint`]).
//!
//! 지문 key는 change set에서 정합니다.
//!
//! - 바뀐 속성 경로는 그대로 key입니다. root가 node가 아니면(modifier 속성) 소유 node를 change set의 바뀐
//!   node에서 찾아 함께 기록합니다(`owner_hint`).
//! - 생성·삭제 node와, 속성 경로 없이 구조만 기록된 node(본문·path 편집, 자식 목록 변경)는 node 레코드 전체
//!   key(`PropertyPath::new(node)`)입니다. 어떤 command도 node root만 있는 경로를 change set에 기록하지 않으므로
//!   속성 key와 섞이지 않습니다.
//!
//! 지문은 같은 프로세스 안에서만 비교하는 세션 메모리 값입니다(저장·직렬화하지 않음). canonical bytes(속성 값
//! codec, node 레코드 codec, modifier stack·layout record·문서 정의 encode)의 128-bit hash입니다.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{DefaultHasher, Hasher};

use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierId, ModifierStack};
use stellar_document::record_codec::encode_node_record;

use crate::change::{node_keys, ChangeSet};
use crate::codec::Writer;
use crate::property::read_property;
use crate::transaction::{AppliedTransaction, CommandTransaction};

/// 에이전트 batch 식별자: 에이전트 세션 id와 세션 안 batch 순번(1부터).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentBatchId {
    /// 에이전트 세션 id(AgentSessionId).
    pub session: StableId,
    /// 세션 안 batch 순번.
    pub seq: u32,
}

/// 값 지문. `Absent`는 그 시점에 대상(node·modifier·문서)이 없음입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ValueFingerprint {
    /// 대상이 없습니다.
    Absent,
    /// canonical bytes hash.
    Present(u128),
}

impl ValueFingerprint {
    fn of(tag: &str, parts: &[&[u8]]) -> ValueFingerprint {
        let mut high = DefaultHasher::new();
        let mut low = DefaultHasher::new();
        high.write_u8(0x5a);
        low.write_u8(0xa5);
        for hasher in [&mut high, &mut low] {
            hasher.write(tag.as_bytes());
            for part in parts {
                hasher.write_usize(part.len());
                hasher.write(part);
            }
        }
        ValueFingerprint::Present((u128::from(high.finish()) << 64) | u128::from(low.finish()))
    }
}

/// 지문 key가 node 레코드 전체 key인지(root만 있는 경로이고 root가 node).
pub fn is_record_key(model: &DocumentModel, path: &PropertyPath) -> bool {
    path.keys().is_empty() && path.tail().is_none() && model.contains(path.root())
}

/// node 레코드 전체 key.
pub fn record_key(node: StableId) -> PropertyPath {
    PropertyPath::new(node)
}

fn modifier_entry_bytes(stack: &ModifierStack, modifier: StableId) -> Option<Vec<u8>> {
    let entry = stack
        .entries()
        .iter()
        .find(|e| e.id == ModifierId::new(modifier))?;
    let mut single = ModifierStack::new(stack.node);
    single.insert(entry.clone()).ok()?;
    Some(single.encode())
}

fn node_record_fingerprint(model: &DocumentModel, node: StableId) -> ValueFingerprint {
    let Some(record) = model.record(node) else {
        return ValueFingerprint::Absent;
    };
    let record = encode_node_record(&record);
    let stack = model
        .modifier_stack(node)
        .map(ModifierStack::encode)
        .unwrap_or_default();
    let layout = model
        .layout_record(node)
        .map(|r| r.encode())
        .unwrap_or_default();
    ValueFingerprint::of("record", &[&record, &stack, &layout])
}

fn definition_fingerprint(model: &DocumentModel) -> ValueFingerprint {
    let definition = model.definition().encode();
    ValueFingerprint::of(
        "definition",
        &[&definition, model.style_defaults().as_bytes()],
    )
}

fn defines(model: &DocumentModel, id: StableId) -> bool {
    let definition = model.definition();
    definition.id == id
        || definition
            .pages
            .iter()
            .any(|page| page.id == id || page.views.iter().any(|view| view.id == id))
}

/// 지문 key 하나의 현재 값 지문. `owner`는 root가 node가 아닐 때(modifier 속성) 소유 node입니다.
///
/// - node root, key 하나: 등록된 속성이면 그 값(부모·순서 포함), 아니면(layout longhand 등) node 레코드 전체.
/// - node root만: node 레코드 전체(레코드·modifier stack·layout record).
/// - modifier root: 소유 node stack 안의 그 modifier 항목 하나.
/// - 문서·페이지·View root: 문서 정의와 기본값 세트 이름.
pub fn fingerprint(
    model: &DocumentModel,
    path: &PropertyPath,
    owner: Option<StableId>,
) -> ValueFingerprint {
    let root = path.root();
    if let Some(record) = model.record(root) {
        let [key] = path.keys() else {
            return node_record_fingerprint(model, root);
        };
        if path.tail().is_some() {
            return node_record_fingerprint(model, root);
        }
        let mut out = Writer::new();
        match *key {
            node_keys::PARENT => out.opt_id(record.parent),
            node_keys::ORDER_KEY => out.bytes(record.order_key.as_bytes()),
            key => match read_property(&record, key) {
                Some(value) => value.encode_into(&mut out),
                None => return node_record_fingerprint(model, root),
            },
        }
        return ValueFingerprint::of("property", &[&path.to_canonical_bytes(), &out.into_bytes()]);
    }
    if defines(model, root) {
        return definition_fingerprint(model);
    }
    let Some(owner) = owner else {
        return ValueFingerprint::Absent;
    };
    match model
        .modifier_stack(owner)
        .and_then(|stack| modifier_entry_bytes(stack, root))
    {
        Some(entry) => ValueFingerprint::of("modifier", &[&path.to_canonical_bytes(), &entry]),
        None => ValueFingerprint::Absent,
    }
}

/// 적용 결과 change set의 지문 key와 소유 node(modifier 속성만). 모듈 문서 규칙을 따릅니다.
pub fn fingerprint_keys(
    changes: &ChangeSet,
    after: &DocumentModel,
) -> BTreeMap<PropertyPath, Option<StableId>> {
    let changed = changes.changed_nodes();
    let mut keys: BTreeMap<PropertyPath, Option<StableId>> = BTreeMap::new();
    let mut described: BTreeSet<StableId> = BTreeSet::new();
    for path in changes.changed_properties() {
        let root = path.root();
        let owner = if after.contains(root) || defines(after, root) {
            None
        } else {
            changed.iter().copied().find(|node| {
                after.modifier_stack(*node).is_some_and(|stack| {
                    stack
                        .entries()
                        .iter()
                        .any(|e| e.id == ModifierId::new(root))
                })
            })
        };
        described.insert(owner.unwrap_or(root));
        keys.insert(path, owner);
    }
    for node in changes
        .inserted_nodes()
        .into_iter()
        .chain(changes.removed_nodes())
    {
        keys.insert(record_key(node), None);
    }
    for node in changed {
        if !described.contains(&node) {
            keys.entry(record_key(node)).or_insert(None);
        }
    }
    keys
}

/// 승인 batch 하나의 기록(세션 메모리 전용, 직렬화 경로 없음).
#[derive(Debug, Clone, PartialEq)]
pub struct AgentBatchRecord {
    /// batch id.
    pub id: AgentBatchId,
    /// 적용한 정방향 transaction(출처 `FutureAgent`).
    pub forward: CommandTransaction,
    /// 효과 전체를 되돌리는 역 transaction.
    pub inverse: CommandTransaction,
    /// 적용 결과 change set.
    pub changes: ChangeSet,
    /// 생성된 node(순 효과).
    pub created: BTreeSet<StableId>,
    /// 삭제된 node(순 효과).
    pub deleted: BTreeSet<StableId>,
    /// 지문 key별 적용 직후 값 지문.
    pub fingerprints: BTreeMap<PropertyPath, ValueFingerprint>,
    /// modifier 속성 key의 소유 node.
    pub owners: BTreeMap<PropertyPath, StableId>,
}

impl AgentBatchRecord {
    /// 적용 직후 `after` 문서에서 기록을 만듭니다. `applied`는 `forward`를 실행한 결과입니다.
    pub fn capture(
        id: AgentBatchId,
        forward: CommandTransaction,
        applied: AppliedTransaction,
        after: &DocumentModel,
    ) -> AgentBatchRecord {
        let keys = fingerprint_keys(&applied.changes, after);
        let mut fingerprints = BTreeMap::new();
        let mut owners = BTreeMap::new();
        for (path, owner) in keys {
            fingerprints.insert(path.clone(), fingerprint(after, &path, owner));
            if let Some(owner) = owner {
                owners.insert(path, owner);
            }
        }
        AgentBatchRecord {
            id,
            forward,
            created: applied.changes.inserted_nodes().into_iter().collect(),
            deleted: applied.changes.removed_nodes().into_iter().collect(),
            inverse: applied.inverse,
            changes: applied.changes,
            fingerprints,
            owners,
        }
    }

    /// key의 현재 값 지문(소유 node 정보를 씀).
    pub fn current(&self, model: &DocumentModel, path: &PropertyPath) -> ValueFingerprint {
        fingerprint(model, path, self.owners.get(path).copied())
    }

    /// 현재 값이 적용 직후 값과 다른 key.
    pub fn diverged(&self, model: &DocumentModel) -> Vec<PropertyPath> {
        self.fingerprints
            .iter()
            .filter(|(path, recorded)| self.current(model, path) != **recorded)
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// 바뀐 node(속성 key의 소유 node 포함, 삭제 node 포함).
    pub fn touched_nodes(&self) -> BTreeSet<StableId> {
        let mut nodes: BTreeSet<StableId> = self.changes.changed_nodes().into_iter().collect();
        nodes.extend(self.deleted.iter().copied());
        nodes.extend(self.owners.values().copied());
        nodes
    }
}

#[cfg(test)]
#[path = "agent_batch_tests.rs"]
mod tests;

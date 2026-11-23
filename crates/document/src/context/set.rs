//! 문서 하나의 Context 요소 모음 `ContextSet`(XXX.7.1·XXX.7.2, ADR-0099 결정 1·2).
//!
//! `ContextKey`, `ContextProvision`, 컴포넌트의 `ContextRequirement`(컴포넌트 StableId → key 집합), Slot의 공개 Context key
//! 목록(Slot StableId → key 집합, ADR-0025 결정 3.11)을 한 resource로 둡니다. 순회·직렬화는 모두 StableId 오름차순입니다.
//!
//! 구조 규칙(외부 상태 없이 검사):
//!
//! - key·provision StableId는 set 전체에서 유일합니다.
//! - key 이름은 같은 scope 안에서 유일합니다(서로 다른 scope는 같은 이름 가능, 토큰과 같은 규칙).
//! - 한 provider node에는 key마다 provision이 하나뿐입니다.
//! - 존재하지 않는 key를 가리키는 provision·requirement·공개 목록 항목은 set에 둘 수 있습니다(외부 라이브러리 key이거나
//!   삭제된 key). 판정은 validator(XXX.7.5)의 orphan 진단입니다.
//!
//! canonical bytes(ADR-0099 결정 2):
//!
//! ```text
//! set      := version:u16 LE (=1) ++ count:u32 ++ record* ++ 0x01 ++ members(requirement) ++ 0x01 ++ members(slot)
//! record   := kind:u8 (0x01 key | 0x02 provision) ++ bytes(record)       key 먼저, 각 StableId 순
//! members  := count:u32 ++ (owner[16] ++ count:u32 ++ key[16]*)*         owner·key StableId 순, 빈 집합은 쓰지 않음
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::reserved::context::{
    COMPONENT_SECTION_CONTEXT_REQUIREMENTS, RECORD_CONTEXT_KEY, RECORD_CONTEXT_PROVISION,
    SLOT_SECTION_CONTEXT_KEYS,
};

use super::key::{ContextKey, ContextKeyError, ContextScope};
use super::provision::{ContextOwner, ContextProvision, ContextProvisionError};
use crate::component::codec::{put_bytes, put_count, put_id, put_u16, put_u8, Malformed, Reader};
use crate::model::DocumentModel;

/// canonical 형식 version.
pub const CONTEXT_SET_FORMAT_VERSION: u16 = 1;

/// set 편집·디코딩 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextSetError {
    /// 같은 StableId가 이미 있습니다.
    DuplicateId {
        /// id.
        id: StableId,
    },
    /// 같은 scope에 같은 이름의 key가 있습니다.
    DuplicateName {
        /// scope.
        scope: ContextScope,
        /// 이름.
        name: String,
    },
    /// provider에 같은 key provision이 이미 있습니다.
    DuplicateProvision {
        /// provider node.
        provider: StableId,
        /// key.
        key: StableId,
    },
    /// key가 없습니다.
    UnknownKey {
        /// key.
        key: StableId,
    },
    /// provision이 없습니다.
    UnknownProvision {
        /// provision.
        provision: StableId,
    },
    /// key 오류.
    Key(ContextKeyError),
    /// provision 오류.
    Provision(ContextProvisionError),
    /// canonical bytes가 잘못됐습니다.
    Malformed(&'static str),
}

impl fmt::Display for ContextSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContextSetError::DuplicateId { id } => write!(f, "Context id {id}가 이미 있습니다"),
            ContextSetError::DuplicateName { scope, name } => {
                write!(f, "{scope}에 ContextKey 이름 {name:?}이 이미 있습니다")
            }
            ContextSetError::DuplicateProvision { provider, key } => {
                write!(f, "node {provider}에 key {key} provision이 이미 있습니다")
            }
            ContextSetError::UnknownKey { key } => write!(f, "ContextKey {key}가 없습니다"),
            ContextSetError::UnknownProvision { provision } => {
                write!(f, "ContextProvision {provision}이 없습니다")
            }
            ContextSetError::Key(error) => write!(f, "{error}"),
            ContextSetError::Provision(error) => write!(f, "{error}"),
            ContextSetError::Malformed(reason) => write!(f, "context-set bytes: {reason}"),
        }
    }
}

impl std::error::Error for ContextSetError {}

impl From<Malformed> for ContextSetError {
    fn from(value: Malformed) -> ContextSetError {
        ContextSetError::Malformed(value.0)
    }
}

/// 문서 하나의 Context 요소.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContextSet {
    keys: BTreeMap<StableId, ContextKey>,
    provisions: BTreeMap<StableId, ContextProvision>,
    by_provider: BTreeMap<(StableId, StableId), StableId>,
    requirements: BTreeMap<StableId, BTreeSet<StableId>>,
    slot_keys: BTreeMap<StableId, BTreeSet<StableId>>,
}

static EMPTY: BTreeSet<StableId> = BTreeSet::new();

impl ContextSet {
    /// 빈 set.
    pub fn new() -> ContextSet {
        ContextSet::default()
    }

    /// 아무 요소도 없으면 `true`(저장 경로는 이때 아무것도 쓰지 않습니다).
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
            && self.provisions.is_empty()
            && self.requirements.is_empty()
            && self.slot_keys.is_empty()
    }

    fn id_taken(&self, id: StableId) -> bool {
        self.keys.contains_key(&id) || self.provisions.contains_key(&id)
    }

    // --- key ---

    /// key(StableId 순).
    pub fn keys(&self) -> impl Iterator<Item = &ContextKey> + '_ {
        self.keys.values()
    }

    /// key.
    pub fn key(&self, id: StableId) -> Option<&ContextKey> {
        self.keys.get(&id)
    }

    fn check_name(&self, key: &ContextKey) -> Result<(), ContextSetError> {
        if self
            .keys
            .values()
            .any(|other| other.id() != key.id() && other.scope() == key.scope() && other.name() == key.name())
        {
            return Err(ContextSetError::DuplicateName {
                scope: key.scope(),
                name: key.name().to_owned(),
            });
        }
        Ok(())
    }

    /// key를 추가합니다.
    pub fn insert_key(&mut self, key: ContextKey) -> Result<(), ContextSetError> {
        if self.id_taken(key.id()) {
            return Err(ContextSetError::DuplicateId { id: key.id() });
        }
        self.check_name(&key)?;
        self.keys.insert(key.id(), key);
        Ok(())
    }

    /// 같은 id의 key를 바꾸고 이전 key를 돌려줍니다(rename·default 변경).
    pub fn replace_key(&mut self, key: ContextKey) -> Result<ContextKey, ContextSetError> {
        if !self.keys.contains_key(&key.id()) {
            return Err(ContextSetError::UnknownKey { key: key.id() });
        }
        self.check_name(&key)?;
        Ok(self.keys.insert(key.id(), key).expect("checked"))
    }

    /// key만 지웁니다(그 key를 가리키는 provision·requirement·공개 목록 항목은 남습니다).
    pub fn remove_key(&mut self, id: StableId) -> Option<ContextKey> {
        self.keys.remove(&id)
    }

    // --- provision ---

    /// provision(StableId 순).
    pub fn provisions(&self) -> impl Iterator<Item = &ContextProvision> + '_ {
        self.provisions.values()
    }

    /// provision.
    pub fn provision(&self, id: StableId) -> Option<&ContextProvision> {
        self.provisions.get(&id)
    }

    /// provider node의 key provision.
    pub fn provision_at(&self, provider: StableId, key: StableId) -> Option<&ContextProvision> {
        self.by_provider
            .get(&(provider, key))
            .and_then(|id| self.provisions.get(id))
    }

    /// provider node에 붙은 provision(key 순).
    pub fn provisions_at(&self, provider: StableId) -> impl Iterator<Item = &ContextProvision> + '_ {
        self.by_provider
            .range((provider, StableId::NIL)..)
            .take_while(move |((p, _), _)| *p == provider)
            .filter_map(|(_, id)| self.provisions.get(id))
    }

    /// key를 공급하는 provision(StableId 순).
    pub fn provisions_of(&self, key: StableId) -> impl Iterator<Item = &ContextProvision> + '_ {
        self.provisions.values().filter(move |p| p.key() == key)
    }

    /// provision을 추가합니다.
    pub fn insert_provision(&mut self, provision: ContextProvision) -> Result<(), ContextSetError> {
        if self.id_taken(provision.id()) {
            return Err(ContextSetError::DuplicateId { id: provision.id() });
        }
        let slot = (provision.provider(), provision.key());
        if self.by_provider.contains_key(&slot) {
            return Err(ContextSetError::DuplicateProvision {
                provider: slot.0,
                key: slot.1,
            });
        }
        self.by_provider.insert(slot, provision.id());
        self.provisions.insert(provision.id(), provision);
        Ok(())
    }

    /// 같은 id provision의 값 식을 바꾸고 이전 provision을 돌려줍니다. provider·key·owner는 바꿀 수 없습니다.
    pub fn replace_provision(
        &mut self,
        provision: ContextProvision,
    ) -> Result<ContextProvision, ContextSetError> {
        let current = self
            .provisions
            .get(&provision.id())
            .ok_or(ContextSetError::UnknownProvision {
                provision: provision.id(),
            })?;
        if (current.provider(), current.key(), current.owner())
            != (provision.provider(), provision.key(), provision.owner())
        {
            return Err(ContextSetError::DuplicateProvision {
                provider: provision.provider(),
                key: provision.key(),
            });
        }
        Ok(self
            .provisions
            .insert(provision.id(), provision)
            .expect("checked"))
    }

    /// provision을 지웁니다.
    pub fn remove_provision(&mut self, id: StableId) -> Option<ContextProvision> {
        let provision = self.provisions.remove(&id)?;
        self.by_provider
            .remove(&(provision.provider(), provision.key()));
        Some(provision)
    }

    // --- requirement · Slot 공개 목록 ---

    /// 컴포넌트의 `ContextRequirement` key 집합.
    pub fn requirements(&self, component: StableId) -> &BTreeSet<StableId> {
        self.requirements.get(&component).unwrap_or(&EMPTY)
    }

    /// requirement를 둔 컴포넌트와 key 집합(컴포넌트 순).
    pub fn all_requirements(&self) -> impl Iterator<Item = (StableId, &BTreeSet<StableId>)> + '_ {
        self.requirements.iter().map(|(c, keys)| (*c, keys))
    }

    /// requirement를 더합니다. 이미 있으면 `false`.
    pub fn add_requirement(&mut self, component: StableId, key: StableId) -> bool {
        self.requirements.entry(component).or_default().insert(key)
    }

    /// requirement를 지웁니다. 없으면 `false`.
    pub fn remove_requirement(&mut self, component: StableId, key: StableId) -> bool {
        remove_member(&mut self.requirements, component, key)
    }

    /// Slot 공개 Context key 집합.
    pub fn slot_keys(&self, slot: StableId) -> &BTreeSet<StableId> {
        self.slot_keys.get(&slot).unwrap_or(&EMPTY)
    }

    /// 공개 목록을 둔 Slot과 key 집합(Slot 순).
    pub fn all_slot_keys(&self) -> impl Iterator<Item = (StableId, &BTreeSet<StableId>)> + '_ {
        self.slot_keys.iter().map(|(s, keys)| (*s, keys))
    }

    /// Slot 공개 목록에 key를 더합니다. 이미 있으면 `false`.
    pub fn add_slot_key(&mut self, slot: StableId, key: StableId) -> bool {
        self.slot_keys.entry(slot).or_default().insert(key)
    }

    /// Slot 공개 목록에서 key를 지웁니다. 없으면 `false`.
    pub fn remove_slot_key(&mut self, slot: StableId, key: StableId) -> bool {
        remove_member(&mut self.slot_keys, slot, key)
    }

    /// View owner provision 가운데 provider node가 모델에 없는 것을 뺀 영속 출력용 set(ADR-0099 결정 1). 삭제 undo로 node가
    /// 되돌아오면 모델의 set에는 provision이 그대로 남아 있어 다시 보입니다.
    pub fn live(&self, model: &DocumentModel) -> ContextSet {
        let mut out = self.clone();
        let dead: Vec<StableId> = self
            .provisions
            .values()
            .filter(|p| matches!(p.owner(), ContextOwner::View(_)) && !model.contains(p.provider()))
            .map(|p| p.id())
            .collect();
        for id in dead {
            out.remove_provision(id);
        }
        out
    }

    // --- canonical bytes ---

    /// canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        put_u16(&mut buf, CONTEXT_SET_FORMAT_VERSION);
        put_count(&mut buf, self.keys.len() + self.provisions.len());
        for key in self.keys.values() {
            put_u8(&mut buf, RECORD_CONTEXT_KEY);
            put_bytes(&mut buf, &key.encode());
        }
        for provision in self.provisions.values() {
            put_u8(&mut buf, RECORD_CONTEXT_PROVISION);
            put_bytes(&mut buf, &provision.encode());
        }
        put_u8(&mut buf, COMPONENT_SECTION_CONTEXT_REQUIREMENTS);
        put_members(&mut buf, &self.requirements);
        put_u8(&mut buf, SLOT_SECTION_CONTEXT_KEYS);
        put_members(&mut buf, &self.slot_keys);
        buf
    }

    /// canonical bytes에서 디코딩합니다. 다시 인코딩한 bytes가 입력과 같아야 합니다.
    pub fn decode(bytes: &[u8]) -> Result<ContextSet, ContextSetError> {
        let mut reader = Reader::new(bytes);
        if reader.u16()? != CONTEXT_SET_FORMAT_VERSION {
            return Err(ContextSetError::Malformed("지원하지 않는 context-set version"));
        }
        let count = reader.count()?;
        let mut set = ContextSet::new();
        for _ in 0..count {
            match reader.u8()? {
                RECORD_CONTEXT_KEY => {
                    let key = ContextKey::decode(reader.bytes()?).map_err(ContextSetError::Key)?;
                    set.insert_key(key)?;
                }
                RECORD_CONTEXT_PROVISION => {
                    let provision = ContextProvision::decode(reader.bytes()?)
                        .map_err(ContextSetError::Provision)?;
                    set.insert_provision(provision)?;
                }
                _ => return Err(ContextSetError::Malformed("알 수 없는 context-set record kind")),
            }
        }
        if reader.u8()? != COMPONENT_SECTION_CONTEXT_REQUIREMENTS {
            return Err(ContextSetError::Malformed("requirement section이 없습니다"));
        }
        set.requirements = read_members(&mut reader)?;
        if reader.u8()? != SLOT_SECTION_CONTEXT_KEYS {
            return Err(ContextSetError::Malformed("Slot section이 없습니다"));
        }
        set.slot_keys = read_members(&mut reader)?;
        reader.expect_end()?;
        if set.encode() != bytes {
            return Err(ContextSetError::Malformed("canonical bytes가 아닙니다"));
        }
        Ok(set)
    }
}

fn remove_member(
    map: &mut BTreeMap<StableId, BTreeSet<StableId>>,
    owner: StableId,
    key: StableId,
) -> bool {
    let Some(keys) = map.get_mut(&owner) else {
        return false;
    };
    let removed = keys.remove(&key);
    if keys.is_empty() {
        map.remove(&owner);
    }
    removed
}

fn put_members(buf: &mut Vec<u8>, members: &BTreeMap<StableId, BTreeSet<StableId>>) {
    put_count(buf, members.len());
    for (owner, keys) in members {
        put_id(buf, *owner);
        put_count(buf, keys.len());
        for key in keys {
            put_id(buf, *key);
        }
    }
}

fn read_members(
    reader: &mut Reader<'_>,
) -> Result<BTreeMap<StableId, BTreeSet<StableId>>, ContextSetError> {
    let count = reader.count()?;
    let mut out = BTreeMap::new();
    for _ in 0..count {
        let owner = reader.id()?;
        let len = reader.count()?;
        if len == 0 {
            return Err(ContextSetError::Malformed("빈 key 목록은 쓰지 않습니다"));
        }
        let mut keys = BTreeSet::new();
        for _ in 0..len {
            keys.insert(reader.id()?);
        }
        if out.insert(owner, keys).is_some() {
            return Err(ContextSetError::DuplicateId { id: owner });
        }
    }
    Ok(out)
}

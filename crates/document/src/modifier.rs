//! Modifier stack (VI.9).
//!
//! modifier는 node payload와 **분리 저장**되는 순서 있는 목록입니다. 각 modifier는
//! 고유 [`ModifierId`](StableId newtype)를 갖고, 순서는 [`OrderKey`](III.5)로
//! 정합니다 — 삽입·재정렬이 다른 modifier의 id·key를 바꾸지 않아 semantic merge에
//! 유리합니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;

/// modifier의 안정 식별자. node StableId와 타입이 섞이지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModifierId(StableId);

impl ModifierId {
    /// `StableId`로부터 만듭니다.
    pub const fn new(id: StableId) -> ModifierId {
        ModifierId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }
}

/// modifier stack 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModifierError {
    /// 같은 ModifierId가 이미 있습니다.
    DuplicateId {
        /// 중복 id.
        id: ModifierId,
    },
    /// 같은 OrderKey가 이미 있습니다(순서 모호).
    DuplicateOrderKey,
    /// 대상 modifier가 없습니다.
    NotFound {
        /// 요청 id.
        id: ModifierId,
    },
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for ModifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModifierError::DuplicateId { id } => write!(f, "modifier id 중복: {}", id.0),
            ModifierError::DuplicateOrderKey => f.write_str("modifier OrderKey 중복"),
            ModifierError::NotFound { id } => write!(f, "modifier가 없습니다: {}", id.0),
            ModifierError::Malformed { reason } => {
                write!(f, "Modifier binary 형식 위반: {reason}")
            }
        }
    }
}

impl std::error::Error for ModifierError {}

/// stack의 한 항목(id + 순서 key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModifierEntry {
    /// 고유 id.
    pub id: ModifierId,
    /// 순서 key.
    pub order: OrderKey,
}

/// node 하나의 modifier stack. node payload와 별도로 저장됩니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModifierStack {
    /// 소유 node.
    pub node: StableId,
    /// OrderKey 오름차순으로 유지되는 항목.
    entries: Vec<ModifierEntry>,
}

impl ModifierStack {
    /// 빈 stack.
    pub fn new(node: StableId) -> ModifierStack {
        ModifierStack {
            node,
            entries: Vec::new(),
        }
    }

    /// 순서대로 정렬된 항목.
    pub fn entries(&self) -> &[ModifierEntry] {
        &self.entries
    }

    /// 순서대로 id 목록.
    pub fn ids_in_order(&self) -> Vec<ModifierId> {
        self.entries.iter().map(|e| e.id).collect()
    }

    /// 항목을 추가합니다. id·OrderKey 중복은 거부합니다. 입력 순서와 무관하게
    /// OrderKey 순으로 유지합니다.
    pub fn insert(&mut self, entry: ModifierEntry) -> Result<(), ModifierError> {
        if self.entries.iter().any(|e| e.id == entry.id) {
            return Err(ModifierError::DuplicateId { id: entry.id });
        }
        if self.entries.iter().any(|e| e.order == entry.order) {
            return Err(ModifierError::DuplicateOrderKey);
        }
        let at = self.entries.partition_point(|e| e.order < entry.order);
        self.entries.insert(at, entry);
        Ok(())
    }

    /// 맨 뒤(가장 나중 적용)에 새 modifier를 추가하고 그 key를 반환합니다.
    pub fn push_back(&mut self, id: ModifierId) -> Result<OrderKey, ModifierError> {
        let order = OrderKey::between(self.entries.last().map(|e| &e.order), None);
        self.insert(ModifierEntry {
            id,
            order: order.clone(),
        })?;
        Ok(order)
    }

    /// 항목을 제거합니다.
    pub fn remove(&mut self, id: ModifierId) -> Result<ModifierEntry, ModifierError> {
        let index = self
            .entries
            .iter()
            .position(|e| e.id == id)
            .ok_or(ModifierError::NotFound { id })?;
        Ok(self.entries.remove(index))
    }

    /// canonical bytes: node id + 항목 수 + (id 16 + key 길이 u32 + key bytes)*.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.node.to_bytes());
        buf.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for entry in &self.entries {
            buf.extend_from_slice(&entry.id.0.to_bytes());
            let key = entry.order.as_bytes();
            buf.extend_from_slice(&(key.len() as u32).to_le_bytes());
            buf.extend_from_slice(key);
        }
        buf
    }

    /// bytes에서 복원합니다. 중복·정렬 위반은 거부합니다.
    pub fn decode(bytes: &[u8]) -> Result<ModifierStack, ModifierError> {
        let mut pos = 0usize;
        let mut take = |n: usize| -> Result<&[u8], ModifierError> {
            if bytes.len() - pos < n {
                return Err(ModifierError::Malformed {
                    reason: "입력 부족",
                });
            }
            let slice = &bytes[pos..pos + n];
            pos += n;
            Ok(slice)
        };
        let id_of = |s: &[u8]| {
            let mut b = [0u8; 16];
            b.copy_from_slice(s);
            StableId::from_bytes(b)
        };
        let node = id_of(take(16)?);
        let count = u32::from_le_bytes(take(4)?.try_into().expect("4바이트")) as usize;
        let mut stack = ModifierStack::new(node);
        let mut previous: Option<OrderKey> = None;
        for _ in 0..count {
            let id = ModifierId(id_of(take(16)?));
            let key_len = u32::from_le_bytes(take(4)?.try_into().expect("4바이트")) as usize;
            let order = OrderKey::from_bytes(take(key_len)?.to_vec()).map_err(|_| {
                ModifierError::Malformed {
                    reason: "잘못된 OrderKey",
                }
            })?;
            if previous.as_ref().is_some_and(|p| *p >= order) {
                return Err(ModifierError::Malformed {
                    reason: "OrderKey 정렬 위반",
                });
            }
            previous = Some(order.clone());
            stack.insert(ModifierEntry { id, order })?;
        }
        if pos != bytes.len() {
            return Err(ModifierError::Malformed {
                reason: "trailing bytes",
            });
        }
        Ok(stack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(seed: u64) -> ModifierId {
        ModifierId::new(StableId::new_v7(seed, 0, seed))
    }

    #[test]
    fn id_and_order_round_trip() {
        // 검증 기준(VI.9.1): ID와 순서 round-trip.
        let mut stack = ModifierStack::new(StableId::new_v7(1, 0, 1));
        stack.push_back(id(10)).unwrap();
        stack.push_back(id(11)).unwrap();
        stack.push_back(id(12)).unwrap();
        // 맨 앞에 삽입(다른 항목의 key 불변).
        let first_key = OrderKey::between(None, Some(&stack.entries()[0].order));
        let keys_before: Vec<_> = stack.entries().iter().map(|e| e.order.clone()).collect();
        stack
            .insert(ModifierEntry {
                id: id(9),
                order: first_key,
            })
            .unwrap();
        assert_eq!(stack.ids_in_order(), vec![id(9), id(10), id(11), id(12)]);
        let keys_after: Vec<_> = stack.entries()[1..]
            .iter()
            .map(|e| e.order.clone())
            .collect();
        assert_eq!(keys_after, keys_before, "기존 key 불변");

        let bytes = stack.encode();
        let decoded = ModifierStack::decode(&bytes).unwrap();
        assert_eq!(decoded, stack);
        assert_eq!(decoded.encode(), bytes);
    }

    #[test]
    fn stack_is_stored_apart_from_node_payload() {
        // stack은 소유 node id만 참조하고 node payload bytes를 포함하지 않습니다.
        let stack = ModifierStack::new(StableId::new_v7(1, 0, 1));
        assert_eq!(stack.encode().len(), 16 + 4);
    }

    #[test]
    fn duplicates_and_malformed_are_rejected() {
        let mut stack = ModifierStack::new(StableId::new_v7(1, 0, 1));
        let key = stack.push_back(id(10)).unwrap();
        assert_eq!(
            stack.push_back(id(10)).unwrap_err(),
            ModifierError::DuplicateId { id: id(10) }
        );
        assert_eq!(
            stack
                .insert(ModifierEntry {
                    id: id(11),
                    order: key
                })
                .unwrap_err(),
            ModifierError::DuplicateOrderKey
        );
        assert_eq!(stack.remove(id(10)).unwrap().id, id(10));
        assert!(matches!(
            stack.remove(id(10)),
            Err(ModifierError::NotFound { .. })
        ));
        let mut bytes = stack.encode();
        bytes.push(0);
        assert!(ModifierStack::decode(&bytes).is_err());
        assert!(ModifierStack::decode(&[0; 5]).is_err());
    }
}

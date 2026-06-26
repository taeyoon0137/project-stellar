//! Generational arena (III.6.1).
//!
//! [`Arena`]는 [`RuntimeEntityId`]를 불투명 핸들로 사용하는 generational 저장소입니다. 값은
//! 연속된 `Vec`에 slot 인덱스로 저장하며, 개별 노드를 `Box`로 따로 할당하지 않습니다(공개
//! API에 box allocation을 노출하지 않음).
//!
//! 핸들 수명은 core-types의 [`RuntimeEntityAllocator`]가 관리합니다. slot이 재사용되면
//! generation이 올라가므로, 삭제된(stale) 핸들로 접근하면 [`Arena::get`]·[`Arena::contains`]가
//! 실패합니다.

use stellar_core_types::ids::{RuntimeEntityAllocator, RuntimeEntityId};

/// `RuntimeEntityId` 핸들로 값을 저장하는 generational arena.
#[derive(Debug, Default)]
pub struct Arena<T> {
    alloc: RuntimeEntityAllocator,
    storage: Vec<Option<T>>,
    len: usize,
}

impl<T> Arena<T> {
    /// 빈 arena를 만듭니다.
    pub fn new() -> Arena<T> {
        Arena {
            alloc: RuntimeEntityAllocator::new(),
            storage: Vec::new(),
            len: 0,
        }
    }

    /// 값을 삽입하고 핸들을 반환합니다. 재사용 가능한 slot이 있으면 그 slot을 씁니다.
    pub fn insert(&mut self, value: T) -> RuntimeEntityId {
        let id = self.alloc.allocate();
        let slot = self.alloc.live_slot(id).expect("갓 할당한 핸들은 live");
        if slot == self.storage.len() {
            self.storage.push(Some(value));
        } else {
            // 재사용 slot.
            self.storage[slot] = Some(value);
        }
        self.len += 1;
        id
    }

    /// 핸들이 가리키는 값을 참조합니다. stale 핸들이면 `None`.
    pub fn get(&self, id: RuntimeEntityId) -> Option<&T> {
        let slot = self.alloc.live_slot(id)?;
        self.storage[slot].as_ref()
    }

    /// 핸들이 가리키는 값을 가변 참조합니다. stale 핸들이면 `None`.
    pub fn get_mut(&mut self, id: RuntimeEntityId) -> Option<&mut T> {
        let slot = self.alloc.live_slot(id)?;
        self.storage[slot].as_mut()
    }

    /// 핸들이 살아 있는지 여부.
    pub fn contains(&self, id: RuntimeEntityId) -> bool {
        self.alloc.live_slot(id).is_some()
    }

    /// 값을 제거하고 반환합니다. slot을 재사용 풀에 넣고 generation을 올려 옛 핸들을 무효화합니다.
    /// stale 핸들이면 `None`.
    pub fn remove(&mut self, id: RuntimeEntityId) -> Option<T> {
        let slot = self.alloc.live_slot(id)?;
        let value = self.storage[slot].take();
        self.alloc.deallocate(id);
        if value.is_some() {
            self.len -= 1;
        }
        value
    }

    /// 살아 있는 값의 개수.
    pub fn len(&self) -> usize {
        self.len
    }

    /// 비어 있는지 여부.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_get_remove() {
        let mut arena: Arena<u32> = Arena::new();
        assert!(arena.is_empty());
        let a = arena.insert(10);
        let b = arena.insert(20);
        assert_eq!(arena.len(), 2);
        assert_eq!(arena.get(a), Some(&10));
        assert_eq!(arena.get(b), Some(&20));
        assert!(arena.contains(a));

        assert_eq!(arena.remove(a), Some(10));
        assert_eq!(arena.len(), 1);
        // 제거 후 stale 핸들 접근 실패.
        assert_eq!(arena.get(a), None);
        assert!(!arena.contains(a));
        // 중복 제거 실패.
        assert_eq!(arena.remove(a), None);
    }

    #[test]
    fn reused_slot_invalidates_stale_handle() {
        let mut arena: Arena<&'static str> = Arena::new();
        let a = arena.insert("a");
        arena.remove(a);
        // slot 재사용.
        let b = arena.insert("b");
        // 옛 핸들 a는 slot이 재사용돼도 generation 불일치로 접근 실패.
        assert_eq!(arena.get(a), None);
        assert_eq!(arena.get(b), Some(&"b"));
    }

    #[test]
    fn get_mut_updates_value() {
        let mut arena: Arena<u32> = Arena::new();
        let a = arena.insert(1);
        *arena.get_mut(a).unwrap() = 99;
        assert_eq!(arena.get(a), Some(&99));
        // stale 핸들 get_mut은 None.
        arena.remove(a);
        assert!(arena.get_mut(a).is_none());
    }

    /// 결정론적 xorshift64(테스트 전용, 삽입/제거 선택용).
    fn xorshift(state: &mut u64) -> u64 {
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *state = x;
        x
    }

    #[test]
    fn stress_million_insert_remove_stale_all_fail() {
        let mut arena: Arena<u64> = Arena::new();
        let mut live: Vec<(RuntimeEntityId, u64)> = Vec::new();
        let mut stale: Vec<RuntimeEntityId> = Vec::new();
        let mut rng: u64 = 0x1234_5678_9ABC_DEF0;
        let mut counter: u64 = 0;

        for _ in 0..1_000_000 {
            let r = xorshift(&mut rng);
            if live.is_empty() || r.is_multiple_of(2) {
                counter += 1;
                let id = arena.insert(counter);
                assert_eq!(arena.get(id), Some(&counter));
                live.push((id, counter));
            } else {
                let idx = (r as usize) % live.len();
                let (id, val) = live.swap_remove(idx);
                assert_eq!(arena.remove(id), Some(val));
                // 제거 직후 stale 접근 실패.
                assert_eq!(arena.get(id), None);
                assert!(!arena.contains(id));
                stale.push(id);
            }
        }

        // 모든 stale 핸들은 slot 재사용 후에도 접근 실패.
        for id in &stale {
            assert!(arena.get(*id).is_none(), "stale 핸들 접근 성공(버그)");
            assert!(!arena.contains(*id));
        }
        // 모든 live 핸들은 접근 성공.
        for (id, val) in &live {
            assert_eq!(arena.get(*id), Some(val));
        }
        assert_eq!(arena.len(), live.len());
    }
}

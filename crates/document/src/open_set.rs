//! 열린 문서 집합 (VI.1.4).
//!
//! **저장된 문서 index**([`ProjectDocumentIndex`](crate::definitions))와
//! **현재 열려 있는 문서**(런타임 [`Document`])를 분리합니다. 열림 상태는
//! 저장 대상이 아니며, 저장된 index는 열기·닫기로 변하지 않습니다.
//!
//! handle은 slot+generation 방식이라 닫힌 문서의 옛 handle은 무효가 되고,
//! 같은 문서를 중복 open하면 **기존 handle을 그대로 반환**합니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;

use crate::snapshot::Document;

/// 열린 문서의 불투명 런타임 handle. 직렬화하지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OpenDocumentHandle {
    slot: u32,
    generation: u32,
}

impl fmt::Display for OpenDocumentHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "open-doc[{}#{}]", self.slot, self.generation)
    }
}

/// open 요청 결과.
#[derive(Debug, PartialEq, Eq)]
pub enum OpenOutcome {
    /// 이미 열려 있던 문서 — 기존 handle 반환.
    Existing(OpenDocumentHandle),
    /// 새로 열림.
    NewlyOpened(OpenDocumentHandle),
}

impl OpenOutcome {
    /// 결과의 handle.
    pub fn handle(&self) -> OpenDocumentHandle {
        match self {
            OpenOutcome::Existing(h) | OpenOutcome::NewlyOpened(h) => *h,
        }
    }
}

#[derive(Debug, Default)]
struct Slot {
    generation: u32,
    entry: Option<(StableId, Document)>,
}

/// 현재 열려 있는 문서들의 런타임 집합.
#[derive(Debug, Default)]
pub struct OpenDocumentSet {
    slots: Vec<Slot>,
    free: Vec<u32>,
    by_id: BTreeMap<StableId, OpenDocumentHandle>,
}

impl OpenDocumentSet {
    /// 빈 집합.
    pub fn new() -> OpenDocumentSet {
        OpenDocumentSet::default()
    }

    /// 열린 문서 수.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// 해당 문서가 열려 있는지.
    pub fn is_open(&self, document_id: StableId) -> bool {
        self.by_id.contains_key(&document_id)
    }

    /// 문서를 엽니다. 이미 열려 있으면 **기존 handle**을 반환하고 새 런타임
    /// 문서를 만들지 않습니다.
    pub fn open(&mut self, document_id: StableId) -> OpenOutcome {
        if let Some(handle) = self.by_id.get(&document_id) {
            return OpenOutcome::Existing(*handle);
        }
        let slot = match self.free.pop() {
            Some(slot) => slot,
            None => {
                self.slots.push(Slot::default());
                (self.slots.len() - 1) as u32
            }
        };
        let generation = self.slots[slot as usize].generation;
        self.slots[slot as usize].entry = Some((document_id, Document::new()));
        let handle = OpenDocumentHandle { slot, generation };
        self.by_id.insert(document_id, handle);
        OpenOutcome::NewlyOpened(handle)
    }

    fn valid_entry(&self, handle: OpenDocumentHandle) -> Option<&(StableId, Document)> {
        let slot = self.slots.get(handle.slot as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        slot.entry.as_ref()
    }

    /// handle의 문서 StableId.
    pub fn document_id(&self, handle: OpenDocumentHandle) -> Option<StableId> {
        self.valid_entry(handle).map(|(id, _)| *id)
    }

    /// handle의 런타임 문서(읽기).
    pub fn document(&self, handle: OpenDocumentHandle) -> Option<&Document> {
        self.valid_entry(handle).map(|(_, doc)| doc)
    }

    /// handle의 런타임 문서(수정).
    pub fn document_mut(&mut self, handle: OpenDocumentHandle) -> Option<&mut Document> {
        let slot = self.slots.get_mut(handle.slot as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        slot.entry.as_mut().map(|(_, doc)| doc)
    }

    /// 문서를 닫습니다. 이후 그 handle은 무효입니다. 성공 여부를 반환합니다.
    pub fn close(&mut self, handle: OpenDocumentHandle) -> bool {
        let Some(slot) = self.slots.get_mut(handle.slot as usize) else {
            return false;
        };
        if slot.generation != handle.generation || slot.entry.is_none() {
            return false;
        }
        let (document_id, _) = slot.entry.take().expect("검사 완료");
        slot.generation += 1;
        self.free.push(handle.slot);
        self.by_id.remove(&document_id);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::{DocumentDefinition, ProjectDocumentIndex};

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    #[test]
    fn duplicate_open_returns_existing_handle() {
        // 검증 기준(VI.1.4): 같은 문서를 중복 open할 때 기존 handle 반환.
        let mut set = OpenDocumentSet::new();
        let first = set.open(stable(1));
        let OpenOutcome::NewlyOpened(handle) = first else {
            panic!("첫 open은 NewlyOpened");
        };
        let second = set.open(stable(1));
        assert_eq!(second, OpenOutcome::Existing(handle));
        assert_eq!(set.len(), 1, "런타임 문서가 중복 생성되면 안 됨");
    }

    #[test]
    fn open_set_is_separate_from_saved_index() {
        // 저장된 index는 열기·닫기로 변하지 않습니다.
        let mut index = ProjectDocumentIndex::new();
        index
            .add_document(DocumentDefinition::new(stable(1), "문서".to_string()))
            .unwrap();
        let saved_bytes = index.encode();

        let mut set = OpenDocumentSet::new();
        let handle = set.open(stable(1)).handle();
        assert!(set.is_open(stable(1)));
        set.close(handle);

        assert_eq!(index.encode(), saved_bytes, "저장 index 불변");
    }

    #[test]
    fn close_invalidates_handle_and_allows_reopen() {
        let mut set = OpenDocumentSet::new();
        let handle = set.open(stable(1)).handle();
        assert_eq!(set.document_id(handle), Some(stable(1)));
        assert!(set.close(handle));

        // 옛 handle은 무효.
        assert_eq!(set.document_id(handle), None);
        assert!(set.document(handle).is_none());
        assert!(!set.close(handle), "이중 close 불가");
        assert!(!set.is_open(stable(1)));

        // 재open은 새 handle(다른 generation).
        let reopened = set.open(stable(1));
        let OpenOutcome::NewlyOpened(new_handle) = reopened else {
            panic!("close 후 open은 NewlyOpened");
        };
        assert_ne!(new_handle, handle);
        assert_eq!(set.document_id(new_handle), Some(stable(1)));
    }

    #[test]
    fn multiple_documents_have_distinct_handles() {
        let mut set = OpenDocumentSet::new();
        let a = set.open(stable(1)).handle();
        let b = set.open(stable(2)).handle();
        assert_ne!(a, b);
        assert_eq!(set.len(), 2);
        // 각 handle이 자기 문서를 가리킵니다.
        assert_eq!(set.document_id(a), Some(stable(1)));
        assert_eq!(set.document_id(b), Some(stable(2)));
        // 런타임 문서 수정이 handle 단위로 격리.
        set.document_mut(a).unwrap();
    }
}

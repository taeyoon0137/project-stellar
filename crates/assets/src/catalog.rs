//! `AssetId → AssetRecord` catalog (XIII.5.3).
//!
//! 에셋의 논리 식별자(AssetId)와 content 주소(record의 content hash)를 분리해, 여러
//! AssetId가 같은 content를 참조할 수 있게 합니다(dedup된 import). content 파일은 hash로
//! 공유되므로 한 AssetId를 지워도 다른 AssetId가 참조하는 content는 계속 live입니다
//! (GC mark 입력, XIII.5.6).
//!
//! 이 catalog는 메모리 index입니다. 문서 저장(chunk)·command 적용은 상위 계층이 담당합니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_storage::hash::ContentHash;

use crate::id::AssetId;
use crate::record::AssetRecord;

/// catalog 변경 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    /// 이미 등록된 AssetId입니다.
    DuplicateAssetId(AssetId),
    /// 등록되지 않은 AssetId입니다.
    UnknownAssetId(AssetId),
    /// 변경의 `previous`가 현재 record와 다릅니다(이미 적용됐거나 다른 변경이 먼저 적용됨).
    StaleUpdate(AssetId),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CatalogError::DuplicateAssetId(id) => write!(f, "이미 등록된 AssetId입니다: {id}"),
            CatalogError::UnknownAssetId(id) => write!(f, "등록되지 않은 AssetId입니다: {id}"),
            CatalogError::StaleUpdate(id) => {
                write!(f, "현재 record와 맞지 않는 변경입니다: {id}")
            }
        }
    }
}

impl std::error::Error for CatalogError {}

/// AssetId별 record 목록(AssetId 정렬 순).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssetCatalog {
    records: BTreeMap<AssetId, AssetRecord>,
}

impl AssetCatalog {
    /// 빈 catalog.
    pub fn new() -> AssetCatalog {
        AssetCatalog::default()
    }

    /// 새 AssetId로 record를 등록합니다. 같은 content hash의 다른 AssetId가 있어도 됩니다.
    pub fn insert(&mut self, id: AssetId, record: AssetRecord) -> Result<(), CatalogError> {
        if self.records.contains_key(&id) {
            return Err(CatalogError::DuplicateAssetId(id));
        }
        self.records.insert(id, record);
        Ok(())
    }

    /// AssetId의 record.
    pub fn get(&self, id: &AssetId) -> Option<&AssetRecord> {
        self.records.get(id)
    }

    /// 등록된 AssetId의 record를 교체합니다(검사는 호출자 — `apply_update`, XIII.5.5).
    pub(crate) fn replace(&mut self, id: AssetId, record: AssetRecord) {
        self.records.insert(id, record);
    }

    /// AssetId를 제거하고 record를 돌려줍니다. content 파일은 건드리지 않습니다(GC 담당).
    pub fn remove(&mut self, id: &AssetId) -> Result<AssetRecord, CatalogError> {
        self.records
            .remove(id)
            .ok_or(CatalogError::UnknownAssetId(*id))
    }

    /// 등록된 AssetId 수.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// 비어 있으면 `true`.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// AssetId 정렬 순 순회.
    pub fn iter(&self) -> impl Iterator<Item = (&AssetId, &AssetRecord)> {
        self.records.iter()
    }

    /// 주어진 content hash를 참조하는 AssetId 목록(정렬 순).
    pub fn ids_for_content(&self, hash: &ContentHash) -> Vec<AssetId> {
        self.records
            .iter()
            .filter(|(_, record)| record.content_hash == *hash)
            .map(|(id, _)| *id)
            .collect()
    }

    /// 참조 중인 content hash 집합(중복 제거).
    pub fn content_hashes(&self) -> BTreeSet<ContentHash> {
        self.records.values().map(|r| r.content_hash).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_type::MediaType;
    use crate::record::SourceMetadata;

    fn id(n: u8) -> AssetId {
        let mut bytes = [0u8; 16];
        bytes[15] = n;
        AssetId::from_bytes(bytes)
    }

    fn record(content: &[u8], source: &str) -> AssetRecord {
        AssetRecord::new(
            ContentHash::compute(content),
            MediaType::new("image/png").unwrap(),
            content.len() as u64,
            Some(SourceMetadata::project(source).unwrap()),
        )
    }

    #[test]
    fn distinct_ids_can_share_content() {
        let mut catalog = AssetCatalog::new();
        catalog.insert(id(2), record(b"logo", "a.png")).unwrap();
        catalog.insert(id(1), record(b"logo", "b/a.png")).unwrap();
        catalog.insert(id(3), record(b"other", "c.png")).unwrap();
        let logo = ContentHash::compute(b"logo");
        assert_eq!(catalog.ids_for_content(&logo), vec![id(1), id(2)]);
        assert_eq!(catalog.content_hashes().len(), 2);
        assert_eq!(catalog.len(), 3);
        let order: Vec<AssetId> = catalog.iter().map(|(id, _)| *id).collect();
        assert_eq!(order, vec![id(1), id(2), id(3)], "AssetId 정렬 순");

        // 한 AssetId를 지워도 공유 content는 다른 AssetId가 계속 참조합니다.
        let removed = catalog.remove(&id(1)).unwrap();
        assert_eq!(removed.content_hash, logo);
        assert_eq!(catalog.ids_for_content(&logo), vec![id(2)]);
        assert!(catalog.content_hashes().contains(&logo));
    }

    #[test]
    fn duplicate_and_unknown_ids_are_rejected() {
        let mut catalog = AssetCatalog::new();
        assert!(catalog.is_empty());
        catalog.insert(id(1), record(b"x", "x.png")).unwrap();
        assert_eq!(
            catalog.insert(id(1), record(b"y", "y.png")).unwrap_err(),
            CatalogError::DuplicateAssetId(id(1))
        );
        assert_eq!(catalog.get(&id(1)).unwrap().size, 1, "기존 record 유지");
        assert_eq!(
            catalog.remove(&id(9)).unwrap_err(),
            CatalogError::UnknownAssetId(id(9))
        );
        assert!(CatalogError::UnknownAssetId(id(9))
            .to_string()
            .contains("00000000-0000-0000-0000-000000000009"));
    }
}

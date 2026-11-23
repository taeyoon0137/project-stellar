//! AssetRecord 참조 변경 command (XIII.5.5, ADR-0119 결정 9).
//!
//! `Update from Source`([`AssetStore::update_from_source`])가 돌려준 [`AssetRecordUpdate`]를 catalog에 적용하는 closed
//! enum command입니다. 문서 component command(IX.2.4 `PropertyOverrideCommand`)와 같은 계약을 따릅니다.
//!
//! - **validate**: 상태를 바꾸지 않습니다. AssetId가 catalog에 있고, 현재 record가 `previous`와 같고(같지 않으면 다른
//!   변경이 먼저 적용된 것), `next` content가 저장소에 실제로 있어야(이름·크기·재hash 확인, [`AssetStore::locate_content`])
//!   합니다. content가 없는 record로 바꾸면 이미지가 사라지므로 거부합니다.
//! - **apply**: 검증 뒤 record를 바꾸고 정확히 되돌리는 **inverse command**를 돌려줍니다(`previous`와 `next`를 바꾼 같은
//!   command). 이전 content 파일은 지우지 않습니다(undo 대상, GC는 참조가 끊긴 뒤, XIII.5.6).
//!
//! 원본이 없으면 `Update from Source`가 [`UpdateFromSourceOutcome::SourceUnavailable`]를 돌려주므로 command가 만들어지지
//! 않고 기존 asset이 그대로 남습니다.
//!
//! Phase VII command 계층(`stellar-commands` closed enum)과 창 history 연결은 그 crate의 등록 작업입니다. 이 enum이 그
//! 갈래의 apply/inverse 본체입니다.
//!
//! [`UpdateFromSourceOutcome::SourceUnavailable`]: crate::update_source::UpdateFromSourceOutcome::SourceUnavailable

use std::fmt;

use crate::catalog::{AssetCatalog, CatalogError};
use crate::store::{AssetStore, ImportError};
use crate::update_source::AssetRecordUpdate;

/// AssetRecord command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetRecordCommand {
    /// record를 `previous`에서 `next`로 바꿉니다(Update from Source 결과 또는 그 inverse).
    Update(AssetRecordUpdate),
}

/// command 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetCommandError {
    /// catalog 판정 실패(없는 AssetId, 현재 record가 `previous`와 다름).
    Catalog(CatalogError),
    /// `next` record의 content가 저장소에 없음.
    MissingContent,
    /// content 확인 실패.
    Store(ImportError),
}

impl fmt::Display for AssetCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetCommandError::Catalog(e) => write!(f, "{e}"),
            AssetCommandError::MissingContent => {
                f.write_str("바꿀 asset content가 저장소에 없습니다")
            }
            AssetCommandError::Store(e) => write!(f, "asset content 확인 실패: {e}"),
        }
    }
}

impl std::error::Error for AssetCommandError {}

impl AssetRecordCommand {
    /// undo 메뉴 이름.
    pub fn action_name(&self) -> &'static str {
        match self {
            AssetRecordCommand::Update(_) => "원본에서 업데이트",
        }
    }

    /// 적용 가능 여부를 검증합니다. 상태를 바꾸지 않습니다.
    pub fn validate(
        &self,
        catalog: &AssetCatalog,
        store: &AssetStore,
    ) -> Result<(), AssetCommandError> {
        let AssetRecordCommand::Update(update) = self;
        match catalog.get(&update.asset_id) {
            None => {
                return Err(AssetCommandError::Catalog(CatalogError::UnknownAssetId(
                    update.asset_id,
                )))
            }
            Some(current) if *current != update.previous => {
                return Err(AssetCommandError::Catalog(CatalogError::StaleUpdate(
                    update.asset_id,
                )))
            }
            Some(_) => {}
        }
        store
            .locate_content(&update.next.content_hash, update.next.size)
            .map_err(AssetCommandError::Store)?
            .ok_or(AssetCommandError::MissingContent)?;
        Ok(())
    }

    /// 검증 후 적용하고 inverse command를 돌려줍니다. 검증 실패 시 catalog는 바뀌지 않습니다.
    pub fn apply(
        &self,
        catalog: &mut AssetCatalog,
        store: &AssetStore,
    ) -> Result<AssetRecordCommand, AssetCommandError> {
        self.validate(catalog, store)?;
        let AssetRecordCommand::Update(update) = self;
        catalog
            .apply_update(update)
            .map_err(AssetCommandError::Catalog)?;
        Ok(AssetRecordCommand::Update(update.inverted()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::AssetId;
    use crate::media_type::MediaType;
    use crate::project_source::DenyProjectSources;
    use crate::record::AssetRecord;
    use crate::store::test_support::{
        files_under, shipped_policy, test_store, AllowProjectSources, TempDir,
    };
    use crate::store::ImportRequest;
    use crate::update_source::{SourceUnavailable, UpdateFromSourceOutcome};
    use stellar_storage::hash::ContentHash;

    fn id() -> AssetId {
        AssetId::from_bytes([9u8; 16])
    }

    fn png() -> MediaType {
        MediaType::new("image/png").unwrap()
    }

    #[test]
    fn update_from_source_changes_record_through_command_and_undo_restores_it() {
        let dir = TempDir::new("asset-record-command");
        let project = dir.path.join("project");
        std::fs::create_dir_all(project.join("images")).unwrap();
        let store = test_store(&project);
        let source = project.join("images").join("logo.png");
        std::fs::write(&source, b"version 1").unwrap();
        let current = store
            .import_file(&ImportRequest::new(&source, png()))
            .unwrap()
            .record;
        let mut catalog = AssetCatalog::new();
        catalog.insert(id(), current.clone()).unwrap();

        std::fs::write(&source, b"version 2").unwrap();
        let UpdateFromSourceOutcome::Updated { update, .. } = store
            .update_from_source(
                id(),
                &current,
                None,
                &shipped_policy(),
                &AllowProjectSources,
            )
            .unwrap()
        else {
            panic!("Updated 기대");
        };
        let command = AssetRecordCommand::Update(*update);
        assert_eq!(command.action_name(), "원본에서 업데이트");
        let undo = command.apply(&mut catalog, &store).expect("적용");
        assert_eq!(
            catalog.get(&id()).unwrap().content_hash,
            ContentHash::compute(b"version 2")
        );
        // 같은 command를 다시 적용하면 현재 record가 previous와 달라 거부(catalog 불변).
        assert_eq!(
            command.apply(&mut catalog, &store),
            Err(AssetCommandError::Catalog(CatalogError::StaleUpdate(id())))
        );
        // undo: 이전 content 파일이 남아 있으므로 정확히 되돌립니다.
        let redo = undo.apply(&mut catalog, &store).expect("undo");
        assert_eq!(catalog.get(&id()), Some(&current));
        assert_eq!(redo, command);
        assert_eq!(files_under(&project.join("assets")).len(), 2);
    }

    #[test]
    fn missing_source_or_content_keeps_existing_asset() {
        let dir = TempDir::new("asset-record-command-missing");
        let project = dir.path.join("project");
        std::fs::create_dir_all(project.join("images")).unwrap();
        let store = test_store(&project);
        let source = project.join("images").join("logo.png");
        std::fs::write(&source, b"kept").unwrap();
        let current = store
            .import_file(&ImportRequest::new(&source, png()))
            .unwrap()
            .record;
        let mut catalog = AssetCatalog::new();
        catalog.insert(id(), current.clone()).unwrap();

        // 원본 없음: command가 만들어지지 않고 record는 그대로입니다.
        std::fs::remove_file(&source).unwrap();
        assert_eq!(
            store
                .update_from_source(
                    id(),
                    &current,
                    None,
                    &shipped_policy(),
                    &AllowProjectSources
                )
                .unwrap(),
            UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::NotFound)
        );
        assert!(matches!(
            store
                .update_from_source(id(), &current, None, &shipped_policy(), &DenyProjectSources)
                .unwrap(),
            UpdateFromSourceOutcome::SourceUnavailable(_)
        ));
        assert_eq!(catalog.get(&id()), Some(&current));

        // content가 저장소에 없는 record로 바꾸는 command는 거부됩니다(catalog 불변).
        let ghost = AssetRecord::new(ContentHash::compute(b"ghost"), png(), 5, None);
        let command = AssetRecordCommand::Update(AssetRecordUpdate {
            asset_id: id(),
            previous: current.clone(),
            next: ghost,
        });
        assert_eq!(
            command.apply(&mut catalog, &store),
            Err(AssetCommandError::MissingContent)
        );
        assert_eq!(catalog.get(&id()), Some(&current));
        // 없는 AssetId.
        let unknown = AssetRecordCommand::Update(AssetRecordUpdate {
            asset_id: AssetId::from_bytes([1u8; 16]),
            previous: current.clone(),
            next: current.clone(),
        });
        assert!(matches!(
            unknown.validate(&catalog, &store),
            Err(AssetCommandError::Catalog(CatalogError::UnknownAssetId(_)))
        ));
    }
}

//! `Update from Source` (XIII.5.5).
//!
//! 외부 파일 변경은 자동 반영하지 않고(아키텍처 29.1) 사용자가 명시 실행할 때만 source
//! metadata의 원본을 다시 읽습니다. 동작:
//!
//! 1. record의 source metadata로 원본 경로를 정합니다.
//!    - 프로젝트 내부: project root 기준 상대 경로.
//!    - 프로젝트 외부: 절대 경로를 기록하지 않으므로(XIII.5.1 후속 수정) 호출자가 후보 경로를
//!      제시해야 하며, canonicalize한 경로의 출처 지문이 기록과 같아야 합니다.
//! 2. 원본이 없거나(삭제·이동·디렉터리로 바뀜), metadata가 없거나, 외부 경로가 없거나 다른
//!    출처면 [`UpdateFromSourceOutcome::SourceUnavailable`] — **기존 asset을 그대로 유지**합니다
//!    (record·content 파일 모두 변경 없음).
//! 3. 원본을 XIII.5.2~5.4 import(streaming hash·dedup·atomic write)로 다시 들여옵니다. hash가
//!    같으면 [`UpdateFromSourceOutcome::Unchanged`].
//! 4. hash가 다르면 새 content의 새 record를 만들고 [`AssetRecordUpdate`]
//!    (`previous → next`)를 돌려줍니다.
//!
//! # command 경계
//!
//! 이 모듈은 AssetRecord 참조를 **직접 바꾸지 않습니다.** 반환한 [`AssetRecordUpdate`]를
//! Phase VII command(validate/apply/invert)가 적용해야 하며, [`AssetRecordUpdate::inverted`]가
//! undo 값입니다. command 타입은 아직 없으므로(stellar-commands 골격) 연결은 Phase VII
//! 작업입니다. 이전 content 파일은 지우지 않습니다(undo 대상 — 참조가 끊긴 뒤 GC, XIII.5.6).

use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::catalog::{AssetCatalog, CatalogError};
use crate::id::AssetId;
use crate::path::AssetObjectPath;
use crate::record::{AssetRecord, SourceLocation};
use crate::store::{AssetStore, ImportError, ImportOperation, ImportOutcome, ImportRequest};

/// 원본을 쓸 수 없어 기존 asset을 유지한 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceUnavailable {
    /// record에 source metadata가 없습니다.
    NoSourceMetadata,
    /// 외부 원본인데 호출자가 후보 경로를 주지 않았습니다.
    ExternalPathRequired,
    /// 제시한 외부 경로의 출처 지문이 기록과 다릅니다.
    OriginMismatch,
    /// 원본 파일이 없습니다.
    NotFound,
    /// 원본 경로가 일반 파일이 아닙니다.
    NotAFile,
}

impl fmt::Display for SourceUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            SourceUnavailable::NoSourceMetadata => "원본 정보가 없습니다",
            SourceUnavailable::ExternalPathRequired => "외부 원본 경로를 지정해야 합니다",
            SourceUnavailable::OriginMismatch => "지정한 경로가 기록된 원본과 다릅니다",
            SourceUnavailable::NotFound => "원본 파일이 없습니다",
            SourceUnavailable::NotAFile => "원본이 일반 파일이 아닙니다",
        };
        f.write_str(text)
    }
}

/// command가 적용할 AssetRecord 참조 변경(`previous → next`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetRecordUpdate {
    /// 대상 AssetId.
    pub asset_id: AssetId,
    /// 변경 전 record(undo 값).
    pub previous: AssetRecord,
    /// 변경 후 record.
    pub next: AssetRecord,
}

impl AssetRecordUpdate {
    /// undo용 역변경(`next → previous`).
    pub fn inverted(&self) -> AssetRecordUpdate {
        AssetRecordUpdate {
            asset_id: self.asset_id,
            previous: self.next.clone(),
            next: self.previous.clone(),
        }
    }
}

/// `Update from Source` 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateFromSourceOutcome {
    /// 새 content로 바꿀 참조 변경. command로 적용해야 합니다.
    Updated {
        /// 적용할 변경(record 2개를 담아 heap에 둡니다).
        update: Box<AssetRecordUpdate>,
        /// 새 content 파일 경로.
        object_path: AssetObjectPath,
        /// 새 content를 복사했는지(이미 있던 content면 `Deduplicated`).
        import: ImportOutcome,
    },
    /// 원본 내용이 현재 content와 같습니다(참조 변경 없음).
    Unchanged,
    /// 원본을 쓸 수 없어 기존 asset을 유지합니다.
    SourceUnavailable(SourceUnavailable),
}

impl AssetStore {
    /// source metadata의 원본을 다시 읽어 새 hash asset을 만듭니다(참조 변경은 반환만).
    ///
    /// `external_source`는 외부 원본일 때만 쓰이는 후보 경로입니다(내부 원본은 무시).
    pub fn update_from_source(
        &self,
        asset_id: AssetId,
        current: &AssetRecord,
        external_source: Option<&Path>,
    ) -> Result<UpdateFromSourceOutcome, ImportError> {
        let source = match self.resolve_source(current, external_source)? {
            Ok(path) => path,
            Err(reason) => return Ok(UpdateFromSourceOutcome::SourceUnavailable(reason)),
        };
        let request = ImportRequest::new(&source, current.media_type.clone());
        let imported = match self.import_file(&request) {
            Ok(imported) => imported,
            // 확인과 재import 사이에 원본이 사라진 경우도 기존 asset 유지입니다.
            Err(ImportError::NotAFile) => {
                return Ok(UpdateFromSourceOutcome::SourceUnavailable(
                    SourceUnavailable::NotAFile,
                ))
            }
            Err(ImportError::Io {
                operation: ImportOperation::OpenSource,
                kind: ErrorKind::NotFound,
                ..
            })
            | Err(ImportError::SourceMetadata(crate::record::SourceMetadataError::Resolve {
                kind: ErrorKind::NotFound,
            })) => {
                return Ok(UpdateFromSourceOutcome::SourceUnavailable(
                    SourceUnavailable::NotFound,
                ))
            }
            Err(e) => return Err(e),
        };
        if imported.record.content_hash == current.content_hash
            && imported.record.size == current.size
        {
            return Ok(UpdateFromSourceOutcome::Unchanged);
        }
        Ok(UpdateFromSourceOutcome::Updated {
            update: Box::new(AssetRecordUpdate {
                asset_id,
                previous: current.clone(),
                next: imported.record,
            }),
            object_path: imported.object_path,
            import: imported.outcome,
        })
    }

    /// 원본 경로를 정합니다. 쓸 수 없으면 `Ok(Err(reason))`.
    fn resolve_source(
        &self,
        current: &AssetRecord,
        external_source: Option<&Path>,
    ) -> Result<Result<PathBuf, SourceUnavailable>, ImportError> {
        let Some(metadata) = &current.source_metadata else {
            return Ok(Err(SourceUnavailable::NoSourceMetadata));
        };
        let candidate = match metadata.location() {
            SourceLocation::Project { .. } => match metadata.project_path(self.root()) {
                Some(path) => path,
                None => return Ok(Err(SourceUnavailable::NotFound)),
            },
            SourceLocation::External { .. } => {
                let Some(path) = external_source else {
                    return Ok(Err(SourceUnavailable::ExternalPathRequired));
                };
                path.to_path_buf()
            }
        };
        let resolved = match std::fs::canonicalize(&candidate) {
            Ok(path) => path,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return Ok(Err(SourceUnavailable::NotFound))
            }
            Err(e) => {
                return Err(ImportError::Io {
                    operation: ImportOperation::OpenSource,
                    kind: e.kind(),
                    message: e.to_string(),
                })
            }
        };
        if matches!(metadata.location(), SourceLocation::External { .. })
            && !metadata.matches_external_origin(&resolved)
        {
            return Ok(Err(SourceUnavailable::OriginMismatch));
        }
        if !resolved.is_file() {
            return Ok(Err(SourceUnavailable::NotAFile));
        }
        Ok(Ok(resolved))
    }
}

impl AssetCatalog {
    /// [`AssetRecordUpdate`]를 적용합니다(낙관적 검사: 현재 record가 `previous`와 같아야 함).
    ///
    /// Phase VII command의 apply/invert가 호출할 효과 함수입니다. 사용자 편집 경로에서
    /// command를 거치지 않고 직접 호출하지 않습니다.
    pub fn apply_update(&mut self, update: &AssetRecordUpdate) -> Result<(), CatalogError> {
        match self.get(&update.asset_id) {
            None => Err(CatalogError::UnknownAssetId(update.asset_id)),
            Some(record) if *record != update.previous => {
                Err(CatalogError::StaleUpdate(update.asset_id))
            }
            Some(_) => {
                self.replace(update.asset_id, update.next.clone());
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_type::MediaType;
    use crate::store::test_support::{files_under, TempDir};
    use stellar_storage::hash::ContentHash;

    struct Fixture {
        _dir: TempDir,
        project: PathBuf,
        outside: PathBuf,
        store: AssetStore,
    }

    fn fixture() -> Fixture {
        let dir = TempDir::new("update-source");
        let project = dir.path.join("project");
        let outside = dir.path.join("outside");
        std::fs::create_dir_all(project.join("images")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let store = AssetStore::new(&project);
        Fixture {
            _dir: dir,
            project,
            outside,
            store,
        }
    }

    fn id() -> AssetId {
        AssetId::from_bytes([7u8; 16])
    }

    fn png() -> MediaType {
        MediaType::new("image/png").unwrap()
    }

    fn import(fx: &Fixture, source: &Path, bytes: &[u8]) -> AssetRecord {
        std::fs::write(source, bytes).unwrap();
        fx.store
            .import_file(&ImportRequest::new(source, png()))
            .unwrap()
            .record
    }

    #[test]
    fn project_source_change_produces_new_record_update() {
        let fx = fixture();
        let source = fx.project.join("images").join("logo.png");
        let current = import(&fx, &source, b"version 1");
        std::fs::write(&source, b"version 2").unwrap();

        let outcome = fx.store.update_from_source(id(), &current, None).unwrap();
        let UpdateFromSourceOutcome::Updated {
            update,
            object_path,
            import,
        } = outcome
        else {
            panic!("Updated 기대: {outcome:?}");
        };
        assert_eq!(import, ImportOutcome::Copied);
        assert_eq!(update.asset_id, id());
        assert_eq!(update.previous, current);
        assert_eq!(update.next.content_hash, ContentHash::compute(b"version 2"));
        assert_eq!(update.next.size, 9);
        assert_eq!(
            update.next.media_type, current.media_type,
            "media type 유지"
        );
        assert_eq!(update.next.source_metadata, current.source_metadata);
        assert_eq!(
            std::fs::read(fx.store.content_path(&object_path)).unwrap(),
            b"version 2"
        );
        // 이전 content는 undo를 위해 남습니다.
        assert_eq!(files_under(&fx.project.join("assets")).len(), 2);

        // undo 값.
        let inverted = update.inverted();
        assert_eq!(inverted.previous, update.next);
        assert_eq!(inverted.next, current);
        assert_eq!(inverted.inverted(), *update);
    }

    #[test]
    fn same_content_is_unchanged() {
        let fx = fixture();
        let source = fx.project.join("images").join("logo.png");
        let current = import(&fx, &source, b"same");
        let before = files_under(&fx.project.join("assets"));
        assert_eq!(
            fx.store.update_from_source(id(), &current, None).unwrap(),
            UpdateFromSourceOutcome::Unchanged
        );
        assert_eq!(files_under(&fx.project.join("assets")), before);
    }

    #[test]
    fn missing_source_keeps_existing_asset() {
        let fx = fixture();
        let source = fx.project.join("images").join("logo.png");
        let current = import(&fx, &source, b"keep me");
        let before = files_under(&fx.project.join("assets"));
        let snapshot = current.clone();

        std::fs::remove_file(&source).unwrap();
        assert_eq!(
            fx.store.update_from_source(id(), &current, None).unwrap(),
            UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::NotFound)
        );
        // 디렉터리로 바뀐 원본도 마찬가지입니다.
        std::fs::create_dir_all(&source).unwrap();
        assert_eq!(
            fx.store.update_from_source(id(), &current, None).unwrap(),
            UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::NotAFile)
        );

        assert_eq!(current, snapshot, "record 불변");
        assert_eq!(
            files_under(&fx.project.join("assets")),
            before,
            "content 불변"
        );
        assert_eq!(
            std::fs::read(&before[0]).unwrap(),
            b"keep me",
            "기존 content 유지"
        );
    }

    #[test]
    fn record_without_source_metadata_is_kept() {
        let fx = fixture();
        let source = fx.outside.join("a.png");
        std::fs::write(&source, b"x").unwrap();
        let current = fx
            .store
            .import_file(&ImportRequest::new(&source, png()).without_source_metadata())
            .unwrap()
            .record;
        assert_eq!(
            fx.store
                .update_from_source(id(), &current, Some(&source))
                .unwrap(),
            UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::NoSourceMetadata)
        );
    }

    #[test]
    fn external_source_requires_matching_candidate_path() {
        let fx = fixture();
        let source = fx.outside.join("brand.png");
        let current = import(&fx, &source, b"external v1");
        let other = fx.outside.join("other.png");
        std::fs::write(&other, b"external v2").unwrap();

        assert_eq!(
            fx.store.update_from_source(id(), &current, None).unwrap(),
            UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::ExternalPathRequired)
        );
        assert_eq!(
            fx.store
                .update_from_source(id(), &current, Some(&other))
                .unwrap(),
            UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::OriginMismatch)
        );

        std::fs::write(&source, b"external v2").unwrap();
        let outcome = fx
            .store
            .update_from_source(id(), &current, Some(&source))
            .unwrap();
        let UpdateFromSourceOutcome::Updated { update, .. } = outcome else {
            panic!("Updated 기대: {outcome:?}");
        };
        assert_eq!(
            update.next.content_hash,
            ContentHash::compute(b"external v2")
        );
        assert!(update
            .next
            .source_metadata
            .as_ref()
            .unwrap()
            .matches_external_origin(&source));

        std::fs::remove_file(&source).unwrap();
        assert_eq!(
            fx.store
                .update_from_source(id(), &current, Some(&source))
                .unwrap(),
            UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::NotFound)
        );
    }

    #[test]
    fn update_to_existing_content_is_deduplicated() {
        let fx = fixture();
        let source = fx.project.join("images").join("logo.png");
        let current = import(&fx, &source, b"v1");
        // 다른 에셋이 이미 v2 content를 들여온 상태.
        let _ = import(&fx, &fx.outside.join("v2.png"), b"v2");
        std::fs::write(&source, b"v2").unwrap();
        let outcome = fx.store.update_from_source(id(), &current, None).unwrap();
        assert!(matches!(
            outcome,
            UpdateFromSourceOutcome::Updated {
                import: ImportOutcome::Deduplicated,
                ..
            }
        ));
        assert_eq!(files_under(&fx.project.join("assets")).len(), 2);
    }

    #[test]
    fn catalog_applies_update_and_inverse_with_stale_check() {
        let fx = fixture();
        let source = fx.project.join("images").join("logo.png");
        let current = import(&fx, &source, b"one");
        std::fs::write(&source, b"two").unwrap();
        let UpdateFromSourceOutcome::Updated { update, .. } =
            fx.store.update_from_source(id(), &current, None).unwrap()
        else {
            panic!("Updated 기대");
        };

        let mut catalog = AssetCatalog::new();
        catalog.insert(id(), current.clone()).unwrap();
        catalog.apply_update(&update).unwrap();
        assert_eq!(catalog.get(&id()), Some(&update.next));
        // 같은 변경을 두 번 적용하면 stale입니다.
        assert_eq!(
            catalog.apply_update(&update).unwrap_err(),
            CatalogError::StaleUpdate(id())
        );
        catalog.apply_update(&update.inverted()).unwrap();
        assert_eq!(catalog.get(&id()), Some(&current));

        let mut empty = AssetCatalog::new();
        assert_eq!(
            empty.apply_update(&update).unwrap_err(),
            CatalogError::UnknownAssetId(id())
        );
    }

    #[test]
    fn unavailable_reasons_are_described() {
        assert!(SourceUnavailable::OriginMismatch
            .to_string()
            .contains("원본"));
    }
}

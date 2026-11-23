//! `Update from Source` (XIII.5.5).
//!
//! 외부 파일 변경은 자동 반영하지 않고(아키텍처 29.1) 사용자가 명시 실행할 때만 source
//! metadata의 원본을 다시 읽습니다. 동작:
//!
//! 1. record의 source metadata로 원본 경로를 정합니다.
//!    - 프로젝트 내부: project root 기준 상대 경로. record는 Git에서 온 **신뢰할 수 없는 입력**
//!      이므로 다음을 모두 통과해야 원본을 엽니다([`crate::project_source`]).
//!      1. 기록 경로가 고정 거부 영역(`.git` 구성 요소, `design/`·`.stellar/`, `assets/`)이
//!         아님 — 아니면 [`SourceUnavailable::ProtectedPath`](파일 시스템 접근 전 판정).
//!      2. canonicalize(symlink 해석)한 경로가 canonical project root 안 — 밖으로 해석되면
//!         (저장소 안 symlink가 프로젝트 밖을 가리킴) [`SourceUnavailable::OutsideProject`].
//!      3. 해석 경로(root 기준)도 고정 거부 영역이 아님 — 프로젝트 안 symlink로 `.git/config`
//!         등을 가리키는 경우도 [`SourceUnavailable::ProtectedPath`].
//!      4. 호출자 정책([`ProjectSourcePolicy`])이 명시적으로 허용 — 아니면
//!         [`SourceUnavailable::DeniedByPolicy`]. 신뢰되지 않은 프로젝트는
//!         [`crate::project_source::DenyProjectSources`](모두 거부)를 넘기고, 신뢰된 프로젝트는
//!         project 계층이 `git check-ignore` 등으로 commit되면 안 되는 파일을 거부합니다.
//!
//!      어느 단계에서 거부되어도 아무 것도 읽거나 쓰지 않습니다. 내부 원본은 외부 원본으로
//!      바뀌지 않고, 새 record도 기록된 상대 경로를 그대로 유지합니다. import는 판정을 통과한
//!      해석 경로로만 진행합니다(판정 뒤 다른 파일로 해석되면 [`ImportError::SourceChanged`]).
//!      기록된 원본(내부·외부)은 판정 때 `lstat`한 항목 identity로 고정해, hash pass·copy pass
//!      모두 경로의 어느 구성 요소도 symlink를 따라가지 않고 열고(macOS `O_NOFOLLOW_ANY`) 연
//!      파일이 그 항목일 때만 읽습니다(보안 리뷰 S4).
//!    - 프로젝트 외부: 절대 경로를 기록하지 않으므로(XIII.5.1 후속 수정) 호출자가 후보 경로를
//!      제시해야 하며, canonicalize한 경로의 출처 지문이 이 기기 key로 기록과 같아야 합니다
//!      ([`crate::origin`]). 지문이 **다른 기기 key**로 기록되었으면 어떤 후보 경로로도 확인할
//!      수 없으므로 [`SourceUnavailable::OriginUnverifiable`]입니다. 이때 셸은 사용자에게
//!      원본 파일을 다시 선택받아 [`AssetStore::update_from_selected_source`]를 호출합니다
//!      (명시 선택이므로 지문 확인 없이 쓰고, 새 record에는 이 기기 key의 지문을 기록).
//! 2. 원본이 없거나(삭제·이동·디렉터리로 바뀜), metadata가 없거나, 외부 경로가 없거나 다른
//!    출처이거나, 내부 원본이 프로젝트 밖으로 해석되거나 보호 영역·정책 거부이면
//!    [`UpdateFromSourceOutcome::SourceUnavailable`] — **기존 asset을 그대로 유지**합니다
//!    (record·content 파일 모두 변경 없음).
//! 3. 원본을 XIII.5.2~5.4 import(streaming hash·dedup·atomic write)로 다시 들여옵니다. hash가
//!    같으면 [`UpdateFromSourceOutcome::Unchanged`].
//! 4. hash가 다르면 새 content의 새 record를 만들고 [`AssetRecordUpdate`]
//!    (`previous → next`)를 돌려줍니다.
//!
//! # command 경계
//!
//! 이 모듈은 AssetRecord 참조를 **직접 바꾸지 않습니다.** 반환한 [`AssetRecordUpdate`]는
//! [`AssetRecordCommand`](crate::record_command::AssetRecordCommand)(validate/apply/inverse)가 적용하며,
//! [`AssetRecordUpdate::inverted`]가 undo 값입니다. 이전 content 파일은 지우지 않습니다(undo 대상 —
//! 참조가 끊긴 뒤 GC, XIII.5.6).

use std::fmt;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use crate::catalog::{AssetCatalog, CatalogError};
use crate::fs_guard::FileIdentity;
use crate::id::AssetId;
use crate::origin::{OriginCheck, OriginFingerprint};
use crate::path::AssetObjectPath;
use crate::policy::AssetImportPolicy;
use crate::project_source::{
    protected_area, protected_project_path, ProjectSourceCandidate, ProjectSourceDecision,
    ProjectSourcePolicy, ProtectedArea,
};
use crate::record::{AssetRecord, SourceLocation, SourceMetadata, SourceMetadataError};
use crate::store::{AssetStore, ImportError, ImportOperation, ImportOutcome};

/// 원본을 쓸 수 없어 기존 asset을 유지한 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceUnavailable {
    /// record에 source metadata가 없습니다.
    NoSourceMetadata,
    /// 외부 원본인데 호출자가 후보 경로를 주지 않았습니다.
    ExternalPathRequired,
    /// 제시한 외부 경로의 출처 지문이 기록과 다릅니다(같은 기기 key, 다른 경로).
    OriginMismatch,
    /// 외부 원본 지문이 다른 기기(또는 바뀐 key)로 기록되어 이 기기에서 확인할 수 없습니다.
    /// 사용자에게 원본 파일을 다시 선택받아 [`AssetStore::update_from_selected_source`]로
    /// 갱신해야 합니다.
    OriginUnverifiable,
    /// 원본 파일이 없습니다.
    NotFound,
    /// 원본 경로가 일반 파일이 아닙니다.
    NotAFile,
    /// 프로젝트 내부 원본이 symlink 등으로 프로젝트 밖으로 해석됩니다(읽지 않음).
    OutsideProject,
    /// 프로젝트 내부 원본(기록 경로 또는 해석 경로)이 고정 거부 영역입니다(읽지 않음,
    /// [`crate::project_source`]).
    ProtectedPath(ProtectedArea),
    /// 호출자의 프로젝트 원본 정책이 거부했습니다(신뢰되지 않은 프로젝트, Git 무시 파일 등 —
    /// 읽지 않음).
    DeniedByPolicy,
}

impl fmt::Display for SourceUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            SourceUnavailable::NoSourceMetadata => "원본 정보가 없습니다",
            SourceUnavailable::ExternalPathRequired => "외부 원본 경로를 지정해야 합니다",
            SourceUnavailable::OriginMismatch => "지정한 경로가 기록된 원본과 다릅니다",
            SourceUnavailable::OriginUnverifiable => {
                "다른 기기에서 기록된 원본이라 확인할 수 없습니다. 원본 파일을 다시 선택하세요"
            }
            SourceUnavailable::NotFound => "원본 파일이 없습니다",
            SourceUnavailable::NotAFile => "원본이 일반 파일이 아닙니다",
            SourceUnavailable::OutsideProject => "프로젝트 내부 원본이 프로젝트 밖을 가리킵니다",
            SourceUnavailable::ProtectedPath(area) => {
                return write!(f, "보호 영역의 파일은 원본으로 읽지 않습니다: {area}")
            }
            SourceUnavailable::DeniedByPolicy => {
                "프로젝트 원본 정책이 이 파일을 원본으로 허용하지 않습니다"
            }
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

/// 재import할 원본을 정하는 방법.
#[derive(Clone, Copy)]
enum SourceChoice<'a> {
    /// record의 source metadata(외부면 지문으로 확인할 후보 경로, 내부면 호출자 정책).
    Recorded {
        external_candidate: Option<&'a Path>,
        project_sources: &'a dyn ProjectSourcePolicy,
    },
    /// 사용자가 이번 실행에서 직접 고른 경로(지문 확인·내부 원본 판정 없음).
    Selected(&'a Path),
}

/// 판정을 통과한 원본.
struct ResolvedSource {
    /// canonical 원본 경로.
    path: PathBuf,
    /// 기록된 내부 원본이면 판정을 통과한 canonical project root 기준 `/` 상대 경로.
    approved_project_path: Option<String>,
    /// 기록된 원본이면 판정 때 `lstat`한 항목 identity(import는 이 항목만 읽음).
    pin: Option<FileIdentity>,
}

impl ResolvedSource {
    fn unpinned(path: PathBuf) -> ResolvedSource {
        ResolvedSource {
            path,
            approved_project_path: None,
            pin: None,
        }
    }
}

/// 판정한 canonical 경로를 `lstat`합니다(따라가지 않음). 일반 파일이면 identity, 아니면
/// [`SourceUnavailable::NotAFile`]·[`SourceUnavailable::NotFound`].
fn checked_file(resolved: &Path) -> Result<Result<FileIdentity, SourceUnavailable>, ImportError> {
    match FileIdentity::lstat(resolved) {
        Ok((metadata, identity)) if metadata.file_type().is_file() => Ok(Ok(identity)),
        Ok(_) => Ok(Err(SourceUnavailable::NotAFile)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Err(SourceUnavailable::NotFound)),
        Err(e) => Err(open_error(e)),
    }
}

impl AssetStore {
    /// source metadata의 원본을 다시 읽어 새 hash asset을 만듭니다(참조 변경은 반환만).
    ///
    /// `external_source`는 외부 원본일 때만 쓰이는 후보 경로입니다(내부 원본은 무시).
    /// `project_sources`는 내부 원본일 때만 쓰이는 호출자 정책입니다
    /// ([`crate::project_source`]). 고정 거부 영역(`.git`·Stellar 저장 디렉터리·`assets/`)은
    /// 정책과 무관하게 읽지 않고, 나머지는 정책이 명시적으로 허용해야 읽습니다. 신뢰되지 않은
    /// 프로젝트는 [`crate::project_source::DenyProjectSources`]를 넘깁니다.
    ///
    /// import 정책(XIII.5.7)은 항상 적용됩니다. 원본이 최대 크기를 넘으면
    /// [`ImportError::TooLarge`], media type이 더 이상 허용되지 않으면 [`ImportError::Policy`]이며,
    /// 두 경우 모두 아무 것도 쓰지 않아 기존 asset이 유지됩니다.
    pub fn update_from_source(
        &self,
        asset_id: AssetId,
        current: &AssetRecord,
        external_source: Option<&Path>,
        policy: &AssetImportPolicy,
        project_sources: &dyn ProjectSourcePolicy,
    ) -> Result<UpdateFromSourceOutcome, ImportError> {
        let choice = SourceChoice::Recorded {
            external_candidate: external_source,
            project_sources,
        };
        self.update_from_source_inner(asset_id, current, choice, policy)
    }

    /// 사용자가 **직접 고른** 원본 경로로 갱신합니다(출처 지문 확인 없음, 정책은 적용).
    ///
    /// [`SourceUnavailable::OriginUnverifiable`](다른 기기에서 기록) 등으로 사용자에게 원본을
    /// 다시 선택받은 뒤에만 호출합니다. record·Git·캐시에서 온 경로를 넘기지 않습니다(그 경우는
    /// [`AssetStore::update_from_source`]의 후보 경로). 사용자 선택이므로 내부 원본 판정
    /// ([`crate::project_source`])도 거치지 않습니다. 새 record의 source metadata는 고른
    /// 경로에서 새로 만듭니다(외부면 이 기기 key의 지문).
    pub fn update_from_selected_source(
        &self,
        asset_id: AssetId,
        current: &AssetRecord,
        selected: &Path,
        policy: &AssetImportPolicy,
    ) -> Result<UpdateFromSourceOutcome, ImportError> {
        let choice = SourceChoice::Selected(selected);
        self.update_from_source_inner(asset_id, current, choice, policy)
    }

    fn update_from_source_inner(
        &self,
        asset_id: AssetId,
        current: &AssetRecord,
        choice: SourceChoice<'_>,
        policy: &AssetImportPolicy,
    ) -> Result<UpdateFromSourceOutcome, ImportError> {
        let resolved = match choice {
            SourceChoice::Recorded {
                external_candidate,
                project_sources,
            } => self.resolve_source(current, external_candidate, project_sources)?,
            SourceChoice::Selected(path) => resolve_file(path)?.map(ResolvedSource::unpinned),
        };
        let source = match resolved {
            Ok(source) => source,
            Err(reason) => return Ok(UpdateFromSourceOutcome::SourceUnavailable(reason)),
        };
        let mut request = policy
            .request(&source.path, &current.media_type)
            .map_err(ImportError::Policy)?;
        // 기록된 내부 원본: 판정을 통과한 경로로만 import합니다. 판정 뒤 경로가 바뀌면 import가
        // 쓰기 전에 다시 거부합니다(방어 심층화). 사용자가 고른 경로는 새 위치로 기록합니다.
        let recorded = match choice {
            SourceChoice::Recorded { .. } => current.source_metadata.as_ref(),
            SourceChoice::Selected(_) => None,
        };
        let project_location = match recorded.map(|m| m.location()) {
            Some(SourceLocation::Project { relative_path }) => Some(relative_path.clone()),
            _ => None,
        };
        if let Some(approved) = source.approved_project_path {
            request = request.requiring_project_source(approved);
        }
        // 판정한 항목만 읽습니다(경로가 바뀌어도 판정 뒤 다른 파일을 읽지 않음, 보안 리뷰 S4).
        if let Some(pin) = source.pin {
            request = request.pinned_to(pin);
        }
        let mut imported = match self.import_file(&request) {
            Ok(imported) => imported,
            Err(ImportError::SourceMetadata(SourceMetadataError::OutsideProject)) => {
                return Ok(UpdateFromSourceOutcome::SourceUnavailable(
                    SourceUnavailable::OutsideProject,
                ))
            }
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
        if let Some(relative_path) = project_location {
            // 내부 원본 위치는 기록된 상대 경로 그대로 둡니다(프로젝트 안 symlink를 거쳐도
            // 위치가 바뀌지 않음). 보존 field는 이어받지 않습니다(XIII.5.1 후속 수정).
            imported.record.source_metadata =
                Some(SourceMetadata::project(relative_path).map_err(ImportError::SourceMetadata)?);
        }
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
        project_sources: &dyn ProjectSourcePolicy,
    ) -> Result<Result<ResolvedSource, SourceUnavailable>, ImportError> {
        let Some(metadata) = &current.source_metadata else {
            return Ok(Err(SourceUnavailable::NoSourceMetadata));
        };
        match metadata.location() {
            SourceLocation::Project { relative_path } => {
                self.resolve_project_source(metadata, relative_path, project_sources)
            }
            SourceLocation::External { origin, .. } => {
                self.resolve_external_source(metadata, origin, external_source)
            }
        }
    }

    /// 기록된 내부 원본을 판정합니다([`crate::project_source`] 순서: 기록 경로 고정 거부 →
    /// 프로젝트 안 해석 → 해석 경로 고정 거부 → 일반 파일 → 호출자 정책).
    fn resolve_project_source(
        &self,
        metadata: &SourceMetadata,
        recorded: &str,
        project_sources: &dyn ProjectSourcePolicy,
    ) -> Result<Result<ResolvedSource, SourceUnavailable>, ImportError> {
        // 파일 시스템에 접근하기 전에 기록 경로를 거부합니다.
        if let Some(area) = protected_project_path(recorded) {
            return Ok(Err(SourceUnavailable::ProtectedPath(area)));
        }
        let Some(candidate) = metadata.project_path(self.root()) else {
            return Ok(Err(SourceUnavailable::NotFound));
        };
        let resolved = match std::fs::canonicalize(&candidate) {
            Ok(path) => path,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return Ok(Err(SourceUnavailable::NotFound))
            }
            Err(e) => return Err(open_error(e)),
        };
        // 구성 요소 단위 비교(`/p/project-old`는 `/p/project` 안이 아님).
        let root = std::fs::canonicalize(self.root()).map_err(open_error)?;
        let Ok(relative) = resolved.strip_prefix(&root) else {
            return Ok(Err(SourceUnavailable::OutsideProject));
        };
        let mut parts: Vec<&str> = Vec::new();
        for component in relative.components() {
            let Component::Normal(part) = component else {
                return Ok(Err(SourceUnavailable::OutsideProject));
            };
            parts.push(part.to_str().ok_or(ImportError::SourceMetadata(
                SourceMetadataError::NonUtf8Path,
            ))?);
        }
        // 프로젝트 안 symlink가 보호 영역(Git 메타데이터 등)을 가리키는 경우.
        if let Some(area) = protected_area(parts.iter().copied()) {
            return Ok(Err(SourceUnavailable::ProtectedPath(area)));
        }
        if parts.is_empty() {
            return Ok(Err(SourceUnavailable::NotAFile));
        }
        let identity = match checked_file(&resolved)? {
            Ok(identity) => identity,
            Err(reason) => return Ok(Err(reason)),
        };
        let resolved_relative = parts.join("/");
        let candidate = ProjectSourceCandidate::new(recorded, &resolved_relative, &resolved);
        if project_sources.decide(&candidate) != ProjectSourceDecision::Allow {
            return Ok(Err(SourceUnavailable::DeniedByPolicy));
        }
        Ok(Ok(ResolvedSource {
            path: resolved,
            approved_project_path: Some(resolved_relative),
            pin: Some(identity),
        }))
    }

    /// 기록된 외부 원본의 후보 경로를 이 기기 key 출처 지문으로 확인합니다.
    fn resolve_external_source(
        &self,
        metadata: &SourceMetadata,
        origin: &OriginFingerprint,
        external_source: Option<&Path>,
    ) -> Result<Result<ResolvedSource, SourceUnavailable>, ImportError> {
        let key = self
            .origin_key()
            .map_err(|e| ImportError::SourceMetadata(SourceMetadataError::OriginKey(e)))?;
        // 다른 기기 key의 지문은 어떤 후보 경로로도 확인할 수 없습니다.
        if origin.key_id() != key.key_id() {
            return Ok(Err(SourceUnavailable::OriginUnverifiable));
        }
        let Some(candidate) = external_source else {
            return Ok(Err(SourceUnavailable::ExternalPathRequired));
        };
        let resolved = match std::fs::canonicalize(candidate) {
            Ok(path) => path,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return Ok(Err(SourceUnavailable::NotFound))
            }
            Err(e) => return Err(open_error(e)),
        };
        match metadata.check_external_origin(&key, &resolved) {
            OriginCheck::Matches => {}
            OriginCheck::ForeignKey => return Ok(Err(SourceUnavailable::OriginUnverifiable)),
            OriginCheck::DifferentOrigin | OriginCheck::NotExternal => {
                return Ok(Err(SourceUnavailable::OriginMismatch))
            }
        }
        let identity = match checked_file(&resolved)? {
            Ok(identity) => identity,
            Err(reason) => return Ok(Err(reason)),
        };
        Ok(Ok(ResolvedSource {
            path: resolved,
            approved_project_path: None,
            pin: Some(identity),
        }))
    }
}

/// 원본 열기 단계 I/O 오류.
fn open_error(e: std::io::Error) -> ImportError {
    ImportError::Io {
        operation: ImportOperation::OpenSource,
        kind: e.kind(),
        message: e.to_string(),
    }
}

/// 사용자가 고른 경로를 해석합니다. 없거나 일반 파일이 아니면 `Ok(Err(reason))`.
fn resolve_file(path: &Path) -> Result<Result<PathBuf, SourceUnavailable>, ImportError> {
    match std::fs::canonicalize(path) {
        Ok(resolved) if resolved.is_file() => Ok(Ok(resolved)),
        Ok(_) => Ok(Err(SourceUnavailable::NotAFile)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Err(SourceUnavailable::NotFound)),
        Err(e) => Err(open_error(e)),
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
    use crate::origin::OriginKey;
    use crate::project_source::DenyProjectSources;
    use crate::store::test_support::{
        files_under, shipped_policy, test_origin_key, test_store, AllowProjectSources, TempDir,
    };
    use crate::store::ImportRequest;
    use std::cell::RefCell;
    use stellar_storage::hash::ContentHash;

    /// 출하 config 정책(image/png 허용, 512 MiB).
    fn policy() -> AssetImportPolicy {
        shipped_policy()
    }

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
        let store = test_store(&project);
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

        let outcome = fx
            .store
            .update_from_source(id(), &current, None, &policy(), &AllowProjectSources)
            .unwrap();
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
            fx.store
                .update_from_source(id(), &current, None, &policy(), &AllowProjectSources)
                .unwrap(),
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
            fx.store
                .update_from_source(id(), &current, None, &policy(), &AllowProjectSources)
                .unwrap(),
            UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::NotFound)
        );
        // 디렉터리로 바뀐 원본도 마찬가지입니다.
        std::fs::create_dir_all(&source).unwrap();
        assert_eq!(
            fx.store
                .update_from_source(id(), &current, None, &policy(), &AllowProjectSources)
                .unwrap(),
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
                .update_from_source(
                    id(),
                    &current,
                    Some(&source),
                    &policy(),
                    &AllowProjectSources
                )
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
            fx.store
                .update_from_source(id(), &current, None, &policy(), &AllowProjectSources)
                .unwrap(),
            UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::ExternalPathRequired)
        );
        assert_eq!(
            fx.store
                .update_from_source(
                    id(),
                    &current,
                    Some(&other),
                    &policy(),
                    &AllowProjectSources
                )
                .unwrap(),
            UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::OriginMismatch)
        );

        std::fs::write(&source, b"external v2").unwrap();
        let outcome = fx
            .store
            .update_from_source(
                id(),
                &current,
                Some(&source),
                &policy(),
                &AllowProjectSources,
            )
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
            .matches_external_origin(&test_origin_key(), &source));

        std::fs::remove_file(&source).unwrap();
        assert_eq!(
            fx.store
                .update_from_source(
                    id(),
                    &current,
                    Some(&source),
                    &policy(),
                    &AllowProjectSources
                )
                .unwrap(),
            UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::NotFound)
        );
    }

    /// 다른 기기(다른 key)에서 기록된 외부 원본 fixture.
    #[test]
    fn external_source_from_other_machine_requires_user_selection() {
        let fx = fixture();
        let source = fx.outside.join("brand.png");
        // 다른 기기가 같은 경로에서 import해 commit한 record.
        let other_machine =
            AssetStore::new(&fx.project).with_origin_key(OriginKey::from_bytes([0x99; 32]));
        std::fs::write(&source, b"v1").unwrap();
        let current = other_machine
            .import_file(&ImportRequest::new(&source, png()))
            .unwrap()
            .record;
        std::fs::write(&source, b"v2").unwrap();
        let before = files_under(&fx.project.join("assets"));

        // 이 기기에서는 정확히 같은 경로를 주어도 확인할 수 없습니다(경로 추측 확인 불가).
        for candidate in [None, Some(source.as_path())] {
            assert_eq!(
                fx.store
                    .update_from_source(id(), &current, candidate, &policy(), &AllowProjectSources)
                    .unwrap(),
                UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::OriginUnverifiable),
                "{candidate:?}"
            );
        }
        assert_eq!(files_under(&fx.project.join("assets")), before, "변경 없음");

        // 사용자가 원본을 다시 고르면 갱신하고, 새 record는 이 기기 key의 지문을 씁니다.
        let outcome = fx
            .store
            .update_from_selected_source(id(), &current, &source, &policy())
            .unwrap();
        let UpdateFromSourceOutcome::Updated { update, .. } = outcome else {
            panic!("Updated 기대: {outcome:?}");
        };
        assert_eq!(update.next.content_hash, ContentHash::compute(b"v2"));
        let next_source = update.next.source_metadata.as_ref().unwrap();
        assert!(next_source.matches_external_origin(&test_origin_key(), &source));
        // 이후에는 이 기기에서 후보 경로로 확인됩니다.
        std::fs::write(&source, b"v3").unwrap();
        assert!(matches!(
            fx.store
                .update_from_source(
                    id(),
                    &update.next,
                    Some(&source),
                    &policy(),
                    &AllowProjectSources
                )
                .unwrap(),
            UpdateFromSourceOutcome::Updated { .. }
        ));

        // 고른 경로가 없거나 파일이 아니면 기존 asset 유지입니다.
        assert_eq!(
            fx.store
                .update_from_selected_source(
                    id(),
                    &current,
                    &fx.outside.join("missing.png"),
                    &policy()
                )
                .unwrap(),
            UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::NotFound)
        );
        assert_eq!(
            fx.store
                .update_from_selected_source(id(), &current, &fx.outside, &policy())
                .unwrap(),
            UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::NotAFile)
        );
    }

    #[test]
    fn external_source_without_configured_key_is_an_error() {
        let fx = fixture();
        let source = fx.outside.join("brand.png");
        let current = import(&fx, &source, b"v1");
        let unkeyed = AssetStore::new(&fx.project);
        assert_eq!(
            unkeyed
                .update_from_source(
                    id(),
                    &current,
                    Some(&source),
                    &policy(),
                    &AllowProjectSources
                )
                .unwrap_err(),
            ImportError::SourceMetadata(SourceMetadataError::OriginKey(
                crate::origin::OriginKeyError::Unconfigured
            ))
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
        let outcome = fx
            .store
            .update_from_source(id(), &current, None, &policy(), &AllowProjectSources)
            .unwrap();
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
        let UpdateFromSourceOutcome::Updated { update, .. } = fx
            .store
            .update_from_source(id(), &current, None, &policy(), &AllowProjectSources)
            .unwrap()
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

    /// Git으로 들어온 record와 저장소 안 symlink가 프로젝트 밖 파일을 가리키는 공격 fixture.
    #[cfg(unix)]
    #[test]
    fn project_source_symlink_escaping_project_is_rejected() {
        let fx = fixture();
        let secret_dir = fx.outside.join("home").join(".ssh");
        std::fs::create_dir_all(&secret_dir).unwrap();
        std::fs::write(secret_dir.join("id_rsa"), b"SECRET KEY").unwrap();
        std::fs::write(fx.outside.join("secret.png"), b"SECRET PNG").unwrap();
        // 디렉터리 symlink(project/link → ../outside/home/.ssh)와 파일 symlink.
        std::os::unix::fs::symlink("../outside/home/.ssh", fx.project.join("link")).unwrap();
        std::os::unix::fs::symlink(
            "../../outside/secret.png",
            fx.project.join("images").join("logo.png"),
        )
        .unwrap();

        for relative in ["link/id_rsa", "images/logo.png"] {
            let current = AssetRecord::new(
                ContentHash::compute(b"old"),
                png(),
                3,
                Some(SourceMetadata::project(relative).unwrap()),
            );
            assert_eq!(
                fx.store
                    .update_from_source(id(), &current, None, &policy(), &AllowProjectSources)
                    .unwrap(),
                UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::OutsideProject),
                "{relative}"
            );
        }
        // 프로젝트 밖 내용이 assets/로 복사되지 않았습니다.
        assert!(files_under(&fx.project.join("assets")).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn import_guard_rejects_outside_source_before_writing() {
        let fx = fixture();
        let outside = fx.outside.join("a.png");
        std::fs::write(&outside, b"outside").unwrap();
        for request in [
            ImportRequest::new(&outside, png()).requiring_project_source("a.png"),
            ImportRequest::new(&outside, png())
                .without_source_metadata()
                .requiring_project_source("a.png"),
        ] {
            assert_eq!(
                fx.store.import_file(&request).unwrap_err(),
                ImportError::SourceMetadata(SourceMetadataError::OutsideProject)
            );
        }
        assert!(files_under(&fx.project.join("assets")).is_empty());

        // 판정한 경로와 다른 내부 파일로 해석되면(판정 뒤 경로 변경) 쓰지 않습니다.
        let inside = fx.project.join("images").join("in.png");
        std::fs::write(&inside, b"inside").unwrap();
        assert_eq!(
            fx.store
                .import_file(
                    &ImportRequest::new(&inside, png())
                        .requiring_project_source("images/other.png")
                )
                .unwrap_err(),
            ImportError::SourceChanged
        );
        assert!(files_under(&fx.project.join("assets")).is_empty());

        // 판정한 내부 원본은 통과합니다.
        let imported = fx
            .store
            .import_file(
                &ImportRequest::new(&inside, png()).requiring_project_source("images/in.png"),
            )
            .unwrap();
        assert!(matches!(
            imported.record.source_metadata.unwrap().location(),
            SourceLocation::Project { .. }
        ));
    }

    /// 판정 호출을 기록하는 정책(호출되지 않아야 하는 경로 확인용).
    #[derive(Default)]
    struct RecordingPolicy {
        seen: RefCell<Vec<(String, String)>>,
        deny: Vec<&'static str>,
    }

    impl ProjectSourcePolicy for RecordingPolicy {
        fn decide(&self, candidate: &ProjectSourceCandidate<'_>) -> ProjectSourceDecision {
            self.seen.borrow_mut().push((
                candidate.recorded_path().to_string(),
                candidate.resolved_path().to_string(),
            ));
            // `git check-ignore` 모사: 기록 경로나 해석 경로 중 하나라도 무시 대상이면 거부.
            if self
                .deny
                .iter()
                .any(|d| *d == candidate.recorded_path() || *d == candidate.resolved_path())
            {
                ProjectSourceDecision::Deny
            } else {
                ProjectSourceDecision::Allow
            }
        }
    }

    fn project_record(relative: &str) -> AssetRecord {
        AssetRecord::new(
            ContentHash::compute(b"old"),
            png(),
            3,
            Some(SourceMetadata::project(relative).unwrap()),
        )
    }

    /// 보안 리뷰 S1 재현: commit된 record가 `.git/config`(credential URL)·Stellar 저장 영역·
    /// `assets/`를 원본으로 지정해 tracked `assets/`로 복사하게 하는 공격 fixture.
    #[test]
    fn committed_record_cannot_copy_git_metadata_or_storage_into_assets() {
        let fx = fixture();
        let git = fx.project.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        let secret = b"[remote \"origin\"]\n\turl = https://user:ghp_SECRET@example.invalid/r\n";
        std::fs::write(git.join("config"), secret).unwrap();
        std::fs::write(git.join("HEAD"), b"ref: refs/heads/main\n").unwrap();
        let nested = fx.project.join("vendor").join("lib").join(".git");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("config"), secret).unwrap();
        std::fs::create_dir_all(fx.project.join("design")).unwrap();
        std::fs::write(fx.project.join("design").join("current"), b"pointer").unwrap();
        std::fs::create_dir_all(fx.project.join(".stellar").join("cache")).unwrap();
        std::fs::write(fx.project.join(".stellar").join("cache").join("x"), b"c").unwrap();
        // 이미 들여온 asset content 파일 자체를 원본으로 지정.
        let existing = import(&fx, &fx.outside.join("e.png"), b"existing");
        let existing_path = fx
            .store
            .locate_content(&existing.content_hash, existing.size)
            .unwrap()
            .unwrap()
            .to_string();
        let before = files_under(&fx.project.join("assets"));

        let policy_calls = RecordingPolicy::default();
        for (relative, area) in [
            (".git/config", ProtectedArea::GitMetadata),
            (".git/HEAD", ProtectedArea::GitMetadata),
            (".GIT/config", ProtectedArea::GitMetadata),
            (".g\u{200c}it/config", ProtectedArea::GitMetadata),
            ("vendor/lib/.git/config", ProtectedArea::GitMetadata),
            ("design/current", ProtectedArea::StellarStorage),
            ("Design/current", ProtectedArea::StellarStorage),
            (".stellar/cache/x", ProtectedArea::StellarStorage),
            (existing_path.as_str(), ProtectedArea::AssetStore),
        ] {
            for project_sources in [
                &AllowProjectSources as &dyn ProjectSourcePolicy,
                &policy_calls,
            ] {
                assert_eq!(
                    fx.store
                        .update_from_source(
                            id(),
                            &project_record(relative),
                            None,
                            &policy(),
                            project_sources
                        )
                        .unwrap(),
                    UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::ProtectedPath(
                        area
                    )),
                    "{relative:?}"
                );
            }
        }
        // 고정 거부는 호출자 정책보다 먼저이며, 아무 것도 복사하지 않습니다.
        assert!(policy_calls.seen.borrow().is_empty());
        let after = files_under(&fx.project.join("assets"));
        assert_eq!(after, before);
        for path in after {
            assert_ne!(std::fs::read(path).unwrap(), secret);
        }
    }

    /// 프로젝트 안 symlink(commit 가능)가 보호 영역을 가리키는 fixture — 해석 경로로 거부.
    #[cfg(unix)]
    #[test]
    fn project_symlink_into_protected_area_is_rejected() {
        let fx = fixture();
        let git = fx.project.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("config"), b"url = https://user:ghp_SECRET@x/r").unwrap();
        std::fs::create_dir_all(fx.project.join("design")).unwrap();
        std::fs::write(fx.project.join("design").join("current"), b"pointer").unwrap();
        let images = fx.project.join("images");
        std::os::unix::fs::symlink("../.git/config", images.join("cfg.png")).unwrap();
        std::os::unix::fs::symlink("../.git", images.join("meta")).unwrap();
        std::os::unix::fs::symlink("../design/current", images.join("doc.png")).unwrap();

        let policy_calls = RecordingPolicy::default();
        for (relative, area) in [
            ("images/cfg.png", ProtectedArea::GitMetadata),
            ("images/meta/config", ProtectedArea::GitMetadata),
            ("images/doc.png", ProtectedArea::StellarStorage),
        ] {
            assert_eq!(
                fx.store
                    .update_from_source(
                        id(),
                        &project_record(relative),
                        None,
                        &policy(),
                        &policy_calls
                    )
                    .unwrap(),
                UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::ProtectedPath(area)),
                "{relative}"
            );
        }
        assert!(policy_calls.seen.borrow().is_empty());
        assert!(files_under(&fx.project.join("assets")).is_empty());
    }

    /// 고정 거부 밖의 내부 원본은 호출자 정책이 명시적으로 허용해야 읽습니다.
    #[cfg(unix)]
    #[test]
    fn project_sources_require_explicit_policy_allow() {
        let fx = fixture();
        std::fs::write(fx.project.join(".env"), b"API_TOKEN=secret").unwrap();
        let logo = fx.project.join("images").join("logo.png");
        std::fs::write(&logo, b"logo v2").unwrap();
        std::os::unix::fs::symlink("../.env", fx.project.join("images").join("env.png")).unwrap();

        // 신뢰되지 않은 프로젝트: 모든 내부 원본 거부(일반 이미지 포함).
        for relative in [".env", "images/logo.png", "images/env.png"] {
            assert_eq!(
                fx.store
                    .update_from_source(
                        id(),
                        &project_record(relative),
                        None,
                        &policy(),
                        &DenyProjectSources
                    )
                    .unwrap(),
                UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::DeniedByPolicy),
                "{relative}"
            );
        }
        assert!(files_under(&fx.project.join("assets")).is_empty());

        // 신뢰된 프로젝트 + Git 무시 파일 거부: symlink를 거친 해석 경로도 정책에 전달됩니다.
        let ignored = RecordingPolicy {
            deny: vec![".env"],
            ..RecordingPolicy::default()
        };
        for relative in [".env", "images/env.png"] {
            assert_eq!(
                fx.store
                    .update_from_source(id(), &project_record(relative), None, &policy(), &ignored)
                    .unwrap(),
                UpdateFromSourceOutcome::SourceUnavailable(SourceUnavailable::DeniedByPolicy),
                "{relative}"
            );
        }
        assert!(files_under(&fx.project.join("assets")).is_empty());
        let outcome = fx
            .store
            .update_from_source(
                id(),
                &project_record("images/logo.png"),
                None,
                &policy(),
                &ignored,
            )
            .unwrap();
        assert!(matches!(outcome, UpdateFromSourceOutcome::Updated { .. }));
        assert_eq!(
            *ignored.seen.borrow(),
            vec![
                (".env".to_string(), ".env".to_string()),
                ("images/env.png".to_string(), ".env".to_string()),
                ("images/logo.png".to_string(), "images/logo.png".to_string()),
            ]
        );
        let copied = files_under(&fx.project.join("assets"));
        assert_eq!(copied.len(), 1);
        assert_eq!(std::fs::read(&copied[0]).unwrap(), b"logo v2");
    }

    #[cfg(unix)]
    #[test]
    fn project_source_symlink_inside_project_keeps_recorded_location() {
        let fx = fixture();
        let real = fx.project.join("images").join("real.png");
        std::fs::write(&real, b"v1").unwrap();
        std::os::unix::fs::symlink("real.png", fx.project.join("images").join("alias.png"))
            .unwrap();
        let current = AssetRecord::new(
            ContentHash::compute(b"v1"),
            png(),
            2,
            Some(SourceMetadata::project("images/alias.png").unwrap()),
        );
        std::fs::write(&real, b"v2").unwrap();
        let outcome = fx
            .store
            .update_from_source(id(), &current, None, &policy(), &AllowProjectSources)
            .unwrap();
        let UpdateFromSourceOutcome::Updated { update, .. } = outcome else {
            panic!("Updated 기대: {outcome:?}");
        };
        // 프로젝트 내부 원본은 외부로 바뀌지 않고 기록된 상대 경로를 유지합니다.
        assert_eq!(update.next.source_metadata, current.source_metadata);
        assert_eq!(update.next.content_hash, ContentHash::compute(b"v2"));
    }

    #[test]
    fn unavailable_reasons_are_described() {
        assert!(SourceUnavailable::OriginMismatch
            .to_string()
            .contains("원본"));
    }
}

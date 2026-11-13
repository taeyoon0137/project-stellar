//! 에셋 저장소와 외부 파일 import (XIII.5.2).
//!
//! 기본 가져오기는 프로젝트 내부 복사입니다(아키텍처 29.1). 원본을 두 번 streaming으로
//! 읽습니다.
//!
//! 1. **hash pass**: 고정 크기 buffer([`STREAM_BUFFER_SIZE`])로 읽으며 content hash와 크기를
//!    계산합니다. 파일 전체를 메모리에 올리지 않습니다.
//! 2. **copy pass**: hash로 정한 `assets/<prefix>/<hash>.<safe-ext>`(같은 디렉터리의 temp
//!    파일)로 같은 buffer 단위로 복사하면서 다시 hash합니다. 두 pass의 hash·크기가 다르면
//!    (import 도중 원본 변경) temp를 지우고 [`ImportError::SourceChanged`]로 실패합니다.
//!    검증이 끝난 temp만 flush → fsync → 최종 이름 rename → 부모 디렉터리 sync합니다
//!    (storage V.2.5와 같은 protocol, [`crate::atomic_write`], XIII.5.4).
//!
//! # dedup (XIII.5.3)
//!
//! hash pass 뒤 같은 shard 디렉터리에서 같은 content hash의 content 파일(확장자 무관)을
//! 찾습니다. 이름과 크기가 맞는 후보는 **내용을 다시 hash해** 같을 때만 재사용합니다. 그런
//! 파일이 있으면 copy pass를 **건너뛰고**(재복사·재기록 없음) 그 파일을 가리키는 새 record를
//! 돌려줍니다([`ImportOutcome::Deduplicated`]). record는 AssetId를 포함하지 않으므로 호출자가
//! 새 AssetId로 같은 content를 참조할 수 있습니다([`crate::catalog::AssetCatalog`]). 크기나
//! 내용이 다른 같은 이름 파일(bit rot·잘못 commit된 파일 등 손상)은 재사용하지 않고 copy
//! pass로 새로 기록합니다(같은 이름이면 atomic rename으로 교체되어 복구). 재hash는 기존 파일을
//! 한 번 읽지만 copy pass(쓰기·fsync)보다 가볍습니다.
//!
//! # symlink 디렉터리
//!
//! `assets/`와 shard 디렉터리(`assets/<prefix>`)는 Git으로 commit될 수 있으므로 symlink면
//! 따라가지 않습니다. dedup 조회·copy pass 모두 `lstat`으로 확인하고
//! [`ImportError::SymlinkedAssetsDirectory`]로 거부합니다(아무 것도 읽거나 쓰지 않음). 없는
//! 디렉터리는 한 단계씩 `create_dir`로 만듭니다.
//!
//! 원본 위치는 optional metadata입니다([`ImportRequest::without_source_metadata`]로 생략).
//! 기록할 때는 절대 로컬 경로가 아니라 프로젝트 상대 경로 또는 파일 이름+출처 지문만
//! 남깁니다([`SourceMetadata::from_import_path`]). 출처 지문은 프로젝트 밖 기기 key의
//! HMAC이며([`crate::origin`]), key는 [`AssetStore::with_origin_key_file`]로 지정합니다.

use std::fmt;
use std::fs::File;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use stellar_storage::hash::{ContentHash, ParseContentHashError};

use crate::atomic_write::{
    write_atomic_streamed, AtomicWriteError, NoopObserver, WriteObserver, WriteStage,
};
use crate::hash_stream::{hash_reader_with, StreamDigest, StreamHashError, STREAM_BUFFER_SIZE};
use crate::media_type::MediaType;
use crate::origin::{OriginKey, OriginKeyError, OriginKeyFile, OriginKeyProvider};
use crate::path::{assets_dir_state, AssetObjectPath, AssetsDirState, SafeExtension, ASSETS_DIR};
use crate::policy::PolicyViolation;
use crate::record::{AssetRecord, SourceLocation, SourceMetadata, SourceMetadataError};

/// 실패한 import 단계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportOperation {
    /// 원본 열기·메타데이터 조회.
    OpenSource,
    /// hash pass 읽기.
    HashSource,
    /// 기존 content 파일 조회(dedup).
    LocateContent,
    /// shard 디렉터리 생성.
    CreateDirectory,
    /// copy pass 내용 기록(원본 읽기·temp 쓰기).
    CopyContent,
    /// atomic write 단계(temp 생성·flush·sync·rename·부모 sync, XIII.5.4).
    Write(WriteStage),
}

impl fmt::Display for ImportOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportOperation::OpenSource => f.write_str("open-source"),
            ImportOperation::HashSource => f.write_str("hash-source"),
            ImportOperation::LocateContent => f.write_str("locate-content"),
            ImportOperation::CreateDirectory => f.write_str("create-directory"),
            ImportOperation::CopyContent => f.write_str("copy-content"),
            ImportOperation::Write(stage) => write!(f, "write-{stage}"),
        }
    }
}

/// import 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// 파일 I/O 실패.
    Io {
        /// 실패 단계.
        operation: ImportOperation,
        /// I/O 오류 종류.
        kind: ErrorKind,
        /// 원인 메시지.
        message: String,
    },
    /// 원본이 일반 파일이 아닙니다(디렉터리 등).
    NotAFile,
    /// hash pass와 copy pass의 내용이 다릅니다(import 도중 원본 변경). 판정을 통과한 내부
    /// 원본 경로가 import 시점에 다른 파일로 해석될 때도 같습니다(XIII.5.5).
    SourceChanged,
    /// 원본 위치를 metadata로 기록할 수 없습니다.
    SourceMetadata(SourceMetadataError),
    /// digest를 content hash로 만들 수 없습니다(발생하지 않아야 함).
    Hash(ParseContentHashError),
    /// 원본이 요청의 최대 단일 파일 크기를 넘습니다(XIII.5.7). 아무 것도 쓰지 않습니다.
    TooLarge {
        /// 확인한 크기(byte). streaming 중 초과를 발견하면 그때까지 읽은 크기입니다.
        size: u64,
        /// 허용 최대 크기(byte, 이 값까지 허용).
        max: u64,
    },
    /// import 정책 위반(예: 허용되지 않은 media type, XIII.5.7).
    Policy(PolicyViolation),
    /// `assets/` 또는 shard 디렉터리가 symlink입니다. 따라가지 않고 아무 것도 읽거나 쓰지
    /// 않습니다(commit된 symlink가 content 쓰기를 프로젝트 밖으로 돌리는 것을 막음).
    SymlinkedAssetsDirectory {
        /// symlink인 디렉터리 경로.
        path: PathBuf,
    },
}

impl ImportError {
    fn io(operation: ImportOperation, e: &std::io::Error) -> ImportError {
        ImportError::Io {
            operation,
            kind: e.kind(),
            message: e.to_string(),
        }
    }

    fn stream(operation: ImportOperation, e: StreamHashError) -> ImportError {
        match e {
            StreamHashError::Io(e) => ImportError::io(operation, &e),
            StreamHashError::Hash(e) => ImportError::Hash(e),
        }
    }
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::Io {
                operation, message, ..
            } => write!(f, "에셋 import {operation} 실패: {message}"),
            ImportError::NotAFile => f.write_str("에셋 원본이 일반 파일이 아닙니다"),
            ImportError::SourceChanged => f.write_str("에셋 import 도중 원본 내용이 바뀌었습니다"),
            ImportError::SourceMetadata(e) => {
                write!(f, "에셋 원본 위치를 기록할 수 없습니다: {e}")
            }
            ImportError::Hash(e) => write!(f, "에셋 content hash를 만들 수 없습니다: {e}"),
            ImportError::TooLarge { size, max } => write!(
                f,
                "에셋 원본이 최대 크기를 넘습니다: {size} bytes > {max} bytes"
            ),
            ImportError::Policy(e) => write!(f, "에셋 import 정책 위반: {e}"),
            ImportError::SymlinkedAssetsDirectory { path } => write!(
                f,
                "에셋 디렉터리가 symlink라 따라가지 않습니다: {}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ImportError {}

/// 외부 파일 import 요청.
///
/// # import 정책 강제(XIII.5.7)
///
/// crate 밖에서는 [`AssetImportPolicy::request`](crate::policy::AssetImportPolicy::request)로만
/// 만들 수 있습니다. 그래서 공개 import 경로는 항상 `config/asset-import.yaml` 정책(허용 media
/// type과 표의 저장 확장자, 최대 단일 파일 크기)을 거칩니다. 원본 확장자(`html`·`exe` 등)는
/// 저장 확장자로 쓰이지 않습니다.
///
/// ```
/// use stellar_assets::media_type::MediaType;
/// use stellar_assets::policy::AssetImportPolicy;
/// let policy = AssetImportPolicy::from_config_str(
///     "max_file_size_bytes: 10\nunknown_media_type: store_as_binary\nmedia_type: image/png png\n",
/// )
/// .unwrap();
/// let request = policy
///     .request("/x/page.html", &MediaType::new("text/html").unwrap())
///     .unwrap();
/// assert_eq!(request.extension().as_str(), "bin");
/// assert_eq!(request.max_size(), Some(10));
/// ```
///
/// 정책 없이 요청을 만들거나 정책이 정한 확장자·크기를 바꾸면 컴파일이 실패합니다.
///
/// ```compile_fail
/// use stellar_assets::media_type::MediaType;
/// use stellar_assets::store::ImportRequest;
/// let _ = ImportRequest::new("/x/page.html", MediaType::new("text/html").unwrap());
/// ```
///
/// ```compile_fail
/// use stellar_assets::media_type::MediaType;
/// use stellar_assets::path::SafeExtension;
/// use stellar_assets::policy::AssetImportPolicy;
/// let policy = AssetImportPolicy::from_config_str(
///     "max_file_size_bytes: 10\nunknown_media_type: store_as_binary\nmedia_type: image/png png\n",
/// )
/// .unwrap();
/// let png = MediaType::new("image/png").unwrap();
/// let _ = policy
///     .request("/x/a.png", &png)
///     .unwrap()
///     .with_extension(SafeExtension::new("html").unwrap())
///     .with_max_size(u64::MAX);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRequest {
    source: PathBuf,
    media_type: MediaType,
    extension: SafeExtension,
    record_source: bool,
    max_size: Option<u64>,
    /// 원본이 project root 안의 **이 상대 경로**로 해석되어야 합니다(내부 원본
    /// `Update from Source`가 판정을 통과시킨 경로, XIII.5.5).
    require_project_source: Option<String>,
}

impl ImportRequest {
    /// 원본 경로와 media type으로 정책 없는 요청을 만듭니다(crate 내부 전용).
    ///
    /// 확장자는 원본 확장자를 정규화한 값(비안전하면 `bin`)이고, 원본 위치를 기록하며,
    /// 크기 제한은 없습니다. 공개 경로는
    /// [`AssetImportPolicy::request`](crate::policy::AssetImportPolicy::request)가 이 값에 정책의
    /// media type·확장자·최대 크기를 채워 만듭니다.
    pub(crate) fn new(source: impl Into<PathBuf>, media_type: MediaType) -> ImportRequest {
        let source = source.into();
        let extension = SafeExtension::from_source_path(&source);
        ImportRequest {
            source,
            media_type,
            extension,
            record_source: true,
            max_size: None,
            require_project_source: None,
        }
    }

    /// 원본이 canonical project root 안의 `approved_relative`(`/` 구분)로 해석될 때만
    /// import합니다(아무 것도 쓰기 전 검사).
    ///
    /// 프로젝트 내부 원본의 `Update from Source`가 고정 거부·호출자 정책
    /// ([`crate::project_source`])을 통과한 경로를 넘깁니다. Git으로 들어온 record와 symlink가
    /// 판정 뒤 다른 파일을 `assets/`로 끌어오지 못하게 합니다. 밖으로 해석되면
    /// [`SourceMetadataError::OutsideProject`], 안이지만 다른 경로면
    /// [`ImportError::SourceChanged`]로 실패합니다.
    pub(crate) fn requiring_project_source(
        mut self,
        approved_relative: impl Into<String>,
    ) -> ImportRequest {
        self.require_project_source = Some(approved_relative.into());
        self
    }

    /// 최대 단일 파일 크기(byte, 이 값까지 허용)를 지정합니다(crate 내부 전용 — 정책 값).
    pub(crate) fn with_max_size(mut self, max_size: u64) -> ImportRequest {
        self.max_size = Some(max_size);
        self
    }

    /// 최대 단일 파일 크기. 공개 경로(정책 요청)에서는 항상 `Some`입니다.
    pub fn max_size(&self) -> Option<u64> {
        self.max_size
    }

    /// 저장 확장자를 지정합니다(crate 내부 전용 — 정책 표의 확장자).
    pub(crate) fn with_extension(mut self, extension: SafeExtension) -> ImportRequest {
        self.extension = extension;
        self
    }

    /// 원본 위치를 기록하지 않습니다(`source_metadata = None`).
    pub fn without_source_metadata(mut self) -> ImportRequest {
        self.record_source = false;
        self
    }

    /// 원본 경로.
    pub fn source(&self) -> &Path {
        &self.source
    }

    /// media type.
    pub fn media_type(&self) -> &MediaType {
        &self.media_type
    }

    /// 저장 확장자.
    pub fn extension(&self) -> &SafeExtension {
        &self.extension
    }

    /// 원본 위치 기록 여부.
    pub fn records_source(&self) -> bool {
        self.record_source
    }
}

/// content 파일을 새로 썼는지 여부.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportOutcome {
    /// 같은 content가 없어 copy pass로 새로 기록했습니다.
    Copied,
    /// 같은 content hash·크기이고 재hash로 내용이 확인된 파일이 이미 있어 재복사하지
    /// 않았습니다(XIII.5.3).
    Deduplicated,
}

/// import 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedAsset {
    /// 새 에셋 record(보존 field 없음).
    pub record: AssetRecord,
    /// 프로젝트 root 기준 content 파일 경로. dedup이면 기존 파일(확장자가 요청과 다를 수 있음).
    pub object_path: AssetObjectPath,
    /// 복사 여부.
    pub outcome: ImportOutcome,
}

/// 외부 원본 출처 지문용 기기 key 공급원(XIII.5.1 후속 수정, [`crate::origin`]).
#[derive(Debug, Clone, PartialEq, Eq)]
enum OriginKeySource {
    /// 설정 없음 — 외부 원본을 기록·확인하면 [`OriginKeyError::Unconfigured`].
    Unconfigured,
    /// 이미 읽은 key.
    Key(OriginKey),
    /// OS 사용자 데이터 영역의 key 파일(처음 필요할 때 생성).
    File(OriginKeyFile),
}

/// 프로젝트의 `assets/` content 저장소.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetStore {
    root: PathBuf,
    origin_key: OriginKeySource,
}

impl AssetStore {
    /// 프로젝트 root로 저장소를 엽니다(파일 I/O 없음).
    ///
    /// 출처 key가 없으므로 프로젝트 밖 원본의 위치를 기록하거나 확인할 수 없습니다.
    /// 외부 원본을 다루려면 [`AssetStore::with_origin_key_file`]로 key 파일을 지정합니다.
    pub fn new(project_root: impl Into<PathBuf>) -> AssetStore {
        AssetStore {
            root: project_root.into(),
            origin_key: OriginKeySource::Unconfigured,
        }
    }

    /// 프로젝트 밖 OS 사용자 데이터 영역의 출처 key 파일을 씁니다.
    ///
    /// 파일은 외부 원본을 처음 기록·확인할 때 만들어지며(`0600`), project root 안이면
    /// [`OriginKeyError::InsideProject`]로 거부합니다.
    pub fn with_origin_key_file(mut self, file: OriginKeyFile) -> AssetStore {
        self.origin_key = OriginKeySource::File(file);
        self
    }

    /// 이미 읽은 출처 key를 씁니다(테스트·key를 직접 관리하는 호출자용).
    pub fn with_origin_key(mut self, key: OriginKey) -> AssetStore {
        self.origin_key = OriginKeySource::Key(key);
        self
    }

    /// 프로젝트 root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 이 기기의 출처 key. 파일 공급원은 프로젝트 밖인지 확인한 뒤 읽거나 만듭니다.
    pub(crate) fn origin_key(&self) -> Result<OriginKey, OriginKeyError> {
        match &self.origin_key {
            OriginKeySource::Unconfigured => Err(OriginKeyError::Unconfigured),
            OriginKeySource::Key(key) => Ok(key.clone()),
            OriginKeySource::File(file) => {
                file.ensure_outside(&self.root)?;
                file.load_or_create()
            }
        }
    }

    /// content 파일의 실제 경로.
    pub fn content_path(&self, object: &AssetObjectPath) -> PathBuf {
        self.root.join(object.to_relative_path())
    }

    /// 외부 파일을 streaming hash 후 `assets/`로 복사합니다.
    pub fn import_file(&self, request: &ImportRequest) -> Result<ImportedAsset, ImportError> {
        self.import_file_observed(request, &mut NoopObserver)
    }

    /// [`AssetStore::import_file`]에 쓰기 단계 관찰자를 주입한 버전(fault injection 테스트용).
    ///
    /// 관찰자가 `Err`를 반환하면 그 단계를 실행하지 않고 중단합니다(storage V.2.5와 같은 규약).
    pub fn import_file_observed(
        &self,
        request: &ImportRequest,
        observer: &mut dyn WriteObserver,
    ) -> Result<ImportedAsset, ImportError> {
        // metadata를 먼저 만들어 기록할 수 없는 경로면 아무 것도 쓰기 전에 실패합니다.
        let source_metadata = if request.record_source || request.require_project_source.is_some() {
            let metadata = SourceMetadata::from_import_path(
                &self.root,
                &request.source,
                &StoreOriginKey(self),
            )
            .map_err(ImportError::SourceMetadata)?;
            if let Some(approved) = &request.require_project_source {
                match metadata.location() {
                    SourceLocation::Project { relative_path } if relative_path == approved => {}
                    // 판정한 뒤 경로가 바뀌었습니다(다른 내부 파일 — 판정을 다시 해야 함).
                    SourceLocation::Project { .. } => return Err(ImportError::SourceChanged),
                    SourceLocation::External { .. } => {
                        return Err(ImportError::SourceMetadata(
                            SourceMetadataError::OutsideProject,
                        ))
                    }
                }
            }
            request.record_source.then_some(metadata)
        } else {
            None
        };

        let mut buffer = vec![0u8; STREAM_BUFFER_SIZE];
        let digest = hash_source(&request.source, &mut buffer, request.max_size)?;
        let (object_path, outcome) =
            match self.locate_verified_content(&digest.hash, digest.size, &mut buffer)? {
                Some(existing) => (existing, ImportOutcome::Deduplicated),
                None => {
                    let object_path = AssetObjectPath::new(&digest.hash, &request.extension);
                    self.copy_content(
                        &request.source,
                        &digest,
                        &object_path,
                        &mut buffer,
                        observer,
                    )?;
                    (object_path, ImportOutcome::Copied)
                }
            };

        Ok(ImportedAsset {
            record: AssetRecord::new(
                digest.hash,
                request.media_type.clone(),
                digest.size,
                source_metadata,
            ),
            object_path,
            outcome,
        })
    }

    /// 내용이 확인된 같은 content의 기존 파일을 찾습니다(확장자 무관).
    ///
    /// 이름(content hash)·크기가 맞는 후보를 파일 이름 사전순으로 **재hash**해 처음 일치하는
    /// 파일을 돌려줍니다(결정론). shard 디렉터리가 없으면 `None`. 크기·내용이 다르거나 읽을 수
    /// 없는 후보는 손상으로 보고 재사용하지 않습니다. `assets/`나 shard 디렉터리가 symlink면
    /// 따라가지 않고 [`ImportError::SymlinkedAssetsDirectory`]입니다.
    pub fn locate_content(
        &self,
        hash: &ContentHash,
        size: u64,
    ) -> Result<Option<AssetObjectPath>, ImportError> {
        let mut buffer = vec![0u8; STREAM_BUFFER_SIZE];
        self.locate_verified_content(hash, size, &mut buffer)
    }

    /// [`AssetStore::locate_content`]의 buffer 재사용 버전(import hash pass buffer).
    fn locate_verified_content(
        &self,
        hash: &ContentHash,
        size: u64,
        buffer: &mut [u8],
    ) -> Result<Option<AssetObjectPath>, ImportError> {
        let locate_error = |e: std::io::Error| ImportError::io(ImportOperation::LocateContent, &e);
        let assets = self.root.join(ASSETS_DIR);
        let dir = self.root.join(AssetObjectPath::shard_dir_for(hash));
        for path in [&assets, &dir] {
            match assets_dir_state(path).map_err(locate_error)? {
                AssetsDirState::Directory => {}
                AssetsDirState::Missing | AssetsDirState::NotDirectory => return Ok(None),
                AssetsDirState::Symlink => {
                    return Err(ImportError::SymlinkedAssetsDirectory { path: path.clone() })
                }
            }
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(locate_error(e)),
        };
        let prefix = &hash.digest_hex()[..2];
        let mut candidates: Vec<AssetObjectPath> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(locate_error)?;
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let Some((path, entry_hash)) = AssetObjectPath::parse(prefix, &name) else {
                continue;
            };
            if entry_hash != *hash {
                continue;
            }
            // DirEntry::metadata는 symlink를 따라가지 않습니다(symlink는 일반 파일 아님).
            let metadata = entry.metadata().map_err(locate_error)?;
            if !metadata.is_file() || metadata.len() != size {
                continue;
            }
            candidates.push(path);
        }
        candidates.sort();
        for candidate in candidates {
            if self.content_matches(&candidate, hash, size, buffer) {
                return Ok(Some(candidate));
            }
        }
        Ok(None)
    }

    /// 기존 content 파일을 다시 hash해 기대 hash·크기와 같은지 확인합니다.
    ///
    /// 읽기 실패도 "확인할 수 없음"이므로 재사용하지 않습니다(copy pass가 새로 기록).
    fn content_matches(
        &self,
        object: &AssetObjectPath,
        hash: &ContentHash,
        size: u64,
        buffer: &mut [u8],
    ) -> bool {
        let Ok(mut file) = File::open(self.content_path(object)) else {
            return false;
        };
        match hash_reader_with(&mut file, buffer, |_| Ok(())) {
            Ok(digest) => digest.hash == *hash && digest.size == size,
            Err(_) => false,
        }
    }

    /// copy pass: streaming atomic write(XIII.5.4)로 temp에 복사·재hash 검증 후
    /// flush → fsync → rename → 부모 디렉터리 sync합니다.
    fn copy_content(
        &self,
        source: &Path,
        expected: &StreamDigest,
        object: &AssetObjectPath,
        buffer: &mut [u8],
        observer: &mut dyn WriteObserver,
    ) -> Result<(), ImportError> {
        // `assets/`와 shard를 한 단계씩 확인·생성합니다. symlink는 따라가지 않습니다
        // (`create_dir_all`은 symlink 디렉터리를 그대로 통과하므로 쓰지 않음).
        let assets = self.root.join(ASSETS_DIR);
        ensure_real_dir(&assets)?;
        let dir = assets.join(object.prefix());
        ensure_real_dir(&dir)?;
        let target = dir.join(object.file_name());
        let mut input = open_source(source)?;
        write_atomic_streamed(&target, observer, |output| {
            let copied = hash_reader_with(&mut input, buffer, |chunk| output.write_all(chunk))
                .map_err(|e| ImportError::stream(ImportOperation::CopyContent, e))?;
            if copied != *expected {
                return Err(ImportError::SourceChanged);
            }
            Ok(())
        })
        .map_err(|e| match e {
            AtomicWriteError::Stage { stage, error } => {
                ImportError::io(ImportOperation::Write(stage), &error)
            }
            AtomicWriteError::Body(e) => e,
        })
    }
}

/// 실제 디렉터리를 보장합니다. 없으면 만들고(`create_dir`, 한 단계), symlink면 따라가지 않고
/// [`ImportError::SymlinkedAssetsDirectory`], 디렉터리가 아니면 I/O 오류입니다.
fn ensure_real_dir(path: &Path) -> Result<(), ImportError> {
    let create_error = |e: std::io::Error| ImportError::io(ImportOperation::CreateDirectory, &e);
    let state = assets_dir_state(path).map_err(create_error)?;
    let state = match state {
        AssetsDirState::Missing => match std::fs::create_dir(path) {
            Ok(()) => return Ok(()),
            // 동시에 다른 import가 만든 경우: 무엇이 생겼는지 다시 확인합니다.
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                assets_dir_state(path).map_err(create_error)?
            }
            Err(e) => return Err(create_error(e)),
        },
        other => other,
    };
    match state {
        AssetsDirState::Directory => Ok(()),
        AssetsDirState::Symlink => Err(ImportError::SymlinkedAssetsDirectory {
            path: path.to_path_buf(),
        }),
        AssetsDirState::Missing | AssetsDirState::NotDirectory => {
            Err(create_error(std::io::Error::from(ErrorKind::NotADirectory)))
        }
    }
}

/// store의 key 공급원을 metadata 생성에 넘기는 내부 어댑터(key를 공개 API로 노출하지 않음).
struct StoreOriginKey<'a>(&'a AssetStore);

impl OriginKeyProvider for StoreOriginKey<'_> {
    fn origin_key(&self) -> Result<OriginKey, OriginKeyError> {
        self.0.origin_key()
    }
}

/// 원본을 열고 일반 파일인지 확인합니다. 파일과 현재 크기를 돌려줍니다.
fn open_source_with_len(path: &Path) -> Result<(File, u64), ImportError> {
    let file = File::open(path).map_err(|e| ImportError::io(ImportOperation::OpenSource, &e))?;
    let metadata = file
        .metadata()
        .map_err(|e| ImportError::io(ImportOperation::OpenSource, &e))?;
    if !metadata.is_file() {
        return Err(ImportError::NotAFile);
    }
    Ok((file, metadata.len()))
}

/// 원본을 열고 일반 파일인지 확인합니다.
fn open_source(path: &Path) -> Result<File, ImportError> {
    open_source_with_len(path).map(|(file, _)| file)
}

/// hash pass. `max_size`를 넘으면 읽기 전(메타데이터 크기) 또는 읽는 도중(파일이 커진 경우)
/// [`ImportError::TooLarge`]로 멈춥니다.
fn hash_source(
    path: &Path,
    buffer: &mut [u8],
    max_size: Option<u64>,
) -> Result<StreamDigest, ImportError> {
    let (mut file, len) = open_source_with_len(path)?;
    if let Some(max) = max_size {
        if len > max {
            return Err(ImportError::TooLarge { size: len, max });
        }
    }
    let mut seen = 0u64;
    let mut exceeded = false;
    let result = hash_reader_with(&mut file, buffer, |chunk| {
        seen += chunk.len() as u64;
        if max_size.is_some_and(|max| seen > max) {
            exceeded = true;
            return Err(std::io::Error::other("최대 크기 초과"));
        }
        Ok(())
    });
    if let (true, Some(max)) = (exceeded, max_size) {
        return Err(ImportError::TooLarge { size: seen, max });
    }
    result.map_err(|e| ImportError::stream(ImportOperation::HashSource, e))
}

/// temp 이름의 대상 이름 뒤 표식.
const TEMP_MARKER: &str = ".tmp-";

/// 대상과 같은 디렉터리에 만들 temp 파일 경로(`.<file>.tmp-<pid>-<n>`).
///
/// `.`로 시작해 content 파일 이름 규칙(`<algorithm>-<hex>.<ext>`)과 겹치지 않습니다.
/// `<pid>`·`<n>`은 선행 0 없는 10진수입니다. 이 형식의 해석은 [`parse_temp_file_name`]이
/// 맡습니다(GC 정리 대상 판정, XIII.5.6) — 둘을 함께 바꿔야 합니다.
pub(crate) fn temp_path_for(target: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = target
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed".to_string());
    target.with_file_name(format!(
        ".{file_name}{TEMP_MARKER}{}-{n}",
        std::process::id()
    ))
}

/// [`temp_path_for`]가 content 파일(`assets/<prefix>/<canonical 이름>`)에 대해 만드는 temp 이름
/// `.<canonical content 이름>.tmp-<pid>-<n>`이면 대상 content hash를 돌려줍니다.
///
/// 정확히 이 형식만 인정합니다: 대상 이름이 shard `prefix`의 canonical content 이름
/// ([`AssetObjectPath::parse`])이고, `<pid>`는 `u32`, `<n>`은 `u64`의 선행 0 없는 10진수입니다.
/// `.DS_Store.tmp-1-2`처럼 `.tmp-`만 포함한 다른 숨김 파일은 `None`입니다.
pub(crate) fn parse_temp_file_name(prefix: &str, name: &str) -> Option<ContentHash> {
    let rest = name.strip_prefix('.')?;
    let (target, suffix) = rest.rsplit_once(TEMP_MARKER)?;
    let (pid, counter) = suffix.split_once('-')?;
    let canonical_decimal = |text: &str, value: u64| value.to_string() == text;
    let pid_ok = pid
        .parse::<u32>()
        .is_ok_and(|v| canonical_decimal(pid, u64::from(v)));
    let counter_ok = counter
        .parse::<u64>()
        .is_ok_and(|v| canonical_decimal(counter, v));
    if !pid_ok || !counter_ok {
        return None;
    }
    AssetObjectPath::parse(prefix, target).map(|(_, hash)| hash)
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 테스트용 임시 디렉터리(drop 시 삭제).
    pub struct TempDir {
        pub path: PathBuf,
    }

    impl TempDir {
        pub fn new(tag: &str) -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("stellar-assets-{tag}-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            // umask와 무관하게 소유자 전용으로 둡니다(출처 key 디렉터리 검사가 group/other 쓰기를
            // 거부하므로, key 디렉터리로 쓰는 테스트가 umask에 따라 달라지지 않게 함).
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            // macOS temp_dir는 symlink(/var → /private/var)이므로 해석된 경로를 씁니다.
            TempDir {
                path: std::fs::canonicalize(&path).unwrap(),
            }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    /// 디렉터리 아래 모든 파일 경로(재귀, 정렬).
    pub fn files_under(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&current) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push(path);
                }
            }
        }
        out.sort();
        out
    }

    /// 테스트용 기기 출처 key.
    pub fn test_origin_key() -> crate::origin::OriginKey {
        crate::origin::OriginKey::from_bytes([0x42; 32])
    }

    /// 출하 config(`config/asset-import.yaml`) 정책.
    pub fn shipped_policy() -> crate::policy::AssetImportPolicy {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/asset-import.yaml"
        );
        crate::policy::AssetImportPolicy::from_config_file(Path::new(path)).unwrap()
    }

    /// 테스트 key를 설정한 저장소(외부 원본 기록 가능).
    pub fn test_store(project_root: &Path) -> super::AssetStore {
        super::AssetStore::new(project_root).with_origin_key(test_origin_key())
    }

    /// 고정 거부를 통과한 모든 내부 원본을 허용하는 테스트 정책(신뢰된 프로젝트 모사).
    pub struct AllowProjectSources;

    impl crate::project_source::ProjectSourcePolicy for AllowProjectSources {
        fn decide(
            &self,
            _candidate: &crate::project_source::ProjectSourceCandidate<'_>,
        ) -> crate::project_source::ProjectSourceDecision {
            crate::project_source::ProjectSourceDecision::Allow
        }
    }

    /// 패턴으로 채운 bytes.
    pub fn patterned(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u32).wrapping_mul(31).wrapping_add(seed as u32) as u8)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{files_under, patterned, test_origin_key, test_store, TempDir};
    use super::*;

    fn png() -> MediaType {
        MediaType::new("image/png").unwrap()
    }

    struct Fixture {
        _dir: TempDir,
        project: PathBuf,
        outside: PathBuf,
        store: AssetStore,
    }

    fn fixture() -> Fixture {
        let dir = TempDir::new("import");
        let project = dir.path.join("project");
        let outside = dir.path.join("outside");
        std::fs::create_dir_all(project.join("assets")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let store = test_store(&project);
        Fixture {
            _dir: dir,
            project,
            outside,
            store,
        }
    }

    #[test]
    fn external_file_is_copied_to_content_address() {
        let fx = fixture();
        let bytes = patterned(3 * STREAM_BUFFER_SIZE + 5, 1);
        let source = fx.outside.join("Logo.PNG");
        std::fs::write(&source, &bytes).unwrap();

        let imported = fx
            .store
            .import_file(&ImportRequest::new(&source, png()))
            .unwrap();
        let hash = ContentHash::compute(&bytes);
        assert_eq!(imported.record.content_hash, hash);
        assert_eq!(imported.record.size, bytes.len() as u64);
        assert_eq!(imported.record.media_type, png());
        assert_eq!(
            imported.object_path.to_string(),
            format!(
                "assets/{}/sha256-{}.png",
                &hash.digest_hex()[..2],
                hash.digest_hex()
            )
        );
        let content = fx.store.content_path(&imported.object_path);
        assert_eq!(std::fs::read(&content).unwrap(), bytes);
        // 외부 원본은 파일 이름과 출처 지문만 기록합니다.
        let source_meta = imported.record.source_metadata.as_ref().unwrap();
        assert_eq!(source_meta.file_name(), "Logo.PNG");
        assert!(source_meta.matches_external_origin(&test_origin_key(), &source));
        // 원본은 그대로이고, temp 파일이 남지 않습니다.
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        assert_eq!(files_under(&fx.project.join("assets")), vec![content]);
    }

    #[test]
    fn project_file_records_relative_path() {
        let fx = fixture();
        std::fs::create_dir_all(fx.project.join("images")).unwrap();
        let source = fx.project.join("images").join("icon.svg");
        std::fs::write(&source, b"<svg/>").unwrap();
        let imported = fx
            .store
            .import_file(&ImportRequest::new(
                &source,
                MediaType::new("image/svg+xml").unwrap(),
            ))
            .unwrap();
        assert_eq!(
            imported.record.source_metadata.unwrap().location(),
            &SourceLocation::Project {
                relative_path: "images/icon.svg".to_string()
            }
        );
        assert!(imported.object_path.file_name().ends_with(".svg"));
    }

    #[test]
    fn external_source_metadata_requires_origin_key() {
        let fx = fixture();
        let unkeyed = AssetStore::new(&fx.project);
        let outside = fx.outside.join("a.png");
        std::fs::write(&outside, b"outside").unwrap();
        assert_eq!(
            unkeyed
                .import_file(&ImportRequest::new(&outside, png()))
                .unwrap_err(),
            ImportError::SourceMetadata(SourceMetadataError::OriginKey(
                OriginKeyError::Unconfigured
            ))
        );
        assert!(files_under(&fx.project.join("assets")).is_empty());
        // 내부 원본·metadata 생략은 key가 필요 없습니다.
        assert!(unkeyed
            .import_file(&ImportRequest::new(&outside, png()).without_source_metadata())
            .is_ok());
        std::fs::create_dir_all(fx.project.join("images")).unwrap();
        let inside = fx.project.join("images").join("b.png");
        std::fs::write(&inside, b"inside").unwrap();
        assert!(unkeyed
            .import_file(&ImportRequest::new(&inside, png()))
            .is_ok());
    }

    #[test]
    fn origin_key_file_is_created_on_first_external_use_outside_project() {
        let fx = fixture();
        let data = fx._dir.path.join("user-data");
        let file = OriginKeyFile::in_data_dir(&data).unwrap();
        let store = AssetStore::new(&fx.project).with_origin_key_file(file.clone());

        std::fs::create_dir_all(fx.project.join("images")).unwrap();
        let inside = fx.project.join("images").join("in.png");
        std::fs::write(&inside, b"inside").unwrap();
        store
            .import_file(&ImportRequest::new(&inside, png()))
            .unwrap();
        assert!(!file.path().exists(), "내부 원본은 key를 만들지 않음");

        let outside = fx.outside.join("out.png");
        std::fs::write(&outside, b"outside").unwrap();
        let imported = store
            .import_file(&ImportRequest::new(&outside, png()))
            .unwrap();
        let key = file.load().unwrap().expect("처음 외부 원본에서 생성");
        assert!(imported
            .record
            .source_metadata
            .unwrap()
            .matches_external_origin(&key, &outside));
        // key 파일은 프로젝트 밖에만 있고 프로젝트에는 key 사본이 없습니다.
        assert!(!file.path().starts_with(&fx.project));
    }

    #[test]
    fn origin_key_file_inside_project_is_rejected() {
        let fx = fixture();
        let file = OriginKeyFile::new(fx.project.join(".stellar").join("origin-key")).unwrap();
        let store = AssetStore::new(&fx.project).with_origin_key_file(file.clone());
        let outside = fx.outside.join("a.png");
        std::fs::write(&outside, b"outside").unwrap();
        assert_eq!(
            store
                .import_file(&ImportRequest::new(&outside, png()))
                .unwrap_err(),
            ImportError::SourceMetadata(SourceMetadataError::OriginKey(
                OriginKeyError::InsideProject {
                    path: file.path().to_path_buf()
                }
            ))
        );
        assert!(!file.path().exists());
        assert!(files_under(&fx.project.join("assets")).is_empty());
    }

    #[test]
    fn source_metadata_is_optional() {
        let fx = fixture();
        let source = fx.outside.join("a.png");
        std::fs::write(&source, b"png").unwrap();
        let imported = fx
            .store
            .import_file(&ImportRequest::new(&source, png()).without_source_metadata())
            .unwrap();
        assert_eq!(imported.record.source_metadata, None);
    }

    #[test]
    fn extension_follows_request() {
        let fx = fixture();
        let source = fx.outside.join("noext");
        std::fs::write(&source, b"data").unwrap();
        let default = fx
            .store
            .import_file(&ImportRequest::new(&source, png()))
            .unwrap();
        assert!(default.object_path.file_name().ends_with(".bin"));
        std::fs::write(&source, b"other data").unwrap();
        let explicit = fx
            .store
            .import_file(
                &ImportRequest::new(&source, png())
                    .with_extension(SafeExtension::new("png").unwrap()),
            )
            .unwrap();
        assert!(explicit.object_path.file_name().ends_with(".png"));
    }

    /// dedup fixture: 서로 다른 경로·이름·확장자의 같은 bytes 3개와 다른 bytes 1개.
    #[test]
    fn same_content_is_stored_once_and_shared_by_new_asset_ids() {
        use crate::catalog::AssetCatalog;
        use crate::id::AssetId;

        let fx = fixture();
        let bytes = patterned(2 * STREAM_BUFFER_SIZE + 3, 4);
        std::fs::create_dir_all(fx.outside.join("copy")).unwrap();
        std::fs::create_dir_all(fx.project.join("images")).unwrap();
        let sources = [
            fx.outside.join("logo.png"),
            fx.outside.join("copy").join("logo-final.png"),
            fx.project.join("images").join("logo.jpeg"),
        ];
        for source in &sources {
            std::fs::write(source, &bytes).unwrap();
        }

        let first = fx
            .store
            .import_file(&ImportRequest::new(&sources[0], png()))
            .unwrap();
        assert_eq!(first.outcome, ImportOutcome::Copied);
        let content = fx.store.content_path(&first.object_path);
        let before = std::fs::metadata(&content).unwrap();

        let mut catalog = AssetCatalog::new();
        let id = |n: u8| AssetId::from_bytes([n; 16]);
        catalog.insert(id(1), first.record.clone()).unwrap();
        for (n, source) in sources.iter().enumerate().skip(1) {
            let again = fx
                .store
                .import_file(&ImportRequest::new(source, png()))
                .unwrap();
            assert_eq!(again.outcome, ImportOutcome::Deduplicated, "{source:?}");
            // 확장자가 달라도(.jpeg) 기존 content 파일을 가리킵니다.
            assert_eq!(again.object_path, first.object_path);
            assert_eq!(again.record.content_hash, first.record.content_hash);
            assert_ne!(again.record.source_metadata, first.record.source_metadata);
            catalog.insert(id(n as u8 + 1), again.record).unwrap();
        }

        // 파일은 하나뿐이고 재기록되지 않았습니다(inode·mtime·내용 불변).
        assert_eq!(
            files_under(&fx.project.join("assets")),
            vec![content.clone()]
        );
        let after = std::fs::metadata(&content).unwrap();
        assert_eq!(after.modified().unwrap(), before.modified().unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(after.ino(), before.ino(), "rename 교체 없음");
        }
        assert_eq!(std::fs::read(&content).unwrap(), bytes);

        // 새 AssetId 3개가 같은 content를 참조합니다.
        assert_eq!(
            catalog.ids_for_content(&first.record.content_hash),
            vec![id(1), id(2), id(3)]
        );
        assert_eq!(catalog.content_hashes().len(), 1);

        // 다른 bytes는 dedup되지 않습니다.
        let other = fx.outside.join("other.png");
        std::fs::write(&other, b"different").unwrap();
        let distinct = fx
            .store
            .import_file(&ImportRequest::new(&other, png()))
            .unwrap();
        assert_eq!(distinct.outcome, ImportOutcome::Copied);
        assert_eq!(files_under(&fx.project.join("assets")).len(), 2);
    }

    #[test]
    fn damaged_existing_content_is_replaced_not_reused() {
        let fx = fixture();
        let bytes = b"real content".to_vec();
        let source = fx.outside.join("a.png");
        std::fs::write(&source, &bytes).unwrap();
        let hash = ContentHash::compute(&bytes);
        let object = AssetObjectPath::new(&hash, &SafeExtension::new("png").unwrap());
        let content = fx.store.content_path(&object);
        std::fs::create_dir_all(content.parent().unwrap()).unwrap();
        std::fs::write(&content, b"trunc").unwrap(); // 같은 이름, 다른 크기(손상)

        assert_eq!(
            fx.store.locate_content(&hash, bytes.len() as u64).unwrap(),
            None
        );
        let imported = fx
            .store
            .import_file(&ImportRequest::new(&source, png()))
            .unwrap();
        assert_eq!(imported.outcome, ImportOutcome::Copied);
        assert_eq!(
            std::fs::read(&content).unwrap(),
            bytes,
            "atomic 교체로 복구"
        );
    }

    /// 같은 이름·**같은 크기**로 손상된 기존 파일(bit rot, 잘못 commit된 파일) fixture.
    #[test]
    fn same_size_corrupted_content_is_not_reused() {
        let fx = fixture();
        let bytes = patterned(STREAM_BUFFER_SIZE + 7, 3);
        let source = fx.outside.join("a.png");
        std::fs::write(&source, &bytes).unwrap();
        let hash = ContentHash::compute(&bytes);
        let size = bytes.len() as u64;
        let mut corrupted = bytes.clone();
        corrupted[STREAM_BUFFER_SIZE] ^= 0x01; // 1 bit 변경, 크기 동일
        let png_object = AssetObjectPath::new(&hash, &SafeExtension::new("png").unwrap());
        let png_path = fx.store.content_path(&png_object);
        std::fs::create_dir_all(png_path.parent().unwrap()).unwrap();
        std::fs::write(&png_path, &corrupted).unwrap();

        assert_eq!(fx.store.locate_content(&hash, size).unwrap(), None);
        let imported = fx
            .store
            .import_file(&ImportRequest::new(&source, png()))
            .unwrap();
        assert_eq!(
            imported.outcome,
            ImportOutcome::Copied,
            "손상 파일 재사용 안 함"
        );
        assert_eq!(imported.object_path, png_object);
        assert_eq!(
            std::fs::read(&png_path).unwrap(),
            bytes,
            "atomic 교체로 복구"
        );
        assert_eq!(
            fx.store.locate_content(&hash, size).unwrap(),
            Some(png_object.clone())
        );

        // 사전순으로 앞선 다른 확장자 후보(.bin)가 손상되어 있어도 건너뛰고 확인된 파일을 씁니다.
        let bin_object = AssetObjectPath::new(&hash, &SafeExtension::binary());
        assert!(bin_object < png_object);
        std::fs::write(fx.store.content_path(&bin_object), &corrupted).unwrap();
        assert_eq!(
            fx.store.locate_content(&hash, size).unwrap(),
            Some(png_object.clone())
        );
        let again = fx
            .store
            .import_file(&ImportRequest::new(&source, png()))
            .unwrap();
        assert_eq!(again.outcome, ImportOutcome::Deduplicated);
        assert_eq!(again.object_path, png_object);
    }

    /// commit된 symlink(`assets/` 또는 shard)가 content 쓰기·dedup 읽기를 프로젝트 밖으로
    /// 돌리는 fixture.
    #[cfg(unix)]
    #[test]
    fn symlinked_assets_root_or_shard_is_never_followed() {
        let fx = fixture();
        let bytes = b"payload".to_vec();
        let source = fx.outside.join("a.png");
        std::fs::write(&source, &bytes).unwrap();
        let hash = ContentHash::compute(&bytes);
        let object = AssetObjectPath::new(&hash, &SafeExtension::new("png").unwrap());
        let request = ImportRequest::new(&source, png());

        // 1) shard가 symlink: 밖에 같은 content(dedup 미끼)가 있어도 읽지 않고, 쓰지도 않습니다.
        let target = fx.outside.join("redirected-shard");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join(object.file_name()), &bytes).unwrap();
        let shard = fx.project.join("assets").join(object.prefix());
        std::os::unix::fs::symlink(&target, &shard).unwrap();
        let expected = ImportError::SymlinkedAssetsDirectory {
            path: shard.clone(),
        };
        assert_eq!(
            fx.store
                .locate_content(&hash, bytes.len() as u64)
                .unwrap_err(),
            expected
        );
        assert_eq!(fx.store.import_file(&request).unwrap_err(), expected);
        assert_eq!(files_under(&target), vec![target.join(object.file_name())]);
        // shard가 없을 때도 copy pass가 symlink를 통과해 만들지 않습니다(copy_content 직접).
        std::fs::remove_file(target.join(object.file_name())).unwrap();
        let mut buffer = vec![0u8; STREAM_BUFFER_SIZE];
        let digest = hash_source(&source, &mut buffer, None).unwrap();
        assert_eq!(
            fx.store
                .copy_content(&source, &digest, &object, &mut buffer, &mut NoopObserver)
                .unwrap_err(),
            expected
        );
        assert!(files_under(&target).is_empty(), "밖에 쓰지 않음");
        std::fs::remove_file(&shard).unwrap();

        // 2) `assets/` 자체가 symlink.
        let outside_assets = fx.outside.join("redirected-assets");
        std::fs::create_dir_all(&outside_assets).unwrap();
        std::fs::remove_dir_all(fx.project.join("assets")).unwrap();
        std::os::unix::fs::symlink(&outside_assets, fx.project.join("assets")).unwrap();
        let err = fx.store.import_file(&request).unwrap_err();
        assert_eq!(
            err,
            ImportError::SymlinkedAssetsDirectory {
                path: fx.project.join("assets")
            }
        );
        assert!(err.to_string().contains("symlink"));
        assert!(files_under(&outside_assets).is_empty(), "밖에 쓰지 않음");

        // 실제 디렉터리로 되돌리면 정상 import됩니다(없는 assets/·shard는 새로 만듦).
        std::fs::remove_file(fx.project.join("assets")).unwrap();
        let imported = fx.store.import_file(&request).unwrap();
        assert_eq!(imported.outcome, ImportOutcome::Copied);
        assert_eq!(
            std::fs::read(fx.store.content_path(&object)).unwrap(),
            bytes
        );
    }

    #[test]
    fn locate_ignores_temp_foreign_and_missing_shards() {
        let fx = fixture();
        let hash = ContentHash::compute(b"zz");
        assert_eq!(
            fx.store.locate_content(&hash, 2).unwrap(),
            None,
            "shard 없음"
        );
        let dir = fx.project.join(AssetObjectPath::shard_dir_for(&hash));
        std::fs::create_dir_all(&dir).unwrap();
        let png = AssetObjectPath::new(&hash, &SafeExtension::new("png").unwrap());
        let bin = AssetObjectPath::new(&hash, &SafeExtension::binary());
        std::fs::write(dir.join(format!(".{}.tmp-1-0", png.file_name())), b"zz").unwrap();
        std::fs::write(dir.join("readme.txt"), b"zz").unwrap();
        assert_eq!(fx.store.locate_content(&hash, 2).unwrap(), None);
        // 같은 hash가 여러 확장자로 있으면 이름 사전순 첫 번째(결정론).
        std::fs::write(dir.join(png.file_name()), b"zz").unwrap();
        std::fs::write(dir.join(bin.file_name()), b"zz").unwrap();
        assert_eq!(fx.store.locate_content(&hash, 2).unwrap(), Some(bin));
    }

    #[test]
    fn empty_and_block_boundary_files_hash_correctly() {
        let fx = fixture();
        for len in [
            0,
            1,
            63,
            64,
            65,
            STREAM_BUFFER_SIZE - 1,
            STREAM_BUFFER_SIZE,
            STREAM_BUFFER_SIZE + 1,
        ] {
            let bytes = patterned(len, 9);
            let source = fx.outside.join(format!("f{len}.bin"));
            std::fs::write(&source, &bytes).unwrap();
            let imported = fx
                .store
                .import_file(&ImportRequest::new(&source, png()))
                .unwrap();
            assert_eq!(
                imported.record.content_hash,
                ContentHash::compute(&bytes),
                "len {len}"
            );
            assert_eq!(imported.record.size, len as u64);
            assert_eq!(
                std::fs::read(fx.store.content_path(&imported.object_path)).unwrap(),
                bytes
            );
        }
    }

    #[test]
    fn invalid_sources_write_nothing() {
        let fx = fixture();
        // 디렉터리 원본.
        assert_eq!(
            fx.store
                .import_file(&ImportRequest::new(&fx.outside, png()).without_source_metadata())
                .unwrap_err(),
            ImportError::NotAFile
        );
        // 없는 원본: metadata 기록 단계에서 먼저 실패합니다.
        let missing = fx.outside.join("missing.png");
        assert_eq!(
            fx.store
                .import_file(&ImportRequest::new(&missing, png()))
                .unwrap_err(),
            ImportError::SourceMetadata(SourceMetadataError::Resolve {
                kind: ErrorKind::NotFound
            })
        );
        let err = fx
            .store
            .import_file(&ImportRequest::new(&missing, png()).without_source_metadata())
            .unwrap_err();
        assert!(matches!(
            err,
            ImportError::Io {
                operation: ImportOperation::OpenSource,
                kind: ErrorKind::NotFound,
                ..
            }
        ));
        assert!(files_under(&fx.project.join("assets")).is_empty());
    }

    #[test]
    fn changed_source_between_passes_is_rejected_without_final_file() {
        let fx = fixture();
        let source = fx.outside.join("a.png");
        std::fs::write(&source, b"first").unwrap();
        let mut buffer = vec![0u8; STREAM_BUFFER_SIZE];
        let digest = hash_source(&source, &mut buffer, None).unwrap();
        // hash pass 후 원본이 바뀐 상황.
        std::fs::write(&source, b"second").unwrap();
        let object = AssetObjectPath::new(&digest.hash, &SafeExtension::new("png").unwrap());
        assert_eq!(
            fx.store
                .copy_content(&source, &digest, &object, &mut buffer, &mut NoopObserver)
                .unwrap_err(),
            ImportError::SourceChanged
        );
        assert!(!fx.store.content_path(&object).exists());
        assert!(
            files_under(&fx.project.join("assets")).is_empty(),
            "temp 정리"
        );
    }

    #[test]
    fn temp_names_are_hidden_and_unique() {
        let target = Path::new("/p/assets/ab/sha256-ab.png");
        let a = temp_path_for(target);
        let b = temp_path_for(target);
        assert_ne!(a, b);
        assert_eq!(a.parent(), target.parent(), "같은 디렉터리(atomic rename)");
        assert!(a
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with(".sha256-ab.png.tmp-"));
    }

    #[test]
    fn writer_temp_names_round_trip_through_parser() {
        let hash = ContentHash::compute(b"temp");
        let object = AssetObjectPath::new(&hash, &SafeExtension::new("png").unwrap());
        let target = Path::new("/p").join(object.to_relative_path());
        for _ in 0..3 {
            let temp = temp_path_for(&target);
            let name = temp.file_name().unwrap().to_str().unwrap();
            assert_eq!(
                parse_temp_file_name(object.prefix(), name),
                Some(hash),
                "{name}"
            );
            // 다른 shard에서는 인정하지 않습니다.
            assert_eq!(parse_temp_file_name("zz", name), None);
        }
    }

    #[test]
    fn error_messages_are_descriptive() {
        assert!(ImportError::SourceChanged.to_string().contains("원본"));
        let err = ImportError::io(
            ImportOperation::Write(WriteStage::Rename),
            &std::io::Error::from(ErrorKind::PermissionDenied),
        );
        assert!(err.to_string().contains("write-rename"));
    }

    /// 지정 단계 직전에 중단을 주입하는 관찰자.
    struct FailAt(WriteStage);

    impl WriteObserver for FailAt {
        fn before(&mut self, stage: WriteStage) -> std::io::Result<()> {
            if stage == self.0 {
                return Err(std::io::Error::other(format!("주입된 중단: {stage}")));
            }
            Ok(())
        }
    }

    /// 지정 단계 직전에 panic(정리 코드 미실행 = 강제 종료 모사)하는 관찰자.
    struct AbortAt(WriteStage);

    impl WriteObserver for AbortAt {
        fn before(&mut self, stage: WriteStage) -> std::io::Result<()> {
            if stage == self.0 {
                panic!("강제 종료 모사: {stage}");
            }
            Ok(())
        }
    }

    #[test]
    fn interrupted_import_never_leaves_partial_final_file() {
        let bytes = patterned(3 * STREAM_BUFFER_SIZE + 11, 2);
        for fail in WriteStage::ALL {
            let fx = fixture();
            let source = fx.outside.join("a.png");
            std::fs::write(&source, &bytes).unwrap();
            let request = ImportRequest::new(&source, png());
            let err = fx
                .store
                .import_file_observed(&request, &mut FailAt(fail))
                .unwrap_err();
            assert!(
                matches!(err, ImportError::Io { operation: ImportOperation::Write(stage), .. } if stage == fail),
                "{fail}: {err:?}"
            );
            let object = AssetObjectPath::new(&ContentHash::compute(&bytes), request.extension());
            let content = fx.store.content_path(&object);
            if fail == WriteStage::SyncParentDir {
                // rename 이후 중단: 최종 파일은 완전한 내용입니다.
                assert_eq!(std::fs::read(&content).unwrap(), bytes);
            } else {
                assert!(!content.exists(), "{fail}: 최종 파일 없음");
            }
            // 우아한 실패는 temp를 남기지 않습니다.
            let left = files_under(&fx.project.join("assets"));
            assert!(left.iter().all(|p| *p == content), "{fail}: {left:?}");

            // 재시도는 정상 완료됩니다(이미 완전한 파일이 있으면 dedup).
            let retry = fx.store.import_file(&request).unwrap();
            let expected = if fail == WriteStage::SyncParentDir {
                ImportOutcome::Deduplicated
            } else {
                ImportOutcome::Copied
            };
            assert_eq!(retry.outcome, expected, "{fail}");
            assert_eq!(std::fs::read(&content).unwrap(), bytes);
        }
    }

    #[test]
    fn forced_abort_after_copy_leaves_only_hidden_temp() {
        let bytes = patterned(2 * STREAM_BUFFER_SIZE + 1, 6);
        for abort in [WriteStage::Flush, WriteStage::SyncFile, WriteStage::Rename] {
            let fx = fixture();
            let source = fx.outside.join("a.png");
            std::fs::write(&source, &bytes).unwrap();
            let request = ImportRequest::new(&source, png());
            let store = fx.store.clone();
            let request_for_abort = request.clone();
            let outcome = std::panic::catch_unwind(move || {
                let _ = store.import_file_observed(&request_for_abort, &mut AbortAt(abort));
            });
            assert!(outcome.is_err(), "{abort}");

            let object = AssetObjectPath::new(&ContentHash::compute(&bytes), request.extension());
            let content = fx.store.content_path(&object);
            assert!(!content.exists(), "{abort}: 최종 이름에 내용 없음");
            // 정리되지 않은 temp는 남지만 content 파일로 해석되지 않습니다.
            let left = files_under(&fx.project.join("assets"));
            assert_eq!(left.len(), 1, "{abort}");
            assert!(left[0]
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with('.'));
            let size = bytes.len() as u64;
            assert_eq!(
                fx.store
                    .locate_content(&ContentHash::compute(&bytes), size)
                    .unwrap(),
                None
            );
            // 재import는 temp를 dedup 대상으로 쓰지 않고 새로 기록합니다.
            let retry = fx.store.import_file(&request).unwrap();
            assert_eq!(retry.outcome, ImportOutcome::Copied);
            assert_eq!(std::fs::read(&content).unwrap(), bytes);
        }
    }
}

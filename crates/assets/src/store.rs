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
//!    검증이 끝난 temp만 최종 이름으로 rename합니다.
//!
//! 원본 위치는 optional metadata입니다([`ImportRequest::without_source_metadata`]로 생략).
//! 기록할 때는 절대 로컬 경로가 아니라 프로젝트 상대 경로 또는 파일 이름+출처 지문만
//! 남깁니다([`SourceMetadata::from_import_path`]).

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use stellar_storage::hash::ParseContentHashError;

use crate::hash_stream::{hash_reader_with, StreamDigest, StreamHashError, STREAM_BUFFER_SIZE};
use crate::media_type::MediaType;
use crate::path::{AssetObjectPath, SafeExtension};
use crate::record::{AssetRecord, SourceMetadata, SourceMetadataError};

/// 실패한 import 단계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportOperation {
    /// 원본 열기·메타데이터 조회.
    OpenSource,
    /// hash pass 읽기.
    HashSource,
    /// shard 디렉터리 생성.
    CreateDirectory,
    /// copy pass(읽기·temp 쓰기).
    CopyContent,
    /// temp → 최종 이름 rename.
    Rename,
}

impl fmt::Display for ImportOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            ImportOperation::OpenSource => "open-source",
            ImportOperation::HashSource => "hash-source",
            ImportOperation::CreateDirectory => "create-directory",
            ImportOperation::CopyContent => "copy-content",
            ImportOperation::Rename => "rename",
        };
        f.write_str(name)
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
    /// hash pass와 copy pass의 내용이 다릅니다(import 도중 원본 변경).
    SourceChanged,
    /// 원본 위치를 metadata로 기록할 수 없습니다.
    SourceMetadata(SourceMetadataError),
    /// digest를 content hash로 만들 수 없습니다(발생하지 않아야 함).
    Hash(ParseContentHashError),
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
        }
    }
}

impl std::error::Error for ImportError {}

/// 외부 파일 import 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRequest {
    source: PathBuf,
    media_type: MediaType,
    extension: SafeExtension,
    record_source: bool,
}

impl ImportRequest {
    /// 원본 경로와 media type으로 요청을 만듭니다.
    ///
    /// 확장자는 원본 확장자를 정규화한 값(비안전하면 `bin`)이고, 원본 위치를 기록합니다.
    pub fn new(source: impl Into<PathBuf>, media_type: MediaType) -> ImportRequest {
        let source = source.into();
        let extension = SafeExtension::from_source_path(&source);
        ImportRequest {
            source,
            media_type,
            extension,
            record_source: true,
        }
    }

    /// 저장 확장자를 지정합니다.
    pub fn with_extension(mut self, extension: SafeExtension) -> ImportRequest {
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

/// import 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedAsset {
    /// 새 에셋 record(보존 field 없음).
    pub record: AssetRecord,
    /// 프로젝트 root 기준 content 파일 경로.
    pub object_path: AssetObjectPath,
}

/// 프로젝트의 `assets/` content 저장소.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetStore {
    root: PathBuf,
}

impl AssetStore {
    /// 프로젝트 root로 저장소를 엽니다(파일 I/O 없음).
    pub fn new(project_root: impl Into<PathBuf>) -> AssetStore {
        AssetStore {
            root: project_root.into(),
        }
    }

    /// 프로젝트 root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// content 파일의 실제 경로.
    pub fn content_path(&self, object: &AssetObjectPath) -> PathBuf {
        self.root.join(object.to_relative_path())
    }

    /// 외부 파일을 streaming hash 후 `assets/`로 복사합니다.
    pub fn import_file(&self, request: &ImportRequest) -> Result<ImportedAsset, ImportError> {
        // metadata를 먼저 만들어 기록할 수 없는 경로면 아무 것도 쓰기 전에 실패합니다.
        let source_metadata = if request.record_source {
            Some(
                SourceMetadata::from_import_path(&self.root, &request.source)
                    .map_err(ImportError::SourceMetadata)?,
            )
        } else {
            None
        };

        let mut buffer = vec![0u8; STREAM_BUFFER_SIZE];
        let digest = hash_source(&request.source, &mut buffer)?;
        let object_path = AssetObjectPath::new(&digest.hash, &request.extension);
        self.copy_content(&request.source, &digest, &object_path, &mut buffer)?;

        Ok(ImportedAsset {
            record: AssetRecord::new(
                digest.hash,
                request.media_type.clone(),
                digest.size,
                source_metadata,
            ),
            object_path,
        })
    }

    /// copy pass: temp에 복사·재hash 검증 후 최종 이름으로 rename합니다.
    fn copy_content(
        &self,
        source: &Path,
        expected: &StreamDigest,
        object: &AssetObjectPath,
        buffer: &mut [u8],
    ) -> Result<(), ImportError> {
        let dir = self.root.join(object.relative_dir());
        std::fs::create_dir_all(&dir)
            .map_err(|e| ImportError::io(ImportOperation::CreateDirectory, &e))?;
        let target = dir.join(object.file_name());
        let temp = temp_path_for(&target);
        let result = copy_verified(source, &temp, expected, buffer).and_then(|()| {
            std::fs::rename(&temp, &target)
                .map_err(|e| ImportError::io(ImportOperation::Rename, &e))
        });
        if result.is_err() {
            // 실패 경로에서는 temp를 치웁니다(실패해도 무시 — 최종 파일은 만들어지지 않음).
            let _ = std::fs::remove_file(&temp);
        }
        result
    }
}

/// 원본을 열고 일반 파일인지 확인합니다.
fn open_source(path: &Path) -> Result<File, ImportError> {
    let file = File::open(path).map_err(|e| ImportError::io(ImportOperation::OpenSource, &e))?;
    let metadata = file
        .metadata()
        .map_err(|e| ImportError::io(ImportOperation::OpenSource, &e))?;
    if !metadata.is_file() {
        return Err(ImportError::NotAFile);
    }
    Ok(file)
}

/// hash pass.
fn hash_source(path: &Path, buffer: &mut [u8]) -> Result<StreamDigest, ImportError> {
    let mut file = open_source(path)?;
    hash_reader_with(&mut file, buffer, |_| Ok(()))
        .map_err(|e| ImportError::stream(ImportOperation::HashSource, e))
}

/// copy pass: 원본을 temp로 복사하며 다시 hash하고 hash pass 결과와 비교합니다.
fn copy_verified(
    source: &Path,
    temp: &Path,
    expected: &StreamDigest,
    buffer: &mut [u8],
) -> Result<(), ImportError> {
    let mut input = open_source(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp)
        .map_err(|e| ImportError::io(ImportOperation::CopyContent, &e))?;
    let copied = hash_reader_with(&mut input, buffer, |chunk| output.write_all(chunk))
        .map_err(|e| ImportError::stream(ImportOperation::CopyContent, e))?;
    if copied != *expected {
        return Err(ImportError::SourceChanged);
    }
    output
        .flush()
        .map_err(|e| ImportError::io(ImportOperation::CopyContent, &e))?;
    Ok(())
}

/// 대상과 같은 디렉터리에 만들 temp 파일 경로(`.<file>.tmp-<pid>-<n>`).
///
/// `.`로 시작해 content 파일 이름 규칙(`<algorithm>-<hex>.<ext>`)과 겹치지 않습니다.
pub(crate) fn temp_path_for(target: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = target
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed".to_string());
    target.with_file_name(format!(".{file_name}.tmp-{}-{n}", std::process::id()))
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

    /// 패턴으로 채운 bytes.
    pub fn patterned(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u32).wrapping_mul(31).wrapping_add(seed as u32) as u8)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{files_under, patterned, TempDir};
    use super::*;
    use crate::record::SourceLocation;
    use stellar_storage::hash::ContentHash;

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
        let store = AssetStore::new(&project);
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
        assert!(source_meta.matches_external_origin(&source));
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
        let explicit = fx
            .store
            .import_file(
                &ImportRequest::new(&source, png())
                    .with_extension(SafeExtension::new("png").unwrap()),
            )
            .unwrap();
        assert!(explicit.object_path.file_name().ends_with(".png"));
        assert_eq!(default.record.content_hash, explicit.record.content_hash);
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
        let digest = hash_source(&source, &mut buffer).unwrap();
        // hash pass 후 원본이 바뀐 상황.
        std::fs::write(&source, b"second").unwrap();
        let object = AssetObjectPath::new(&digest.hash, &SafeExtension::new("png").unwrap());
        assert_eq!(
            fx.store
                .copy_content(&source, &digest, &object, &mut buffer)
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
    fn error_messages_are_descriptive() {
        assert!(ImportError::SourceChanged.to_string().contains("원본"));
        let err = ImportError::io(
            ImportOperation::Rename,
            &std::io::Error::from(ErrorKind::PermissionDenied),
        );
        assert!(err.to_string().contains("rename"));
    }
}

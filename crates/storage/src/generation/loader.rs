//! 프로젝트 열기 loader (V.4.4).
//!
//! `current pointer → manifest → chunks` 순으로 검증합니다:
//!
//! 1. `design/current`에서 manifest hash를 읽습니다(V.4.3).
//! 2. `design/generations/<algo>-<hex>.starmanifest`를 읽어 bytes 전체 hash가
//!    주소와 일치하는지 확인하고 디코딩·검증합니다(V.4.1/V.4.2).
//! 3. manifest의 project id가 기대값(`stellar.yaml`)과 일치하는지 확인합니다.
//! 4. 모든 chunk pointer의 대상 파일 존재를 확인합니다. chunk **내용** 검증은
//!    읽기 경로(V.2.6)가 수행합니다 — 지연 로딩(V.6.6)과 호환되도록 열기 시점에는
//!    전체 payload를 읽지 않습니다.
//!
//! manifest 파일 읽기·쓰기는 chunk 저장소와 같은 규칙입니다(보안 감사 STOR-1·4,
//! [`crate::chunk::store`] 모듈 문서): 읽기는 symlink를 따르되 정규 파일이고
//! [`MAX_MANIFEST_FILE_BYTES`] 이하일 때만 읽고, 쓰기는 `design`·`design/generations`가 symlink면
//! 거부하며, dedup 검증은 기존 항목을 따라가지 않습니다.

use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use stellar_core_types::ids::StableId;

use crate::chunk::path::ChunkObjectPath;
use crate::fs_guard::{ensure_real_dirs, read_regular_file, DirGuard, FileIdentity, Link, Refusal};
use crate::hash::ContentHash;

use super::current::{read_current_pointer, CurrentPointerError};
use super::manifest::{GenerationManifest, ManifestError};

/// manifest 파일 디렉터리(프로젝트 root 기준) 구성 요소.
const GENERATIONS_DIR: [&str; 2] = ["design", "generations"];

/// manifest 파일 확장자.
const MANIFEST_EXTENSION: &str = "starmanifest";

/// generation manifest 파일 하나의 크기 상한(bytes, 보안 감사 STOR-1).
///
/// 제품 예산이 아니라 비신뢰 입력 방어 상한입니다. chunk pointer 하나가 백 bytes 안팎이라 수십만 개
/// 이상의 chunk를 담고, project loader 기본 상한(`DEFAULT_MAX_MANIFEST_BYTES`, XIV.2.7)과 같은
/// 값입니다. 읽기는 이 값을 넘는 파일을 읽지 않고, 쓰기는 이 값을 넘는 manifest를 기록하지 않습니다.
const MAX_MANIFEST_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// manifest content hash의 파일 절대 경로.
///
/// `design/generations/<algorithm>-<hex64>.starmanifest`. 경로 입력은
/// [`ContentHash`] 타입뿐이라 path traversal이 불가능합니다(V.2.3과 같은 원리).
pub fn manifest_path(root: &Path, hash: &ContentHash) -> PathBuf {
    let mut path = root.to_path_buf();
    for dir in GENERATIONS_DIR {
        path.push(dir);
    }
    path.push(format!(
        "{}-{}.{}",
        hash.algorithm().name(),
        hash.digest_hex(),
        MANIFEST_EXTENSION
    ));
    path
}

/// loader 오류.
#[derive(Debug)]
pub enum LoadError {
    /// current pointer 읽기 실패.
    CurrentPointer(CurrentPointerError),
    /// manifest 파일이 없습니다.
    MissingManifest {
        /// 없는 경로.
        path: PathBuf,
    },
    /// manifest bytes의 hash가 주소와 다릅니다(corruption).
    ManifestHashMismatch {
        /// 기대 hash(주소).
        expected: ContentHash,
        /// 실제 bytes hash.
        found: ContentHash,
    },
    /// manifest 디코딩·검증 실패.
    Manifest(ManifestError),
    /// manifest의 project id가 기대값과 다릅니다.
    WrongProjectId {
        /// 기대 project id (`stellar.yaml`).
        expected: StableId,
        /// manifest에 기록된 id.
        found: StableId,
    },
    /// manifest가 가리키는 chunk 파일이 없습니다.
    MissingChunk {
        /// 없는 chunk의 상대 경로.
        path: ChunkObjectPath,
    },
    /// 파일시스템 I/O 실패.
    Io {
        /// 실패한 경로.
        path: PathBuf,
        /// 원인.
        source: std::io::Error,
    },
    /// 같은 경로의 기존 manifest 파일이 기대 bytes와 다릅니다(불변성 위반).
    CorruptExistingManifest {
        /// 대상 경로.
        path: PathBuf,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::CurrentPointer(e) => write!(f, "current pointer 오류: {e}"),
            LoadError::MissingManifest { path } => {
                write!(f, "manifest 파일이 없습니다: {}", path.display())
            }
            LoadError::ManifestHashMismatch { expected, found } => {
                write!(f, "manifest hash 불일치: 기대 {expected}, 실제 {found}")
            }
            LoadError::Manifest(e) => write!(f, "manifest 오류: {e}"),
            LoadError::WrongProjectId { expected, found } => {
                write!(f, "project id 불일치: 기대 {expected}, manifest {found}")
            }
            LoadError::MissingChunk { path } => {
                write!(f, "chunk 파일이 없습니다: {path}")
            }
            LoadError::Io { path, source } => {
                write!(f, "I/O 실패 ({}): {source}", path.display())
            }
            LoadError::CorruptExistingManifest { path } => {
                write!(
                    f,
                    "기존 manifest 파일이 기대 내용과 다릅니다(불변성 위반): {}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for LoadError {}

impl From<CurrentPointerError> for LoadError {
    fn from(e: CurrentPointerError) -> LoadError {
        LoadError::CurrentPointer(e)
    }
}

impl From<ManifestError> for LoadError {
    fn from(e: ManifestError) -> LoadError {
        LoadError::Manifest(e)
    }
}

/// manifest를 content-addressed 파일로 기록합니다(immutable, V.2.4와 같은 규칙).
///
/// 같은 hash 파일이 있으면 내용 검증 후 재사용하고, 다르면 불변성 위반 오류를
/// 반환합니다(그 자리가 symlink이거나 정규 파일이 아니어도 같은 오류, 따라가지 않음). 기록은
/// atomic write(V.2.5)를 사용합니다. 저장소 디렉터리가 symlink이거나 manifest가
/// [`MAX_MANIFEST_FILE_BYTES`]를 넘으면 [`LoadError::Io`]입니다(기록하지 않음).
pub fn write_manifest_file(
    root: &Path,
    manifest: &GenerationManifest,
) -> Result<ContentHash, LoadError> {
    write_manifest_file_observed(root, manifest, &mut crate::atomic_file::NoopObserver)
}

/// [`write_manifest_file`]에 atomic write 단계 관찰자를 주입한 버전
/// (fault injection 테스트용, V.5.6).
pub fn write_manifest_file_observed(
    root: &Path,
    manifest: &GenerationManifest,
    observer: &mut dyn crate::atomic_file::WriteObserver,
) -> Result<ContentHash, LoadError> {
    write_manifest_file_limited(root, manifest, observer, MAX_MANIFEST_FILE_BYTES)
}

/// [`write_manifest_file_observed`] 본체. `limit`는 manifest 파일 크기 상한입니다(테스트가 작은
/// 상한을 주는 지점이며, 실제 경로는 [`MAX_MANIFEST_FILE_BYTES`]).
fn write_manifest_file_limited(
    root: &Path,
    manifest: &GenerationManifest,
    observer: &mut dyn crate::atomic_file::WriteObserver,
    limit: u64,
) -> Result<ContentHash, LoadError> {
    let (bytes, hash) = manifest.encode_with_hash()?;
    let path = manifest_path(root, &hash);
    let io_error = |source| LoadError::Io {
        path: path.clone(),
        source,
    };
    let expected_len = bytes.len() as u64;
    if expected_len > limit {
        return Err(io_error(Refusal::TooLarge { limit }.into_io()));
    }
    // `design/generations`를 symlink를 따르지 않고 확인·생성합니다.
    let dirs = ensure_real_dirs(root, &GENERATIONS_DIR)
        .map_err(|(path, source)| LoadError::Io { path, source })?;
    match FileIdentity::lstat(&path) {
        Ok(_) => {
            return match read_regular_file(&path, expected_len, Link::NoFollow) {
                Ok(existing) if existing == bytes => Ok(hash),
                Ok(_) => Err(LoadError::CorruptExistingManifest { path }),
                // symlink·정규 파일 아님·크기 다름: 불변 manifest 파일이 아닙니다(덮어쓰지 않음).
                Err(e)
                    if matches!(
                        Refusal::of(&e),
                        Some(
                            Refusal::SymlinkFile
                                | Refusal::NotRegularFile
                                | Refusal::TooLarge { .. }
                        )
                    ) =>
                {
                    Err(LoadError::CorruptExistingManifest { path })
                }
                Err(source) => Err(io_error(source)),
            };
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(source) => return Err(io_error(source)),
    }
    let mut guard = DirGuard::new(observer, dirs);
    crate::atomic_file::write_file_atomic_observed(&path, &bytes, &mut guard).map_err(io_error)?;
    Ok(hash)
}

/// hash 주소의 manifest 파일을 읽고 검증(주소 일치 + 디코딩)합니다.
///
/// 파일은 symlink를 따라 읽되(19.3) 정규 파일이어야 하고 [`MAX_MANIFEST_FILE_BYTES`] 이하여야
/// 합니다. 아니면 읽지 않고 [`LoadError::Io`]입니다.
pub fn read_manifest_file(
    root: &Path,
    hash: &ContentHash,
) -> Result<GenerationManifest, LoadError> {
    read_manifest_file_limited(root, hash, MAX_MANIFEST_FILE_BYTES)
}

/// [`read_manifest_file`] 본체. `limit`는 파일 크기 상한입니다(테스트가 작은 상한을 주는 지점).
fn read_manifest_file_limited(
    root: &Path,
    hash: &ContentHash,
    limit: u64,
) -> Result<GenerationManifest, LoadError> {
    let path = manifest_path(root, hash);
    let bytes = match read_regular_file(&path, limit, Link::Follow) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(LoadError::MissingManifest { path })
        }
        Err(source) => return Err(LoadError::Io { path, source }),
    };
    let actual = ContentHash::compute(&bytes);
    if actual != *hash {
        return Err(LoadError::ManifestHashMismatch {
            expected: *hash,
            found: actual,
        });
    }
    Ok(GenerationManifest::decode(&bytes)?)
}

/// 열기 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedGeneration {
    /// current manifest의 content hash.
    pub manifest_hash: ContentHash,
    /// 검증된 manifest.
    pub manifest: GenerationManifest,
}

/// hash 주소의 세대 하나를 검증합니다: manifest 읽기·검증 → project id →
/// chunk 존재 확인. loader(V.4.4)와 fallback scanner(V.4.5)가 공유합니다.
pub fn verify_generation(
    root: &Path,
    manifest_hash: &ContentHash,
    expected_project_id: StableId,
) -> Result<LoadedGeneration, LoadError> {
    let manifest = read_manifest_file(root, manifest_hash)?;
    if manifest.project_id != expected_project_id {
        return Err(LoadError::WrongProjectId {
            expected: expected_project_id,
            found: manifest.project_id,
        });
    }
    for pointer in &manifest.chunk_pointers {
        let chunk_path = ChunkObjectPath::for_hash(&pointer.content_hash);
        if !root.join(chunk_path.to_relative_path()).is_file() {
            return Err(LoadError::MissingChunk { path: chunk_path });
        }
    }
    Ok(LoadedGeneration {
        manifest_hash: *manifest_hash,
        manifest,
    })
}

/// current generation을 열고 검증합니다 (V.4.4).
pub fn load_current_generation(
    root: &Path,
    expected_project_id: StableId,
) -> Result<LoadedGeneration, LoadError> {
    // 1. current pointer → 2. manifest → 3. project id → 4. chunk 존재.
    let manifest_hash = read_current_pointer(root)?;
    verify_generation(root, &manifest_hash, expected_project_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::envelope::ChunkKindCode;
    use crate::chunk::id::ChunkId;
    use crate::chunk::pointer::ChunkPointer;
    use crate::chunk::store::{ChunkStore, WriteOutcome};
    use crate::generation::current::write_current_pointer;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("stellar-loader-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    const KIND: ChunkKindCode = ChunkKindCode(0x8000_0001);

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    /// chunk 2개 + manifest + current pointer가 완비된 프로젝트 fixture.
    fn valid_project(dir: &TempDir, project_id: StableId) -> GenerationManifest {
        let store = ChunkStore::new(&dir.path);
        let mut pointers = Vec::new();
        for (i, payload) in [b"chunk-a".as_slice(), b"chunk-b".as_slice()]
            .iter()
            .enumerate()
        {
            let WriteOutcome::Written { hash, .. } = store.write_chunk(KIND, payload).unwrap()
            else {
                panic!("Written이어야 함");
            };
            pointers.push(ChunkPointer {
                chunk_id: ChunkId::from_stable_id(stable(1_000 + i as u64)),
                content_hash: hash,
                kind: KIND,
                schema_version: 1,
            });
        }
        let manifest = GenerationManifest {
            generation: 1,
            parent_generation: None,
            project_id,
            schema_version: 1,
            chunk_pointers: pointers,
            root_document_ids: vec![stable(7_000)],
        };
        let hash = write_manifest_file(&dir.path, &manifest).unwrap();
        write_current_pointer(&dir.path, &hash).unwrap();
        manifest
    }

    #[test]
    fn valid_project_loads() {
        let dir = TempDir::new();
        let project_id = stable(42);
        let manifest = valid_project(&dir, project_id);
        let loaded = load_current_generation(&dir.path, project_id).unwrap();
        let mut expected = manifest;
        expected.chunk_pointers.sort();
        assert_eq!(loaded.manifest, expected);
    }

    #[test]
    fn missing_manifest_fails() {
        // 검증 기준(V.4.4): 누락 manifest fixture 실패.
        let dir = TempDir::new();
        let project_id = stable(42);
        let manifest = valid_project(&dir, project_id);
        let (_, hash) = manifest.encode_with_hash().unwrap();
        std::fs::remove_file(manifest_path(&dir.path, &hash)).unwrap();
        let err = load_current_generation(&dir.path, project_id).unwrap_err();
        assert!(matches!(err, LoadError::MissingManifest { .. }), "{err:?}");
    }

    #[test]
    fn missing_chunk_fails() {
        // 검증 기준(V.4.4): 누락 chunk fixture 실패.
        let dir = TempDir::new();
        let project_id = stable(42);
        let manifest = valid_project(&dir, project_id);
        let victim = ChunkObjectPath::for_hash(&manifest.chunk_pointers[0].content_hash);
        std::fs::remove_file(dir.path.join(victim.to_relative_path())).unwrap();
        let err = load_current_generation(&dir.path, project_id).unwrap_err();
        assert!(matches!(err, LoadError::MissingChunk { .. }), "{err:?}");
    }

    #[test]
    fn wrong_project_id_fails() {
        // 검증 기준(V.4.4): 잘못된 project id fixture 실패.
        let dir = TempDir::new();
        valid_project(&dir, stable(42));
        let err = load_current_generation(&dir.path, stable(43)).unwrap_err();
        assert!(matches!(err, LoadError::WrongProjectId { .. }), "{err:?}");
    }

    #[test]
    fn corrupted_manifest_bytes_fail() {
        let dir = TempDir::new();
        let project_id = stable(42);
        let manifest = valid_project(&dir, project_id);
        let (_, hash) = manifest.encode_with_hash().unwrap();
        let path = manifest_path(&dir.path, &hash);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[20] ^= 0x01;
        std::fs::write(&path, &bytes).unwrap();
        let err = load_current_generation(&dir.path, project_id).unwrap_err();
        assert!(
            matches!(err, LoadError::ManifestHashMismatch { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn missing_current_pointer_fails() {
        let dir = TempDir::new();
        let err = load_current_generation(&dir.path, stable(42)).unwrap_err();
        assert!(matches!(err, LoadError::CurrentPointer(_)), "{err:?}");
    }

    #[test]
    fn manifest_file_is_immutable() {
        let dir = TempDir::new();
        let project_id = stable(42);
        let manifest = valid_project(&dir, project_id);
        // 같은 manifest 재기록 → 재사용.
        let hash = write_manifest_file(&dir.path, &manifest).unwrap();
        // 파일을 손상시킨 뒤 재기록 시도 → 불변성 위반.
        let path = manifest_path(&dir.path, &hash);
        std::fs::write(&path, b"tampered").unwrap();
        let err = write_manifest_file(&dir.path, &manifest).unwrap_err();
        assert!(
            matches!(err, LoadError::CorruptExistingManifest { .. }),
            "{err:?}"
        );
    }

    fn io_refusal(err: LoadError) -> (PathBuf, Refusal) {
        match err {
            LoadError::Io { path, source } => {
                let refusal = Refusal::of(&source)
                    .unwrap_or_else(|| panic!("guard 거부가 아닌 I/O 오류: {source:?}"));
                (path, refusal)
            }
            other => panic!("Io 오류가 아님: {other:?}"),
        }
    }

    /// 보안 감사 STOR-1: manifest 자리의 장치·FIFO는 열기·읽기에서 멈추거나 메모리를 끝없이 쓰지
    /// 않고 거부되고, 열기(V.4.4)는 그 오류로 끝납니다.
    #[cfg(unix)]
    #[test]
    fn manifest_devices_and_fifos_are_refused_without_blocking() {
        use crate::fs_guard::test_support::{bounded, mkfifo};

        let dir = TempDir::new();
        let project_id = stable(42);
        let manifest = valid_project(&dir, project_id);
        let (_, hash) = manifest.encode_with_hash().unwrap();
        let path = manifest_path(&dir.path, &hash);
        let fifo = dir.path.join("fifo");
        mkfifo(&fifo);
        for target in [Path::new("/dev/zero"), fifo.as_path()] {
            std::fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(target, &path).unwrap();
            let root = dir.path.clone();
            let err = bounded("manifest 읽기", move || {
                load_current_generation(&root, project_id).unwrap_err()
            });
            assert_eq!(io_refusal(err), (path.clone(), Refusal::NotRegularFile));
        }
    }

    /// 보안 감사 STOR-1: 상한을 넘는 manifest 파일(거대한 sparse 파일 포함)은 읽지 않고 거부합니다.
    #[test]
    fn oversized_manifest_is_refused_without_reading() {
        let dir = TempDir::new();
        let project_id = stable(42);
        let manifest = valid_project(&dir, project_id);
        let (bytes, hash) = manifest.encode_with_hash().unwrap();
        let path = manifest_path(&dir.path, &hash);
        let len = bytes.len() as u64;
        assert!(read_manifest_file_limited(&dir.path, &hash, len).is_ok());
        assert_eq!(
            io_refusal(read_manifest_file_limited(&dir.path, &hash, len - 1).unwrap_err()),
            (path.clone(), Refusal::TooLarge { limit: len - 1 })
        );
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_MANIFEST_FILE_BYTES + 1)
            .unwrap();
        assert_eq!(
            io_refusal(read_manifest_file(&dir.path, &hash).unwrap_err()),
            (
                path,
                Refusal::TooLarge {
                    limit: MAX_MANIFEST_FILE_BYTES
                }
            )
        );
    }

    /// 보안 감사 STOR-4: `design`·`design/generations`가 symlink면 따라가지 않고 manifest 기록을
    /// 거부합니다(프로젝트 밖에 쓰지 않음).
    #[cfg(unix)]
    #[test]
    fn manifest_write_refuses_symlinked_directories() {
        let manifest = {
            let scratch = TempDir::new();
            valid_project(&scratch, stable(42))
        };
        for link in [&["design"][..], &["design", "generations"][..]] {
            let dir = TempDir::new();
            let outside = TempDir::new();
            let mut link_path = dir.path.clone();
            for component in link {
                link_path.push(component);
            }
            std::fs::create_dir_all(link_path.parent().unwrap()).unwrap();
            std::os::unix::fs::symlink(&outside.path, &link_path).unwrap();
            let err = write_manifest_file(&dir.path, &manifest).unwrap_err();
            assert_eq!(
                io_refusal(err),
                (link_path.clone(), Refusal::SymlinkDirectory),
                "{link:?}"
            );
            assert_eq!(std::fs::read_dir(&outside.path).unwrap().count(), 0);
        }
    }

    /// dedup 검증은 기존 manifest 자리의 symlink를 따르지 않습니다(같은 bytes를 가리켜도 불변성 위반).
    #[cfg(unix)]
    #[test]
    fn manifest_write_does_not_follow_existing_symlink() {
        let dir = TempDir::new();
        let outside = TempDir::new();
        let manifest = valid_project(&dir, stable(42));
        let (_, hash) = manifest.encode_with_hash().unwrap();
        let path = manifest_path(&dir.path, &hash);
        let moved = outside.path.join("manifest");
        std::fs::rename(&path, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &path).unwrap();
        // 읽기는 따릅니다(19.3).
        assert!(read_manifest_file(&dir.path, &hash).is_ok());
        let err = write_manifest_file(&dir.path, &manifest).unwrap_err();
        assert!(
            matches!(err, LoadError::CorruptExistingManifest { .. }),
            "{err:?}"
        );
        assert!(std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
    }

    /// 읽기 상한을 넘는 manifest는 다시 읽을 수 없으므로 기록하지 않습니다.
    #[test]
    fn manifest_over_the_read_limit_is_not_written() {
        let dir = TempDir::new();
        let manifest = {
            let scratch = TempDir::new();
            valid_project(&scratch, stable(42))
        };
        let (bytes, hash) = manifest.encode_with_hash().unwrap();
        let limit = bytes.len() as u64 - 1;
        let mut noop = crate::atomic_file::NoopObserver;
        let err = write_manifest_file_limited(&dir.path, &manifest, &mut noop, limit).unwrap_err();
        assert_eq!(
            io_refusal(err),
            (manifest_path(&dir.path, &hash), Refusal::TooLarge { limit })
        );
        assert!(!manifest_path(&dir.path, &hash).exists());
        assert_eq!(
            write_manifest_file_limited(&dir.path, &manifest, &mut noop, limit + 1).unwrap(),
            hash
        );
    }
}

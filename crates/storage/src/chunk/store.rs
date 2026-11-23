//! immutable chunk write (V.2.4).
//!
//! chunk object 파일은 content-addressed **불변** 파일입니다:
//! - 같은 hash 파일이 이미 있으면 **내용을 검증**한 뒤 재사용합니다(dedup).
//! - 내용이 기대 bytes와 다르면 덮어쓰지 않고 corruption 오류를 반환합니다.
//! - 손상된 bytes를 자동 수정하지 않습니다.
//!
//! 파일 기록은 temp write → flush → fsync → atomic rename(V.2.5,
//! [`crate::atomic_file`])을 사용합니다. 중단 시 부분 파일이 대상 경로에
//! 나타나지 않습니다.
//!
//! # 비신뢰 프로젝트 폴더 (보안 감사 STOR-1·4)
//!
//! - 읽기는 symlink를 따르되(기술 아키텍처 19.3) 연 handle이 정규 파일이고 크기가
//!   [`MAX_CHUNK_FILE_BYTES`] 이하일 때만 읽습니다(FIFO·장치·거대 파일 거부, crate 내부 `fs_guard`).
//! - 쓰기는 `design`·`design/objects`·shard 디렉터리가 symlink면 따라가지 않고 거부합니다. 없으면
//!   한 단계씩 만들고, temp 생성·rename 직전에 같은 디렉터리인지 다시 확인합니다.
//! - dedup 검증은 기존 항목을 따라가지 않고(`O_NOFOLLOW`) 기대 크기까지만 읽습니다. symlink·정규
//!   파일이 아닌 항목·크기가 다른 파일은 [`ChunkStoreError::CorruptExisting`]입니다(덮어쓰지 않음).
//!
//! 새 거부는 공개 오류 enum을 바꾸지 않도록 [`ChunkStoreError::Io`]로 보고합니다.

use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::fs_guard::{ensure_real_dirs, read_regular_file, DirGuard, FileIdentity, Link, Refusal};
use crate::hash::ContentHash;

use super::envelope::{ChunkEnvelope, ChunkKindCode, EnvelopeError};
use super::path::ChunkObjectPath;

/// chunk store 오류.
#[derive(Debug)]
pub enum ChunkStoreError {
    /// 같은 경로의 기존 파일이 기대 bytes와 다릅니다(불변성 위반 — 덮어쓰지 않음).
    CorruptExisting {
        /// 대상 상대 경로.
        path: ChunkObjectPath,
    },
    /// envelope 인코딩 실패.
    Envelope(EnvelopeError),
    /// 파일시스템 I/O 실패.
    Io {
        /// 실패한 경로.
        path: PathBuf,
        /// 원인.
        source: std::io::Error,
    },
    /// 요청한 hash의 chunk 파일이 없습니다.
    MissingChunk {
        /// 없는 상대 경로.
        path: ChunkObjectPath,
    },
    /// envelope kind가 기대(kind pointer)와 다릅니다.
    KindMismatch {
        /// 기대 kind.
        expected: ChunkKindCode,
        /// 파일에 기록된 kind.
        found: ChunkKindCode,
    },
    /// payload 길이가 envelope의 uncompressed length와 다릅니다.
    LengthMismatch {
        /// envelope에 기록된 길이.
        expected: u64,
        /// 실제 payload 길이.
        found: u64,
    },
    /// envelope의 payload hash가 요청 주소(hash)와 다릅니다.
    AddressMismatch {
        /// 요청한 hash.
        requested: ContentHash,
        /// envelope에 기록된 hash.
        found: ContentHash,
    },
    /// payload를 다시 hash한 값이 envelope의 hash와 다릅니다(bit corruption).
    PayloadHashMismatch {
        /// 손상이 탐지된 상대 경로.
        path: ChunkObjectPath,
    },
}

impl fmt::Display for ChunkStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChunkStoreError::CorruptExisting { path } => {
                write!(
                    f,
                    "기존 chunk 파일이 기대 내용과 다릅니다(불변성 위반): {path}"
                )
            }
            ChunkStoreError::Envelope(e) => write!(f, "envelope 오류: {e}"),
            ChunkStoreError::Io { path, source } => {
                write!(f, "I/O 실패 ({}): {source}", path.display())
            }
            ChunkStoreError::MissingChunk { path } => {
                write!(f, "chunk 파일이 없습니다: {path}")
            }
            ChunkStoreError::KindMismatch { expected, found } => {
                write!(
                    f,
                    "chunk kind가 다릅니다: 기대 {}, 실제 {}",
                    expected.0, found.0
                )
            }
            ChunkStoreError::LengthMismatch { expected, found } => {
                write!(f, "payload 길이가 다릅니다: 기대 {expected}, 실제 {found}")
            }
            ChunkStoreError::AddressMismatch { requested, found } => {
                write!(f, "chunk 주소 불일치: 요청 {requested}, 파일 {found}")
            }
            ChunkStoreError::PayloadHashMismatch { path } => {
                write!(f, "payload hash 불일치(corruption 탐지): {path}")
            }
        }
    }
}

impl std::error::Error for ChunkStoreError {}

impl From<EnvelopeError> for ChunkStoreError {
    fn from(e: EnvelopeError) -> ChunkStoreError {
        ChunkStoreError::Envelope(e)
    }
}

/// chunk 파일 하나(envelope header + payload)의 크기 상한(bytes, 보안 감사 STOR-1).
///
/// 제품 예산이 아니라 비신뢰 입력 방어 상한입니다. 분할 임계치(ADR-0012, 256 KiB)의 1,000배 이상이라
/// 의미 경계로 더 나눌 수 없는 큰 chunk(oversized)도 담고, project loader 기본 상한
/// (`DEFAULT_MAX_CHUNK_BYTES`, XIV.2.7)과 같은 값입니다. 읽기는 이 값을 넘는 파일을 읽지 않고, 쓰기는
/// 다시 읽을 수 없는 파일을 만들지 않도록 이 값을 넘는 chunk를 기록하지 않습니다.
const MAX_CHUNK_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// write 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// 새 파일을 기록했습니다.
    Written {
        /// payload content hash.
        hash: ContentHash,
        /// 기록된 상대 경로.
        path: ChunkObjectPath,
    },
    /// 같은 내용의 파일이 이미 있어 검증 후 재사용했습니다(dedup).
    Reused {
        /// payload content hash.
        hash: ContentHash,
        /// 재사용한 상대 경로.
        path: ChunkObjectPath,
    },
}

/// content-addressed chunk 저장소.
///
/// `root`는 프로젝트 root입니다. 모든 chunk는 `root/design/objects/...` 아래에
/// 기록됩니다(V.2.3 경로 규칙).
#[derive(Debug)]
pub struct ChunkStore {
    root: PathBuf,
}

impl ChunkStore {
    /// 프로젝트 root 위에 store를 만듭니다.
    pub fn new(root: impl Into<PathBuf>) -> ChunkStore {
        ChunkStore { root: root.into() }
    }

    /// 프로젝트 root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// chunk object의 절대 경로.
    fn absolute_path(&self, path: &ChunkObjectPath) -> PathBuf {
        self.root.join(path.to_relative_path())
    }

    /// chunk 파일 bytes(envelope header + payload)를 인코딩합니다.
    fn encode_file_bytes(
        kind: ChunkKindCode,
        payload: &[u8],
    ) -> Result<(ChunkEnvelope, Vec<u8>), ChunkStoreError> {
        let envelope = ChunkEnvelope::for_payload(kind, payload);
        let mut bytes = envelope.encode()?;
        bytes.extend_from_slice(payload);
        Ok((envelope, bytes))
    }

    /// payload를 immutable chunk로 기록합니다.
    ///
    /// - 대상 파일이 없으면 기록하고 [`WriteOutcome::Written`].
    /// - 대상 파일이 있고 내용이 기대 bytes와 같으면 [`WriteOutcome::Reused`].
    /// - 내용이 다르면 [`ChunkStoreError::CorruptExisting`] — 덮어쓰지 않습니다. 대상 자리가
    ///   symlink이거나 정규 파일이 아니어도 같은 오류입니다(따라가지 않음).
    /// - 저장소 디렉터리가 symlink이거나 파일이 [`MAX_CHUNK_FILE_BYTES`]를 넘으면
    ///   [`ChunkStoreError::Io`]입니다(기록하지 않음).
    pub fn write_chunk(
        &self,
        kind: ChunkKindCode,
        payload: &[u8],
    ) -> Result<WriteOutcome, ChunkStoreError> {
        self.write_chunk_observed(kind, payload, &mut crate::atomic_file::NoopObserver)
    }

    /// [`ChunkStore::write_chunk`]에 atomic write 단계 관찰자를 주입한 버전
    /// (fault injection 테스트용, V.5.6).
    pub fn write_chunk_observed(
        &self,
        kind: ChunkKindCode,
        payload: &[u8],
        observer: &mut dyn crate::atomic_file::WriteObserver,
    ) -> Result<WriteOutcome, ChunkStoreError> {
        self.write_chunk_limited(kind, payload, observer, MAX_CHUNK_FILE_BYTES)
    }

    /// [`ChunkStore::write_chunk_observed`] 본체. `limit`는 chunk 파일 크기 상한입니다(테스트가 작은
    /// 상한을 주는 지점이며, 실제 경로는 [`MAX_CHUNK_FILE_BYTES`]).
    fn write_chunk_limited(
        &self,
        kind: ChunkKindCode,
        payload: &[u8],
        observer: &mut dyn crate::atomic_file::WriteObserver,
        limit: u64,
    ) -> Result<WriteOutcome, ChunkStoreError> {
        let (envelope, expected_bytes) = ChunkStore::encode_file_bytes(kind, payload)?;
        let hash = envelope.payload_hash;
        let path = ChunkObjectPath::for_hash(&hash);
        let target = self.absolute_path(&path);
        let io_error = |source| ChunkStoreError::Io {
            path: target.clone(),
            source,
        };
        let expected_len = expected_bytes.len() as u64;
        if expected_len > limit {
            return Err(io_error(Refusal::TooLarge { limit }.into_io()));
        }

        // `.stellar/cache/chunks/objects/<prefix>`를 symlink를 따르지 않고 확인·생성합니다
        // (`create_dir_all`은 symlink 디렉터리를 통과하므로 쓰지 않음).
        let mut components = crate::chunk::path::OBJECTS_DIR.to_vec();
        components.push(path.prefix());
        let dirs = ensure_real_dirs(&self.root, &components)
            .map_err(|(path, source)| ChunkStoreError::Io { path, source })?;
        crate::location::ensure_cache_ignore(&self.root).map_err(io_error)?;

        match FileIdentity::lstat(&target) {
            Ok(_) => {
                return match read_regular_file(&target, expected_len, Link::NoFollow) {
                    Ok(existing) if existing == expected_bytes => {
                        Ok(WriteOutcome::Reused { hash, path })
                    }
                    Ok(_) => Err(ChunkStoreError::CorruptExisting { path }),
                    // symlink·정규 파일 아님·크기 다름: 불변 chunk 파일이 아닙니다(덮어쓰지 않음).
                    Err(e) if is_not_immutable_file(&e) => {
                        Err(ChunkStoreError::CorruptExisting { path })
                    }
                    Err(source) => Err(io_error(source)),
                };
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(io_error(source)),
        }

        let mut guard = DirGuard::new(observer, dirs);
        crate::atomic_file::write_file_atomic_observed(&target, &expected_bytes, &mut guard)
            .map_err(io_error)?;
        Ok(WriteOutcome::Written { hash, path })
    }

    /// hash 주소의 chunk를 읽고 **전부** 검증합니다(V.2.6).
    ///
    /// 검증: envelope(magic·version·hash 형식) → kind가 `expected_kind`와 일치 →
    /// payload 길이가 envelope 기록과 일치 → envelope hash가 요청 주소와 일치 →
    /// payload 재해시가 envelope hash와 일치. 어떤 실패도 bytes를 자동 수정하지
    /// 않습니다. `expected_kind`는 chunk pointer(V.3.1)가 아는 kind입니다 —
    /// header kind 필드의 bit corruption도 이 비교로 탐지됩니다.
    ///
    /// 파일은 symlink를 따라 읽되(19.3) 정규 파일이어야 하고 [`MAX_CHUNK_FILE_BYTES`] 이하여야
    /// 합니다. 아니면 읽지 않고 [`ChunkStoreError::Io`]입니다.
    pub fn read_chunk(
        &self,
        hash: &ContentHash,
        expected_kind: ChunkKindCode,
    ) -> Result<Vec<u8>, ChunkStoreError> {
        self.read_chunk_limited(hash, expected_kind, MAX_CHUNK_FILE_BYTES)
    }

    /// [`ChunkStore::read_chunk`] 본체. `limit`는 파일 크기 상한입니다(테스트가 작은 상한을 주는
    /// 지점이며, 실제 경로는 [`MAX_CHUNK_FILE_BYTES`]).
    fn read_chunk_limited(
        &self,
        hash: &ContentHash,
        expected_kind: ChunkKindCode,
        limit: u64,
    ) -> Result<Vec<u8>, ChunkStoreError> {
        let path = ChunkObjectPath::for_hash(hash);
        let target = self.absolute_path(&path);
        let bytes = match read_regular_file(&target, limit, Link::Follow) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return Err(ChunkStoreError::MissingChunk { path })
            }
            Err(source) => {
                return Err(ChunkStoreError::Io {
                    path: target,
                    source,
                })
            }
        };

        let mut dec = crate::canonical::decode::CanonicalDecoder::new(&bytes);
        let envelope = ChunkEnvelope::decode(&mut dec)?;
        if envelope.kind != expected_kind {
            return Err(ChunkStoreError::KindMismatch {
                expected: expected_kind,
                found: envelope.kind,
            });
        }
        let payload = dec
            .take_raw(dec.remaining())
            .expect("남은 전부는 항상 성공");
        if payload.len() as u64 != envelope.uncompressed_len {
            return Err(ChunkStoreError::LengthMismatch {
                expected: envelope.uncompressed_len,
                found: payload.len() as u64,
            });
        }
        if envelope.payload_hash != *hash {
            return Err(ChunkStoreError::AddressMismatch {
                requested: *hash,
                found: envelope.payload_hash,
            });
        }
        if ContentHash::compute(payload) != envelope.payload_hash {
            return Err(ChunkStoreError::PayloadHashMismatch { path });
        }
        Ok(payload.to_vec())
    }
}

/// dedup 검증에서 기존 항목이 불변 chunk 파일이 아니라는 거부인지(symlink·정규 파일 아님·크기 다름).
fn is_not_immutable_file(error: &std::io::Error) -> bool {
    matches!(
        Refusal::of(error),
        Some(Refusal::SymlinkFile | Refusal::NotRegularFile | Refusal::TooLarge { .. })
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-chunkstore-test-{}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    const KIND: ChunkKindCode = ChunkKindCode(1);

    #[test]
    fn first_write_creates_file() {
        let dir = TempDir::new();
        let store = ChunkStore::new(&dir.path);
        let outcome = store.write_chunk(KIND, b"payload-a").unwrap();
        let WriteOutcome::Written { hash, path } = outcome else {
            panic!("Written이어야 함: {outcome:?}");
        };
        assert_eq!(hash, ContentHash::compute(b"payload-a"));
        let file = dir.path.join(path.to_relative_path());
        assert!(file.is_file());
        // 파일은 envelope header + payload.
        let bytes = std::fs::read(&file).unwrap();
        assert!(bytes.ends_with(b"payload-a"));
    }

    #[test]
    fn same_payload_is_deduplicated() {
        let dir = TempDir::new();
        let store = ChunkStore::new(&dir.path);
        let first = store.write_chunk(KIND, b"dedup me").unwrap();
        let second = store.write_chunk(KIND, b"dedup me").unwrap();
        assert!(matches!(first, WriteOutcome::Written { .. }));
        let WriteOutcome::Reused { hash, path } = second else {
            panic!("Reused여야 함: {second:?}");
        };
        assert_eq!(hash, ContentHash::compute(b"dedup me"));
        // 파일은 하나만 존재.
        let file = dir.path.join(path.to_relative_path());
        assert!(file.is_file());
    }

    #[test]
    fn corrupted_existing_file_is_not_overwritten() {
        // collision/corruption simulation: 같은 hash 경로에 다른 내용이 이미 있음.
        let dir = TempDir::new();
        let store = ChunkStore::new(&dir.path);
        let outcome = store.write_chunk(KIND, b"original").unwrap();
        let WriteOutcome::Written { path, .. } = outcome else {
            panic!("Written이어야 함");
        };
        let file = dir.path.join(path.to_relative_path());
        std::fs::write(&file, b"tampered bytes").unwrap();

        let err = store.write_chunk(KIND, b"original").unwrap_err();
        assert!(
            matches!(err, ChunkStoreError::CorruptExisting { .. }),
            "CorruptExisting이어야 함: {err:?}"
        );
        // 손상 파일을 자동 수정하지 않음.
        assert_eq!(std::fs::read(&file).unwrap(), b"tampered bytes");
    }

    #[test]
    fn different_kind_same_payload_is_collision() {
        // payload hash가 같으므로 경로가 같지만 header(kind)가 달라 bytes 불일치 →
        // 불변성 위반으로 거부되어야 합니다.
        let dir = TempDir::new();
        let store = ChunkStore::new(&dir.path);
        store
            .write_chunk(ChunkKindCode(1), b"same payload")
            .unwrap();
        let err = store
            .write_chunk(ChunkKindCode(2), b"same payload")
            .unwrap_err();
        assert!(matches!(err, ChunkStoreError::CorruptExisting { .. }));
    }

    #[test]
    fn read_round_trip() {
        let dir = TempDir::new();
        let store = ChunkStore::new(&dir.path);
        let outcome = store.write_chunk(KIND, b"read me back").unwrap();
        let WriteOutcome::Written { hash, .. } = outcome else {
            panic!("Written이어야 함");
        };
        let payload = store.read_chunk(&hash, KIND).unwrap();
        assert_eq!(payload, b"read me back");
    }

    #[test]
    fn read_missing_chunk_fails() {
        let dir = TempDir::new();
        let store = ChunkStore::new(&dir.path);
        let hash = ContentHash::compute(b"never written");
        let err = store.read_chunk(&hash, KIND).unwrap_err();
        assert!(matches!(err, ChunkStoreError::MissingChunk { .. }));
    }

    #[test]
    fn read_with_wrong_expected_kind_fails() {
        let dir = TempDir::new();
        let store = ChunkStore::new(&dir.path);
        let WriteOutcome::Written { hash, .. } = store.write_chunk(KIND, b"kind check").unwrap()
        else {
            panic!("Written이어야 함");
        };
        let err = store.read_chunk(&hash, ChunkKindCode(9)).unwrap_err();
        assert!(matches!(err, ChunkStoreError::KindMismatch { .. }));
    }

    #[test]
    fn every_single_bit_corruption_is_detected() {
        // 검증 기준(V.2.6): single-bit corruption fixture 전부 탐지.
        // 파일의 모든 byte·모든 bit을 하나씩 뒤집어 읽기가 반드시 실패해야 합니다.
        let dir = TempDir::new();
        let store = ChunkStore::new(&dir.path);
        let WriteOutcome::Written { hash, path } = store
            .write_chunk(KIND, b"single bit corruption target")
            .unwrap()
        else {
            panic!("Written이어야 함");
        };
        let file = dir.path.join(path.to_relative_path());
        let original = std::fs::read(&file).unwrap();

        for byte_index in 0..original.len() {
            for bit in 0..8u8 {
                let mut corrupted = original.clone();
                corrupted[byte_index] ^= 1 << bit;
                std::fs::write(&file, &corrupted).unwrap();
                assert!(
                    store.read_chunk(&hash, KIND).is_err(),
                    "byte {byte_index} bit {bit} 손상이 탐지되지 않음"
                );
            }
        }
        // 원본 복원 후에는 정상 읽기.
        std::fs::write(&file, &original).unwrap();
        assert!(store.read_chunk(&hash, KIND).is_ok());
    }

    #[test]
    fn truncated_file_is_detected() {
        let dir = TempDir::new();
        let store = ChunkStore::new(&dir.path);
        let WriteOutcome::Written { hash, path } =
            store.write_chunk(KIND, b"truncate target").unwrap()
        else {
            panic!("Written이어야 함");
        };
        let file = dir.path.join(path.to_relative_path());
        let original = std::fs::read(&file).unwrap();
        for cut in [0, 5, 50, original.len() - 1] {
            std::fs::write(&file, &original[..cut]).unwrap();
            assert!(
                store.read_chunk(&hash, KIND).is_err(),
                "절단 길이 {cut} 손상이 탐지되지 않음"
            );
        }
    }

    #[test]
    fn distinct_payloads_do_not_collide() {
        let dir = TempDir::new();
        let store = ChunkStore::new(&dir.path);
        for i in 0..50u32 {
            let payload = format!("payload-{i}").into_bytes();
            let outcome = store.write_chunk(KIND, &payload).unwrap();
            assert!(matches!(outcome, WriteOutcome::Written { .. }));
        }
    }

    /// guard 거부가 담긴 I/O 오류인지.
    fn io_refusal(err: ChunkStoreError) -> (PathBuf, Refusal) {
        match err {
            ChunkStoreError::Io { path, source } => {
                let refusal = Refusal::of(&source)
                    .unwrap_or_else(|| panic!("guard 거부가 아닌 I/O 오류: {source:?}"));
                (path, refusal)
            }
            other => panic!("Io 오류가 아님: {other:?}"),
        }
    }

    fn written_hash(outcome: WriteOutcome) -> (ContentHash, ChunkObjectPath) {
        let WriteOutcome::Written { hash, path } = outcome else {
            panic!("Written이어야 함: {outcome:?}");
        };
        (hash, path)
    }

    /// 보안 감사 STOR-1: chunk 자리의 장치·FIFO는 열기·읽기에서 멈추거나 메모리를 끝없이 쓰지 않고
    /// 거부됩니다.
    #[cfg(unix)]
    #[test]
    fn read_refuses_devices_and_fifos_without_blocking() {
        use crate::fs_guard::test_support::{bounded, mkfifo};

        let dir = TempDir::new();
        let hash = ContentHash::compute(b"special");
        let file = dir
            .path
            .join(ChunkObjectPath::for_hash(&hash).to_relative_path());
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let fifo = dir.path.join("fifo");
        mkfifo(&fifo);
        for target in [Path::new("/dev/zero"), fifo.as_path()] {
            let _ = std::fs::remove_file(&file);
            std::os::unix::fs::symlink(target, &file).unwrap();
            let root = dir.path.clone();
            let err = bounded("chunk 읽기", move || {
                ChunkStore::new(root).read_chunk(&hash, KIND).unwrap_err()
            });
            assert_eq!(io_refusal(err), (file.clone(), Refusal::NotRegularFile));
        }
    }

    /// 보안 감사 STOR-1: 상한을 넘는 파일(거대한 sparse 파일 포함)은 읽지 않고 거부합니다.
    #[test]
    fn read_refuses_files_over_the_limit_without_reading() {
        let dir = TempDir::new();
        let store = ChunkStore::new(&dir.path);
        let (hash, path) = written_hash(store.write_chunk(KIND, b"bounded").unwrap());
        let file = dir.path.join(path.to_relative_path());
        let len = std::fs::metadata(&file).unwrap().len();
        assert_eq!(
            store.read_chunk_limited(&hash, KIND, len).unwrap(),
            b"bounded"
        );
        assert_eq!(
            io_refusal(store.read_chunk_limited(&hash, KIND, len - 1).unwrap_err()),
            (file.clone(), Refusal::TooLarge { limit: len - 1 })
        );
        // 실제 상한 + 1 bytes sparse 파일: 크기 확인만으로 거부합니다.
        std::fs::File::create(&file)
            .unwrap()
            .set_len(MAX_CHUNK_FILE_BYTES + 1)
            .unwrap();
        assert_eq!(
            io_refusal(store.read_chunk(&hash, KIND).unwrap_err()),
            (
                file,
                Refusal::TooLarge {
                    limit: MAX_CHUNK_FILE_BYTES
                }
            )
        );
    }

    /// 사용자 작업 트리의 symlink는 따릅니다(기술 아키텍처 19.3). 내용 검증은 그대로입니다.
    #[cfg(unix)]
    #[test]
    fn read_follows_symlink_to_regular_chunk_file() {
        let dir = TempDir::new();
        let outside = TempDir::new();
        let store = ChunkStore::new(&dir.path);
        let (hash, path) = written_hash(store.write_chunk(KIND, b"linked chunk").unwrap());
        let file = dir.path.join(path.to_relative_path());
        let moved = outside.path.join("chunk");
        std::fs::rename(&file, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &file).unwrap();
        assert_eq!(store.read_chunk(&hash, KIND).unwrap(), b"linked chunk");
    }

    /// 보안 감사 STOR-4: cache 폴더(`.stellar`·`cache`·`chunks`)·`objects`·shard 디렉터리가 symlink면 따라가지
    /// 않고 기록을 거부합니다(프로젝트 밖에 디렉터리·파일을 만들지 않음).
    #[cfg(unix)]
    #[test]
    fn write_refuses_symlinked_storage_directories() {
        let payload = b"payload";
        let prefix = ChunkObjectPath::for_hash(&ContentHash::compute(payload))
            .prefix()
            .to_string();
        let objects = crate::chunk::path::OBJECTS_DIR;
        for depth in 1..=objects.len() + 1 {
            let mut link: Vec<&str> = objects.to_vec();
            link.push(prefix.as_str());
            link.truncate(depth);
            let real_parents = link[..depth - 1].to_vec();
            let dir = TempDir::new();
            let outside = TempDir::new();
            let mut parent = dir.path.clone();
            for component in &real_parents {
                parent.push(component);
            }
            std::fs::create_dir_all(&parent).unwrap();
            let link_path = parent.join(link.last().unwrap());
            std::os::unix::fs::symlink(&outside.path, &link_path).unwrap();

            let err = ChunkStore::new(&dir.path)
                .write_chunk(KIND, payload)
                .unwrap_err();
            assert_eq!(
                io_refusal(err),
                (link_path.clone(), Refusal::SymlinkDirectory),
                "{link:?}"
            );
            assert_eq!(
                std::fs::read_dir(&outside.path).unwrap().count(),
                0,
                "{link:?}: symlink 대상에 아무것도 만들지 않음"
            );
        }
    }

    /// dedup 검증은 대상 자리의 symlink를 따르지 않습니다. 같은 bytes를 가리켜도 불변 chunk 파일이
    /// 아니므로 CorruptExisting이고 링크·대상은 그대로 둡니다. 장치·FIFO도 멈추지 않고 같은 오류입니다.
    #[cfg(unix)]
    #[test]
    fn write_does_not_follow_existing_target_entry() {
        use crate::fs_guard::test_support::{bounded, mkfifo};

        let payload = b"existing target";
        let dir = TempDir::new();
        let outside = TempDir::new();
        let staging = ChunkStore::new(&outside.path);
        let (_, path) = written_hash(staging.write_chunk(KIND, payload).unwrap());
        let same_bytes = outside.path.join(path.to_relative_path());
        let fifo = outside.path.join("fifo");
        mkfifo(&fifo);

        let file = dir.path.join(path.to_relative_path());
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        for target in [same_bytes.as_path(), Path::new("/dev/zero"), fifo.as_path()] {
            let _ = std::fs::remove_file(&file);
            std::os::unix::fs::symlink(target, &file).unwrap();
            let root = dir.path.clone();
            let result = bounded("dedup 검증", move || {
                ChunkStore::new(root).write_chunk(KIND, payload)
            });
            assert!(
                matches!(result, Err(ChunkStoreError::CorruptExisting { .. })),
                "{target:?}: {result:?}"
            );
            assert!(std::fs::symlink_metadata(&file)
                .unwrap()
                .file_type()
                .is_symlink());
        }
        assert!(std::fs::read(&same_bytes).unwrap().ends_with(payload));

        // 대상 자리의 디렉터리도 불변 chunk 파일이 아닙니다.
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(matches!(
            ChunkStore::new(&dir.path).write_chunk(KIND, payload),
            Err(ChunkStoreError::CorruptExisting { .. })
        ));
    }

    /// 읽기 상한을 넘는 chunk는 다시 읽을 수 없으므로 기록하지 않습니다.
    #[test]
    fn write_refuses_chunk_over_the_read_limit() {
        let dir = TempDir::new();
        let store = ChunkStore::new(&dir.path);
        let mut noop = crate::atomic_file::NoopObserver;
        let (envelope, bytes) = ChunkStore::encode_file_bytes(KIND, b"over limit").unwrap();
        let limit = bytes.len() as u64 - 1;
        let target = dir
            .path
            .join(ChunkObjectPath::for_hash(&envelope.payload_hash).to_relative_path());
        assert_eq!(
            io_refusal(
                store
                    .write_chunk_limited(KIND, b"over limit", &mut noop, limit)
                    .unwrap_err()
            ),
            (target.clone(), Refusal::TooLarge { limit })
        );
        assert!(!target.exists());
        assert!(matches!(
            store
                .write_chunk_limited(KIND, b"over limit", &mut noop, limit + 1)
                .unwrap(),
            WriteOutcome::Written { .. }
        ));
    }
}

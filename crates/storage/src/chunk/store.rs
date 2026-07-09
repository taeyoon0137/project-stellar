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

use std::fmt;
use std::path::{Path, PathBuf};

use crate::atomic_file::write_file_atomic;
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
        }
    }
}

impl std::error::Error for ChunkStoreError {}

impl From<EnvelopeError> for ChunkStoreError {
    fn from(e: EnvelopeError) -> ChunkStoreError {
        ChunkStoreError::Envelope(e)
    }
}

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
    /// - 내용이 다르면 [`ChunkStoreError::CorruptExisting`] — 덮어쓰지 않습니다.
    pub fn write_chunk(
        &self,
        kind: ChunkKindCode,
        payload: &[u8],
    ) -> Result<WriteOutcome, ChunkStoreError> {
        let (envelope, expected_bytes) = ChunkStore::encode_file_bytes(kind, payload)?;
        let hash = envelope.payload_hash;
        let path = ChunkObjectPath::for_hash(&hash);
        let target = self.absolute_path(&path);

        if target.exists() {
            let existing = std::fs::read(&target).map_err(|source| ChunkStoreError::Io {
                path: target.clone(),
                source,
            })?;
            if existing == expected_bytes {
                return Ok(WriteOutcome::Reused { hash, path });
            }
            return Err(ChunkStoreError::CorruptExisting { path });
        }

        let parent = target.parent().expect("chunk 경로는 항상 부모가 있음");
        std::fs::create_dir_all(parent).map_err(|source| ChunkStoreError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
        write_file_atomic(&target, &expected_bytes).map_err(|source| ChunkStoreError::Io {
            path: target.clone(),
            source,
        })?;
        Ok(WriteOutcome::Written { hash, path })
    }
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
    fn distinct_payloads_do_not_collide() {
        let dir = TempDir::new();
        let store = ChunkStore::new(&dir.path);
        for i in 0..50u32 {
            let payload = format!("payload-{i}").into_bytes();
            let outcome = store.write_chunk(KIND, &payload).unwrap();
            assert!(matches!(outcome, WriteOutcome::Written { .. }));
        }
    }
}

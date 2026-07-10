//! 프로젝트 저장소 open과 읽기 전용 recovery (V.4.6).
//!
//! open 흐름:
//!
//! 1. current generation 로드(V.4.4)를 시도합니다. 성공하면 정상 open입니다.
//! 2. 실패하면 fallback scanner(V.4.5)로 최신 정상 세대를 찾습니다. 찾으면
//!    **읽기 전용 recovery** 결과를 반환합니다 — current pointer를 자동으로
//!    바꾸지 않습니다(복구 확정은 사용자 결정·저장 경로 소관).
//! 3. 정상 세대가 하나도 없으면 오류입니다.
//!
//! open은 어떤 경로로도 디스크 bytes를 변경하지 않습니다.

use std::fmt;
use std::path::Path;

use stellar_core_types::ids::StableId;

use super::loader::{load_current_generation, LoadError, LoadedGeneration};
use super::scanner::{scan_latest_valid_generation, ScanError};

/// open 결과.
#[derive(Debug)]
pub enum OpenOutcome {
    /// current generation이 정상입니다.
    Current(LoadedGeneration),
    /// current가 손상되어 정상 세대로 fallback했습니다. **읽기 전용**입니다 —
    /// current pointer는 변경되지 않았습니다.
    ReadOnlyRecovery {
        /// fallback으로 선택된 정상 세대.
        fallback: LoadedGeneration,
        /// current 로드가 실패한 원인.
        current_error: LoadError,
    },
}

/// open 실패.
#[derive(Debug)]
pub enum OpenError {
    /// current도 손상이고 정상 fallback 세대도 없습니다.
    NoValidGeneration {
        /// current 로드가 실패한 원인.
        current_error: LoadError,
    },
    /// fallback scan 자체가 실패했습니다(디렉터리 접근 오류).
    Scan(ScanError),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::NoValidGeneration { current_error } => {
                write!(
                    f,
                    "정상 세대를 찾을 수 없습니다 (current 오류: {current_error})"
                )
            }
            OpenError::Scan(e) => write!(f, "fallback scan 실패: {e}"),
        }
    }
}

impl std::error::Error for OpenError {}

/// 프로젝트 저장소를 엽니다. 디스크 bytes를 변경하지 않습니다.
pub fn open_project_storage(
    root: &Path,
    expected_project_id: StableId,
) -> Result<OpenOutcome, OpenError> {
    let current_error = match load_current_generation(root, expected_project_id) {
        Ok(loaded) => return Ok(OpenOutcome::Current(loaded)),
        Err(e) => e,
    };
    match scan_latest_valid_generation(root, expected_project_id) {
        Ok(Some(fallback)) => Ok(OpenOutcome::ReadOnlyRecovery {
            fallback,
            current_error,
        }),
        Ok(None) => Err(OpenError::NoValidGeneration { current_error }),
        Err(e) => Err(OpenError::Scan(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::envelope::ChunkKindCode;
    use crate::chunk::id::ChunkId;
    use crate::chunk::pointer::ChunkPointer;
    use crate::chunk::store::{ChunkStore, WriteOutcome};
    use crate::generation::current::{read_current_pointer, write_current_pointer};
    use crate::generation::loader::{manifest_path, write_manifest_file};
    use crate::generation::manifest::GenerationManifest;
    use crate::hash::ContentHash;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("stellar-open-test-{}-{n}", std::process::id()));
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

    fn write_generation(dir: &TempDir, project_id: StableId, n: u64) -> ContentHash {
        let store = ChunkStore::new(&dir.path);
        let payload = format!("gen-{n}-chunk").into_bytes();
        let outcome = store.write_chunk(KIND, &payload).unwrap();
        let (WriteOutcome::Written { hash, .. } | WriteOutcome::Reused { hash, .. }) = outcome;
        let manifest = GenerationManifest {
            generation: n,
            parent_generation: (n > 1).then(|| n - 1),
            project_id,
            schema_version: 1,
            chunk_pointers: vec![ChunkPointer {
                chunk_id: ChunkId::from_stable_id(stable(1_000)),
                content_hash: hash,
                kind: KIND,
                schema_version: 1,
            }],
            root_document_ids: vec![stable(7_000)],
        };
        write_manifest_file(&dir.path, &manifest).unwrap()
    }

    /// 디렉터리 아래 전체 파일 bytes 스냅샷.
    fn disk_snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut snapshot = BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let bytes = std::fs::read(&path).unwrap();
                    snapshot.insert(path, bytes);
                }
            }
        }
        snapshot
    }

    #[test]
    fn healthy_project_opens_as_current() {
        let dir = TempDir::new();
        let project_id = stable(42);
        let hash = write_generation(&dir, project_id, 1);
        write_current_pointer(&dir.path, &hash).unwrap();

        let before = disk_snapshot(&dir.path);
        let outcome = open_project_storage(&dir.path, project_id).unwrap();
        assert!(matches!(outcome, OpenOutcome::Current(_)));
        assert_eq!(
            disk_snapshot(&dir.path),
            before,
            "open은 디스크를 바꾸지 않음"
        );
    }

    #[test]
    fn corrupt_current_falls_back_read_only_without_disk_change() {
        // 검증 기준(V.4.6): open만으로 디스크 bytes가 변하지 않습니다.
        let dir = TempDir::new();
        let project_id = stable(42);
        let gen1 = write_generation(&dir, project_id, 1);
        let gen2 = write_generation(&dir, project_id, 2);
        write_current_pointer(&dir.path, &gen2).unwrap();
        // current가 가리키는 generation 2 manifest를 손상.
        let path = manifest_path(&dir.path, &gen2);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[15] ^= 0x40;
        std::fs::write(&path, &bytes).unwrap();

        let before = disk_snapshot(&dir.path);
        let outcome = open_project_storage(&dir.path, project_id).unwrap();
        let OpenOutcome::ReadOnlyRecovery {
            fallback,
            current_error,
        } = outcome
        else {
            panic!("ReadOnlyRecovery여야 함");
        };
        assert_eq!(fallback.manifest.generation, 1);
        assert_eq!(fallback.manifest_hash, gen1);
        assert!(matches!(
            current_error,
            LoadError::ManifestHashMismatch { .. }
        ));
        // current pointer는 여전히 손상된 generation 2를 가리킵니다(자동 변경 없음).
        assert_eq!(read_current_pointer(&dir.path).unwrap(), gen2);
        // 디스크 bytes 완전 불변.
        assert_eq!(disk_snapshot(&dir.path), before);
    }

    #[test]
    fn no_valid_generation_is_error_without_disk_change() {
        let dir = TempDir::new();
        let project_id = stable(42);
        // current pointer만 있고 manifest 없음.
        write_current_pointer(&dir.path, &ContentHash::compute(b"ghost")).unwrap();

        let before = disk_snapshot(&dir.path);
        let err = open_project_storage(&dir.path, project_id).unwrap_err();
        assert!(matches!(err, OpenError::NoValidGeneration { .. }));
        assert_eq!(disk_snapshot(&dir.path), before);
    }
}

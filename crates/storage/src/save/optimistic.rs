//! 저장 전 optimistic base check (V.5.2).
//!
//! SavePlan은 특정 base generation 위에서 계산된 결과입니다. 계획 수립과 실행
//! 사이에 current pointer가 다른 세대로 바뀌었으면(다른 창·프로세스의 저장,
//! Git 조작 등) 그 계획은 무효입니다. 실행기(V.5.3)는 **어떤 write도 시작하기
//! 전에** 이 검사를 통과해야 합니다 — stale base에서는 chunk 파일이 하나도
//! 생기지 않습니다.

use std::fmt;
use std::path::Path;

use crate::generation::current::{read_current_pointer, CurrentPointerError};
use crate::hash::ContentHash;

use super::plan::SavePlan;

/// optimistic check 실패.
#[derive(Debug)]
pub enum BaseCheckError {
    /// current pointer가 계획의 base와 다른 세대를 가리킵니다.
    StaleBase {
        /// 계획이 가정한 base manifest hash(첫 저장은 `None`).
        expected: Option<ContentHash>,
        /// 실제 current pointer(파일 없음은 `None`).
        found: Option<ContentHash>,
    },
    /// current pointer 읽기 실패(형식 위반·I/O).
    CurrentPointer(CurrentPointerError),
}

impl fmt::Display for BaseCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BaseCheckError::StaleBase { expected, found } => {
                let show =
                    |h: &Option<ContentHash>| h.map_or("(없음)".to_string(), |h| h.to_string());
                write!(
                    f,
                    "base generation이 오래되었습니다: 계획 base {}, 현재 current {}",
                    show(expected),
                    show(found)
                )
            }
            BaseCheckError::CurrentPointer(e) => write!(f, "current pointer 오류: {e}"),
        }
    }
}

impl std::error::Error for BaseCheckError {}

/// current pointer가 계획의 base generation과 일치하는지 검사합니다.
///
/// - 계획 base가 있으면: current pointer가 정확히 그 hash여야 합니다.
/// - 첫 저장(base 없음)이면: current pointer 파일이 없어야 합니다.
///
/// 일치하지 않으면 [`BaseCheckError::StaleBase`] — 호출자는 write를 시작하면
/// 안 됩니다.
pub fn check_base_generation(root: &Path, plan: &SavePlan) -> Result<(), BaseCheckError> {
    check_base_manifest(root, plan.base_manifest_hash)
}

/// 열린 문서가 기억하는 manifest hash를 현재 포인터와 대조합니다.
pub fn check_base_manifest(
    root: &Path,
    expected: Option<ContentHash>,
) -> Result<(), BaseCheckError> {
    let found = match read_current_pointer(root) {
        Ok(hash) => Some(hash),
        Err(CurrentPointerError::Missing { .. }) => None,
        Err(e) => return Err(BaseCheckError::CurrentPointer(e)),
    };
    if found != expected {
        return Err(BaseCheckError::StaleBase { expected, found });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::envelope::ChunkKindCode;
    use crate::chunk::id::ChunkId;
    use crate::chunk::planner::LogicalChunkState;
    use crate::generation::current::write_current_pointer;
    use crate::generation::loader::{write_manifest_file, LoadedGeneration};
    use crate::generation::manifest::GenerationManifest;
    use crate::save::plan::build_save_plan;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use stellar_core_types::ids::StableId;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-optimistic-test-{}-{n}",
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

    const KIND: ChunkKindCode = ChunkKindCode(0x8000_0001);

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    /// chunk 파일 없는 빈 세대(존재 검증을 안 거치는 단위 테스트용).
    fn generation(dir: &TempDir, n: u64) -> LoadedGeneration {
        let manifest = GenerationManifest {
            generation: n,
            parent_generation: (n > 1).then(|| n - 1),
            project_id: stable(42),
            schema_version: 1,
            chunk_pointers: vec![],
            root_document_ids: vec![],
        };
        let hash = write_manifest_file(&dir.path, &manifest).unwrap();
        LoadedGeneration {
            manifest_hash: hash,
            manifest,
        }
    }

    fn dirty_state() -> Vec<LogicalChunkState> {
        vec![LogicalChunkState::Dirty {
            chunk_id: ChunkId::from_stable_id(stable(1_000)),
            kind: KIND,
            schema_version: 1,
            payload: b"payload".to_vec(),
        }]
    }

    /// design/objects 아래 chunk 파일 수.
    fn chunk_file_count(root: &std::path::Path) -> usize {
        let objects = root.join("design").join("objects");
        if !objects.is_dir() {
            return 0;
        }
        let mut count = 0;
        let mut stack = vec![objects];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    count += 1;
                }
            }
        }
        count
    }

    #[test]
    fn matching_base_passes() {
        let dir = TempDir::new();
        let base = generation(&dir, 1);
        write_current_pointer(&dir.path, &base.manifest_hash).unwrap();
        let plan = build_save_plan(Some(&base), stable(42), 1, &dirty_state(), vec![]).unwrap();
        assert!(check_base_generation(&dir.path, &plan).is_ok());
    }

    #[test]
    fn first_save_requires_no_current_pointer() {
        let dir = TempDir::new();
        let plan = build_save_plan(None, stable(42), 1, &dirty_state(), vec![]).unwrap();
        assert!(check_base_generation(&dir.path, &plan).is_ok());

        // 누군가 먼저 저장했다면 첫 저장 계획은 stale.
        let other = generation(&dir, 1);
        write_current_pointer(&dir.path, &other.manifest_hash).unwrap();
        assert!(matches!(
            check_base_generation(&dir.path, &plan).unwrap_err(),
            BaseCheckError::StaleBase { .. }
        ));
    }

    #[test]
    fn stale_base_fails_and_no_chunk_files_are_created() {
        // 검증 기준(V.5.2): stale base fixture에서 chunk 파일이 새로 생기지 않습니다.
        let dir = TempDir::new();
        let base = generation(&dir, 1);
        write_current_pointer(&dir.path, &base.manifest_hash).unwrap();
        // base 위에서 계획 수립.
        let plan = build_save_plan(Some(&base), stable(42), 1, &dirty_state(), vec![]).unwrap();
        // 그 사이 다른 창이 generation 2를 저장 → current가 바뀜.
        let newer = generation(&dir, 2);
        write_current_pointer(&dir.path, &newer.manifest_hash).unwrap();

        let before = chunk_file_count(&dir.path);
        let err = check_base_generation(&dir.path, &plan).unwrap_err();
        assert!(matches!(err, BaseCheckError::StaleBase { .. }));
        // 검사 실패 → write 시작 금지 → chunk 파일 수 불변.
        assert_eq!(chunk_file_count(&dir.path), before);
        assert_eq!(before, 0);
    }

    #[test]
    fn malformed_current_pointer_is_reported() {
        let dir = TempDir::new();
        let current = dir.path.join("design").join("current");
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::write(&current, b"garbage\n").unwrap();
        let plan = build_save_plan(None, stable(42), 1, &dirty_state(), vec![]).unwrap();
        assert!(matches!(
            check_base_generation(&dir.path, &plan).unwrap_err(),
            BaseCheckError::CurrentPointer(_)
        ));
    }
}

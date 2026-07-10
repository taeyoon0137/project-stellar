//! unreferenced 파일 sweep (V.7.3).
//!
//! mark 집합(V.7.2)에 없는 chunk·manifest 파일을 **grace period가 지난 뒤에만**
//! 삭제합니다. 방금 기록됐지만 아직 current로 연결되지 않은 파일(진행 중인
//! 저장의 산출물)을 지우지 않기 위한 유예입니다.
//!
//! 파일 삭제 실패는 오류가 아니라 결과에 기록만 하고 **다음 실행으로
//! 이월**합니다 — sweep은 언제 다시 실행해도 안전합니다(idempotent).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::hash::ContentHash;

use super::retention::RetentionPolicy;

/// sweep 결과.
#[derive(Debug, Default)]
pub struct SweepOutcome {
    /// 삭제된 파일 경로.
    pub deleted: Vec<PathBuf>,
    /// mark되어 보존된 파일 수.
    pub kept_reachable: usize,
    /// unmark지만 grace period 이내라 보존된 파일 수.
    pub kept_in_grace: usize,
    /// 삭제 실패 — 다음 실행으로 이월.
    pub carry_over: Vec<PathBuf>,
}

/// 파일 이름(`<algo>-<hex>.<ext>`)에서 hash를 복원합니다. 형식이 아니면 `None`.
fn hash_from_file_name(name: &str, extension: &str) -> Option<ContentHash> {
    let stem = name.strip_suffix(extension)?;
    let (algo, hex) = stem.split_once('-')?;
    format!("{algo}:{hex}").parse().ok()
}

/// 디렉터리 아래의 content-addressed 파일을 재귀 수집합니다.
fn collect_files(dir: &Path, extension: &str, out: &mut Vec<(ContentHash, PathBuf)>) {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if let Some(hash) = hash_from_file_name(name, extension) {
                    out.push((hash, path));
                }
            }
        }
    }
}

/// unreferenced chunk·manifest를 sweep합니다.
///
/// - `reachable`: mark 집합(V.7.2). 포함된 파일은 절대 삭제하지 않습니다.
/// - `now`: 현재 시각(테스트 주입용). 파일 mtime + grace period가 `now` 이후면
///   유예 중으로 보존합니다.
pub fn sweep_unreferenced(
    root: &Path,
    reachable: &BTreeSet<ContentHash>,
    policy: &RetentionPolicy,
    now: SystemTime,
) -> SweepOutcome {
    let grace = Duration::from_secs(policy.grace_period_seconds);
    let mut files: Vec<(ContentHash, PathBuf)> = Vec::new();
    collect_files(
        &root.join("design").join("objects"),
        ".starchunk",
        &mut files,
    );
    collect_files(
        &root.join("design").join("generations"),
        ".starmanifest",
        &mut files,
    );

    let mut outcome = SweepOutcome::default();
    for (hash, path) in files {
        if reachable.contains(&hash) {
            outcome.kept_reachable += 1;
            continue;
        }
        // grace period 판정: mtime을 읽지 못하면 보수적으로 보존(이월).
        let old_enough = match path.metadata().and_then(|m| m.modified()) {
            Ok(mtime) => now
                .duration_since(mtime)
                .map(|age| age >= grace)
                .unwrap_or(false),
            Err(_) => {
                outcome.carry_over.push(path);
                continue;
            }
        };
        if !old_enough {
            outcome.kept_in_grace += 1;
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => outcome.deleted.push(path),
            // 삭제 실패는 다음 실행으로 이월합니다.
            Err(_) => outcome.carry_over.push(path),
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::envelope::ChunkKindCode;
    use crate::chunk::id::ChunkId;
    use crate::chunk::path::ChunkObjectPath;
    use crate::chunk::pointer::ChunkPointer;
    use crate::chunk::store::{ChunkStore, WriteOutcome};
    use crate::gc::mark::mark_reachable;
    use crate::generation::loader::write_manifest_file;
    use crate::generation::manifest::GenerationManifest;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use stellar_core_types::ids::StableId;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("stellar-sweep-test-{}-{n}", std::process::id()));
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

    fn policy(grace_seconds: u64) -> RetentionPolicy {
        RetentionPolicy {
            keep_recent_generations: 2,
            grace_period_seconds: grace_seconds,
        }
    }

    fn write_chunk(dir: &TempDir, payload: &[u8]) -> ContentHash {
        let store = ChunkStore::new(&dir.path);
        let (WriteOutcome::Written { hash, .. } | WriteOutcome::Reused { hash, .. }) =
            store.write_chunk(KIND, payload).unwrap();
        hash
    }

    /// grace period(1시간)를 지난 시점.
    fn after_grace() -> SystemTime {
        SystemTime::now() + Duration::from_secs(3_600 + 60)
    }

    #[test]
    fn active_chunks_are_kept_and_orphans_are_deleted() {
        // 검증 기준(V.7.3): active chunk 미삭제, orphan 삭제 fixture.
        let dir = TempDir::new();
        let active = write_chunk(&dir, b"active-chunk");
        let orphan = write_chunk(&dir, b"orphan-chunk");
        let manifest_hash = write_manifest_file(
            &dir.path,
            &GenerationManifest {
                generation: 1,
                parent_generation: None,
                project_id: stable(42),
                schema_version: 1,
                chunk_pointers: vec![ChunkPointer {
                    chunk_id: ChunkId::from_stable_id(stable(100)),
                    content_hash: active,
                    kind: KIND,
                    schema_version: 1,
                }],
                root_document_ids: vec![],
            },
        )
        .unwrap();

        let reachable = mark_reachable(&dir.path, &[manifest_hash]).unwrap();
        let outcome = sweep_unreferenced(&dir.path, &reachable, &policy(3_600), after_grace());

        let active_path = dir
            .path
            .join(ChunkObjectPath::for_hash(&active).to_relative_path());
        let orphan_path = dir
            .path
            .join(ChunkObjectPath::for_hash(&orphan).to_relative_path());
        assert!(active_path.is_file(), "active chunk는 삭제 금지");
        assert!(!orphan_path.exists(), "orphan chunk는 삭제");
        assert_eq!(outcome.deleted.len(), 1);
        assert_eq!(outcome.kept_reachable, 2, "active chunk + 보존 manifest");
        assert!(outcome.carry_over.is_empty());
    }

    #[test]
    fn orphan_inside_grace_period_is_kept() {
        let dir = TempDir::new();
        write_chunk(&dir, b"fresh-orphan");
        let reachable = BTreeSet::new();
        // 방금 만든 파일 + 1시간 유예 → 아직 삭제 금지.
        let outcome = sweep_unreferenced(&dir.path, &reachable, &policy(3_600), SystemTime::now());
        assert!(outcome.deleted.is_empty());
        assert_eq!(outcome.kept_in_grace, 1);
    }

    #[test]
    fn zero_grace_deletes_immediately() {
        let dir = TempDir::new();
        write_chunk(&dir, b"orphan-now");
        let outcome =
            sweep_unreferenced(&dir.path, &BTreeSet::new(), &policy(0), SystemTime::now());
        assert_eq!(outcome.deleted.len(), 1);
    }

    #[test]
    fn unrelated_files_are_untouched() {
        let dir = TempDir::new();
        let objects = dir.path.join("design").join("objects").join("aa");
        std::fs::create_dir_all(&objects).unwrap();
        let stray = objects.join("notes.txt");
        std::fs::write(&stray, b"not a chunk").unwrap();
        let outcome = sweep_unreferenced(&dir.path, &BTreeSet::new(), &policy(0), after_grace());
        assert!(stray.is_file(), "형식이 아닌 파일은 건드리지 않음");
        assert!(outcome.deleted.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn deletion_failure_is_carried_over_to_next_run() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new();
        let orphan = write_chunk(&dir, b"protected-orphan");
        let orphan_path = dir
            .path
            .join(ChunkObjectPath::for_hash(&orphan).to_relative_path());
        let shard = orphan_path.parent().unwrap().to_path_buf();

        // shard 디렉터리를 읽기 전용으로 → remove_file 실패.
        std::fs::set_permissions(&shard, std::fs::Permissions::from_mode(0o555)).unwrap();
        let outcome = sweep_unreferenced(&dir.path, &BTreeSet::new(), &policy(0), after_grace());
        assert_eq!(outcome.carry_over.len(), 1, "삭제 실패는 이월");
        assert!(orphan_path.is_file());

        // 권한 복구 후 다음 실행에서 삭제.
        std::fs::set_permissions(&shard, std::fs::Permissions::from_mode(0o755)).unwrap();
        let outcome = sweep_unreferenced(&dir.path, &BTreeSet::new(), &policy(0), after_grace());
        assert_eq!(outcome.deleted.len(), 1);
        assert!(!orphan_path.exists());
    }
}

//! 정상 종료 marker와 스냅샷 cleanup (V.9.4).
//!
//! 창 세션 수명주기:
//!
//! - [`begin_window_session`]: 세션 시작 — 이전 정상 종료 marker를 제거합니다
//!   (실행 중 crash가 나면 marker 없음 = 비정상 종료로 판정).
//! - [`end_window_session_clean`]: 정상 종료 — 스냅샷을 삭제하고 정상 종료
//!   marker를 기록합니다.
//! - **비정상 종료(crash)에서는 아무 것도 호출되지 않으므로** 스냅샷이 그대로
//!   남고 marker가 없습니다 — 다음 실행에서 복구 제안(V.9.3) 대상이 됩니다.

use std::path::PathBuf;

use stellar_core_types::ids::WindowSessionId;
use stellar_storage::atomic_file::write_file_atomic;

use crate::identity::ProjectIdentity;
use crate::recovery::{RecoverySnapshotStore, RecoveryStoreError};

/// 정상 종료 marker 파일 확장자.
const CLEAN_MARKER_EXTENSION: &str = "clean-exit";

fn marker_path(
    store: &RecoverySnapshotStore,
    identity: &ProjectIdentity,
    window_session: WindowSessionId,
) -> PathBuf {
    store
        .project_dir(identity)
        .join(format!("{window_session}.{CLEAN_MARKER_EXTENSION}"))
}

/// 세션 시작: 이전 정상 종료 marker를 제거합니다.
pub fn begin_window_session(
    store: &RecoverySnapshotStore,
    identity: &ProjectIdentity,
    window_session: WindowSessionId,
) -> Result<(), RecoveryStoreError> {
    let path = marker_path(store, identity, window_session);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(RecoveryStoreError::Io { path, source }),
    }
}

/// 정상 종료: 스냅샷을 삭제하고 정상 종료 marker를 기록합니다.
pub fn end_window_session_clean(
    store: &RecoverySnapshotStore,
    identity: &ProjectIdentity,
    window_session: WindowSessionId,
) -> Result<(), RecoveryStoreError> {
    store.remove_snapshot(identity, window_session)?;
    let path = marker_path(store, identity, window_session);
    let parent = path.parent().expect("marker 경로는 항상 부모가 있음");
    std::fs::create_dir_all(parent).map_err(|source| RecoveryStoreError::Io {
        path: parent.to_path_buf(),
        source,
    })?;
    write_file_atomic(&path, b"clean\n").map_err(|source| RecoveryStoreError::Io {
        path: path.clone(),
        source,
    })
}

/// 마지막 종료가 정상이었는지 판정합니다(marker 존재 여부).
pub fn was_clean_shutdown(
    store: &RecoverySnapshotStore,
    identity: &ProjectIdentity,
    window_session: WindowSessionId,
) -> bool {
    marker_path(store, identity, window_session).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery_payload::RecoveryPayload;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use stellar_core_types::ids::StableId;

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new(prefix: &str) -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("{prefix}-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    fn fixture() -> (TempDir, TempDir, ProjectIdentity, RecoverySnapshotStore) {
        let project = TempDir::new("stellar-shutdown-project");
        let app_data = TempDir::new("stellar-shutdown-appdata");
        let identity = ProjectIdentity::new(stable(1), &project.path).unwrap();
        let store = RecoverySnapshotStore::new(&app_data.path);
        (project, app_data, identity, store)
    }

    fn snapshot_bytes(revision: u64) -> Vec<u8> {
        RecoveryPayload {
            base_manifest_hash: None,
            revision,
            unsaved_chunks: vec![],
            open_tabs: vec![],
            active_tab: 0,
            selection: vec![],
            target_profile: stable(4_000),
        }
        .encode()
        .unwrap()
    }

    #[test]
    fn clean_shutdown_removes_snapshot_and_leaves_marker() {
        // 검증 기준(V.9.4): 정상 fixture 결과 정확.
        let (_p, _a, identity, store) = fixture();
        let window = WindowSessionId::from_stable_id(stable(100));

        begin_window_session(&store, &identity, window).unwrap();
        store
            .write_snapshot(&identity, window, &snapshot_bytes(3))
            .unwrap();
        end_window_session_clean(&store, &identity, window).unwrap();

        assert_eq!(store.read_snapshot(&identity, window).unwrap(), None);
        assert!(was_clean_shutdown(&store, &identity, window));
    }

    #[test]
    fn crash_leaves_snapshot_and_no_marker() {
        // 검증 기준(V.9.4): 비정상 종료 simulation에서는 snapshot이 남습니다.
        let (_p, _a, identity, store) = fixture();
        let window = WindowSessionId::from_stable_id(stable(100));

        begin_window_session(&store, &identity, window).unwrap();
        store
            .write_snapshot(&identity, window, &snapshot_bytes(3))
            .unwrap();
        // crash: end_window_session_clean이 호출되지 않음.

        assert!(store.read_snapshot(&identity, window).unwrap().is_some());
        assert!(!was_clean_shutdown(&store, &identity, window));
    }

    #[test]
    fn next_session_start_clears_previous_clean_marker() {
        let (_p, _a, identity, store) = fixture();
        let window = WindowSessionId::from_stable_id(stable(100));

        begin_window_session(&store, &identity, window).unwrap();
        end_window_session_clean(&store, &identity, window).unwrap();
        assert!(was_clean_shutdown(&store, &identity, window));

        // 같은 세션 id로 재시작하면 marker가 지워집니다 — 실행 중 crash 시
        // 비정상 종료로 판정되도록.
        begin_window_session(&store, &identity, window).unwrap();
        assert!(!was_clean_shutdown(&store, &identity, window));
    }

    #[test]
    fn clean_shutdown_without_snapshot_is_fine() {
        let (_p, _a, identity, store) = fixture();
        let window = WindowSessionId::from_stable_id(stable(100));
        begin_window_session(&store, &identity, window).unwrap();
        // 미저장 변경이 없어 스냅샷을 만들지 않은 세션의 정상 종료.
        end_window_session_clean(&store, &identity, window).unwrap();
        assert!(was_clean_shutdown(&store, &identity, window));
    }

    #[test]
    fn marker_is_not_listed_as_snapshot_session() {
        let (_p, _a, identity, store) = fixture();
        let window = WindowSessionId::from_stable_id(stable(100));
        end_window_session_clean(&store, &identity, window).unwrap();
        // marker 파일은 스냅샷 목록(V.9.1 list_sessions)에 나타나면 안 됩니다.
        assert!(store.list_sessions(&identity).unwrap().is_empty());
    }
}

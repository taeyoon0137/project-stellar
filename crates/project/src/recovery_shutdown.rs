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
//! - [`discard_snapshot`]: 정상 저장·정상 닫기·사용자 폐기로 스냅샷을 지웁니다. 먼저 지울 bytes의 hash를
//!   폐기 표식에 쓰고 스냅샷을 지운 뒤 표식을 지웁니다. 삭제가 실패하면 표식이 남고, 다음 시작의 probe는
//!   같은 bytes의 스냅샷을 stale([`StaleReason::Discarded`](crate::recovery_probe::StaleReason))로
//!   무시합니다(XIII.8.7). 같은 창이 뒤에 새 bytes를 기록하면 hash가 달라 다시 제안 대상입니다.

use std::path::PathBuf;

use stellar_core_types::ids::WindowSessionId;
use stellar_storage::atomic_file::write_file_atomic;
use stellar_storage::hash::ContentHash;

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

/// 정상 종료: 스냅샷을 폐기하고([`discard_snapshot`]) 정상 종료 marker를 기록합니다.
pub fn end_window_session_clean(
    store: &RecoverySnapshotStore,
    identity: &ProjectIdentity,
    window_session: WindowSessionId,
) -> Result<(), RecoveryStoreError> {
    discard_snapshot(store, identity, window_session)?;
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

/// 스냅샷을 폐기합니다(정상 저장·정상 닫기·사용자 폐기, XIII.8.7).
///
/// 순서: 지금 스냅샷 bytes의 hash를 폐기 표식에 기록 → 스냅샷 삭제 → 표식 삭제. 스냅샷이 없으면 남은 표식만
/// 지웁니다. 삭제가 실패하면 오류를 돌려주지만 표식은 남아, 다음 시작에서 그 스냅샷을 제안하지 않습니다.
/// 표식 기록이 실패해도 삭제는 시도합니다.
pub fn discard_snapshot(
    store: &RecoverySnapshotStore,
    identity: &ProjectIdentity,
    window_session: WindowSessionId,
) -> Result<(), RecoveryStoreError> {
    let marker = store.discarded_marker_path(identity, window_session);
    let Some(bytes) = store.read_snapshot(identity, window_session)? else {
        return remove_if_exists(&marker);
    };
    let hash = ContentHash::compute(&bytes).to_string();
    let marked =
        write_file_atomic(&marker, hash.as_bytes()).map_err(|source| RecoveryStoreError::Io {
            path: marker.clone(),
            source,
        });
    store.remove_snapshot(identity, window_session)?;
    marked?;
    remove_if_exists(&marker)
}

/// `bytes`가 폐기 표식이 가리키는(지우려다 남은) 스냅샷인지 판정합니다.
pub fn is_discarded(
    store: &RecoverySnapshotStore,
    identity: &ProjectIdentity,
    window_session: WindowSessionId,
    bytes: &[u8],
) -> bool {
    std::fs::read(store.discarded_marker_path(identity, window_session))
        .ok()
        .and_then(|text| String::from_utf8(text).ok())
        .and_then(|text| text.trim().parse::<ContentHash>().ok())
        .is_some_and(|hash| hash == ContentHash::compute(bytes))
}

fn remove_if_exists(path: &std::path::Path) -> Result<(), RecoveryStoreError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(RecoveryStoreError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
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

    /// macOS 사용자 변경 불가 flag로 파일 삭제를 실패시킵니다(같은 폴더의 다른 파일 쓰기는 됩니다).
    #[cfg(target_os = "macos")]
    fn set_immutable(path: &std::path::Path, on: bool) {
        let status = std::process::Command::new("/usr/bin/chflags")
            .arg(if on { "uchg" } else { "nouchg" })
            .arg(path)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn discard_removes_snapshot_and_marker() {
        let (_p, _a, identity, store) = fixture();
        let window = WindowSessionId::from_stable_id(stable(100));
        store
            .write_snapshot(&identity, window, &snapshot_bytes(3))
            .unwrap();
        discard_snapshot(&store, &identity, window).unwrap();
        assert_eq!(store.read_snapshot(&identity, window).unwrap(), None);
        assert!(!store.discarded_marker_path(&identity, window).exists());
        // 지울 것이 없어도 무해합니다.
        discard_snapshot(&store, &identity, window).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn failed_removal_leaves_a_marker_that_matches_only_those_bytes() {
        // XIII.8.7: 삭제 실패는 다음 시작에서 stale로 무시합니다. 같은 창이 새로 기록한 bytes는 다시 대상입니다.
        let (_p, _a, identity, store) = fixture();
        let window = WindowSessionId::from_stable_id(stable(100));
        let old = snapshot_bytes(3);
        store.write_snapshot(&identity, window, &old).unwrap();
        let path = store.snapshot_path(&identity, window);
        set_immutable(&path, true);
        let result = discard_snapshot(&store, &identity, window);
        set_immutable(&path, false);
        assert!(result.is_err(), "삭제 실패를 보고합니다");
        assert_eq!(
            store.read_snapshot(&identity, window).unwrap(),
            Some(old.clone())
        );
        assert!(is_discarded(&store, &identity, window, &old));
        let newer = snapshot_bytes(4);
        assert!(!is_discarded(&store, &identity, window, &newer));
        // 다음 폐기가 성공하면 표식도 정리됩니다.
        store.write_snapshot(&identity, window, &newer).unwrap();
        discard_snapshot(&store, &identity, window).unwrap();
        assert!(!store.discarded_marker_path(&identity, window).exists());
    }
}

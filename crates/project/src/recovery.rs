//! 복구 스냅샷 저장소 인터페이스 (V.9.1).
//!
//! 복구 스냅샷은 **프로젝트 저장소 밖** OS 사용자 앱 데이터 영역에 둡니다.
//! 키는 [`ProjectIdentity`](IV.2.3) + [`WindowSessionId`]입니다.
//!
//! 경로 정책(개인정보): 디렉터리·파일 이름에 프로젝트 이름이나 경로 텍스트를
//! 노출하지 않습니다. 프로젝트 식별 디렉터리는 identity의 canonical 인코딩에
//! 대한 content hash로, 스냅샷 파일 이름은 window session UUID로만 만듭니다.

use std::fmt;
use std::path::{Path, PathBuf};

use stellar_core_types::ids::WindowSessionId;
use stellar_storage::atomic_file::write_file_atomic;
use stellar_storage::canonical::encode::CanonicalEncoder;
use stellar_storage::hash::ContentHash;

use crate::identity::ProjectIdentity;

/// 스냅샷 파일 확장자.
const SNAPSHOT_EXTENSION: &str = "starrecovery";

/// 복구 스냅샷 저장소 오류.
#[derive(Debug)]
pub enum RecoveryStoreError {
    /// 파일 I/O 실패.
    Io {
        /// 대상 경로.
        path: PathBuf,
        /// 원인.
        source: std::io::Error,
    },
}

impl fmt::Display for RecoveryStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecoveryStoreError::Io { path, source } => {
                write!(f, "복구 저장소 I/O 실패({}): {source}", path.display())
            }
        }
    }
}

impl std::error::Error for RecoveryStoreError {}

/// OS 앱 데이터 영역의 복구 스냅샷 저장소.
///
/// `base_dir`(플랫폼 셸이 주입하는 앱 데이터 경로) 아래
/// `recovery/<identity-hash>/<window-session-uuid>.starrecovery` 구조를
/// 사용합니다. payload 형식은 V.9.2에서 정의합니다.
#[derive(Debug, Clone)]
pub struct RecoverySnapshotStore {
    base_dir: PathBuf,
}

impl RecoverySnapshotStore {
    /// 앱 데이터 영역 경로로 저장소를 만듭니다. 디스크는 건드리지 않습니다.
    pub fn new(app_data_dir: &Path) -> RecoverySnapshotStore {
        RecoverySnapshotStore {
            base_dir: app_data_dir.to_path_buf(),
        }
    }

    /// identity를 텍스트 노출 없는 디렉터리 이름으로 변환합니다.
    ///
    /// project StableId(16바이트) + canonical target 경로 bytes를 canonical
    /// 인코딩해 content hash를 취합니다 — 같은 프로젝트(동일 target)의 다른
    /// symlink 경로도 같은 디렉터리로 모입니다.
    fn identity_dir_name(identity: &ProjectIdentity) -> String {
        let mut enc = CanonicalEncoder::new();
        enc.put_fixed_raw(&identity.project_id().to_bytes());
        enc.put_bytes(identity.canonical_target().as_os_str().as_encoded_bytes())
            .expect("경로 길이는 u32 범위");
        let hash = ContentHash::compute(enc.bytes());
        format!("{}-{}", hash.algorithm().name(), hash.digest_hex())
    }

    /// 한 프로젝트의 스냅샷 디렉터리.
    pub fn project_dir(&self, identity: &ProjectIdentity) -> PathBuf {
        self.base_dir
            .join("recovery")
            .join(RecoverySnapshotStore::identity_dir_name(identity))
    }

    /// 한 (프로젝트, 창 세션)의 스냅샷 파일 경로.
    pub fn snapshot_path(
        &self,
        identity: &ProjectIdentity,
        window_session: WindowSessionId,
    ) -> PathBuf {
        self.project_dir(identity)
            .join(format!("{window_session}.{SNAPSHOT_EXTENSION}"))
    }

    /// 스냅샷 bytes를 atomic write로 기록합니다(V.2.5 정책).
    pub fn write_snapshot(
        &self,
        identity: &ProjectIdentity,
        window_session: WindowSessionId,
        payload: &[u8],
    ) -> Result<(), RecoveryStoreError> {
        let path = self.snapshot_path(identity, window_session);
        let parent = path.parent().expect("스냅샷 경로는 항상 부모가 있음");
        std::fs::create_dir_all(parent).map_err(|source| RecoveryStoreError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
        write_file_atomic(&path, payload).map_err(|source| RecoveryStoreError::Io {
            path: path.clone(),
            source,
        })
    }

    /// 스냅샷 bytes를 읽습니다. 없으면 `Ok(None)`.
    pub fn read_snapshot(
        &self,
        identity: &ProjectIdentity,
        window_session: WindowSessionId,
    ) -> Result<Option<Vec<u8>>, RecoveryStoreError> {
        let path = self.snapshot_path(identity, window_session);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(RecoveryStoreError::Io { path, source }),
        }
    }

    /// 한 프로젝트의 스냅샷 세션 목록.
    pub fn list_sessions(
        &self,
        identity: &ProjectIdentity,
    ) -> Result<Vec<String>, RecoveryStoreError> {
        let dir = self.project_dir(identity);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(RecoveryStoreError::Io { path: dir, source }),
        };
        let mut sessions: Vec<String> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_str()?.to_string();
                name.strip_suffix(&format!(".{SNAPSHOT_EXTENSION}"))
                    .map(str::to_string)
            })
            .collect();
        sessions.sort_unstable();
        Ok(sessions)
    }

    /// 스냅샷을 삭제합니다. 없으면 아무 일도 하지 않습니다.
    pub fn remove_snapshot(
        &self,
        identity: &ProjectIdentity,
        window_session: WindowSessionId,
    ) -> Result<(), RecoveryStoreError> {
        let path = self.snapshot_path(identity, window_session);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(RecoveryStoreError::Io { path, source }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn session(ts: u64) -> WindowSessionId {
        WindowSessionId::from_stable_id(stable(ts))
    }

    /// 이름이 특징적인 프로젝트 identity fixture.
    fn identity_with_name(name: &str) -> (TempDir, ProjectIdentity) {
        let dir = TempDir::new("stellar-recovery-project");
        let project_root = dir.path.join(name);
        std::fs::create_dir_all(&project_root).unwrap();
        let identity = ProjectIdentity::new(stable(1), &project_root).unwrap();
        (dir, identity)
    }

    #[test]
    fn paths_do_not_leak_project_name_or_path_text() {
        // 검증 기준(V.9.1): 경로에 프로젝트명·텍스트가 노출되지 않습니다.
        let secret = "SecretDesignProject";
        let (_project_dir, identity) = identity_with_name(secret);
        let app_data = TempDir::new("stellar-recovery-appdata");
        let store = RecoverySnapshotStore::new(&app_data.path);

        let path = store.snapshot_path(&identity, session(9));
        let relative = path.strip_prefix(&app_data.path).unwrap();
        let text = relative.to_string_lossy();
        assert!(
            !text.contains(secret),
            "스냅샷 경로에 프로젝트 이름이 노출됨: {text}"
        );
        // 프로젝트 root 경로의 어떤 구성 요소도 노출 금지(루트 경로 텍스트).
        for component in identity.root_path().components() {
            let piece = component.as_os_str().to_string_lossy();
            if piece.len() > 3 && piece != "/" {
                assert!(
                    !text.contains(piece.as_ref()),
                    "경로 텍스트 노출: {piece} in {text}"
                );
            }
        }
    }

    #[test]
    fn same_target_symlinks_share_directory() {
        let secret = "AliasedProject";
        let (project_dir, identity_a) = identity_with_name(secret);
        // 같은 target을 가리키는 symlink 경로.
        let link = project_dir.path.join("link-alias");
        std::os::unix::fs::symlink(project_dir.path.join(secret), &link).unwrap();
        let identity_b = ProjectIdentity::new(stable(1), &link).unwrap();

        let app_data = TempDir::new("stellar-recovery-appdata");
        let store = RecoverySnapshotStore::new(&app_data.path);
        assert_eq!(
            store.project_dir(&identity_a),
            store.project_dir(&identity_b),
            "같은 target은 같은 스냅샷 디렉터리를 써야 함"
        );
    }

    #[test]
    fn write_read_list_remove_round_trip() {
        let (_project_dir, identity) = identity_with_name("RoundTripProject");
        let app_data = TempDir::new("stellar-recovery-appdata");
        let store = RecoverySnapshotStore::new(&app_data.path);
        let window = session(100);

        assert_eq!(store.read_snapshot(&identity, window).unwrap(), None);
        store
            .write_snapshot(&identity, window, b"snapshot-payload")
            .unwrap();
        assert_eq!(
            store.read_snapshot(&identity, window).unwrap().as_deref(),
            Some(b"snapshot-payload".as_slice())
        );
        assert_eq!(
            store.list_sessions(&identity).unwrap(),
            vec![window.to_string()]
        );

        // 덮어쓰기(atomic replace).
        store
            .write_snapshot(&identity, window, b"newer-payload")
            .unwrap();
        assert_eq!(
            store.read_snapshot(&identity, window).unwrap().as_deref(),
            Some(b"newer-payload".as_slice())
        );

        store.remove_snapshot(&identity, window).unwrap();
        assert_eq!(store.read_snapshot(&identity, window).unwrap(), None);
        assert!(store.list_sessions(&identity).unwrap().is_empty());
        // 이중 삭제는 무해.
        store.remove_snapshot(&identity, window).unwrap();
    }

    #[test]
    fn different_sessions_have_distinct_files() {
        let (_project_dir, identity) = identity_with_name("MultiWindowProject");
        let app_data = TempDir::new("stellar-recovery-appdata");
        let store = RecoverySnapshotStore::new(&app_data.path);
        store.write_snapshot(&identity, session(1), b"a").unwrap();
        store.write_snapshot(&identity, session(2), b"b").unwrap();
        assert_eq!(store.list_sessions(&identity).unwrap().len(), 2);
    }
}

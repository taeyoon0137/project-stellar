//! 앱 측 IPC test double (XIX.1.2).
//!
//! 앱(endpoint를 등록하는 쪽)은 아직 없으므로, registry 계약을 따르는 최소 등록 동작을 테스트
//! 전용으로 둡니다. 실제 앱 구현은 이 계약(0700 디렉터리, `<instance-id>.sock`)을 따라야 합니다.

use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::os;
use super::registry::{Endpoint, InstanceId, Registry};

/// socket 경로 한도를 넘지 않도록 짧은 이름을 쓰는 임시 디렉터리.
pub(crate) struct TempDir {
    pub(crate) path: PathBuf,
}

impl TempDir {
    pub(crate) fn new() -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("smcp-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // 테스트가 바꾼 권한을 되돌린 뒤 지웁니다.
        if let Ok(entries) = std::fs::read_dir(&self.path) {
            for entry in entries.flatten() {
                let _ =
                    std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o700));
            }
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// `tmp/<name>`에 앱처럼 0700 registry 디렉터리를 만듭니다.
pub(crate) fn secure_registry(tmp: &TempDir, name: &str) -> Registry {
    let dir = tmp.path.join(name);
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    Registry::at(dir, os::current_uid())
}

/// 등록된 앱 endpoint(test double).
pub(crate) struct AppDouble {
    pub(crate) endpoint: Endpoint,
    pub(crate) listener: UnixListener,
}

/// 앱처럼 `<instance-id>.sock` listener를 엽니다.
pub(crate) fn register(registry: &Registry, instance: &InstanceId) -> AppDouble {
    let socket_path = registry.socket_path(instance).unwrap();
    let listener = UnixListener::bind(&socket_path).unwrap();
    AppDouble {
        endpoint: Endpoint {
            instance: instance.clone(),
            socket_path,
        },
        listener,
    }
}

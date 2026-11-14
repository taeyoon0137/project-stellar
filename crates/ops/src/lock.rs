//! 교차 프로세스 저장 잠금 (XXIV.4.5).
//!
//! headless 쓰기(`stellar apply` 등)는 세대 전환 저장 전에 프로젝트별 배타 잠금을 잡고, 잠금을 쥔
//! 상태에서 디스크 generation을 base revision과 비교한 뒤 current pointer를 교체합니다. 그래서 두
//! headless writer가 같은 base 위에서 동시에 저장해 한쪽 변경이 사라지는 일이 없습니다.
//!
//! - **위치**: 프로젝트 폴더와 Git 밖의 OS 사용자 데이터 영역입니다.
//!   `<OS 사용자 데이터>/xyz.taeyoon.stellar/locks/<key>.lock`이며, macOS의 OS 사용자 데이터는
//!   `$HOME/Library/Application Support`입니다(`stellar_project::trust_store::os_user_data_base`).
//!   하위 폴더 이름은 앱 bundle identifier입니다(Apple File System Programming Guide: Application
//!   Support 아래 앱 폴더는 bundle identifier로 둡니다). 디렉터리는 소유자 전용(`0700`)입니다.
//!   잠금 디렉터리 자리가 symlink나 파일이면 따라가지 않고 거부합니다(`NotADirectory`).
//! - **key**: 프로젝트 식별(IV.2.3) `project id ‖ canonical target`의 SHA-256 앞 16바이트 16진수.
//!   같은 프로젝트를 symlink 경로로 열어도 같은 잠금이고, 다른 worktree는 다른 잠금입니다.
//! - **OS API**: `flock(2)`의 `LOCK_EX | LOCK_NB`(Rust std `File::try_lock`). macOS `flock(2)`
//!   매뉴얼대로 advisory 잠금이며 open file description에 붙고, 그 description의 마지막 fd가 닫히면
//!   (프로세스 비정상 종료 포함) 해제됩니다. 그래서 오래된 잠금 파일이 남아도 다음 writer를 막지
//!   않습니다. 잠금 파일 내용은 쓰지 않습니다.
//! - **기다리지 않음**: 이미 잡혀 있으면 즉시 [`LockError::Busy`]입니다. 호출자는 일시적 실패
//!   (sysexits `EX_TEMPFAIL` 75)로 끝내고, 재시도는 호출자 몫입니다.
//! - **merge driver 모드**: Git이 넘긴 임시 경로만 쓰므로 잠금 대상이 아닙니다([`LockPolicy::None`]).

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

use stellar_project::identity::ProjectIdentity;
use stellar_storage::hash::ContentHash;

use crate::core_types::ids::StableId;

/// OS 사용자 데이터 영역 아래 Stellar 폴더 이름(macOS 앱 bundle identifier).
pub const APP_DATA_DIR: &str = "xyz.taeyoon.stellar";
/// 저장 잠금 하위 폴더.
pub const LOCKS_DIR: &str = "locks";

/// 저장 잠금 정책.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockPolicy {
    /// `dir` 아래에 프로젝트별 잠금을 잡습니다(보통 [`default_lock_dir`]).
    Required {
        /// 잠금 디렉터리.
        dir: PathBuf,
    },
    /// 잠금을 쓰지 않습니다. Git merge driver 모드(임시 경로만 씀) 전용입니다.
    None,
}

/// 잠금 실패.
#[derive(Debug)]
pub enum LockError {
    /// OS 사용자 데이터 위치를 알 수 없습니다(`HOME` 없음).
    NoUserDataDir,
    /// 다른 writer가 잠금을 쥐고 있습니다.
    Busy {
        /// 잠금 파일.
        path: PathBuf,
    },
    /// 잠금 파일·디렉터리를 준비하거나 잠글 수 없습니다.
    Io {
        /// 대상 경로.
        path: PathBuf,
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockError::NoUserDataDir => {
                f.write_str("OS 사용자 데이터 위치를 알 수 없어 저장 잠금을 쓸 수 없습니다")
            }
            LockError::Busy { .. } => {
                f.write_str("다른 writer가 이 프로젝트의 저장 잠금을 쓰고 있습니다")
            }
            LockError::Io { kind, .. } => write!(f, "저장 잠금을 준비할 수 없습니다: {kind:?}"),
        }
    }
}

impl std::error::Error for LockError {}

/// 기본 잠금 디렉터리: `<OS 사용자 데이터>/xyz.taeyoon.stellar/locks`.
pub fn default_lock_dir() -> Result<PathBuf, LockError> {
    stellar_project::trust_store::os_user_data_base()
        .map(|base| base.join(APP_DATA_DIR).join(LOCKS_DIR))
        .ok_or(LockError::NoUserDataDir)
}

/// 프로젝트 식별(IV.2.3)의 잠금 key.
pub fn lock_key(project_id: StableId, root: &Path) -> Result<String, LockError> {
    let identity = project_identity(project_id, root)?;
    Ok(identity_lock_key(&identity))
}

fn project_identity(project_id: StableId, root: &Path) -> Result<ProjectIdentity, LockError> {
    ProjectIdentity::new(project_id, root).map_err(|e| LockError::Io {
        path: root.to_path_buf(),
        kind: e.kind(),
    })
}

fn identity_lock_key(identity: &ProjectIdentity) -> String {
    let mut bytes = identity.project_id().to_bytes().to_vec();
    bytes.extend_from_slice(identity.canonical_target().as_os_str().as_encoded_bytes());
    let digest = ContentHash::compute(&bytes);
    digest.digest()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 잡은 저장 잠금. drop 하면(파일 닫힘) 해제됩니다.
#[derive(Debug)]
pub struct SaveLock {
    path: PathBuf,
    _file: File,
}

impl SaveLock {
    /// 잠금 파일 경로.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `dir` 아래 이 프로젝트의 잠금을 기다리지 않고 잡습니다.
    pub fn try_acquire(
        dir: &Path,
        project_id: StableId,
        root: &Path,
    ) -> Result<SaveLock, LockError> {
        Self::for_identity(dir, &project_identity(project_id, root)?)
    }

    pub(crate) fn for_identity(
        dir: &Path,
        identity: &ProjectIdentity,
    ) -> Result<SaveLock, LockError> {
        ensure_private_dir(dir)?;
        let path = dir.join(format!("{}.lock", identity_lock_key(identity)));
        let io_error = |e: io::Error| LockError::Io {
            path: path.clone(),
            kind: e.kind(),
        };
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path).map_err(io_error)?;
        match file.try_lock() {
            Ok(()) => Ok(SaveLock { path, _file: file }),
            Err(TryLockError::WouldBlock) => Err(LockError::Busy { path }),
            Err(TryLockError::Error(e)) => Err(io_error(e)),
        }
    }
}

/// 잠금 디렉터리를 소유자 전용으로 준비합니다(더 넓으면 좁힘).
fn ensure_private_dir(dir: &Path) -> Result<(), LockError> {
    let io_error = |path: &Path, e: io::Error| LockError::Io {
        path: path.to_path_buf(),
        kind: e.kind(),
    };
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
    }
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(io_error(dir, e)),
    }
    // symlink를 따라가지 않습니다. 잠금 자리가 symlink나 파일이면 대상의 권한을 바꾸거나 그 안에
    // 잠금 파일을 만들지 않고 거부합니다.
    let meta = std::fs::symlink_metadata(dir).map_err(|e| io_error(dir, e))?;
    if !meta.file_type().is_dir() {
        return Err(LockError::Io {
            path: dir.to_path_buf(),
            kind: io::ErrorKind::NotADirectory,
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| io_error(dir, e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{project_id, scaffold_project, TempDir};

    #[test]
    fn second_holder_is_busy_until_the_first_is_dropped() {
        let dir = TempDir::new("lock");
        let root = dir.path.join("project");
        scaffold_project(&root);
        let locks = dir.path.join("locks");
        let first = SaveLock::try_acquire(&locks, project_id(), &root).unwrap();
        assert!(first.path().starts_with(&locks));
        assert!(!first.path().starts_with(&root), "잠금은 프로젝트 밖");
        assert!(matches!(
            SaveLock::try_acquire(&locks, project_id(), &root),
            Err(LockError::Busy { .. })
        ));
        drop(first);
        let again = SaveLock::try_acquire(&locks, project_id(), &root).unwrap();
        drop(again);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&locks).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_lock_dir_is_refused_without_touching_the_target() {
        // 잠금 디렉터리 자리가 symlink면 따라가지 않고 거부합니다. 대상 디렉터리의 권한을 바꾸거나
        // 그 안에 잠금 파일을 만들지 않습니다.
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("lock-symlink");
        let root = dir.path.join("project");
        scaffold_project(&root);
        let target = dir.path.join("elsewhere");
        std::fs::create_dir(&target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        let locks = dir.path.join("locks");
        std::os::unix::fs::symlink(&target, &locks).unwrap();
        assert!(matches!(
            SaveLock::try_acquire(&locks, project_id(), &root),
            Err(LockError::Io {
                kind: io::ErrorKind::NotADirectory,
                ..
            })
        ));
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "대상 권한 무변경");
        assert_eq!(
            std::fs::read_dir(&target).unwrap().count(),
            0,
            "잠금 파일 0"
        );
        // 일반 파일 자리도 거부합니다.
        let file = dir.path.join("file-locks");
        std::fs::write(&file, b"").unwrap();
        assert!(matches!(
            SaveLock::try_acquire(&file, project_id(), &root),
            Err(LockError::Io {
                kind: io::ErrorKind::NotADirectory,
                ..
            })
        ));
    }

    #[test]
    fn key_follows_project_identity() {
        let dir = TempDir::new("lock-key");
        let root = dir.path.join("project");
        scaffold_project(&root);
        let other = dir.path.join("worktree");
        scaffold_project(&other);
        let key = lock_key(project_id(), &root).unwrap();
        assert_eq!(key.len(), 32);
        // 다른 worktree(다른 canonical target)는 다른 잠금입니다.
        assert_ne!(key, lock_key(project_id(), &other).unwrap());
        #[cfg(unix)]
        {
            // symlink 경로로 열어도 같은 프로젝트는 같은 잠금입니다.
            let link = dir.path.join("link");
            std::os::unix::fs::symlink(&root, &link).unwrap();
            assert_eq!(key, lock_key(project_id(), &link).unwrap());
        }
    }
}

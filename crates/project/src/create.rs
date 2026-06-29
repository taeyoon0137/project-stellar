//! 새 프로젝트 생성 요청과 사전 검증 (IV.4.1).
//!
//! [`ProjectCreateRequest`]는 새 프로젝트의 입력(부모 디렉터리·폴더 이름·프로젝트 이름)을
//! 담습니다. [`validate_create_request`]는 **사전 검증만** 수행하며 파일을 만들지 않습니다
//! (실제 생성은 IV.4.2 이후). 폴더 이름 규칙은 IV.2.1을 재사용합니다.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::folder_name::{validate_folder_name, FolderNameError};

/// 새 프로젝트 생성 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectCreateRequest {
    /// 부모 디렉터리(여기 아래에 새 폴더 생성).
    pub parent_dir: PathBuf,
    /// 새 폴더 이름.
    pub folder_name: String,
    /// 사람이 읽는 프로젝트 이름.
    pub project_name: String,
}

/// 사전 검증 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateValidationError {
    /// 프로젝트 이름이 비어 있음.
    EmptyProjectName,
    /// 폴더 이름 규칙 위반(IV.2.1).
    InvalidFolderName(FolderNameError),
    /// 부모가 디렉터리가 아님(없거나 파일).
    ParentNotADirectory,
    /// 대상 경로(부모/폴더)가 이미 존재함.
    TargetExists,
    /// 부모 디렉터리에 쓰기 권한이 없음.
    ParentNotWritable,
}

impl std::fmt::Display for CreateValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreateValidationError::EmptyProjectName => f.write_str("프로젝트 이름이 비어 있습니다"),
            CreateValidationError::InvalidFolderName(e) => write!(f, "폴더 이름 오류: {e}"),
            CreateValidationError::ParentNotADirectory => {
                f.write_str("부모 경로가 디렉터리가 아닙니다")
            }
            CreateValidationError::TargetExists => f.write_str("대상 경로가 이미 존재합니다"),
            CreateValidationError::ParentNotWritable => {
                f.write_str("부모 디렉터리에 쓰기 권한이 없습니다")
            }
        }
    }
}

impl std::error::Error for CreateValidationError {}

impl ProjectCreateRequest {
    /// 생성될 대상 경로(부모/폴더).
    pub fn target_path(&self) -> PathBuf {
        self.parent_dir.join(&self.folder_name)
    }
}

/// 생성 요청을 사전 검증합니다(파일을 만들지 않음; 쓰기 권한은 임시 probe로만 확인).
pub fn validate_create_request(
    request: &ProjectCreateRequest,
) -> Result<(), CreateValidationError> {
    if request.project_name.trim().is_empty() {
        return Err(CreateValidationError::EmptyProjectName);
    }
    validate_folder_name(&request.folder_name).map_err(CreateValidationError::InvalidFolderName)?;
    if !request.parent_dir.is_dir() {
        return Err(CreateValidationError::ParentNotADirectory);
    }
    if request.target_path().exists() {
        return Err(CreateValidationError::TargetExists);
    }
    if !is_writable(&request.parent_dir) {
        return Err(CreateValidationError::ParentNotWritable);
    }
    Ok(())
}

/// 디렉터리 쓰기 권한을 임시 probe 파일로 확인합니다(즉시 삭제).
fn is_writable(dir: &Path) -> bool {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let probe = dir.join(format!(".stellar-write-probe-{}-{n}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
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
            let path = std::env::temp_dir()
                .join(format!("stellar-create-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            // 권한 테스트가 0555로 바꿨을 수 있으니 정리 전에 복원.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o755));
            }
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn request(parent: &Path, folder: &str) -> ProjectCreateRequest {
        ProjectCreateRequest {
            parent_dir: parent.to_path_buf(),
            folder_name: folder.to_string(),
            project_name: "My Project".to_string(),
        }
    }

    #[test]
    fn valid_request_passes() {
        let dir = TempDir::new();
        assert_eq!(
            validate_create_request(&request(&dir.path, "my-project")),
            Ok(())
        );
    }

    #[test]
    fn existing_target_is_rejected() {
        let dir = TempDir::new();
        std::fs::create_dir_all(dir.path.join("taken")).unwrap();
        assert_eq!(
            validate_create_request(&request(&dir.path, "taken")),
            Err(CreateValidationError::TargetExists)
        );
    }

    #[test]
    fn forbidden_folder_name_is_rejected() {
        let dir = TempDir::new();
        // 예약 장치 이름.
        assert!(matches!(
            validate_create_request(&request(&dir.path, "CON")),
            Err(CreateValidationError::InvalidFolderName(_))
        ));
        // 예약 문자.
        assert!(matches!(
            validate_create_request(&request(&dir.path, "a/b")),
            Err(CreateValidationError::InvalidFolderName(_))
        ));
    }

    #[test]
    fn parent_not_a_directory_is_rejected() {
        let dir = TempDir::new();
        // 존재하지 않는 부모.
        let missing = dir.path.join("nope");
        assert_eq!(
            validate_create_request(&request(&missing, "x")),
            Err(CreateValidationError::ParentNotADirectory)
        );
        // 파일을 부모로 지정.
        let file = dir.path.join("afile");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(
            validate_create_request(&request(&file, "x")),
            Err(CreateValidationError::ParentNotADirectory)
        );
    }

    #[test]
    fn empty_project_name_is_rejected() {
        let dir = TempDir::new();
        let mut req = request(&dir.path, "ok-folder");
        req.project_name = "   ".to_string();
        assert_eq!(
            validate_create_request(&req),
            Err(CreateValidationError::EmptyProjectName)
        );
    }

    #[cfg(unix)]
    #[test]
    fn unwritable_parent_is_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        // 부모를 읽기 전용(0555)으로.
        std::fs::set_permissions(&dir.path, std::fs::Permissions::from_mode(0o555)).unwrap();

        // root로 실행하면 0555도 우회되므로, 실제로 쓰기가 막혔는지 직접 확인.
        let probe = dir.path.join(".perm-probe");
        let write_blocked = std::fs::File::create(&probe).is_err();

        let result = validate_create_request(&request(&dir.path, "child"));

        // 권한 복원(드롭 정리용) + probe 정리.
        std::fs::set_permissions(&dir.path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_file(&probe);

        if write_blocked {
            assert_eq!(result, Err(CreateValidationError::ParentNotWritable));
        }
        // 쓰기가 막히지 않은 환경(root 등)에서는 권한 검증을 건너뜁니다.
    }
}

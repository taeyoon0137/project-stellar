//! 터미널 작업 디렉터리 선택 (XVIII.1.1).
//!
//! 세션의 cwd는 프로젝트 루트, 사용자가 고른 worktree root, 사용자가 명시 입력한 임의 경로 중
//! 하나입니다(기술 아키텍처 32 "프로젝트 루트 또는 특정 worktree에 귀속").

use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};

/// cwd 경로 길이 상한(byte, macOS `PATH_MAX`).
pub const MAX_CWD_BYTES: usize = 1024;

/// cwd 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwdError {
    /// 입력이 비었습니다.
    Empty,
    /// 절대 경로가 아닙니다.
    NotAbsolute {
        /// 입력 경로.
        path: PathBuf,
    },
    /// NUL byte가 있습니다.
    ContainsNul,
    /// [`MAX_CWD_BYTES`]를 넘습니다.
    TooLong {
        /// 길이(byte).
        len: usize,
    },
}

impl fmt::Display for CwdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CwdError::Empty => f.write_str("작업 디렉터리가 비었습니다"),
            CwdError::NotAbsolute { path } => {
                write!(
                    f,
                    "작업 디렉터리는 절대 경로여야 합니다: {}",
                    path.display()
                )
            }
            CwdError::ContainsNul => f.write_str("작업 디렉터리 경로에 NUL 문자가 있습니다"),
            CwdError::TooLong { len } => write!(
                f,
                "작업 디렉터리 경로가 너무 깁니다({len} byte, 상한 {MAX_CWD_BYTES})"
            ),
        }
    }
}

impl std::error::Error for CwdError {}

/// 경로 문법 검사: 비어 있지 않음, NUL 없음, 길이 상한, 절대 경로.
pub(crate) fn check_path_syntax(path: &Path) -> Result<(), CwdError> {
    let bytes = path.as_os_str().as_encoded_bytes();
    if bytes.is_empty() {
        return Err(CwdError::Empty);
    }
    if bytes.contains(&0) {
        return Err(CwdError::ContainsNul);
    }
    if bytes.len() > MAX_CWD_BYTES {
        return Err(CwdError::TooLong { len: bytes.len() });
    }
    if !path.is_absolute() {
        return Err(CwdError::NotAbsolute {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

/// 사용자가 명시 입력한 cwd. [`ExplicitCwd::from_user_input`]으로만 만듭니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitCwd(PathBuf);

impl ExplicitCwd {
    /// 사용자 입력을 문법 검사합니다(존재·디렉터리 검사는 실행 시점에 합니다).
    pub fn from_user_input(input: impl AsRef<OsStr>) -> Result<ExplicitCwd, CwdError> {
        let path = PathBuf::from(input.as_ref());
        check_path_syntax(&path)?;
        Ok(ExplicitCwd(path))
    }

    /// 입력 경로.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// 세션 cwd 선택.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwdSelection {
    /// 프로젝트 루트(기본).
    ProjectRoot,
    /// 사용자가 고른 worktree root.
    WorktreeRoot(PathBuf),
    /// 사용자가 명시 입력한 임의 경로.
    Explicit(ExplicitCwd),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_cwd_syntax_is_checked() {
        assert_eq!(
            ExplicitCwd::from_user_input("/tmp").unwrap().path(),
            Path::new("/tmp")
        );
        assert_eq!(ExplicitCwd::from_user_input(""), Err(CwdError::Empty));
        assert!(matches!(
            ExplicitCwd::from_user_input("relative/dir"),
            Err(CwdError::NotAbsolute { .. })
        ));
        assert_eq!(
            ExplicitCwd::from_user_input("/tmp/\0x"),
            Err(CwdError::ContainsNul)
        );
        let long = format!("/{}", "a".repeat(MAX_CWD_BYTES));
        assert_eq!(
            ExplicitCwd::from_user_input(&long),
            Err(CwdError::TooLong {
                len: MAX_CWD_BYTES + 1
            })
        );
    }
}

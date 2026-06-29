//! Windows Git 설치 어댑터 (IV.3.7).
//!
//! 사용자가 **승인한 경우에만** `winget install --id Git.Git -e --source winget`을 **argv 방식**
//! (shell 없음)으로 실행합니다. WinGet이 없으면 설치를 시도하지 않고 **수동 설치 안내 상태**
//! (`WingetMissing`)를 반환합니다. 무승인 설치는 하지 않습니다.
//!
//! 프로세스 생성을 [`WingetInstaller`] trait로 주입해 승인·winget 가용성 게이트를 플랫폼
//! 독립적으로 테스트합니다(실제 설치를 트리거하지 않음).

/// 설치 승인 여부.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallApproval {
    /// 사용자가 설치를 승인함.
    Approved,
    /// 승인하지 않음(또는 거부).
    NotApproved,
}

/// 설치 요청 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsInstallOutcome {
    /// 설치 명령을 시작함(승인 + winget 존재).
    Started,
    /// 승인하지 않아 아무 것도 하지 않음.
    NotApproved,
    /// 승인했으나 WinGet이 없어 수동 설치 안내(설치 시도 없음).
    WingetMissing,
}

/// 설치 실패.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallError {
    /// `winget` 프로세스 생성 실패.
    Spawn {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Spawn { kind } => write!(f, "winget 실행 실패: {kind:?}"),
        }
    }
}

impl std::error::Error for InstallError {}

/// WinGet으로 Git을 설치하는 추상화(주입 가능).
pub trait WingetInstaller {
    /// WinGet이 사용 가능한지 확인합니다.
    fn is_winget_available(&self) -> bool;
    /// `winget install ... Git.Git ...`을 시작합니다. **승인 + winget 존재**일 때만 호출됩니다.
    fn start_git_install(&mut self) -> Result<(), InstallError>;
}

/// 승인 + WinGet 존재일 때만 Git 설치를 시작합니다.
///
/// - `NotApproved`이면 winget을 확인하지도, 설치하지도 않습니다(프로세스 생성 0).
/// - 승인했으나 WinGet이 없으면 설치를 시도하지 않고 `WingetMissing`(수동 안내)을 반환합니다.
/// - 승인 + WinGet 존재이면 `start_git_install`을 호출합니다.
pub fn request_install(
    approval: InstallApproval,
    installer: &mut impl WingetInstaller,
) -> Result<WindowsInstallOutcome, InstallError> {
    match approval {
        InstallApproval::NotApproved => Ok(WindowsInstallOutcome::NotApproved),
        InstallApproval::Approved => {
            if !installer.is_winget_available() {
                return Ok(WindowsInstallOutcome::WingetMissing);
            }
            installer.start_git_install()?;
            Ok(WindowsInstallOutcome::Started)
        }
    }
}

/// 실제 WinGet 실행기(Windows 전용). argv 방식, shell·권한 상승 없음.
#[cfg(windows)]
#[derive(Debug, Default)]
pub struct RealWingetInstaller;

#[cfg(windows)]
impl WingetInstaller for RealWingetInstaller {
    fn is_winget_available(&self) -> bool {
        std::process::Command::new("winget")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn start_git_install(&mut self) -> Result<(), InstallError> {
        std::process::Command::new("winget")
            .args(["install", "--id", "Git.Git", "-e", "--source", "winget"])
            .status()
            .map(|_status| ())
            .map_err(|e| InstallError::Spawn { kind: e.kind() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockWinget {
        available: bool,
        install_calls: usize,
        fail: bool,
    }

    impl WingetInstaller for MockWinget {
        fn is_winget_available(&self) -> bool {
            self.available
        }
        fn start_git_install(&mut self) -> Result<(), InstallError> {
            self.install_calls += 1;
            if self.fail {
                Err(InstallError::Spawn {
                    kind: std::io::ErrorKind::PermissionDenied,
                })
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn no_approval_does_nothing() {
        let mut mock = MockWinget {
            available: true,
            install_calls: 0,
            fail: false,
        };
        let outcome = request_install(InstallApproval::NotApproved, &mut mock).unwrap();
        assert_eq!(outcome, WindowsInstallOutcome::NotApproved);
        assert_eq!(mock.install_calls, 0, "승인 없이 설치가 시작됨(보안 결함)");
    }

    #[test]
    fn approval_with_winget_starts_install() {
        let mut mock = MockWinget {
            available: true,
            install_calls: 0,
            fail: false,
        };
        let outcome = request_install(InstallApproval::Approved, &mut mock).unwrap();
        assert_eq!(outcome, WindowsInstallOutcome::Started);
        assert_eq!(mock.install_calls, 1);
    }

    #[test]
    fn approval_without_winget_returns_manual_guidance() {
        let mut mock = MockWinget {
            available: false,
            install_calls: 0,
            fail: false,
        };
        let outcome = request_install(InstallApproval::Approved, &mut mock).unwrap();
        assert_eq!(outcome, WindowsInstallOutcome::WingetMissing);
        // winget 없으면 설치를 시도하지 않음.
        assert_eq!(mock.install_calls, 0);
    }

    #[test]
    fn approval_propagates_install_spawn_error() {
        let mut mock = MockWinget {
            available: true,
            install_calls: 0,
            fail: true,
        };
        let result = request_install(InstallApproval::Approved, &mut mock);
        assert!(matches!(result, Err(InstallError::Spawn { .. })));
        assert_eq!(mock.install_calls, 1);
    }
}

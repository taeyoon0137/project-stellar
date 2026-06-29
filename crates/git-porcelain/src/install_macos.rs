//! macOS Git 설치 어댑터 (IV.3.6).
//!
//! 사용자가 **명시적으로 승인한 경우에만** `xcode-select --install`을 시작합니다. 승인하지
//! 않으면 어떤 프로세스도 생성하지 않습니다(spawn 0). **자동 관리자 권한 상승(sudo 등)과 무승인
//! 설치는 금지**합니다.
//!
//! 승인 게이팅과 결과는 플랫폼 독립적으로 테스트할 수 있도록 프로세스 생성을
//! [`XcodeSelectInstaller`] trait로 주입합니다(테스트에서 실제 설치를 트리거하지 않음).

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
pub enum InstallOutcome {
    /// 설치 명령을 시작함(승인됨).
    Started,
    /// 승인하지 않아 아무 것도 하지 않음.
    NotApproved,
}

/// 설치 실패.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallError {
    /// `xcode-select` 프로세스 생성 실패.
    Spawn {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Spawn { kind } => write!(f, "xcode-select 실행 실패: {kind:?}"),
        }
    }
}

impl std::error::Error for InstallError {}

/// `xcode-select --install`을 시작하는 추상화(주입 가능).
pub trait XcodeSelectInstaller {
    /// 설치를 시작합니다. **승인된 경우에만** 호출됩니다.
    fn start_install(&mut self) -> Result<(), InstallError>;
}

/// 승인된 경우에만 설치를 시작합니다.
///
/// `NotApproved`이면 `installer`를 **호출하지 않고**(프로세스 생성 0) `NotApproved`를 반환합니다.
/// `Approved`이면 `installer.start_install()`을 호출합니다.
pub fn request_install(
    approval: InstallApproval,
    installer: &mut impl XcodeSelectInstaller,
) -> Result<InstallOutcome, InstallError> {
    match approval {
        InstallApproval::NotApproved => Ok(InstallOutcome::NotApproved),
        InstallApproval::Approved => {
            installer.start_install()?;
            Ok(InstallOutcome::Started)
        }
    }
}

/// 실제 `xcode-select --install` 실행기(macOS 전용).
///
/// argv 방식(shell 없음)으로 실행하며 **sudo·권한 상승을 사용하지 않습니다**.
/// `xcode-select --install`은 OS 설치 UI를 트리거하므로 관리자 권한이 필요하지 않습니다.
#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
pub struct RealXcodeSelectInstaller;

#[cfg(target_os = "macos")]
impl XcodeSelectInstaller for RealXcodeSelectInstaller {
    fn start_install(&mut self) -> Result<(), InstallError> {
        // shell·sudo 없이 argv로 실행. xcode-select --install은 즉시 반환(설치 UI는 비동기).
        // 이미 설치된 경우 비영 종료할 수 있으나, 요청 자체는 io 오류만 실패로 봅니다.
        std::process::Command::new("xcode-select")
            .arg("--install")
            .status()
            .map(|_status| ())
            .map_err(|e| InstallError::Spawn { kind: e.kind() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 테스트용 mock: start_install 호출 횟수를 기록(실제 프로세스 생성 없음).
    struct MockInstaller {
        calls: usize,
        fail: bool,
    }

    impl XcodeSelectInstaller for MockInstaller {
        fn start_install(&mut self) -> Result<(), InstallError> {
            self.calls += 1;
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
    fn no_approval_spawns_nothing() {
        let mut mock = MockInstaller {
            calls: 0,
            fail: false,
        };
        let outcome = request_install(InstallApproval::NotApproved, &mut mock).unwrap();
        assert_eq!(outcome, InstallOutcome::NotApproved);
        // 핵심: 승인 없으면 프로세스 생성 0.
        assert_eq!(mock.calls, 0, "승인 없이 설치가 시작됨(보안 결함)");
    }

    #[test]
    fn approval_starts_install() {
        let mut mock = MockInstaller {
            calls: 0,
            fail: false,
        };
        let outcome = request_install(InstallApproval::Approved, &mut mock).unwrap();
        assert_eq!(outcome, InstallOutcome::Started);
        assert_eq!(mock.calls, 1, "승인 시 설치 요청이 호출되어야 함");
    }

    #[test]
    fn approval_propagates_spawn_error() {
        let mut mock = MockInstaller {
            calls: 0,
            fail: true,
        };
        let result = request_install(InstallApproval::Approved, &mut mock);
        assert!(matches!(result, Err(InstallError::Spawn { .. })));
        assert_eq!(mock.calls, 1);
    }
}

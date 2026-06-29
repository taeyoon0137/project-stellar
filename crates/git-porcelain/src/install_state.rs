//! Git 설치 상태 머신 (IV.3.5).
//!
//! [`GitInstallationState`]는 Git 설치·가용성 상태를 표현하고, [`GitInstallEvent`] 기반으로
//! 전이합니다. probe(발견/미발견)는 어느 상태에서나 현실을 반영하고, 설치 요청·완료는 정해진
//! 상태에서만 허용됩니다.

use crate::version::GitVersion;

/// Git 설치·가용성 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitInstallationState {
    /// 사용 가능(probe로 지원 버전 확인).
    Available {
        /// 확인된 버전.
        version: GitVersion,
    },
    /// Git 없음.
    Missing,
    /// 발견했으나 지원하지 않는 버전.
    UnsupportedVersion {
        /// 확인된(미지원) 버전.
        version: GitVersion,
    },
    /// 설치 진행 중.
    InstallInProgress,
    /// 설치 후 재탐지/재probe 필요.
    ReprobeRequired,
}

/// 상태 전이 이벤트.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitInstallEvent {
    /// probe로 Git을 찾음(`supported`로 지원 여부 표시).
    ProbeFound {
        /// 확인된 버전.
        version: GitVersion,
        /// 지원 버전 여부(최소 버전 정책은 호출자가 결정).
        supported: bool,
    },
    /// probe로 Git을 찾지 못함.
    ProbeMissing,
    /// 사용자 승인 설치 요청.
    RequestInstall,
    /// 설치 프로세스 종료.
    InstallCompleted,
}

/// 허용되지 않는 상태 전이.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionError {
    /// 현재 상태에서 설치를 시작할 수 없음.
    CannotInstallFrom,
    /// 설치 진행 중이 아닌데 설치 완료 이벤트가 옴.
    NoInstallInProgress,
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransitionError::CannotInstallFrom => {
                f.write_str("현재 상태에서는 설치를 시작할 수 없습니다")
            }
            TransitionError::NoInstallInProgress => f.write_str("설치 진행 중이 아닙니다"),
        }
    }
}

impl std::error::Error for TransitionError {}

impl GitInstallationState {
    /// 이벤트를 적용해 다음 상태를 반환합니다. 허용되지 않는 전이는 `Err`이며 상태는 보존됩니다.
    ///
    /// - probe(발견/미발견): 어느 상태에서나 현실을 반영합니다.
    /// - `RequestInstall`: `Missing`·`UnsupportedVersion`에서만 `InstallInProgress`로.
    /// - `InstallCompleted`: `InstallInProgress`에서만 `ReprobeRequired`로.
    pub fn apply(&self, event: GitInstallEvent) -> Result<GitInstallationState, TransitionError> {
        match event {
            GitInstallEvent::ProbeFound { version, supported } => Ok(if supported {
                GitInstallationState::Available { version }
            } else {
                GitInstallationState::UnsupportedVersion { version }
            }),
            GitInstallEvent::ProbeMissing => Ok(GitInstallationState::Missing),
            GitInstallEvent::RequestInstall => match self {
                GitInstallationState::Missing | GitInstallationState::UnsupportedVersion { .. } => {
                    Ok(GitInstallationState::InstallInProgress)
                }
                _ => Err(TransitionError::CannotInstallFrom),
            },
            GitInstallEvent::InstallCompleted => match self {
                GitInstallationState::InstallInProgress => {
                    Ok(GitInstallationState::ReprobeRequired)
                }
                _ => Err(TransitionError::NoInstallInProgress),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(major: u32, minor: u32, patch: u32) -> GitVersion {
        GitVersion {
            major,
            minor,
            patch,
            raw: format!("git version {major}.{minor}.{patch}"),
        }
    }

    #[test]
    fn full_install_happy_path() {
        let v = version(2, 39, 3);
        // Missing → 설치 요청 → 진행 중 → 완료 → 재probe 필요 → probe 성공 → Available.
        let s = GitInstallationState::Missing;
        let s = s.apply(GitInstallEvent::RequestInstall).unwrap();
        assert_eq!(s, GitInstallationState::InstallInProgress);
        let s = s.apply(GitInstallEvent::InstallCompleted).unwrap();
        assert_eq!(s, GitInstallationState::ReprobeRequired);
        let s = s
            .apply(GitInstallEvent::ProbeFound {
                version: v.clone(),
                supported: true,
            })
            .unwrap();
        assert_eq!(s, GitInstallationState::Available { version: v });
    }

    #[test]
    fn probe_reflects_reality_from_any_state() {
        let v = version(2, 40, 0);
        // probe found(supported) → Available.
        assert_eq!(
            GitInstallationState::Missing
                .apply(GitInstallEvent::ProbeFound {
                    version: v.clone(),
                    supported: true,
                })
                .unwrap(),
            GitInstallationState::Available { version: v.clone() }
        );
        // probe found(unsupported) → UnsupportedVersion.
        assert_eq!(
            GitInstallationState::Available { version: v.clone() }
                .apply(GitInstallEvent::ProbeFound {
                    version: v.clone(),
                    supported: false,
                })
                .unwrap(),
            GitInstallationState::UnsupportedVersion { version: v }
        );
        // probe missing → Missing(설치 진행 중에도 현실 반영).
        assert_eq!(
            GitInstallationState::InstallInProgress
                .apply(GitInstallEvent::ProbeMissing)
                .unwrap(),
            GitInstallationState::Missing
        );
        assert_eq!(
            GitInstallationState::ReprobeRequired
                .apply(GitInstallEvent::ProbeMissing)
                .unwrap(),
            GitInstallationState::Missing
        );
    }

    #[test]
    fn unsupported_version_can_request_install() {
        let s = GitInstallationState::UnsupportedVersion {
            version: version(1, 8, 0),
        };
        assert_eq!(
            s.apply(GitInstallEvent::RequestInstall).unwrap(),
            GitInstallationState::InstallInProgress
        );
    }

    #[test]
    fn install_not_allowed_from_available_or_in_progress() {
        let available = GitInstallationState::Available {
            version: version(2, 39, 3),
        };
        assert_eq!(
            available.apply(GitInstallEvent::RequestInstall),
            Err(TransitionError::CannotInstallFrom)
        );
        assert_eq!(
            GitInstallationState::InstallInProgress.apply(GitInstallEvent::RequestInstall),
            Err(TransitionError::CannotInstallFrom)
        );
    }

    #[test]
    fn install_completed_only_from_in_progress() {
        assert_eq!(
            GitInstallationState::Missing.apply(GitInstallEvent::InstallCompleted),
            Err(TransitionError::NoInstallInProgress)
        );
        assert_eq!(
            GitInstallationState::ReprobeRequired.apply(GitInstallEvent::InstallCompleted),
            Err(TransitionError::NoInstallInProgress)
        );
        assert_eq!(
            GitInstallationState::InstallInProgress
                .apply(GitInstallEvent::InstallCompleted)
                .unwrap(),
            GitInstallationState::ReprobeRequired
        );
    }
}

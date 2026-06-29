//! 설치 후 재탐지·재probe (IV.3.8).
//!
//! 설치 프로세스 종료 후([`GitInstallationState::ReprobeRequired`]) Git executable을 **다시
//! 탐지**(IV.3.1)하고 **버전을 재probe**(IV.3.2)해 상태를 전이합니다. 재탐지·재probe는 모두
//! in-process로 수행하므로 **앱 재시작을 요구하지 않습니다**.

use crate::discovery::{discover_git, GitDiscoveryConfig};
use crate::install_state::{GitInstallEvent, GitInstallationState, TransitionError};
use crate::version::{probe_git_version, GitVersion};

/// probe 결과(주입)를 받아 상태를 전이합니다.
///
/// `probe`가 `Some(version)`이면 `is_supported`로 지원 여부를 판정해 Available/UnsupportedVersion
/// 으로, `None`이면 Missing으로 전이합니다. 전이는 [`GitInstallationState::apply`]를 따릅니다.
pub fn reprobe_into_state(
    state: &GitInstallationState,
    probe: impl FnOnce() -> Option<GitVersion>,
    is_supported: impl FnOnce(&GitVersion) -> bool,
) -> Result<GitInstallationState, TransitionError> {
    let event = match probe() {
        Some(version) => {
            let supported = is_supported(&version);
            GitInstallEvent::ProbeFound { version, supported }
        }
        None => GitInstallEvent::ProbeMissing,
    };
    state.apply(event)
}

/// 실제 파일시스템으로 재탐지·재probe 합니다(앱 재시작 불요).
///
/// Git을 탐지·probe해서 성공하면 `is_supported`로 지원 여부를 판정해 상태를 전이합니다.
pub fn reprobe_with_filesystem(
    state: &GitInstallationState,
    config: &GitDiscoveryConfig,
    is_supported: impl FnOnce(&GitVersion) -> bool,
) -> Result<GitInstallationState, TransitionError> {
    reprobe_into_state(
        state,
        || discover_git(config).and_then(|path| probe_git_version(&path).ok()),
        is_supported,
    )
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
    fn mock_install_then_reprobe_becomes_available() {
        // 전체 흐름: Missing → 설치 요청 → 진행 → 완료 → 재probe 필요 → (mock install 후 재탐지) → Available.
        let v = version(2, 39, 3);
        let state = GitInstallationState::Missing;
        let state = state.apply(GitInstallEvent::RequestInstall).unwrap();
        let state = state.apply(GitInstallEvent::InstallCompleted).unwrap();
        assert_eq!(state, GitInstallationState::ReprobeRequired);

        // mock install이 끝나 git이 발견된다고 가정(앱 재시작 없이 in-process 재probe).
        let after = reprobe_into_state(&state, || Some(v.clone()), |_| true).unwrap();
        assert_eq!(after, GitInstallationState::Available { version: v });
    }

    #[test]
    fn reprobe_missing_stays_missing() {
        let state = GitInstallationState::ReprobeRequired;
        let after = reprobe_into_state(&state, || None, |_| true).unwrap();
        assert_eq!(after, GitInstallationState::Missing);
    }

    #[test]
    fn reprobe_unsupported_becomes_unsupported_version() {
        let v = version(1, 8, 0);
        let state = GitInstallationState::ReprobeRequired;
        // is_supported가 false → UnsupportedVersion.
        let after = reprobe_into_state(&state, || Some(v.clone()), |ver| ver.major >= 2).unwrap();
        assert_eq!(
            after,
            GitInstallationState::UnsupportedVersion { version: v }
        );
    }

    #[test]
    fn filesystem_reprobe_finds_installed_git() {
        // DEV-MAC/CI에 git이 있으면 실제 재탐지·재probe가 Available로 전이.
        let state = GitInstallationState::ReprobeRequired;
        let after =
            reprobe_with_filesystem(&state, &GitDiscoveryConfig::default(), |_| true).unwrap();
        match after {
            GitInstallationState::Available { version } => {
                assert!(version.major >= 1);
            }
            GitInstallationState::Missing => {
                // git 미설치 환경에서는 Missing(테스트 환경 의존).
            }
            other => panic!("예상과 다른 상태: {other:?}"),
        }
    }
}

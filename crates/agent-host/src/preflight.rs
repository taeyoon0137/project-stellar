//! 세션 실행 전 확인 (XXIX.2.7, ADR-0005 §4.3, ADR-0092 결정 3).
//!
//! 순서(앞 단계가 실패하면 뒤 단계와 실행을 하지 않음, fail closed):
//!
//! 1. launch profile이 활성인지(약관 검토 `allowed` + `enabled`).
//! 2. 실행 대상 신원(XXIX.2.3 [`crate::target::verify_target`]). 이 단계까지 CLI를 실행하지 않습니다.
//! 3. CLI 버전이 profile 지원 범위 안인지(`version_command`, 판정 불가면 실패).
//! 4. 연결 상태가 [`ConnectionState::Connected`]인지(XXIX.2.5).
//!
//! 통과하면 [`PreflightPass`]를 돌려줍니다. 세션 실행([`PreflightPass::launch`])은 이 값으로만 할 수 있고
//! 이 값은 이 모듈 밖에서 만들 수 없습니다.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::env::ChildEnv;
use crate::process::{
    run_capture, ExitKind, LaunchError, McpServerLaunch, OutputSink, ScratchDir, SessionLaunch,
    SessionProcess,
};
use crate::profile::{CliVersion, LaunchProfile};
use crate::status::{probe_connection, ConnectionState};
use crate::target::{verify_target, CodeVerifier, ConfirmedTarget, TargetError, VerifiedTarget};

/// 버전 출력 보관 상한.
const VERSION_OUTPUT_MAX: usize = 4096;

/// 실행 전 확인 거부 이유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightRefusal {
    /// 활성 목록에 없는 profile.
    ProfileInactive,
    /// 실행 대상 신원 확인 실패.
    Target(TargetError),
    /// 버전을 판정할 수 없음.
    VersionUndeterminable,
    /// 지원 범위 밖 버전.
    VersionOutOfRange(CliVersion),
    /// 연결 상태가 사용 가능하지 않음.
    Connection(ConnectionState),
    /// scratch를 만들 수 없음.
    Scratch,
}

impl PreflightRefusal {
    /// 고정 오류 코드(진단용, 내용 없음).
    pub fn code(&self) -> &'static str {
        match self {
            PreflightRefusal::ProfileInactive => "profile-inactive",
            PreflightRefusal::Target(_) => "target-identity",
            PreflightRefusal::VersionUndeterminable => "version-undeterminable",
            PreflightRefusal::VersionOutOfRange(_) => "version-out-of-range",
            PreflightRefusal::Connection(_) => "not-connected",
            PreflightRefusal::Scratch => "scratch",
        }
    }
}

/// 실행 전 확인 입력.
pub struct PreflightInput<'a> {
    /// launch profile.
    pub profile: &'a LaunchProfile,
    /// 사용자 확인 기록.
    pub target: &'a ConfirmedTarget,
    /// 코드 서명 확인 수단.
    pub verifier: &'a dyn CodeVerifier,
    /// 앱 프로세스 환경(허용 목록으로 거름).
    pub app_env: Vec<(OsString, OsString)>,
    /// scratch를 만들 곳.
    pub scratch_base: PathBuf,
}

/// 실행 전 확인 통과 증표.
#[derive(Debug)]
pub struct PreflightPass {
    profile: LaunchProfile,
    target: VerifiedTarget,
    version: CliVersion,
    env: ChildEnv,
    scratch_base: PathBuf,
}

impl PreflightPass {
    /// 확인한 CLI 버전.
    pub fn version(&self) -> CliVersion {
        self.version
    }

    /// profile.
    pub fn profile(&self) -> &LaunchProfile {
        &self.profile
    }

    /// child 환경.
    pub fn env(&self) -> &ChildEnv {
        &self.env
    }

    /// 세션 프로세스를 시작합니다. 사용자가 세션 시작을 명시적으로 확인한 동작에서만 부릅니다.
    pub fn launch(
        &self,
        first_message: String,
        mcp: Option<McpServerLaunch>,
        sink: Arc<dyn OutputSink>,
    ) -> Result<SessionProcess, LaunchError> {
        SessionProcess::spawn(SessionLaunch {
            target: &self.target,
            profile: &self.profile,
            env: self.env.clone(),
            scratch_base: self.scratch_base.clone(),
            mcp,
            first_message,
            sink,
        })
    }
}

fn probe_version(
    target: &VerifiedTarget,
    profile: &LaunchProfile,
    env: &ChildEnv,
    base: &Path,
) -> Result<CliVersion, PreflightRefusal> {
    let scratch = ScratchDir::create(base).map_err(|_| PreflightRefusal::Scratch)?;
    let outcome = run_capture(
        target.path(),
        &profile.version_command,
        env,
        &scratch.work_dir(),
        Duration::from_millis(profile.timeouts.version_ms),
        VERSION_OUTPUT_MAX,
    )
    .map_err(|_| PreflightRefusal::VersionUndeterminable)?;
    if outcome.exit != Some(ExitKind::Code(0)) {
        return Err(PreflightRefusal::VersionUndeterminable);
    }
    let text = String::from_utf8_lossy(&outcome.stdout);
    CliVersion::find_in_output(&text).ok_or(PreflightRefusal::VersionUndeterminable)
}

/// 실행 전 확인을 합니다. 프로세스를 실행하므로 UI·Document 스레드 밖에서 부릅니다.
pub fn preflight(input: PreflightInput<'_>) -> Result<PreflightPass, PreflightRefusal> {
    let profile = input.profile;
    if !profile.is_active() {
        return Err(PreflightRefusal::ProfileInactive);
    }
    let target =
        verify_target(input.target, profile, input.verifier).map_err(PreflightRefusal::Target)?;
    let env = ChildEnv::allowlisted(&profile.env, input.app_env);
    let version = probe_version(&target, profile, &env, &input.scratch_base)?;
    if !profile.versions.contains(version) {
        return Err(PreflightRefusal::VersionOutOfRange(version));
    }
    let state = probe_connection(&target, profile, &env, &input.scratch_base);
    if !state.is_usable() {
        return Err(PreflightRefusal::Connection(state));
    }
    Ok(PreflightPass {
        profile: profile.clone(),
        target,
        version,
        env,
        scratch_base: input.scratch_base,
    })
}

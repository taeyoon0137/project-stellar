//! 프로젝트 창별 터미널 세션 registry (XVIII.1.2, XVIII.1.4, XVIII.2.4).
//!
//! 프로젝트 창 하나([`WindowSessionId`])가 registry 하나를 가지고, 그 안에 여러 터미널 세션을
//! 둡니다(기술 아키텍처 32 "프로젝트 창별 다중 세션"). registry는 세션 메모리에만 있으며 창을 닫으면
//! 함께 사라집니다.
//!
//! - 세션 수는 [`RegistryLimits::max_sessions`]로 제한합니다.
//! - 식별자는 registry의 [`SessionIdAllocator`]가 발급하고 재사용하지 않습니다. 닫은 세션의
//!   식별자·handle은 다시 해석되지 않습니다.
//! - 세션은 만든 순서(발급 순번)로 나열됩니다.
//! - **시작 gate**([`TerminalRegistry::start`]): 신뢰된 프로젝트 창의 [`TerminalCapability`]를
//!   가장 먼저 확인합니다. token이 없거나 다른 창의 것이면 cwd·명령·환경을 보지도 않고, 실행기를
//!   호출하지 않습니다(untrusted spawn 0). 그다음 cwd 검증(XVIII.1.3), 셸 해석(XVIII.2.2)·명령·환경
//!   변경 구성(상속 환경 + 식별 변수·`SHELL`·profile 명시 지정, XVIII.2.5·ADR-0023), 실행 순입니다.
//! - 닫기([`TerminalRegistry::close`])는 실행 중인 child를 종료한 뒤 세션을 제거합니다.
//! - **프로세스 tree 정리**(XVIII.2.4): child 스스로 종료·외부 신호 종료는 [`TerminalRegistry::poll`]이
//!   비차단으로 정리하고 세션을 `Finished{ChildExited}`로 남깁니다. 취소([`TerminalRegistry::cancel`])는
//!   tree를 종료하고 `Finished{Cancelled}`로 남깁니다. 창·앱 종료([`TerminalRegistry::close_all`],
//!   registry drop)는 모든 세션에 먼저 종료를 요청한 뒤 기다려 전체 시간이 세션 하나의 유예 상한과
//!   비슷합니다. 터미널은 복원하지 않습니다(기술 아키텍처 32 "창 종료 시 프로세스 종료").

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::WindowSessionId;

use crate::capability::{TerminalCapability, TerminalNotTrusted};
use crate::cwd::{CwdContext, CwdError, ResolvedCwd};
use crate::env::{interactive_env, TerminalIdentity};
use crate::header::{HeaderCommand, TerminalHeader};
use crate::id::{SessionIdAllocator, TerminalSessionId};
use crate::launch::{
    ExitReport, LaunchError, LaunchRequest, TerminalIoError, TerminalLauncher, TerminalProcess,
};
use crate::profile::{ProfileCommand, TerminalProfile};
use crate::shell::{resolve_shell, ShellError, ShellSettings, UserShellSource};
use crate::size::TerminalSize;
use crate::state::{TerminalProcessState, TerminationReason};

/// 창 하나의 기본 세션 상한.
pub const DEFAULT_MAX_SESSIONS: usize = 32;

/// registry 한도.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryLimits {
    /// 동시에 열어 둘 수 있는 세션 수.
    pub max_sessions: usize,
}

impl Default for RegistryLimits {
    fn default() -> Self {
        RegistryLimits {
            max_sessions: DEFAULT_MAX_SESSIONS,
        }
    }
}

/// registry 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryError {
    /// 세션 수 상한에 도달했습니다.
    LimitReached {
        /// 상한.
        limit: usize,
    },
    /// 이 registry에 없는 세션입니다(닫혔거나 다른 창의 세션).
    UnknownSession,
    /// 식별자 공간을 다 썼습니다.
    IdSpaceExhausted,
    /// child 종료 중 OS 호출이 실패했습니다.
    Terminate(TerminalIoError),
    /// 실행 중인 child가 없습니다.
    NotRunning,
    /// 실행 중 child에 대한 OS 호출(resize 등)이 실패했습니다.
    Process(TerminalIoError),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::LimitReached { limit } => {
                write!(f, "터미널 세션은 창마다 {limit}개까지 열 수 있습니다")
            }
            RegistryError::UnknownSession => f.write_str("터미널 세션을 찾을 수 없습니다"),
            RegistryError::IdSpaceExhausted => {
                f.write_str("터미널 세션 식별자를 더 발급할 수 없습니다")
            }
            RegistryError::Terminate(err) => {
                write!(f, "터미널 프로세스를 종료하지 못했습니다: {err}")
            }
            RegistryError::NotRunning => f.write_str("실행 중인 터미널 프로세스가 없습니다"),
            RegistryError::Process(err) => write!(f, "터미널 프로세스 호출 실패: {err}"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// 시작 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartError {
    /// 신뢰되지 않은 프로젝트입니다(capability를 받지 못함).
    NotTrusted,
    /// capability가 이 registry의 프로젝트 창 것이 아닙니다.
    CapabilityWindowMismatch,
    /// 이 registry에 없는 세션입니다.
    UnknownSession,
    /// 이미 시작했거나 끝난 세션입니다(상태 코드).
    AlreadyStarted {
        /// 현재 상태 코드.
        state: &'static str,
    },
    /// cwd 검증 실패.
    Cwd(CwdError),
    /// 셸 설정을 실행할 수 없음(XVIII.2.2).
    Shell(ShellError),
    /// PTY·child 생성 실패.
    Launch(LaunchError),
}

impl From<TerminalNotTrusted> for StartError {
    fn from(_: TerminalNotTrusted) -> StartError {
        StartError::NotTrusted
    }
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartError::NotTrusted => TerminalNotTrusted.fmt(f),
            StartError::CapabilityWindowMismatch => {
                f.write_str("다른 프로젝트 창의 터미널 capability입니다")
            }
            StartError::UnknownSession => f.write_str("터미널 세션을 찾을 수 없습니다"),
            StartError::AlreadyStarted { state } => {
                write!(f, "이미 시작한 터미널 세션입니다(상태 {state})")
            }
            StartError::Cwd(err) => err.fmt(f),
            StartError::Shell(err) => err.fmt(f),
            StartError::Launch(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for StartError {}

/// 시작에 필요한 창 수준 입력.
#[derive(Debug, Clone, Copy)]
pub struct LaunchEnvironment<'a> {
    /// cwd 기준(프로젝트 루트·worktree root).
    pub cwd: &'a CwdContext,
    /// [`ProfileCommand::DefaultShell`]이 실행할 셸 설정(XVIII.2.2).
    pub shell: &'a ShellSettings,
    /// 사용자 기본 셸 후보 공급자(실제 앱은 [`SystemShellSource`](crate::shell::SystemShellSource)).
    pub shell_source: &'a dyn UserShellSource,
    /// 터미널 식별 변수 값(`TERM`·`COLORTERM`·`TERM_PROGRAM`·`TERM_PROGRAM_VERSION`, ADR-0023 결정 4).
    pub identity: &'a TerminalIdentity,
    /// PTY 초기 크기(터미널 view 크기).
    pub size: TerminalSize,
}

/// registry 안의 세션 하나.
#[derive(Debug)]
pub struct TerminalSession {
    id: TerminalSessionId,
    profile: TerminalProfile,
    state: TerminalProcessState,
    cwd: Option<ResolvedCwd>,
    process: Option<Box<dyn TerminalProcess>>,
}

impl TerminalSession {
    /// 식별자.
    pub fn id(&self) -> TerminalSessionId {
        self.id
    }

    /// 세션을 만들 때 복사한 profile.
    pub fn profile(&self) -> &TerminalProfile {
        &self.profile
    }

    /// 현재 상태.
    pub fn state(&self) -> TerminalProcessState {
        self.state
    }

    /// 시작할 때 검증한 cwd(시작 전이면 `None`).
    pub fn cwd(&self) -> Option<&ResolvedCwd> {
        self.cwd.as_ref()
    }

    /// 실행 중인 child(실행 중이 아니면 `None`).
    pub fn process(&self) -> Option<&dyn TerminalProcess> {
        self.process.as_deref()
    }

    /// 실행 중인 child(가변).
    pub fn process_mut(&mut self) -> Option<&mut (dyn TerminalProcess + 'static)> {
        self.process.as_deref_mut()
    }

    /// child를 종료하고 상태를 `Finished`로 바꿉니다. child가 없으면 상태를 그대로 둡니다. 종료가
    /// 실패하면 child handle을 세션에 그대로 남겨 호출자가 다시 시도하거나 보고할 수 있게 합니다.
    fn finish(&mut self, reason: TerminationReason) -> Result<(), TerminalIoError> {
        if let Some(process) = self.process.as_mut() {
            let report = process.terminate()?;
            self.process = None;
            self.record_exit(report, reason);
        }
        Ok(())
    }

    fn record_exit(&mut self, report: ExitReport, reason: TerminationReason) {
        self.state = TerminalProcessState::Finished {
            status: report.status,
            reason,
            remaining_processes: report.remaining_processes,
        };
    }
}

/// 프로젝트 창 하나의 터미널 세션 registry.
#[derive(Debug)]
pub struct TerminalRegistry {
    window: WindowSessionId,
    limits: RegistryLimits,
    ids: SessionIdAllocator,
    sessions: BTreeMap<TerminalSessionId, TerminalSession>,
}

impl TerminalRegistry {
    /// 기본 한도의 registry.
    pub fn new(window: WindowSessionId) -> TerminalRegistry {
        TerminalRegistry::with_limits(window, RegistryLimits::default())
    }

    /// 한도를 지정한 registry.
    pub fn with_limits(window: WindowSessionId, limits: RegistryLimits) -> TerminalRegistry {
        TerminalRegistry {
            window,
            limits,
            ids: SessionIdAllocator::new(),
            sessions: BTreeMap::new(),
        }
    }

    /// 이 registry가 속한 프로젝트 창.
    pub fn window(&self) -> WindowSessionId {
        self.window
    }

    /// 한도.
    pub fn limits(&self) -> RegistryLimits {
        self.limits
    }

    /// profile로 새 세션을 만듭니다(아직 시작하지 않음).
    pub fn create(&mut self, profile: TerminalProfile) -> Result<TerminalSessionId, RegistryError> {
        if self.sessions.len() >= self.limits.max_sessions {
            return Err(RegistryError::LimitReached {
                limit: self.limits.max_sessions,
            });
        }
        let id = self
            .ids
            .allocate()
            .map_err(|_| RegistryError::IdSpaceExhausted)?;
        self.sessions.insert(
            id,
            TerminalSession {
                id,
                profile,
                state: TerminalProcessState::NotStarted,
                cwd: None,
                process: None,
            },
        );
        Ok(id)
    }

    /// 세션을 시작합니다. 성공하면 세션 leader pid를 돌려줍니다.
    ///
    /// `capability`는 시작 한 번에 소비됩니다. 검사 순서: capability(창 일치) → 세션 존재·상태 →
    /// cwd → 명령·환경 구성 → 실행기. capability 검사가 실패하면 실행기를 호출하지 않습니다.
    pub fn start(
        &mut self,
        id: TerminalSessionId,
        capability: TerminalCapability,
        environment: &LaunchEnvironment<'_>,
        launcher: &mut dyn TerminalLauncher,
    ) -> Result<u32, StartError> {
        if capability.window() != self.window {
            return Err(StartError::CapabilityWindowMismatch);
        }
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(StartError::UnknownSession)?;
        match session.state {
            TerminalProcessState::NotStarted | TerminalProcessState::StartFailed { .. } => {}
            state => {
                return Err(StartError::AlreadyStarted {
                    state: state.code(),
                });
            }
        }
        let cwd = environment
            .cwd
            .resolve(&session.profile.cwd)
            .map_err(StartError::Cwd)?;
        let (program, args, arg0) = match &session.profile.command {
            ProfileCommand::DefaultShell => {
                let shell = resolve_shell(environment.shell, environment.shell_source)
                    .map_err(StartError::Shell)?;
                (
                    shell.program().to_path_buf(),
                    Vec::new(),
                    Some(shell.arg0()),
                )
            }
            ProfileCommand::Argv(command) => (
                command.program().to_path_buf(),
                command.args().to_vec(),
                None,
            ),
        };
        // 상속 환경 위의 변경: 식별 변수 → 기본 셸이면 실제로 실행하는 셸의 SHELL → profile 명시 지정(ADR-0023).
        let shell = matches!(session.profile.command, ProfileCommand::DefaultShell)
            .then_some(program.as_path());
        let env = interactive_env(environment.identity, shell, &session.profile.environment);
        // token을 요청에 소비합니다. 요청은 이 경로에서만 만들어지고 실행기에 값으로 넘어갑니다.
        let request = LaunchRequest::new(
            capability,
            program,
            args,
            arg0,
            cwd.path().to_path_buf(),
            env,
            environment.size,
        );
        match launcher.launch(request) {
            Ok(process) => {
                let pid = process.pid();
                session.state = TerminalProcessState::Running { pid };
                session.cwd = Some(cwd);
                session.process = Some(process);
                Ok(pid)
            }
            Err(err) => {
                session.state = TerminalProcessState::StartFailed {
                    failure: err.failure(),
                };
                Err(StartError::Launch(err))
            }
        }
    }

    /// 세션을 닫고 registry에서 제거합니다. 실행 중이면 child를 종료한 뒤 최종 상태를 돌려줍니다.
    ///
    /// child 종료가 실패하면([`RegistryError::Terminate`]) 세션을 제거하지 않습니다. 세션은 실행 중
    /// 상태와 child handle을 그대로 가지므로 다시 닫거나 취소할 수 있습니다.
    pub fn close(&mut self, id: TerminalSessionId) -> Result<TerminalProcessState, RegistryError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(RegistryError::UnknownSession)?;
        session
            .finish(TerminationReason::Closed)
            .map_err(RegistryError::Terminate)?;
        let state = session.state;
        self.sessions.remove(&id);
        Ok(state)
    }

    /// 세션.
    pub fn session(&self, id: TerminalSessionId) -> Option<&TerminalSession> {
        self.sessions.get(&id)
    }

    /// 실행 중 세션의 종료를 비차단으로 확인하고 정리를 진행합니다. 이번에 끝난 세션 식별자를
    /// 돌려줍니다. 끝난 세션은 사용자가 닫을 때까지 `Finished{ChildExited}`로 남고 child handle은
    /// 해제됩니다. OS 호출 오류는 다음 호출에서 다시 확인합니다. leader를 이 crate 밖에서 회수한
    /// 경우(`ECHILD`)는 오류가 아니라 종료 상태 `Unknown`으로 끝납니다(`pty` 모듈 문서).
    pub fn poll(&mut self) -> Vec<TerminalSessionId> {
        let mut finished = Vec::new();
        for session in self.sessions.values_mut() {
            let Some(process) = session.process.as_mut() else {
                continue;
            };
            if let Ok(Some(report)) = process.poll_exit() {
                session.process = None;
                session.record_exit(report, TerminationReason::ChildExited);
                finished.push(session.id);
            }
        }
        finished
    }

    /// 실행을 취소합니다. 프로세스 tree를 종료하고 세션은 `Finished{Cancelled}`로 남깁니다.
    pub fn cancel(&mut self, id: TerminalSessionId) -> Result<TerminalProcessState, RegistryError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(RegistryError::UnknownSession)?;
        if session.process.is_none() {
            return Err(RegistryError::NotRunning);
        }
        session
            .finish(TerminationReason::Cancelled)
            .map_err(RegistryError::Terminate)?;
        Ok(session.state)
    }

    /// 모든 세션을 닫고 제거합니다(창·앱 종료). 각 세션의 최종 상태를 만든 순서로 돌려줍니다.
    /// 종료에 실패한 세션도 결과와 함께 제거하며, macOS PTY handle은 drop에서 마지막 `SIGKILL`만
    /// 보내고 다시 기다리지 않습니다.
    pub fn close_all(
        &mut self,
        reason: TerminationReason,
    ) -> Vec<(
        TerminalSessionId,
        Result<TerminalProcessState, RegistryError>,
    )> {
        for session in self.sessions.values_mut() {
            if let Some(process) = session.process.as_mut() {
                process.begin_terminate();
            }
        }
        std::mem::take(&mut self.sessions)
            .into_values()
            .map(|mut session| {
                let result = session
                    .finish(reason)
                    .map(|()| session.state)
                    .map_err(RegistryError::Terminate);
                (session.id, result)
            })
            .collect()
    }

    /// 실행 중 세션의 터미널 크기를 바꿉니다(XVIII.2.3, winsize ioctl).
    pub fn resize(
        &mut self,
        id: TerminalSessionId,
        size: TerminalSize,
    ) -> Result<(), RegistryError> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(RegistryError::UnknownSession)?;
        let process = session.process.as_mut().ok_or(RegistryError::NotRunning)?;
        process.resize(size).map_err(RegistryError::Process)
    }

    /// 세션 header metadata(XVIII.4.5). `context`는 창의 현재 cwd 기준, `observed_cwd`는 호출자가
    /// 관찰한 현재 cwd(없으면 시작 cwd)입니다.
    pub fn header(
        &self,
        id: TerminalSessionId,
        context: &CwdContext,
        observed_cwd: Option<&std::path::Path>,
    ) -> Option<TerminalHeader> {
        let session = self.sessions.get(&id)?;
        Some(TerminalHeader::build(
            session.profile.name.as_str(),
            HeaderCommand::of(&session.profile.command),
            &session.profile.cwd,
            session.cwd.as_ref(),
            context,
            observed_cwd,
        ))
    }

    /// 세션(가변: 입력 쓰기 등).
    pub fn session_mut(&mut self, id: TerminalSessionId) -> Option<&mut TerminalSession> {
        self.sessions.get_mut(&id)
    }

    /// 열린 세션 식별자(만든 순서).
    pub fn ids(&self) -> Vec<TerminalSessionId> {
        self.sessions.keys().copied().collect()
    }

    /// 열린 세션 수.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// 열린 세션이 없는지.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// FFI·UI handle을 이 registry의 **열린** 세션 식별자로 되돌립니다.
    pub fn resolve_handle(&self, handle: u64) -> Option<TerminalSessionId> {
        self.ids
            .decode(handle)
            .filter(|id| self.sessions.contains_key(id))
    }
}

impl Drop for TerminalRegistry {
    /// 창이 닫히면 남은 세션의 프로세스 tree를 모두 종료합니다.
    fn drop(&mut self) {
        let _ = self.close_all(TerminationReason::AppClosed);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::capability::TerminalTrust;
    use crate::cwd::{CwdSelection, ExplicitCwd};
    use crate::profile::{ArgvCommand, ProfileName};
    use crate::shell::tests::FixedShellSource;
    use crate::shell::{ShellChoice, ShellMode, ShellPathProblem};
    use crate::state::{StartFailure, TerminalExitStatus};
    use crate::test_support::TempDir;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use stellar_core_types::ids::StableId;

    pub(crate) fn window(n: u8) -> WindowSessionId {
        WindowSessionId::from_stable_id(StableId::from_bytes([n; 16]))
    }

    pub(crate) fn profile(name: &str) -> TerminalProfile {
        TerminalProfile::default_shell(ProfileName::new(name).unwrap())
    }

    /// 가짜 실행기·프로세스 계측.
    #[derive(Debug, Default, Clone)]
    pub(crate) struct FakeCounters {
        launches: Arc<AtomicUsize>,
        terminated: Arc<AtomicUsize>,
        dropped: Arc<AtomicUsize>,
        resized: Arc<AtomicUsize>,
        /// 켜져 있으면 terminate가 `TimedOut`으로 실패합니다.
        fail_terminate: Arc<AtomicBool>,
    }

    impl FakeCounters {
        pub(crate) fn launches(&self) -> usize {
            self.launches.load(Ordering::SeqCst)
        }

        pub(crate) fn terminated(&self) -> usize {
            self.terminated.load(Ordering::SeqCst)
        }

        pub(crate) fn dropped(&self) -> usize {
            self.dropped.load(Ordering::SeqCst)
        }

        pub(crate) fn resized(&self) -> usize {
            self.resized.load(Ordering::SeqCst)
        }

        pub(crate) fn set_fail_terminate(&self, fail: bool) {
            self.fail_terminate.store(fail, Ordering::SeqCst);
        }
    }

    #[derive(Debug)]
    pub(crate) struct FakeProcess {
        pid: u32,
        counters: FakeCounters,
        size: TerminalSize,
    }

    impl TerminalProcess for FakeProcess {
        fn pid(&self) -> u32 {
            self.pid
        }

        fn poll_exit(&mut self) -> Result<Option<ExitReport>, TerminalIoError> {
            Ok(None)
        }

        fn begin_terminate(&mut self) {}

        fn terminate(&mut self) -> Result<ExitReport, TerminalIoError> {
            self.counters.terminated.fetch_add(1, Ordering::SeqCst);
            if self.counters.fail_terminate.load(Ordering::SeqCst) {
                return Err(TerminalIoError {
                    operation: "terminate",
                    kind: std::io::ErrorKind::TimedOut,
                    os_code: None,
                });
            }
            Ok(ExitReport {
                status: TerminalExitStatus::Signaled(1),
                remaining_processes: 0,
            })
        }

        fn write_input(&mut self, _bytes: &[u8]) -> Result<(), TerminalIoError> {
            Ok(())
        }

        fn output_reader(&self) -> Result<Box<dyn std::io::Read + Send>, TerminalIoError> {
            Ok(Box::new(std::io::empty()))
        }

        fn resize(&mut self, size: TerminalSize) -> Result<(), TerminalIoError> {
            self.counters.resized.fetch_add(1, Ordering::SeqCst);
            self.size = size;
            Ok(())
        }

        fn size(&self) -> Result<TerminalSize, TerminalIoError> {
            Ok(self.size)
        }
    }

    impl Drop for FakeProcess {
        fn drop(&mut self) {
            self.counters.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// 요청을 기록하고 가짜 프로세스를 돌려주는 실행기.
    #[derive(Debug, Default)]
    pub(crate) struct FakeLauncher {
        pub(crate) counters: FakeCounters,
        pub(crate) requests: Vec<LaunchRequest>,
        pub(crate) fail: Option<LaunchError>,
    }

    impl TerminalLauncher for FakeLauncher {
        fn launch(
            &mut self,
            request: LaunchRequest,
        ) -> Result<Box<dyn TerminalProcess>, LaunchError> {
            self.counters.launches.fetch_add(1, Ordering::SeqCst);
            let size = request.size();
            self.requests.push(request);
            if let Some(err) = self.fail {
                return Err(err);
            }
            let pid = 1000 + u32::try_from(self.requests.len()).unwrap_or(0);
            Ok(Box::new(FakeProcess {
                pid,
                counters: self.counters.clone(),
                size,
            }))
        }
    }

    /// 시험용 식별 변수 값(앱 버전 `0.0.0-test`).
    pub(crate) fn test_identity() -> TerminalIdentity {
        TerminalIdentity::stellar("0.0.0-test").unwrap()
    }

    /// 요청의 변경을 가짜 상속 환경에 적용한 결과(이름 순).
    fn child_env(request: &LaunchRequest) -> Vec<(OsString, OsString)> {
        crate::env::apply_env_changes(
            [
                ("PATH", "/usr/bin:/bin"),
                ("HOME", "/Users/example"),
                ("SECRET_TOKEN", "app-env-only"),
            ]
            .map(|(n, v)| (OsString::from(n), OsString::from(v))),
            request.env_changes(),
        )
    }

    pub(crate) fn sh() -> ShellSettings {
        ShellSettings {
            choice: ShellChoice::Custom(PathBuf::from("/bin/sh")),
            mode: ShellMode::Login,
        }
    }

    pub(crate) const NO_SHELL_SOURCE: FixedShellSource = FixedShellSource {
        account: None,
        env: None,
    };

    fn trusted(registry: &TerminalRegistry) -> TerminalCapability {
        TerminalCapability::grant(registry.window(), TerminalTrust::Trusted).unwrap()
    }

    #[test]
    fn untrusted_projects_never_spawn() {
        let dir = TempDir::new();
        let context = CwdContext::new(dir.path(), &[]).unwrap();
        let shell = sh();
        let environment = LaunchEnvironment {
            cwd: &context,
            shell: &shell,
            shell_source: &NO_SHELL_SOURCE,
            identity: &test_identity(),
            size: TerminalSize::DEFAULT,
        };
        let mut launcher = FakeLauncher::default();
        let mut registry = TerminalRegistry::new(window(10));
        let id = registry.create(profile("untrusted")).unwrap();
        for _ in 0..100 {
            let result = TerminalCapability::grant(registry.window(), TerminalTrust::NotTrusted)
                .map_err(StartError::from)
                .and_then(|capability| registry.start(id, capability, &environment, &mut launcher));
            assert_eq!(result, Err(StartError::NotTrusted));
        }
        // 다른 창(신뢰됨)의 token은 이 창에서 쓸 수 없고, 세션 존재 검사보다 먼저 거부됩니다.
        let foreign = TerminalCapability::grant(window(11), TerminalTrust::Trusted).unwrap();
        let mut other = TerminalRegistry::new(window(12));
        let other_id = other.create(profile("other")).unwrap();
        assert_eq!(
            registry.start(other_id, foreign, &environment, &mut launcher),
            Err(StartError::CapabilityWindowMismatch)
        );
        let foreign = TerminalCapability::grant(window(11), TerminalTrust::Trusted).unwrap();
        assert_eq!(
            registry.start(id, foreign, &environment, &mut launcher),
            Err(StartError::CapabilityWindowMismatch)
        );
        assert_eq!(launcher.counters.launches(), 0);
        assert!(launcher.requests.is_empty());
        assert_eq!(
            registry.session(id).unwrap().state(),
            TerminalProcessState::NotStarted
        );
        assert!(registry.session(id).unwrap().process().is_none());
    }

    #[test]
    fn trusted_start_builds_request_and_close_terminates_child() {
        let dir = TempDir::new();
        let worktree = dir.mkdir("wt");
        let context = CwdContext::new(dir.path(), std::slice::from_ref(&worktree)).unwrap();
        let shell = sh();
        let environment = LaunchEnvironment {
            cwd: &context,
            shell: &shell,
            shell_source: &NO_SHELL_SOURCE,
            identity: &test_identity(),
            size: TerminalSize::DEFAULT,
        };
        let mut launcher = FakeLauncher::default();
        let mut registry = TerminalRegistry::new(window(13));
        let mut agent = profile("agent");
        agent.command =
            ProfileCommand::Argv(ArgvCommand::new("/usr/bin/env", ["a b", "--flag"]).unwrap());
        agent.cwd = CwdSelection::WorktreeRoot(worktree.clone());
        agent
            .environment
            .set("TERM_PROGRAM", "agent-profile")
            .unwrap();
        let shell_id = registry.create(profile("shell")).unwrap();
        let agent_id = registry.create(agent).unwrap();

        let pid = registry
            .start(shell_id, trusted(&registry), &environment, &mut launcher)
            .unwrap();
        assert_eq!(
            registry.session(shell_id).unwrap().state(),
            TerminalProcessState::Running { pid }
        );
        registry
            .start(agent_id, trusted(&registry), &environment, &mut launcher)
            .unwrap();
        assert_eq!(launcher.counters.launches(), 2);

        let shell_request = &launcher.requests[0];
        // 요청은 이 창의 token을 소비해 만들어졌습니다.
        assert_eq!(shell_request.window(), window(13));
        assert_eq!(shell_request.program(), std::path::Path::new("/bin/sh"));
        assert_eq!(shell_request.arg0(), Some(&OsString::from("-sh")));
        assert!(shell_request.args().is_empty());
        assert_eq!(shell_request.cwd(), dir.path());
        // 상속 환경은 그대로이고(허용 목록 밖 변수 포함), 식별 변수 4개와 기본 셸의 SHELL만 더합니다.
        assert_eq!(
            child_env(shell_request),
            [
                (OsString::from("COLORTERM"), OsString::from("truecolor")),
                (OsString::from("HOME"), OsString::from("/Users/example")),
                (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
                (
                    OsString::from("SECRET_TOKEN"),
                    OsString::from("app-env-only")
                ),
                (OsString::from("SHELL"), OsString::from("/bin/sh")),
                (OsString::from("TERM"), OsString::from("xterm-256color")),
                (OsString::from("TERM_PROGRAM"), OsString::from("Stellar")),
                (
                    OsString::from("TERM_PROGRAM_VERSION"),
                    OsString::from("0.0.0-test")
                ),
            ]
        );

        let agent_request = &launcher.requests[1];
        assert_eq!(
            agent_request.program(),
            std::path::Path::new("/usr/bin/env")
        );
        assert_eq!(agent_request.args(), ["a b", "--flag"]);
        assert_eq!(agent_request.cwd(), worktree);
        // profile 명시 지정이 식별 변수보다 이기고, argv profile은 SHELL을 바꾸지 않습니다.
        let agent_env = child_env(agent_request);
        assert!(agent_env
            .iter()
            .any(|(name, value)| name == "TERM_PROGRAM" && value == "agent-profile"));
        assert!(agent_env.iter().all(|(name, _)| name != "SHELL"));
        assert_eq!(
            registry.session(agent_id).unwrap().cwd().unwrap().path(),
            worktree
        );

        // 이미 실행 중인 세션은 다시 시작하지 않습니다.
        assert_eq!(
            registry.start(shell_id, trusted(&registry), &environment, &mut launcher),
            Err(StartError::AlreadyStarted { state: "running" })
        );

        let closed = registry.close(shell_id).unwrap();
        assert_eq!(
            closed,
            TerminalProcessState::Finished {
                status: TerminalExitStatus::Signaled(1),
                reason: TerminationReason::Closed,
                remaining_processes: 0,
            }
        );
        assert_eq!(launcher.counters.terminated(), 1);
        assert_eq!(launcher.counters.dropped(), 1);
    }

    #[test]
    fn start_failures_do_not_leave_processes() {
        let dir = TempDir::new();
        let context = CwdContext::new(dir.path(), &[]).unwrap();
        let shell = sh();
        let environment = LaunchEnvironment {
            cwd: &context,
            shell: &shell,
            shell_source: &NO_SHELL_SOURCE,
            identity: &test_identity(),
            size: TerminalSize::DEFAULT,
        };
        let mut launcher = FakeLauncher::default();
        let mut registry = TerminalRegistry::new(window(14));

        // cwd 검증 실패: 실행기를 호출하지 않고 상태도 그대로입니다.
        let mut missing = profile("missing-cwd");
        missing.cwd =
            CwdSelection::Explicit(ExplicitCwd::from_user_input(dir.path().join("gone")).unwrap());
        let missing_id = registry.create(missing).unwrap();
        assert!(matches!(
            registry.start(missing_id, trusted(&registry), &environment, &mut launcher),
            Err(StartError::Cwd(CwdError::NotFound { .. }))
        ));
        assert_eq!(launcher.counters.launches(), 0);
        assert_eq!(
            registry.session(missing_id).unwrap().state(),
            TerminalProcessState::NotStarted
        );

        // 실행 실패: StartFailed로 기록하고, 원인을 고친 뒤 다시 시작할 수 있습니다.
        let id = registry.create(profile("spawn-fails")).unwrap();
        let failure = LaunchError::Spawn(TerminalIoError {
            operation: "spawn",
            kind: std::io::ErrorKind::NotFound,
            os_code: Some(2),
        });
        launcher.fail = Some(failure);
        assert_eq!(
            registry.start(id, trusted(&registry), &environment, &mut launcher),
            Err(StartError::Launch(failure))
        );
        assert_eq!(
            registry.session(id).unwrap().state(),
            TerminalProcessState::StartFailed {
                failure: StartFailure::Spawn
            }
        );
        assert!(registry.session(id).unwrap().process().is_none());
        launcher.fail = None;
        assert!(registry
            .start(id, trusted(&registry), &environment, &mut launcher)
            .is_ok());
        assert_eq!(launcher.counters.dropped(), 0);
        registry.close(id).unwrap();
        assert_eq!(launcher.counters.dropped(), 1);
    }

    #[test]
    fn twenty_started_sessions_close_without_leaking_processes() {
        let dir = TempDir::new();
        let context = CwdContext::new(dir.path(), &[]).unwrap();
        let shell = sh();
        let environment = LaunchEnvironment {
            cwd: &context,
            shell: &shell,
            shell_source: &NO_SHELL_SOURCE,
            identity: &test_identity(),
            size: TerminalSize::DEFAULT,
        };
        let mut launcher = FakeLauncher::default();
        let mut registry = TerminalRegistry::new(window(15));
        let ids: Vec<TerminalSessionId> = (0..20)
            .map(|n| registry.create(profile(&format!("s{n}"))).unwrap())
            .collect();
        for id in &ids {
            registry
                .start(*id, trusted(&registry), &environment, &mut launcher)
                .unwrap();
        }
        for id in &ids {
            registry.close(*id).unwrap();
        }
        assert!(registry.is_empty());
        assert_eq!(launcher.counters.launches(), 20);
        assert_eq!(launcher.counters.terminated(), 20);
        assert_eq!(launcher.counters.dropped(), 20);
    }

    #[test]
    fn failed_termination_keeps_session_for_retry() {
        let dir = TempDir::new();
        let context = CwdContext::new(dir.path(), &[]).unwrap();
        let shell = sh();
        let environment = LaunchEnvironment {
            cwd: &context,
            shell: &shell,
            shell_source: &NO_SHELL_SOURCE,
            identity: &test_identity(),
            size: TerminalSize::DEFAULT,
        };
        let mut launcher = FakeLauncher::default();
        let mut registry = TerminalRegistry::new(window(22));
        let id = registry.create(profile("stuck")).unwrap();
        let pid = registry
            .start(id, trusted(&registry), &environment, &mut launcher)
            .unwrap();
        launcher.counters.set_fail_terminate(true);
        let timed_out = Err(RegistryError::Terminate(TerminalIoError {
            operation: "terminate",
            kind: std::io::ErrorKind::TimedOut,
            os_code: None,
        }));
        // 닫기 실패: 세션·handle이 그대로 남고 다시 시도할 수 있습니다(handle drop 0).
        assert_eq!(registry.close(id), timed_out);
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.resolve_handle(id.to_handle()), Some(id));
        assert_eq!(
            registry.session(id).unwrap().state(),
            TerminalProcessState::Running { pid }
        );
        assert!(registry.session(id).unwrap().process().is_some());
        assert_eq!(registry.cancel(id), timed_out);
        assert!(registry.session(id).unwrap().process().is_some());
        assert_eq!(launcher.counters.dropped(), 0);
        // 원인이 사라지면 같은 세션을 닫을 수 있습니다.
        launcher.counters.set_fail_terminate(false);
        assert!(matches!(
            registry.close(id),
            Ok(TerminalProcessState::Finished {
                reason: TerminationReason::Closed,
                ..
            })
        ));
        assert!(registry.is_empty());
        assert_eq!(launcher.counters.terminated(), 3);
        assert_eq!(launcher.counters.dropped(), 1);

        // 창 종료(close_all)는 실패한 세션도 결과와 함께 제거합니다.
        let id = registry.create(profile("stuck-on-close")).unwrap();
        registry
            .start(id, trusted(&registry), &environment, &mut launcher)
            .unwrap();
        launcher.counters.set_fail_terminate(true);
        let results = registry.close_all(TerminationReason::AppClosed);
        assert_eq!(results, [(id, timed_out)]);
        assert!(registry.is_empty());
        assert_eq!(launcher.counters.dropped(), 2);
    }

    #[test]
    fn twenty_sessions_create_and_close_without_leaks() {
        let mut registry = TerminalRegistry::new(window(1));
        let mut all_ids = Vec::new();
        for round in 0..5 {
            let ids: Vec<TerminalSessionId> = (0..20)
                .map(|n| registry.create(profile(&format!("s{round}-{n}"))).unwrap())
                .collect();
            assert_eq!(registry.len(), 20);
            assert_eq!(registry.ids(), ids);
            for id in &ids {
                assert_eq!(registry.resolve_handle(id.to_handle()), Some(*id));
                assert_eq!(registry.close(*id), Ok(TerminalProcessState::NotStarted));
            }
            assert!(registry.is_empty());
            assert!(registry.sessions.is_empty());
            for id in &ids {
                // 닫은 세션은 다시 해석·조회·닫기되지 않습니다.
                assert_eq!(registry.resolve_handle(id.to_handle()), None);
                assert!(registry.session(*id).is_none());
                assert_eq!(registry.close(*id), Err(RegistryError::UnknownSession));
            }
            all_ids.extend(ids);
        }
        // 식별자는 재사용하지 않습니다.
        let mut unique = all_ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), all_ids.len());
    }

    #[test]
    fn session_limit_is_enforced_and_freed_by_close() {
        let mut registry =
            TerminalRegistry::with_limits(window(2), RegistryLimits { max_sessions: 2 });
        let a = registry.create(profile("a")).unwrap();
        let _b = registry.create(profile("b")).unwrap();
        assert_eq!(
            registry.create(profile("c")),
            Err(RegistryError::LimitReached { limit: 2 })
        );
        registry.close(a).unwrap();
        assert!(registry.create(profile("c")).is_ok());
    }

    #[test]
    fn registries_of_different_windows_do_not_share_sessions() {
        let mut first = TerminalRegistry::new(window(3));
        let mut second = TerminalRegistry::new(window(4));
        let id = first.create(profile("first")).unwrap();
        let _ = second.create(profile("second")).unwrap();
        assert_eq!(first.window(), window(3));
        assert_eq!(second.resolve_handle(id.to_handle()), None);
        assert!(second.session(id).is_none());
        assert_eq!(second.close(id), Err(RegistryError::UnknownSession));
        assert_eq!(first.session(id).unwrap().profile().name.as_str(), "first");
        assert_eq!(
            first.session(id).unwrap().state(),
            TerminalProcessState::NotStarted
        );
    }

    #[test]
    fn missing_shell_is_reported_without_spawning() {
        let dir = TempDir::new();
        let context = CwdContext::new(dir.path(), &[]).unwrap();
        let missing = dir.path().join("no-such-shell");
        let custom = ShellSettings {
            choice: ShellChoice::Custom(missing.clone()),
            mode: ShellMode::Login,
        };
        let user_default = ShellSettings::default();
        let mut launcher = FakeLauncher::default();
        let mut registry = TerminalRegistry::new(window(16));
        let id = registry.create(profile("shell")).unwrap();

        let environment = LaunchEnvironment {
            cwd: &context,
            shell: &custom,
            shell_source: &NO_SHELL_SOURCE,
            identity: &test_identity(),
            size: TerminalSize::DEFAULT,
        };
        assert_eq!(
            registry.start(id, trusted(&registry), &environment, &mut launcher),
            Err(StartError::Shell(ShellError::InvalidShell {
                path: missing,
                problem: ShellPathProblem::NotFound,
            }))
        );
        let environment = LaunchEnvironment {
            shell: &user_default,
            ..environment
        };
        assert_eq!(
            registry.start(id, trusted(&registry), &environment, &mut launcher),
            Err(StartError::Shell(ShellError::NoDefaultShell))
        );
        assert_eq!(launcher.counters.launches(), 0);
        assert_eq!(
            registry.session(id).unwrap().state(),
            TerminalProcessState::NotStarted
        );

        // argv profile은 셸 설정과 무관하게 시작합니다. 명시한 SHELL은 그대로 둡니다.
        let mut argv = profile("argv");
        argv.command = ProfileCommand::Argv(ArgvCommand::new("/bin/echo", ["x"]).unwrap());
        let argv_id = registry.create(argv).unwrap();
        registry
            .start(argv_id, trusted(&registry), &environment, &mut launcher)
            .unwrap();
        assert_eq!(launcher.requests[0].arg0(), None);
        let mut explicit = profile("explicit-shell-env");
        explicit.environment.set("SHELL", "/custom/shell").unwrap();
        explicit.environment.set("TERM", "dumb").unwrap();
        let explicit_id = registry.create(explicit).unwrap();
        let shell = sh();
        let environment = LaunchEnvironment {
            shell: &shell,
            ..environment
        };
        registry
            .start(explicit_id, trusted(&registry), &environment, &mut launcher)
            .unwrap();
        let explicit_env = child_env(&launcher.requests[1]);
        assert!(explicit_env
            .iter()
            .any(|(name, value)| name == "SHELL" && value == "/custom/shell"));
        // profile이 정한 TERM은 기본값으로 바꾸지 않습니다.
        let terms: Vec<&OsString> = explicit_env
            .iter()
            .filter(|(name, _)| name == "TERM")
            .map(|(_, value)| value)
            .collect();
        assert_eq!(terms, [&OsString::from("dumb")]);
    }

    /// 실제 PTY에서 login/non-login 셸의 `$0`과 `SHELL`을 확인합니다.
    #[cfg(target_os = "macos")]
    #[test]
    fn login_and_non_login_shells_run_in_pty() {
        let _pty = crate::test_support::pty_shared();
        use crate::pty::tests::TIMEOUT;
        use crate::pty::PtyLauncher;
        use crate::test_support::OutputCollector;

        let dir = TempDir::new();
        let context = CwdContext::new(dir.path(), &[]).unwrap();
        let mut launcher = PtyLauncher::with_grace(std::time::Duration::from_millis(500));
        let mut registry = TerminalRegistry::new(window(17));
        for (mode, expected) in [(ShellMode::Login, "-sh"), (ShellMode::NonLogin, "sh")] {
            let settings = ShellSettings {
                choice: ShellChoice::Custom(PathBuf::from("/bin/sh")),
                mode,
            };
            let environment = LaunchEnvironment {
                cwd: &context,
                shell: &settings,
                shell_source: &NO_SHELL_SOURCE,
                identity: &test_identity(),
                size: TerminalSize::DEFAULT,
            };
            let mut shell_profile = profile("sh");
            // 사용자 startup 파일을 읽지 않도록 HOME을 임시 디렉터리로 둡니다.
            shell_profile
                .environment
                .set("HOME", dir.path().as_os_str())
                .unwrap();
            let id = registry.create(shell_profile).unwrap();
            registry
                .start(id, trusted(&registry), &environment, &mut launcher)
                .unwrap();
            let session = registry.session_mut(id).unwrap();
            let process = session.process_mut().unwrap();
            let mut output = OutputCollector::start(process.output_reader().unwrap());
            process
                .write_input(b"echo \"[$0][$SHELL]\"; exit 7\n")
                .unwrap();
            output.wait_for(&format!("[{expected}][/bin/sh]"), TIMEOUT);
            let deadline = std::time::Instant::now() + TIMEOUT;
            let report = loop {
                if let Some(report) = process.poll_exit().unwrap() {
                    break report;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            assert_eq!(report.status, TerminalExitStatus::Exited(7));
            registry.close(id).unwrap();
        }
    }

    #[test]
    fn resize_is_forwarded_to_running_sessions_only() {
        let dir = TempDir::new();
        let context = CwdContext::new(dir.path(), &[]).unwrap();
        let shell = sh();
        let environment = LaunchEnvironment {
            cwd: &context,
            shell: &shell,
            shell_source: &NO_SHELL_SOURCE,
            identity: &test_identity(),
            size: TerminalSize::new(30, 90).unwrap(),
        };
        let mut launcher = FakeLauncher::default();
        let mut registry = TerminalRegistry::new(window(18));
        let id = registry.create(profile("resize")).unwrap();
        let size = TerminalSize::new(50, 132).unwrap();
        assert_eq!(registry.resize(id, size), Err(RegistryError::NotRunning));
        registry
            .start(id, trusted(&registry), &environment, &mut launcher)
            .unwrap();
        assert_eq!(
            launcher.requests[0].size(),
            TerminalSize::new(30, 90).unwrap()
        );
        registry.resize(id, size).unwrap();
        assert_eq!(launcher.counters.resized(), 1);
        assert_eq!(
            registry.session(id).unwrap().process().unwrap().size(),
            Ok(size)
        );
        registry.close(id).unwrap();
        assert_eq!(
            registry.resize(id, size),
            Err(RegistryError::UnknownSession)
        );
    }

    /// 실제 PTY 세션 registry 환경.
    #[cfg(target_os = "macos")]
    fn tree_profile(name: &str, tail: &str) -> TerminalProfile {
        let mut tree = profile(name);
        let script = format!("{}; {tail}", crate::pty::tests::TREE_SCRIPT);
        tree.command =
            ProfileCommand::Argv(ArgvCommand::new("/bin/sh", ["-c", script.as_str()]).unwrap());
        tree
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn registry_cleans_up_on_child_exit_cancel_and_app_close() {
        use crate::pty::tests::{wait_gone, wait_tree_ready, KillOnDrop, TIMEOUT};
        use crate::pty::PtyLauncher;
        use crate::test_support::{pty_shared, OutputCollector};

        let _pty = pty_shared();
        let dir = TempDir::new();
        let context = CwdContext::new(dir.path(), &[]).unwrap();
        let shell = sh();
        let environment = LaunchEnvironment {
            cwd: &context,
            shell: &shell,
            shell_source: &NO_SHELL_SOURCE,
            identity: &test_identity(),
            size: TerminalSize::DEFAULT,
        };
        let mut launcher = PtyLauncher::with_grace(std::time::Duration::from_millis(300));
        let mut registry = TerminalRegistry::new(window(19));
        let exiting = registry
            .create(tree_profile("exit", "read line; exit 3"))
            .unwrap();
        let cancelled = registry
            .create(tree_profile("cancel", "while :; do /bin/sleep 1; done"))
            .unwrap();
        let app_a = registry
            .create(tree_profile("app-a", "while :; do /bin/sleep 1; done"))
            .unwrap();
        let app_b = registry
            .create(tree_profile("app-b", "while :; do /bin/sleep 1; done"))
            .unwrap();
        let mut pids = std::collections::BTreeMap::new();
        let mut guards = Vec::new();
        for id in [exiting, cancelled, app_a, app_b] {
            let leader = registry
                .start(id, trusted(&registry), &environment, &mut launcher)
                .unwrap();
            let process = registry.session(id).unwrap().process().unwrap();
            let mut output = OutputCollector::start(process.output_reader().unwrap());
            let mut found = wait_tree_ready(&mut output);
            found.push(i32::try_from(leader).unwrap());
            guards.push(KillOnDrop(found.clone()));
            pids.insert(id, found);
        }

        // child 종료: poll이 세션 전체를 정리하고 Finished{ChildExited}로 남깁니다.
        registry
            .session_mut(exiting)
            .unwrap()
            .process_mut()
            .unwrap()
            .write_input(b"\n")
            .unwrap();
        let deadline = std::time::Instant::now() + TIMEOUT;
        while !registry.poll().contains(&exiting) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            registry.session(exiting).unwrap().state(),
            TerminalProcessState::Finished {
                status: TerminalExitStatus::Exited(3),
                reason: TerminationReason::ChildExited,
                remaining_processes: 0,
            }
        );
        assert!(registry.session(exiting).unwrap().process().is_none());
        wait_gone(&pids[&exiting]);

        // 취소: tree 종료 후 Finished{Cancelled}.
        let state = registry.cancel(cancelled).unwrap();
        assert!(matches!(
            state,
            TerminalProcessState::Finished {
                reason: TerminationReason::Cancelled,
                remaining_processes: 0,
                ..
            }
        ));
        assert_eq!(registry.cancel(cancelled), Err(RegistryError::NotRunning));
        wait_gone(&pids[&cancelled]);

        // 창·앱 종료: registry drop이 남은 세션의 tree를 모두 정리합니다.
        let started = std::time::Instant::now();
        drop(registry);
        // 두 세션을 함께 요청한 뒤 기다리므로 세션 하나의 상한 안에 끝납니다.
        assert!(started.elapsed() < std::time::Duration::from_millis(300) + crate::pty::KILL_WAIT);
        wait_gone(&pids[&app_a]);
        wait_gone(&pids[&app_b]);
        drop(guards);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn twenty_pty_sessions_create_and_close_without_leaks() {
        use crate::pty::tests::{alive, tty_fd_count};
        use crate::pty::PtyLauncher;
        use crate::test_support::pty_exclusive;

        let _pty = pty_exclusive();
        let dir = TempDir::new();
        let context = CwdContext::new(dir.path(), &[]).unwrap();
        let shell = sh();
        let environment = LaunchEnvironment {
            cwd: &context,
            shell: &shell,
            shell_source: &NO_SHELL_SOURCE,
            identity: &test_identity(),
            size: TerminalSize::DEFAULT,
        };
        let mut launcher = PtyLauncher::with_grace(std::time::Duration::from_millis(300));
        let mut registry = TerminalRegistry::new(window(20));
        let baseline = tty_fd_count();
        for round in 0..2 {
            let mut leaders = Vec::new();
            let ids: Vec<TerminalSessionId> = (0..20)
                .map(|n| {
                    let mut sleeper = profile(&format!("s{round}-{n}"));
                    sleeper.command =
                        ProfileCommand::Argv(ArgvCommand::new("/bin/sleep", ["300"]).unwrap());
                    registry.create(sleeper).unwrap()
                })
                .collect();
            for id in &ids {
                leaders.push(
                    registry
                        .start(*id, trusted(&registry), &environment, &mut launcher)
                        .unwrap(),
                );
            }
            // 세션마다 master + 보관 slave.
            assert_eq!(tty_fd_count(), baseline + 40);
            for id in &ids {
                let state = registry.close(*id).unwrap();
                assert!(matches!(
                    state,
                    TerminalProcessState::Finished {
                        reason: TerminationReason::Closed,
                        remaining_processes: 0,
                        ..
                    }
                ));
            }
            assert!(registry.is_empty());
            assert_eq!(tty_fd_count(), baseline);
            for pid in leaders {
                assert!(!alive(i32::try_from(pid).unwrap()), "{pid}");
            }
        }
        assert_eq!(launcher.spawn_count(), 40);
    }

    #[test]
    fn registry_header_separates_general_and_worktree_sessions() {
        use crate::header::{CwdWarning, HeaderScope};

        let dir = TempDir::new();
        let worktree = dir.mkdir("wt");
        let context = CwdContext::new(dir.path(), std::slice::from_ref(&worktree)).unwrap();
        let shell = sh();
        let environment = LaunchEnvironment {
            cwd: &context,
            shell: &shell,
            shell_source: &NO_SHELL_SOURCE,
            identity: &test_identity(),
            size: TerminalSize::DEFAULT,
        };
        let mut launcher = FakeLauncher::default();
        let mut registry = TerminalRegistry::new(window(21));
        let general = registry.create(profile("general")).unwrap();
        let mut bound = profile("bound");
        bound.cwd = CwdSelection::WorktreeRoot(worktree.clone());
        let bound = registry.create(bound).unwrap();
        for id in [general, bound] {
            registry
                .start(id, trusted(&registry), &environment, &mut launcher)
                .unwrap();
        }
        let general_header = registry.header(general, &context, None).unwrap();
        assert_eq!(
            general_header.scope,
            HeaderScope::Project {
                root: dir.path().to_path_buf()
            }
        );
        let bound_header = registry.header(bound, &context, None).unwrap();
        assert!(bound_header.scope.is_worktree());
        assert_eq!(bound_header.title, "bound");
        assert!(!bound_header.has_warning());
        // worktree 터미널이 worktree 밖(프로젝트 루트)으로 이동하면 경고합니다.
        let moved = registry.header(bound, &context, Some(dir.path())).unwrap();
        assert_eq!(
            moved.warnings,
            [CwdWarning::OutsideBinding {
                root: worktree.clone(),
                cwd: dir.path().to_path_buf(),
            }]
        );
        registry.close(bound).unwrap();
        assert!(registry.header(bound, &context, None).is_none());
    }
}

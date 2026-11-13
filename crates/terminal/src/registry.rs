//! 프로젝트 창별 터미널 세션 registry (XVIII.1.2, XVIII.1.4).
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
//!   호출하지 않습니다(untrusted spawn 0). 그다음 cwd 검증(XVIII.1.3), 명령·환경 구성, 실행 순입니다.
//! - 닫기([`TerminalRegistry::close`])는 실행 중인 child를 종료한 뒤 세션을 제거합니다.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;

use stellar_core_types::ids::WindowSessionId;

use crate::capability::{TerminalCapability, TerminalNotTrusted};
use crate::cwd::{CwdContext, CwdError, ResolvedCwd};
use crate::id::{SessionIdAllocator, TerminalSessionId};
use crate::launch::{
    ExitReport, LaunchError, LaunchRequest, TerminalIoError, TerminalLauncher, TerminalProcess,
};
use crate::profile::{ArgvCommand, ProfileCommand, TerminalProfile};
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
    /// [`ProfileCommand::DefaultShell`]이 실행할 셸 명령.
    pub default_shell: &'a ArgvCommand,
    /// 상속 허용 변수의 현재 값을 읽는 함수(앱 프로세스 환경이면 [`process_env`]).
    pub env_lookup: fn(&str) -> Option<OsString>,
}

/// 앱 프로세스 환경에서 변수 값을 읽습니다.
pub fn process_env(name: &str) -> Option<OsString> {
    std::env::var_os(name)
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

    /// child를 종료하고 상태를 `Finished`로 바꿉니다. child가 없으면 상태를 그대로 둡니다.
    fn finish(&mut self, reason: TerminationReason) -> Result<(), TerminalIoError> {
        if let Some(mut process) = self.process.take() {
            let report = process.terminate()?;
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
        let command = match &session.profile.command {
            ProfileCommand::DefaultShell => environment.default_shell,
            ProfileCommand::Argv(command) => command,
        };
        let request = LaunchRequest::new(
            command.program().to_path_buf(),
            command.args().to_vec(),
            None,
            cwd.path().to_path_buf(),
            session.profile.environment.resolve(environment.env_lookup),
        );
        match launcher.launch(&request) {
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
    pub fn close(&mut self, id: TerminalSessionId) -> Result<TerminalProcessState, RegistryError> {
        let mut session = self
            .sessions
            .remove(&id)
            .ok_or(RegistryError::UnknownSession)?;
        session
            .finish(TerminationReason::Closed)
            .map_err(RegistryError::Terminate)?;
        Ok(session.state)
    }

    /// 세션.
    pub fn session(&self, id: TerminalSessionId) -> Option<&TerminalSession> {
        self.sessions.get(&id)
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::capability::TerminalTrust;
    use crate::cwd::{CwdSelection, ExplicitCwd};
    use crate::profile::ProfileName;
    use crate::state::{StartFailure, TerminalExitStatus};
    use crate::test_support::TempDir;
    use std::sync::atomic::{AtomicUsize, Ordering};
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
    }

    #[derive(Debug)]
    pub(crate) struct FakeProcess {
        pid: u32,
        counters: FakeCounters,
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
            Ok(ExitReport {
                status: TerminalExitStatus::Signaled(1),
                remaining_processes: 0,
            })
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
            request: &LaunchRequest,
        ) -> Result<Box<dyn TerminalProcess>, LaunchError> {
            self.counters.launches.fetch_add(1, Ordering::SeqCst);
            self.requests.push(request.clone());
            if let Some(err) = self.fail {
                return Err(err);
            }
            let pid = 1000 + u32::try_from(self.requests.len()).unwrap_or(0);
            Ok(Box::new(FakeProcess {
                pid,
                counters: self.counters.clone(),
            }))
        }
    }

    pub(crate) fn fake_env(name: &str) -> Option<OsString> {
        match name {
            "PATH" => Some("/usr/bin:/bin".into()),
            "HOME" => Some("/Users/example".into()),
            "SECRET_TOKEN" => Some("must-not-leak".into()),
            _ => None,
        }
    }

    pub(crate) fn sh() -> ArgvCommand {
        ArgvCommand::new("/bin/sh", Vec::<String>::new()).unwrap()
    }

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
            default_shell: &shell,
            env_lookup: fake_env,
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
            default_shell: &shell,
            env_lookup: fake_env,
        };
        let mut launcher = FakeLauncher::default();
        let mut registry = TerminalRegistry::new(window(13));
        let mut agent = profile("agent");
        agent.command =
            ProfileCommand::Argv(ArgvCommand::new("/usr/bin/env", ["a b", "--flag"]).unwrap());
        agent.cwd = CwdSelection::WorktreeRoot(worktree.clone());
        agent.environment.set("TERM_PROGRAM", "Stellar").unwrap();
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
        assert_eq!(shell_request.program(), shell.program());
        assert!(shell_request.args().is_empty());
        assert_eq!(shell_request.cwd(), dir.path());
        let names: Vec<&OsString> = shell_request.env().iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["HOME", "PATH"]);

        let agent_request = &launcher.requests[1];
        assert_eq!(
            agent_request.program(),
            std::path::Path::new("/usr/bin/env")
        );
        assert_eq!(agent_request.args(), ["a b", "--flag"]);
        assert_eq!(agent_request.cwd(), worktree);
        assert!(agent_request
            .env()
            .iter()
            .any(|(name, value)| name == "TERM_PROGRAM" && value == "Stellar"));
        assert!(agent_request
            .env()
            .iter()
            .all(|(name, _)| name != "SECRET_TOKEN"));
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
            default_shell: &shell,
            env_lookup: fake_env,
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
            default_shell: &shell,
            env_lookup: fake_env,
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
}

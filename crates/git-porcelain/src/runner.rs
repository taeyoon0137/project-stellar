//! 보안 Git 실행기 (IV.3.3).
//!
//! 모든 Git 실행은 [`GitRunner`]를 거칩니다. 핵심 보안·결정성 속성:
//! - **argv 배열**: 인자를 배열로 전달하고 **shell을 거치지 않습니다**. metacharacter
//!   (`;`, `&&`, `$(...)`, `|`, `` ` ``)는 명령으로 해석되지 않고 리터럴 인자로 전달됩니다.
//! - **cwd**: 작업 디렉터리를 명시합니다(상속에 의존하지 않음).
//! - **환경 allowlist**: `env_clear` 후 허용 목록의 변수만 전달합니다.
//! - **timeout**: 제한 시간을 넘으면 자식과 자식 프로세스 tree를 종료합니다.
//! - **취소**: [`GitRunner::run_cancellable`]은 [`GitCancelToken`] 취소를 관찰하면 프로세스 tree를
//!   종료하고 [`GitRunError::Cancelled`]를 반환합니다(XIV.1.4). 부분 출력은 성공으로 돌려주지
//!   않습니다.
//! - **완료 기준**: 실행은 Git이 끝나고 **stdout·stderr·stdin pipe가 모두 닫혀야** 끝납니다. Git이
//!   끝난 뒤에도 pipe를 상속한 손자(hook·credential helper의 백그라운드 프로세스 등)가 남아 있으면
//!   같은 deadline·취소가 계속 적용되고, 그 사이 Git을 회수하지 않으므로 deadline·취소 때 process
//!   group 전체를 종료할 수 있습니다(SEC-2). 종료 뒤에도 pipe가 닫히지 않으면(group을 벗어난 자손)
//!   짧은 유예 뒤 drain 스레드를 분리하고 오류를 반환합니다. pipe를 닫은 자손은 정상 완료 경로에서
//!   기다리지도 종료하지도 않습니다(예: 분리된 자동 gc).
//! - **byte capture**: stdout/stderr를 원시 바이트로 캡처합니다(문자열 가정 없음).
//! - **stdin 입력**: [`GitRunner::run_with_input`]은 바이트를 자식 stdin으로 전달합니다(XIV.8.2).
//!   commit message처럼 사용자 텍스트를 argv·shell escaping 없이 넘길 때 사용합니다. 그 밖의
//!   실행은 stdin을 비워(null) 둡니다.
//! - **spawn 계측**: 실제 프로세스 생성 시도 횟수를 [`GitRunner::spawn_count`]로 셉니다(XIV.1.1).
//!   trust gate가 프로세스를 하나도 만들지 않았음을 검증하는 데 사용합니다.
//! - **trace**: [`GitRunner::with_trace_sink`]로 연결한 sink에 프로세스 생성을 시도한 모든 실행의
//!   [`GitCommandTrace`](명령 종류·소요 시간·종료 상태만)와 redaction된 [`GitDebugLine`]을 보냅니다
//!   (XIV.1.6). URL·branch·경로·commit hash는 trace에 들어가지 않습니다.

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use crate::cancel::{kill_leader_only, terminate_process_tree, GitCancelToken};
use crate::trace::{
    GitCommandKind, GitCommandTrace, GitDebugLine, GitTraceExit, GitTraceSink, TraceSinkHandle,
};

/// stderr 조각을 받는 콜백.
type StderrSink<'a> = &'a mut dyn FnMut(&[u8]);

pub(crate) type InputCheck<'a> =
    &'a mut dyn FnMut(u32, Instant, &[u8]) -> Result<bool, GitRunError>;

struct Input<'a> {
    bytes: &'a [u8],
    check: Option<InputCheck<'a>>,
}

impl<'a> Input<'a> {
    fn immediate(bytes: &'a [u8]) -> Self {
        Self { bytes, check: None }
    }
}

/// 중단(timeout·취소) 뒤 pipe가 닫히기를 더 기다리는 시간. 지나면 drain 스레드를 분리합니다.
const PIPE_DETACH_GRACE: Duration = Duration::from_millis(500);

/// 자식 pipe를 다루는 스레드(stdout·stderr drain, stdin 쓰기).
struct PipeThreads {
    stdout: thread::JoinHandle<Vec<u8>>,
    stderr: thread::JoinHandle<Vec<u8>>,
    stdin: Option<thread::JoinHandle<()>>,
}

impl PipeThreads {
    /// 모든 스레드가 끝났는지. 참이면 자식 tree의 모든 프로세스가 pipe를 닫았습니다.
    fn finished(&self) -> bool {
        self.stdout.is_finished()
            && self.stderr.is_finished()
            && self
                .stdin
                .as_ref()
                .is_none_or(thread::JoinHandle::is_finished)
    }

    /// 끝난 스레드를 모아 stdout·stderr를 돌려줍니다([`PipeThreads::finished`]가 참일 때만 호출).
    fn join(self) -> (Vec<u8>, Vec<u8>) {
        if let Some(handle) = self.stdin {
            let _ = handle.join();
        }
        let stdout = self.stdout.join().unwrap_or_default();
        let stderr = self.stderr.join().unwrap_or_default();
        (stdout, stderr)
    }

    /// 중단 경로: `grace` 동안 pipe가 닫히기를 기다린 뒤 출력을 버립니다. 그래도 닫히지 않으면(group을
    /// 벗어난 자손이 pipe를 잡음) 스레드를 분리합니다. 분리된 스레드는 pipe가 닫히면 스스로 끝납니다.
    fn discard(self, grace: Duration) {
        let deadline = Instant::now() + grace;
        while !self.finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        if self.finished() {
            let _ = self.join();
        }
    }
}

/// Git 실행 결과(원시 바이트 보존).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    /// 종료 코드(시그널 종료면 `None`).
    pub exit_code: Option<i32>,
    /// stdout 원시 바이트.
    pub stdout: Vec<u8>,
    /// stderr 원시 바이트.
    pub stderr: Vec<u8>,
}

impl GitOutput {
    /// 종료 코드가 0이면 성공.
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// Git 실행 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRunError {
    /// 프로세스 생성 실패(executable 없음·권한 등).
    Spawn {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
    /// 제한 시간 초과(자식 프로세스 tree를 종료함).
    Timeout,
    /// 종료 대기 중 io 오류.
    Wait {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
    /// 취소 요청으로 중단함(자식 프로세스 tree를 종료함, XIV.1.4). 부분 출력은 버립니다.
    ///
    /// Git이 중단 전에 저장소를 일부 변경했을 수 있으므로, 호출자는 이 결과를 성공으로 취급하지
    /// 말고 필요하면 저장소 상태를 다시 조회해야 합니다.
    Cancelled,
}

impl std::fmt::Display for GitRunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitRunError::Spawn { kind } => write!(f, "Git 프로세스 생성 실패: {kind:?}"),
            GitRunError::Timeout => f.write_str("Git 실행 제한 시간 초과"),
            GitRunError::Wait { kind } => write!(f, "Git 종료 대기 오류: {kind:?}"),
            GitRunError::Cancelled => f.write_str("Git 실행이 취소되었습니다"),
        }
    }
}

impl std::error::Error for GitRunError {}

/// allowlist 변수 값을 읽어 올 부모 환경.
#[derive(Debug, Clone)]
enum ParentEnv {
    /// 현재 프로세스 환경(`std::env::var_os`).
    Process,
    /// 고정 값(테스트 전용). 병렬 테스트가 프로세스 환경을 바꾸지 않고(`set_var`는 다른 스레드의
    /// `getenv`와 경쟁) allowlist 동작을 검증하는 데 씁니다.
    #[cfg(test)]
    Fixed(Arc<Vec<(OsString, OsString)>>),
}

impl ParentEnv {
    fn var(&self, name: &OsStr) -> Option<OsString> {
        match self {
            ParentEnv::Process => std::env::var_os(name),
            #[cfg(test)]
            ParentEnv::Fixed(vars) => vars
                .iter()
                .rev()
                .find(|(key, _)| key.as_os_str() == name)
                .map(|(_, value)| value.clone()),
        }
    }
}

/// argv·cwd·env allowlist·timeout으로 Git을 실행하는 실행기.
///
/// `Clone`한 실행기는 spawn 계측 카운터를 **공유**합니다(같은 실행기의 사본으로 취급).
#[derive(Debug, Clone)]
pub struct GitRunner {
    git_path: PathBuf,
    env_allowlist: Vec<OsString>,
    env_overrides: Vec<(OsString, OsString)>,
    parent_env: ParentEnv,
    timeout: Duration,
    spawn_count: Arc<AtomicU64>,
    trace_sink: Option<TraceSinkHandle>,
}

impl GitRunner {
    /// Git 경로·환경 allowlist·timeout으로 실행기를 만듭니다.
    pub fn new(git_path: PathBuf, env_allowlist: Vec<OsString>, timeout: Duration) -> GitRunner {
        GitRunner {
            git_path,
            env_allowlist,
            env_overrides: Vec::new(),
            parent_env: ParentEnv::Process,
            timeout,
            spawn_count: Arc::new(AtomicU64::new(0)),
            trace_sink: None,
        }
    }

    /// 실행 trace를 받을 sink를 연결합니다(XIV.1.6). 사본은 같은 sink를 공유합니다.
    ///
    /// sink는 프로세스 생성을 시도한 실행마다 [`GitCommandTrace`]를 받고, 이어서 redaction된
    /// [`GitDebugLine`](실패하면 redaction된 stderr 포함)을 받습니다.
    pub fn with_trace_sink(mut self, sink: Arc<dyn GitTraceSink>) -> GitRunner {
        self.trace_sink = Some(TraceSinkHandle(sink));
        self
    }

    /// allowlist 값을 현재 프로세스 환경 대신 `vars`에서 읽는 사본(테스트 전용, 카운터 공유).
    #[cfg(test)]
    pub(crate) fn with_parent_env_for_test(mut self, vars: Vec<(OsString, OsString)>) -> GitRunner {
        self.parent_env = ParentEnv::Fixed(Arc::new(vars));
        self
    }

    #[cfg(test)]
    pub(crate) fn with_test_executable(mut self, path: PathBuf) -> Self {
        self.git_path = path;
        self
    }

    /// 지금까지 이 실행기(및 사본)가 시도한 프로세스 생성 횟수(XIV.1.1 계측).
    ///
    /// spawn이 실패한 시도도 셉니다. 실행 전에 거부된 요청(trust gate 등)은 세지 않습니다.
    pub fn spawn_count(&self) -> u64 {
        self.spawn_count.load(Ordering::SeqCst)
    }

    /// 자식에 설정할 명시적 환경 변수(allowlist 이후 적용). Git 동작 제어(`GIT_CONFIG_GLOBAL`,
    /// `GIT_TERMINAL_PROMPT=0` 등)에 사용합니다. `env_clear` 기반 보안은 유지됩니다.
    pub fn with_env_overrides(mut self, overrides: Vec<(OsString, OsString)>) -> GitRunner {
        self.env_overrides = overrides;
        self
    }

    /// 기존 override를 유지한 채 override를 **덧붙입니다**(같은 이름이면 나중 값이 이깁니다).
    pub fn with_added_env(mut self, extra: Vec<(OsString, OsString)>) -> GitRunner {
        self.env_overrides.extend(extra);
        self
    }

    /// 환경 allowlist에 이름을 **덧붙입니다**(이미 있으면 그대로). 값은 실행 시 현재 프로세스에서 읽습니다.
    pub fn with_allowed_env(mut self, names: Vec<OsString>) -> GitRunner {
        for name in names {
            if !self.env_allowlist.contains(&name) {
                self.env_allowlist.push(name);
            }
        }
        self
    }

    /// 자식 프로세스가 실제로 보게 될 환경 변수 값입니다(override 우선, 그다음 allowlist에 있는
    /// 부모(현재 프로세스) 값). allowlist에도 override에도 없으면 자식은 그 변수를 보지 못하므로 `None`.
    pub fn child_env_var(&self, name: &str) -> Option<OsString> {
        if let Some((_, value)) = self
            .env_overrides
            .iter()
            .rev()
            .find(|(key, _)| key.as_os_str() == OsStr::new(name))
        {
            return Some(value.clone());
        }
        if self
            .env_allowlist
            .iter()
            .any(|key| key.as_os_str() == OsStr::new(name))
        {
            return self.parent_env.var(OsStr::new(name));
        }
        None
    }

    /// 저장소를 **발견하지 않도록** 고정한 사본을 만듭니다(spawn 계측 카운터 공유).
    ///
    /// 중립 디렉터리에서 실행하는 환경 probe(버전·capability·LFS)용입니다. `GIT_DIR`을 저장소가
    /// 될 수 없는 null device로 지정해 Git의 상위 디렉터리 탐색 자체를 끄고, 그 위에
    /// `GIT_CEILING_DIRECTORIES`를 `neutral_cwd`로 지정해 상위 탐색 경계를 한 번 더 둡니다. 그래서
    /// 임시 디렉터리가 우연히 저장소 안에 있어도 그 저장소의 config(alias·fsmonitor 등)를 읽지
    /// 않습니다.
    pub fn isolated_from_repositories(&self, neutral_cwd: &Path) -> GitRunner {
        self.clone().with_added_env(vec![
            (OsString::from("GIT_DIR"), OsString::from(NULL_DEVICE)),
            (
                OsString::from("GIT_CEILING_DIRECTORIES"),
                neutral_cwd.as_os_str().to_os_string(),
            ),
        ])
    }

    /// 주어진 argv와 cwd로 Git을 실행합니다(shell 미사용). stdout/stderr를 캡처합니다.
    pub fn run(&self, args: &[OsString], cwd: &Path) -> Result<GitOutput, GitRunError> {
        self.run_inner(args, cwd, None, None, None, 0)
    }

    /// `input` 바이트를 자식 stdin으로 전달하며 실행합니다(XIV.8.2). 전달 후 stdin을 닫습니다.
    ///
    /// 입력은 argv나 shell을 거치지 않으므로 quote·escape가 필요 없습니다.
    pub fn run_with_input(
        &self,
        args: &[OsString],
        cwd: &Path,
        input: &[u8],
    ) -> Result<GitOutput, GitRunError> {
        self.run_inner(args, cwd, Some(Input::immediate(input)), None, None, 0)
    }

    /// [`GitRunner::run_with_input`]과 같지만 stdout buffer를 `stdout_capacity` bytes로 미리 잡습니다.
    ///
    /// 출력 크기를 미리 아는 실행(크기를 확인한 `cat-file --batch` 등, SEC-3)이 큰 출력을 받는 동안
    /// buffer를 거듭 키우며 재할당하지 않게 합니다. 출력이 더 크면 평소처럼 늘어나므로 값은 상한이 아니라
    /// 힌트이며, 호출자는 이미 상한을 검사한 크기만 넘겨야 합니다.
    pub fn run_with_input_sized(
        &self,
        args: &[OsString],
        cwd: &Path,
        input: &[u8],
        stdout_capacity: usize,
    ) -> Result<GitOutput, GitRunError> {
        self.run_inner(
            args,
            cwd,
            Some(Input::immediate(input)),
            None,
            None,
            stdout_capacity,
        )
    }

    /// 취소 가능한 실행(XIV.1.4). `cancel`이 취소되면 프로세스 tree를 종료하고
    /// [`GitRunError::Cancelled`]를 반환합니다. 실행 전에 이미 취소되어 있으면 프로세스를 만들지
    /// 않습니다. Git이 취소 관찰 전에 스스로 끝났다면 그 결과를 그대로 반환합니다.
    pub fn run_cancellable(
        &self,
        args: &[OsString],
        cwd: &Path,
        cancel: &GitCancelToken,
    ) -> Result<GitOutput, GitRunError> {
        self.run_inner(args, cwd, None, Some(cancel), None, 0)
    }

    /// 취소 가능하게 실행하며 stderr 조각을 도착하는 대로 `on_stderr`에 전달합니다(XXVI.1.2 진행률).
    ///
    /// 콜백은 호출 스레드에서 호출됩니다. 완료 결과의 `stderr`에도 같은 바이트 전체가 담깁니다.
    /// 취소·timeout이면 [`GitRunner::run_cancellable`]과 같이 tree를 종료하고 오류를 반환합니다.
    pub fn run_streaming_stderr(
        &self,
        args: &[OsString],
        cwd: &Path,
        cancel: &GitCancelToken,
        on_stderr: &mut dyn FnMut(&[u8]),
    ) -> Result<GitOutput, GitRunError> {
        self.run_inner(args, cwd, None, Some(cancel), Some(on_stderr), 0)
    }

    /// stdin을 열어 둔 채 호출자 검사를 반복합니다. 승인 전 EOF도 보내지 않습니다.
    /// 검사는 deadline을 지켜야 하며 오류·취소·timeout에는 기존 process-tree 정리를 사용합니다.
    pub(crate) fn run_with_checked_input(
        &self,
        args: &[OsString],
        cwd: &Path,
        input: &[u8],
        cancel: &GitCancelToken,
        check: InputCheck<'_>,
    ) -> Result<GitOutput, GitRunError> {
        self.run_inner(
            args,
            cwd,
            Some(Input {
                bytes: input,
                check: Some(check),
            }),
            Some(cancel),
            None,
            0,
        )
    }

    /// 하나의 operation deadline 안에서 중첩 읽기를 제한하는 사본입니다.
    pub(crate) fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn run_inner(
        &self,
        args: &[OsString],
        cwd: &Path,
        input: Option<Input<'_>>,
        cancel: Option<&GitCancelToken>,
        on_stderr: Option<StderrSink<'_>>,
        stdout_capacity: usize,
    ) -> Result<GitOutput, GitRunError> {
        // 실행 전 취소: 프로세스를 만들지 않으므로 trace도 남기지 않습니다.
        if cancel.is_some_and(GitCancelToken::is_cancelled) {
            return Err(GitRunError::Cancelled);
        }
        let start = Instant::now();
        let result = self.spawn_and_wait(args, cwd, input, cancel, on_stderr, stdout_capacity);
        self.emit_trace(args, start.elapsed(), &result);
        result
    }

    /// trace sink가 있으면 trace와 redaction된 debug line을 보냅니다.
    fn emit_trace(
        &self,
        args: &[OsString],
        duration: Duration,
        result: &Result<GitOutput, GitRunError>,
    ) {
        let Some(TraceSinkHandle(sink)) = &self.trace_sink else {
            return;
        };
        let exit = match result {
            Ok(output) => match output.exit_code {
                Some(code) => GitTraceExit::Code(code),
                None => GitTraceExit::Signal,
            },
            Err(GitRunError::Spawn { .. }) => GitTraceExit::SpawnFailed,
            Err(GitRunError::Timeout) => GitTraceExit::TimedOut,
            Err(GitRunError::Cancelled) => GitTraceExit::Cancelled,
            Err(GitRunError::Wait { .. }) => GitTraceExit::WaitFailed,
        };
        let trace = GitCommandTrace {
            kind: GitCommandKind::from_argv(args),
            duration,
            exit,
        };
        sink.record(&trace);
        let mut line = GitDebugLine::new(trace, args);
        if let Ok(output) = result {
            if !output.success() && !output.stderr.is_empty() {
                line = line.with_stderr(&output.stderr);
            }
        }
        sink.record_debug(&line);
    }

    fn spawn_and_wait(
        &self,
        args: &[OsString],
        cwd: &Path,
        mut input: Option<Input<'_>>,
        cancel: Option<&GitCancelToken>,
        mut on_stderr: Option<StderrSink<'_>>,
        stdout_capacity: usize,
    ) -> Result<GitOutput, GitRunError> {
        let cancelled = || cancel.is_some_and(GitCancelToken::is_cancelled);

        let mut command = Command::new(&self.git_path);
        command
            .args(args)
            .current_dir(cwd)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // 환경 allowlist: 전부 지우고 허용 변수만 전달.
        command.env_clear();
        for name in &self.env_allowlist {
            if let Some(value) = self.parent_env.var(name) {
                command.env(name, value);
            }
        }
        // 명시적 override(allowlist 이후 적용).
        for (key, value) in &self.env_overrides {
            command.env(key, value);
        }
        // 자식을 새 process group의 leader로 시작해 tree 전체를 종료할 수 있게 합니다.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }

        self.spawn_count.fetch_add(1, Ordering::SeqCst);
        let mut child = command
            .spawn()
            .map_err(|e| GitRunError::Spawn { kind: e.kind() })?;

        // 파이프 버퍼가 가득 차 자식이 막히지 않도록 별도 스레드로 동시 drain.
        let mut stdout_pipe = child.stdout.take().expect("stdout piped");
        let mut stderr_pipe = child.stderr.take().expect("stderr piped");
        let out_thread = thread::spawn(move || {
            let mut buf = Vec::with_capacity(stdout_capacity);
            let _ = stdout_pipe.read_to_end(&mut buf);
            buf
        });
        // streaming이면 stderr 조각을 channel로도 보냅니다(호출 스레드가 콜백 실행).
        let checked_input = input
            .as_ref()
            .is_some_and(|pending| pending.check.is_some());
        let (chunk_tx, chunk_rx) = if on_stderr.is_some() || checked_input {
            let (tx, rx) = mpsc::channel::<Vec<u8>>();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let err_thread = thread::spawn(move || {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                match stderr_pipe.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(tx) = &chunk_tx {
                            let _ = tx.send(chunk[..n].to_vec());
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            buf
        });
        let mut check_stderr = Vec::new();
        let mut capture_overflow = false;
        let mut forward = |rx: &Option<mpsc::Receiver<Vec<u8>>>, captured: &mut Vec<u8>| {
            if let Some(rx) = rx {
                while let Ok(piece) = rx.try_recv() {
                    // 승인 protocol의 초기 stderr만 보관합니다. 초과하면 신호를 놓쳐도 fail closed.
                    if checked_input && !capture_overflow {
                        if captured.len() + piece.len() <= 256 * 1024 {
                            captured.extend_from_slice(&piece);
                        } else {
                            captured.clear();
                            capture_overflow = true;
                        }
                    }
                    if let Some(cb) = on_stderr.as_mut() {
                        cb(&piece);
                    }
                }
            }
        };
        // 승인 전에는 stdin을 이 스레드가 보유합니다. 닫으면 Git이 EOF를 commit 승인으로
        // 해석할 수 있으므로 거부·취소 때도 Git 종료까지 보유합니다.
        let mut held_stdin = child.stdin.take();
        let mut pipes = PipeThreads {
            stdout: out_thread,
            stderr: err_thread,
            stdin: None,
        };

        // 완료는 pipe가 모두 닫히고 leader가 끝난 때입니다. pipe가 열려 있는 동안에는 leader를
        // 조회(회수)하지 않으므로 leader가 먼저 끝나도 zombie로 남아 pgid가 예약됩니다. 그래서
        // deadline·취소 때 pipe를 잡은 손자까지 group 신호로 종료할 수 있습니다(SEC-2).
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            if pipes.finished() {
                match child.try_wait() {
                    Ok(Some(status)) => break status,
                    Ok(None) => {}
                    Err(e) => {
                        // 회수 여부를 알 수 없으므로 group 신호 없이 정리합니다.
                        kill_leader_only(&mut child);
                        pipes.discard(PIPE_DETACH_GRACE);
                        return Err(GitRunError::Wait { kind: e.kind() });
                    }
                }
            }
            let interrupted = if cancelled() {
                Some(GitRunError::Cancelled)
            } else if Instant::now() >= deadline {
                Some(GitRunError::Timeout)
            } else if let Some(pending) = input.as_mut() {
                let approval = match pending.check.as_mut() {
                    Some(check) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        check(child.id(), deadline, &check_stderr)
                    })),
                    None => Ok(Ok(true)),
                };
                match approval {
                    Err(panic) => {
                        terminate_process_tree(&mut child, &|| pipes.finished());
                        drop(held_stdin);
                        pipes.discard(PIPE_DETACH_GRACE);
                        std::panic::resume_unwind(panic);
                    }
                    Ok(Err(error)) => Some(error),
                    Ok(Ok(true)) if !cancelled() && Instant::now() < deadline => {
                        if let Some(mut stdin_pipe) = held_stdin.take() {
                            let bytes = pending.bytes.to_vec();
                            pipes.stdin = Some(thread::spawn(move || {
                                let _ = stdin_pipe.write_all(&bytes);
                            }));
                        }
                        input = None;
                        None
                    }
                    Ok(Ok(_)) => None,
                }
            } else {
                None
            };
            let Some(interrupted) = interrupted else {
                forward(&chunk_rx, &mut check_stderr);
                thread::sleep(Duration::from_millis(5));
                continue;
            };
            // 중단: tree 종료 후 pipe가 닫히면 drain 스레드도 끝납니다. 부분 출력은 버립니다.
            terminate_process_tree(&mut child, &|| pipes.finished());
            drop(held_stdin);
            pipes.discard(PIPE_DETACH_GRACE);
            return Err(interrupted);
        };

        let (stdout, stderr) = pipes.join();
        // drain 스레드가 끝난 뒤 남은 조각까지 전달합니다.
        forward(&chunk_rx, &mut check_stderr);
        Ok(GitOutput {
            exit_code: status.code(),
            stdout,
            stderr,
        })
    }
}

/// 저장소가 될 수 없는 null device 경로(저장소 탐색 차단용 `GIT_DIR` 값).
#[cfg(unix)]
pub const NULL_DEVICE: &str = "/dev/null";

/// 저장소가 될 수 없는 null device 경로(저장소 탐색 차단용 `GIT_DIR` 값).
#[cfg(not(unix))]
pub const NULL_DEVICE: &str = "NUL";

/// `&[&str]`을 argv(`Vec<OsString>`)로 만드는 헬퍼.
pub fn argv<I, S>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter()
        .map(|s| s.as_ref().to_os_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("stellar-runner-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn runner_for(path: &str, timeout_ms: u64) -> GitRunner {
        GitRunner::new(
            PathBuf::from(path),
            Vec::new(),
            Duration::from_millis(timeout_ms),
        )
    }

    #[cfg(unix)]
    #[test]
    fn runs_and_captures_stdout() {
        let dir = TempDir::new();
        let runner = runner_for("/bin/echo", 5_000);
        let out = runner.run(&argv(["hello world"]), &dir.path).unwrap();
        assert!(out.success());
        assert_eq!(out.stdout, b"hello world\n");
    }

    #[cfg(unix)]
    #[test]
    fn shell_metacharacters_are_not_interpreted() {
        // 보안: metacharacter 인자가 shell로 해석되면 파일이 생성됨. argv라 생성되지 않아야 함.
        let dir = TempDir::new();
        let runner = runner_for("/bin/echo", 5_000);
        let injections = [
            "; touch PWNED1",
            "&& touch PWNED2",
            "$(touch PWNED3)",
            "| touch PWNED4",
            "`touch PWNED5`",
            "> PWNED6",
        ];
        let out = runner.run(&argv(injections), &dir.path).unwrap();
        // echo는 인자를 그대로 출력(리터럴).
        assert!(out.success());
        // 어떤 주입 파일도 생성되지 않음.
        for name in ["PWNED1", "PWNED2", "PWNED3", "PWNED4", "PWNED5", "PWNED6"] {
            assert!(
                !dir.path.join(name).exists(),
                "shell 주입으로 {name} 생성됨(보안 결함)"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn env_overrides_are_applied_to_child() {
        let dir = TempDir::new();
        let runner = GitRunner::new(
            PathBuf::from("/usr/bin/env"),
            Vec::new(),
            Duration::from_millis(5_000),
        )
        .with_env_overrides(vec![(
            OsString::from("STELLAR_OVERRIDE"),
            OsString::from("overridden"),
        )]);
        let out = runner.run(&[], &dir.path).unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("STELLAR_OVERRIDE=overridden"),
            "override 미적용: {stdout}"
        );
    }

    /// 부모 환경 fixture(프로세스 환경을 바꾸지 않음 — 병렬 테스트의 `getenv`와 경쟁 없음).
    fn parent_env(vars: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        vars.iter()
            .map(|(k, v)| (OsString::from(k), OsString::from(v)))
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn env_allowlist_filters_environment() {
        let dir = TempDir::new();
        let allowed = "STELLAR_TEST_ALLOWED";
        let runner = GitRunner::new(
            PathBuf::from("/usr/bin/env"),
            vec![OsString::from(allowed)],
            Duration::from_millis(5_000),
        )
        .with_parent_env_for_test(parent_env(&[
            (allowed, "yes-allowed"),
            ("STELLAR_TEST_BLOCKED", "secret-blocked"),
        ]));
        let out = runner.run(&[], &dir.path).unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);

        assert!(
            stdout.contains(&format!("{allowed}=yes-allowed")),
            "허용 변수 누락: {stdout}"
        );
        assert!(
            !stdout.contains("secret-blocked"),
            "차단 변수 노출(보안 결함): {stdout}"
        );
    }

    #[test]
    fn process_parent_env_is_read_without_mutation() {
        // 기본 부모 환경은 현재 프로세스 환경입니다. cargo test는 PATH를 설정하므로 그대로 읽힙니다.
        let runner = GitRunner::new(
            PathBuf::from("/nonexistent"),
            vec![OsString::from("PATH")],
            Duration::from_millis(5_000),
        );
        assert_eq!(runner.child_env_var("PATH"), std::env::var_os("PATH"));
        // fixture 부모 환경은 같은 이름이면 마지막 값을 씁니다.
        let fixed = runner
            .clone()
            .with_parent_env_for_test(parent_env(&[("PATH", "/a"), ("PATH", "/b")]));
        assert_eq!(fixed.child_env_var("PATH"), Some(OsString::from("/b")));
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_long_running_child() {
        let dir = TempDir::new();
        let runner = runner_for("/bin/sleep", 100);
        let start = Instant::now();
        let result = runner.run(&argv(["10"]), &dir.path);
        assert_eq!(result, Err(GitRunError::Timeout));
        // 10초 sleep을 기다리지 않고 빠르게 종료.
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    /// pid 파일에 기록된 pid(기록될 때까지 최대 5초 기다림).
    #[cfg(unix)]
    fn wait_for_pid(pidfile: &Path) -> i32 {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(pid) = std::fs::read_to_string(pidfile)
                .ok()
                .and_then(|text| text.trim().parse().ok())
            {
                return pid;
            }
            assert!(Instant::now() < deadline, "pid 파일이 기록되지 않음");
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// `pid`가 `timeout` 안에 사라지는지.
    #[cfg(unix)]
    fn gone_within(pid: i32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while crate::cancel::process_exists(pid) {
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(20));
        }
        true
    }

    /// 테스트가 남긴 프로세스 정리(실패해도 무시).
    #[cfg(unix)]
    fn kill_leftover(pid: i32) {
        let _ = std::process::Command::new("/bin/kill")
            .args(["-9", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    /// 상속한 stdout/stderr pipe를 잡은 손자를 남기고 바로 끝나는 sh 실행(SEC-2 재현).
    #[cfg(unix)]
    fn background_holder_args(prefix: &str, pidfile: &Path) -> Vec<OsString> {
        let script = format!(
            "{prefix}sleep 12 & echo $! > '{}'; exit 0",
            pidfile.display()
        );
        argv(["-c", script.as_str()])
    }

    #[cfg(unix)]
    #[test]
    fn background_grandchild_holding_pipes_does_not_defeat_timeout() {
        // SEC-2: 직접 자식은 곧바로 끝나도 손자가 pipe를 잡고 있으면 출력이 끝나지 않은 것입니다.
        // 같은 deadline이 적용되어 Timeout이고, 손자도 process group 종료로 사라져야 합니다.
        let dir = TempDir::new();
        let pidfile = dir.path.join("pid");
        let runner = runner_for("/bin/sh", 1_000);
        let start = Instant::now();
        let result = runner.run(&background_holder_args("", &pidfile), &dir.path);
        let elapsed = start.elapsed();
        let pid = wait_for_pid(&pidfile);
        let gone = gone_within(pid, Duration::from_secs(5));
        kill_leftover(pid);
        assert_eq!(result, Err(GitRunError::Timeout));
        assert!(
            elapsed < Duration::from_secs(6),
            "timeout이 늦음: {elapsed:?}"
        );
        assert!(gone, "pipe를 잡은 손자가 남음");
    }

    #[cfg(unix)]
    #[test]
    fn cancel_is_observed_while_grandchild_holds_pipes() {
        let dir = TempDir::new();
        let pidfile = dir.path.join("pid");
        let runner = runner_for("/bin/sh", 60_000);
        let token = GitCancelToken::new();
        let canceller = {
            let token = token.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(300));
                token.cancel();
            })
        };
        let start = Instant::now();
        let result =
            runner.run_cancellable(&background_holder_args("", &pidfile), &dir.path, &token);
        let elapsed = start.elapsed();
        canceller.join().unwrap();
        let pid = wait_for_pid(&pidfile);
        let gone = gone_within(pid, Duration::from_secs(5));
        kill_leftover(pid);
        assert_eq!(result, Err(GitRunError::Cancelled));
        assert!(elapsed < Duration::from_secs(6), "취소가 늦음: {elapsed:?}");
        assert!(gone, "pipe를 잡은 손자가 남음");
    }

    #[cfg(unix)]
    #[test]
    fn sigterm_ignoring_grandchild_holding_pipes_is_killed() {
        // SIGTERM을 무시하는 손자: 유예 시간 뒤 group 전체 SIGKILL로 사라집니다.
        let dir = TempDir::new();
        let pidfile = dir.path.join("pid");
        let runner = runner_for("/bin/sh", 500);
        let start = Instant::now();
        let result = runner.run(
            &background_holder_args("trap '' TERM; ", &pidfile),
            &dir.path,
        );
        let elapsed = start.elapsed();
        let pid = wait_for_pid(&pidfile);
        let gone = gone_within(pid, Duration::from_secs(5));
        kill_leftover(pid);
        assert_eq!(result, Err(GitRunError::Timeout));
        assert!(
            elapsed < Duration::from_secs(6),
            "timeout이 늦음: {elapsed:?}"
        );
        assert!(gone, "SIGTERM을 무시한 손자가 남음");
    }

    #[cfg(unix)]
    #[test]
    fn grandchild_escaping_process_group_does_not_block_return() {
        // setsid로 group을 벗어난 손자는 group 신호로 끝낼 수 없습니다. 그래도 pipe를 기다리며
        // 막히지 않고 짧은 유예 뒤 pipe를 버리고 Timeout을 돌려줘야 합니다.
        let perl = Path::new("/usr/bin/perl");
        if !perl.exists() {
            return; // perl 없는 환경은 건너뜀.
        }
        let dir = TempDir::new();
        let pidfile = dir.path.join("pid");
        let script = "use POSIX; if (fork()) { exit 0 } POSIX::setsid(); \
                      open(my $f, '>', $ARGV[0]) or die; print $f $$; close $f; sleep 12;";
        let runner = runner_for("/usr/bin/perl", 500);
        let start = Instant::now();
        let result = runner.run(&argv(["-e", script, pidfile.to_str().unwrap()]), &dir.path);
        let elapsed = start.elapsed();
        let pid = wait_for_pid(&pidfile);
        kill_leftover(pid);
        assert_eq!(result, Err(GitRunError::Timeout));
        assert!(elapsed < Duration::from_secs(6), "반환이 늦음: {elapsed:?}");
    }

    #[cfg(unix)]
    #[test]
    fn background_child_that_closed_pipes_is_left_alone() {
        // 정책: pipe를 닫은(출력에 관여하지 않는) 자손은 정상 완료 경로에서 기다리지도 종료하지도
        // 않습니다(예: 분리된 자동 gc).
        let dir = TempDir::new();
        let pidfile = dir.path.join("pid");
        let runner = runner_for("/bin/sh", 5_000);
        let start = Instant::now();
        let out = runner
            .run(
                &background_holder_args("exec >/dev/null 2>&1; ", &pidfile),
                &dir.path,
            )
            .unwrap();
        let elapsed = start.elapsed();
        let pid = wait_for_pid(&pidfile);
        let alive = crate::cancel::process_exists(pid);
        kill_leftover(pid);
        assert!(out.success());
        assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
        assert!(alive, "정상 완료 경로가 무관한 자손을 종료함");
    }

    #[cfg(unix)]
    #[test]
    fn post_commit_hook_background_child_does_not_defeat_timeout() {
        // SEC-2 원 시나리오: 사용자 hook이 남긴 백그라운드 프로세스가 pipe를 상속합니다.
        use crate::test_support::{
            hermetic_runner_with_timeout, init_repo_with_identity, system_git,
        };
        use std::os::unix::fs::PermissionsExt;
        let Some(git) = system_git() else {
            return; // git 미설치 환경 건너뜀.
        };
        let dir = TempDir::new();
        let pidfile = dir.path.join("hook-pid");
        let repo = dir.path.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let setup = crate::test_support::hermetic_runner(git.clone());
        init_repo_with_identity(&setup, &repo);
        std::fs::write(repo.join("a.txt"), b"a").unwrap();
        crate::test_support::git_ok(&setup, &repo, ["add", "a.txt"]);
        let hook = repo.join(".git/hooks/post-commit");
        std::fs::write(
            &hook,
            format!(
                "#!/bin/sh\nsleep 12 &\necho $! > '{}'\nexit 0\n",
                pidfile.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

        let runner = hermetic_runner_with_timeout(git, Duration::from_secs(2));
        let start = Instant::now();
        let result = runner.run(&argv(["commit", "-q", "-m", "x"]), &repo);
        let elapsed = start.elapsed();
        let pid = wait_for_pid(&pidfile);
        let gone = gone_within(pid, Duration::from_secs(5));
        kill_leftover(pid);
        assert_eq!(result, Err(GitRunError::Timeout));
        assert!(
            elapsed < Duration::from_secs(8),
            "timeout이 늦음: {elapsed:?}"
        );
        assert!(gone, "hook이 남긴 프로세스가 남음");
    }

    #[cfg(unix)]
    #[test]
    fn spawn_count_tracks_attempts_and_is_shared_by_clones() {
        let dir = TempDir::new();
        let runner = runner_for("/bin/echo", 5_000);
        assert_eq!(runner.spawn_count(), 0);
        runner.run(&argv(["a"]), &dir.path).unwrap();
        let copy = runner.clone();
        copy.run(&argv(["b"]), &dir.path).unwrap();
        // 사본과 원본이 같은 카운터를 공유.
        assert_eq!(runner.spawn_count(), 2);
        assert_eq!(copy.spawn_count(), 2);
        // 실패한 spawn 시도도 셉니다.
        let missing = runner_for("/nonexistent/definitely/not/here", 5_000);
        assert!(missing.run(&[], &dir.path).is_err());
        assert_eq!(missing.spawn_count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn input_bytes_reach_stdin_verbatim_without_shell() {
        let dir = TempDir::new();
        let runner = runner_for("/bin/cat", 5_000);
        let input = "줄1 $(touch PWNED) `id`\n; rm -rf x | y && z\n'\"\n".as_bytes();
        let out = runner.run_with_input(&[], &dir.path, input).unwrap();
        assert!(out.success());
        assert_eq!(out.stdout, input);
        assert!(!dir.path.join("PWNED").exists());
    }

    #[cfg(unix)]
    #[test]
    fn sized_run_presizes_stdout_and_still_grows_past_the_hint() {
        let dir = TempDir::new();
        let runner = runner_for("/bin/cat", 5_000);
        let input = vec![b'z'; 100_000];
        let exact = runner
            .run_with_input_sized(&[], &dir.path, &input, input.len())
            .unwrap();
        assert_eq!(exact.stdout, input);
        // 정확한 크기 힌트면 buffer를 다시 키우지 않습니다.
        assert_eq!(exact.stdout.capacity(), input.len());
        // 힌트보다 큰 출력도 잘리지 않습니다(힌트는 상한이 아님).
        let small = runner
            .run_with_input_sized(&[], &dir.path, &input, 10)
            .unwrap();
        assert_eq!(small.stdout, input);
    }

    #[cfg(unix)]
    #[test]
    fn added_env_keeps_overrides_and_child_env_var_matches_child() {
        let dir = TempDir::new();
        let allowed = "STELLAR_CHILD_ENV_ALLOWED";
        let hidden = "STELLAR_CHILD_ENV_HIDDEN";
        let runner = GitRunner::new(
            PathBuf::from("/usr/bin/env"),
            vec![OsString::from(allowed)],
            Duration::from_millis(5_000),
        )
        .with_parent_env_for_test(parent_env(&[
            (allowed, "from-process"),
            (hidden, "not-passed"),
        ]))
        .with_env_overrides(vec![
            (OsString::from("STELLAR_A"), OsString::from("1")),
            (OsString::from("STELLAR_KEEP"), OsString::from("kept")),
        ])
        .with_added_env(vec![(OsString::from("STELLAR_A"), OsString::from("2"))]);

        assert_eq!(
            runner.child_env_var(allowed),
            Some(OsString::from("from-process"))
        );
        assert_eq!(runner.child_env_var(hidden), None);
        assert_eq!(runner.child_env_var("STELLAR_A"), Some(OsString::from("2")));
        assert_eq!(
            runner.child_env_var("STELLAR_KEEP"),
            Some(OsString::from("kept"))
        );

        // 실제 자식 환경도 같은 값을 봅니다.
        let out = runner.run(&[], &dir.path).unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains(&format!("{allowed}=from-process")),
            "{stdout}"
        );
        assert!(stdout.contains("STELLAR_A=2\n"), "{stdout}");
        assert!(!stdout.contains("STELLAR_A=1\n"), "{stdout}");
        assert!(stdout.contains("STELLAR_KEEP=kept"), "{stdout}");
        assert!(!stdout.contains("not-passed"), "{stdout}");
    }

    #[cfg(unix)]
    #[test]
    fn isolated_runner_sets_discovery_guards_and_shares_counter() {
        let dir = TempDir::new();
        let runner = runner_for("/usr/bin/env", 5_000);
        let isolated = runner.isolated_from_repositories(&dir.path);
        assert_eq!(
            isolated.child_env_var("GIT_DIR"),
            Some(OsString::from(NULL_DEVICE))
        );
        assert_eq!(
            isolated.child_env_var("GIT_CEILING_DIRECTORIES"),
            Some(dir.path.as_os_str().to_os_string())
        );
        // 원본은 바뀌지 않음.
        assert_eq!(runner.child_env_var("GIT_DIR"), None);
        isolated.run(&[], &dir.path).unwrap();
        assert_eq!(runner.spawn_count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn streaming_stderr_delivers_chunks_before_exit_and_keeps_full_output() {
        let dir = TempDir::new();
        let runner = runner_for("/bin/sh", 5_000);
        let script = "printf 'step 1\\r' >&2; sleep 0.3; printf 'step 2\\n' >&2; printf out";
        let mut seen: Vec<(Instant, Vec<u8>)> = Vec::new();
        let start = Instant::now();
        let out = runner
            .run_streaming_stderr(
                &argv(["-c", script]),
                &dir.path,
                &GitCancelToken::new(),
                &mut |chunk| seen.push((Instant::now(), chunk.to_vec())),
            )
            .unwrap();
        assert!(out.success());
        assert_eq!(out.stdout, b"out");
        assert_eq!(out.stderr, b"step 1\rstep 2\n");
        let joined: Vec<u8> = seen.iter().flat_map(|(_, c)| c.clone()).collect();
        assert_eq!(joined, out.stderr);
        // 첫 조각은 프로세스 종료(≥0.3초) 전에 전달됨.
        assert!(seen[0].0.duration_since(start) < Duration::from_millis(300));
    }

    /// trace를 모으는 테스트 sink.
    #[derive(Default)]
    struct CollectSink {
        traces: std::sync::Mutex<Vec<GitCommandTrace>>,
        lines: std::sync::Mutex<Vec<String>>,
    }

    impl GitTraceSink for CollectSink {
        fn record(&self, trace: &GitCommandTrace) {
            self.traces.lock().unwrap().push(*trace);
        }

        fn record_debug(&self, line: &GitDebugLine) {
            self.lines.lock().unwrap().push(line.render());
        }
    }

    #[cfg(unix)]
    #[test]
    fn secret_fixture_is_redacted_from_trace_and_debug_log() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        // stdout·stderr에 argv와 비밀 값을 그대로 출력하고 실패하는 가짜 Git.
        let fake = dir.path.join("fake-git");
        std::fs::write(
            &fake,
            "#!/bin/sh\necho \"$@\"\necho \"fatal: unable to access 'https://alice:ghp_SECRET@github.com/org/private-repo.git/': denied\" >&2\n\
             echo \"error: src refspec 'secret-branch' does not match /Users/alice/Projects/secret at 0123456789abcdef0123456789abcdef01234567\" >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let sink = Arc::new(CollectSink::default());
        let runner = GitRunner::new(fake, Vec::new(), Duration::from_millis(5_000))
            .with_trace_sink(sink.clone());
        let secret_argv = argv([
            "-c",
            "credential.helper=store --file=/Users/alice/secret-store",
            "push",
            "--porcelain",
            "https://alice:ghp_SECRET@github.com/org/private-repo.git",
            "refs/heads/secret-branch:refs/heads/secret-branch",
            "--",
            "/Users/alice/Projects/secret/path.yaml",
            "0123456789abcdef0123456789abcdef01234567",
        ]);
        let out = runner.run(&secret_argv, &dir.path).unwrap();
        assert_eq!(out.exit_code, Some(1));
        // 사본도 같은 sink를 씁니다.
        runner.clone().run(&argv(["status"]), &dir.path).unwrap();
        // 실행 전 취소는 프로세스·trace 없음.
        let token = GitCancelToken::new();
        token.cancel();
        assert_eq!(
            runner.run_cancellable(&argv(["fetch"]), &dir.path, &token),
            Err(GitRunError::Cancelled)
        );
        // spawn 실패도 기록합니다.
        let missing =
            runner_for("/nonexistent/definitely/not/here", 5_000).with_trace_sink(sink.clone());
        assert!(missing
            .run(&argv(["clone", "https://x@y/z"]), &dir.path)
            .is_err());

        let traces = sink.traces.lock().unwrap().clone();
        let kinds: Vec<(GitCommandKind, GitTraceExit)> =
            traces.iter().map(|t| (t.kind, t.exit)).collect();
        assert_eq!(
            kinds,
            vec![
                (GitCommandKind::Push, GitTraceExit::Code(1)),
                (GitCommandKind::Status, GitTraceExit::Code(1)),
                (GitCommandKind::Clone, GitTraceExit::SpawnFailed),
            ]
        );
        let lines = sink.lines.lock().unwrap().clone();
        assert_eq!(lines.len(), 3);
        let everything = format!("{traces:?}\n{}", lines.join("\n"));
        for secret in [
            "ghp_SECRET",
            "alice",
            "github.com",
            "private-repo",
            "secret-branch",
            "secret-store",
            "/Users",
            "path.yaml",
            "0123456789abcdef",
            "credential.helper",
            "x@y",
        ] {
            assert!(
                !everything.contains(secret),
                "{secret:?} 노출: {everything}"
            );
        }
        assert!(
            lines[0].starts_with(
                "git -c <config> push --porcelain <url> <ref> -- <path> <oid> kind=push exit=1"
            ),
            "{}",
            lines[0]
        );
        assert!(
            lines[0].contains("stderr=fatal: unable to access '<redacted>'"),
            "{}",
            lines[0]
        );
    }

    #[test]
    fn missing_executable_is_spawn_error() {
        let dir = TempDir::new();
        let runner = runner_for("/nonexistent/definitely/not/here", 5_000);
        match runner.run(&[], &dir.path) {
            Err(GitRunError::Spawn { .. }) => {}
            other => panic!("예상과 다른 결과: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn captures_nonzero_exit_and_does_not_fake_success() {
        let dir = TempDir::new();
        // /usr/bin/false는 종료 코드 1.
        let runner = runner_for("/usr/bin/false", 5_000);
        let out = runner.run(&[], &dir.path).unwrap();
        assert!(!out.success());
        assert_eq!(out.exit_code, Some(1));
    }

    #[cfg(unix)]
    #[test]
    fn cwd_is_respected() {
        let dir = TempDir::new();
        let runner = runner_for("/bin/pwd", 5_000);
        let out = runner.run(&[], &dir.path).unwrap();
        let printed = String::from_utf8_lossy(&out.stdout);
        let printed = printed.trim();
        // canonicalize로 symlink(예: /var→/private/var) 차이를 흡수해 비교.
        let expected = std::fs::canonicalize(&dir.path).unwrap();
        let actual = std::fs::canonicalize(printed).unwrap();
        assert_eq!(actual, expected);
    }
}

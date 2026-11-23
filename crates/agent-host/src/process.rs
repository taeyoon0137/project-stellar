//! macOS 구독 CLI 실행기 (XXIX.2.4, ADR-0005 §4.3·§6, ADR-0023 결정 9, ADR-0092 결정 5·6).
//!
//! ## 실행 방식
//!
//! - **PTY 없이 파이프**: stdin·stdout·stderr를 파이프로 연결해 비대화형으로 실행합니다(§4.4).
//! - **환경**: [`ChildEnv`] 허용 목록만 전달합니다(`env_clear` 뒤 목록 이름만).
//! - **작업 디렉터리**: 세션마다 OS 사용자 임시 디렉터리(`confstr(_CS_DARWIN_USER_TEMP_DIR)`) 아래에 mode
//!   0700 scratch 디렉터리를 만들고 그 안의 `work/`에서 실행합니다. 프로젝트 working tree를 cwd로 주지 않아
//!   CLI의 프로젝트 설정 탐색과 파일 쓰기가 working tree에 닿지 않습니다(불변식 2). MCP 설정 파일(mode 0600)도
//!   scratch에 두고, 세션이 끝나면 scratch 전체를 지웁니다(실행 흔적 0).
//! - **process group**: `posix_spawn` 계열 `setpgid(0, 0)`(`CommandExt::process_group(0)`)로 CLI를 새 process
//!   group leader로 만들고, 중지·timeout·종료 뒤 group 전체에 signal을 보내 CLI가 띄운 하위 프로세스도 남기지
//!   않습니다.
//! - **중지·timeout**: 취소 요청이나 시작 timeout(첫 stdout byte까지 [`crate::profile::Timeouts::startup_ms`])이면
//!   group에 `SIGTERM`, 유예([`crate::profile::Timeouts::terminate_grace_ms`]) 뒤 `SIGKILL`을 보냅니다.
//! - **수신 정지 없음**: stdout·stderr마다 전용 reader 스레드가 64 KiB 단위로 읽어 [`OutputSink`]에 바로
//!   넘깁니다. UI가 증분을 늦게 가져가도 reader는 멈추지 않고, 메모리는 원문 버퍼 상한이 제한합니다(§6).
//! - **stdin**: 전용 writer 스레드가 씁니다. 앱 프로세스가 `SIGPIPE`로 죽지 않도록 파이프 fd에
//!   `F_SETNOSIGPIPE`(macOS `fcntl(2)`)를 켭니다.
//!
//! Stellar는 이 실행기에서 자격 증명을 다루거나 CLI 설정 위치를 읽지 않습니다.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use stellar_local_ipc::binding::{BindingToken, BINDING_ENV};

use crate::env::ChildEnv;
use crate::json::{self, JsonValue};
use crate::profile::{InputMode, LaunchProfile, PLACEHOLDER_MCP_CONFIG};
use crate::raw::RawStream;
use crate::target::VerifiedTarget;

mod sys {
    extern "C" {
        /// `int kill(pid_t pid, int sig)`.
        pub fn kill(pid: i32, sig: i32) -> i32;
        /// `int fcntl(int fd, int cmd, ...)`.
        pub fn fcntl(fd: i32, cmd: i32, ...) -> i32;
    }
    /// `SIGTERM`.
    pub const SIGTERM: i32 = 15;
    /// `SIGKILL`.
    pub const SIGKILL: i32 = 9;
    /// macOS `<sys/fcntl.h>` `F_SETNOSIGPIPE`.
    pub const F_SETNOSIGPIPE: i32 = 73;
}

/// process group 전체에 signal을 보냅니다. 이미 없으면 무시합니다.
fn signal_group(pgid: u32, signal: i32) {
    if let Ok(pgid) = i32::try_from(pgid) {
        if pgid > 1 {
            // SAFETY: 음수 pid는 process group 대상이며, 인자는 정수뿐입니다.
            unsafe {
                sys::kill(-pgid, signal);
            }
        }
    }
}

/// 프로세스가 살아 있는지(`kill(pid, 0)`). 테스트·흔적 확인용.
pub fn process_alive(pid: u32) -> bool {
    match i32::try_from(pid) {
        // SAFETY: signal 0은 존재 확인만 합니다.
        Ok(pid) if pid > 1 => unsafe { sys::kill(pid, 0) == 0 },
        _ => false,
    }
}

fn no_sigpipe(stdin: &ChildStdin) {
    // SAFETY: 유효한 파이프 fd에 정수 인자 하나를 주는 macOS fcntl 명령입니다.
    unsafe {
        sys::fcntl(stdin.as_raw_fd(), sys::F_SETNOSIGPIPE, 1i32);
    }
}

/// CLI 종료 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitKind {
    /// 종료 코드.
    Code(i32),
    /// signal로 끝남.
    Signal(i32),
}

impl ExitKind {
    fn of(status: ExitStatus) -> ExitKind {
        match status.code() {
            Some(code) => ExitKind::Code(code),
            None => ExitKind::Signal(status.signal().unwrap_or(0)),
        }
    }
}

/// 세션 scratch 디렉터리. drop하면 전부 지웁니다.
#[derive(Debug)]
pub struct ScratchDir {
    root: PathBuf,
}

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

impl ScratchDir {
    /// `base` 아래에 mode 0700 scratch와 `work/`를 만듭니다.
    pub fn create(base: &Path) -> io::Result<ScratchDir> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let name = format!(
            "stellar-agent-{}-{}-{nanos:08x}",
            std::process::id(),
            SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let root = base.join(name);
        std::fs::DirBuilder::new().mode(0o700).create(&root)?;
        let scratch = ScratchDir { root };
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(scratch.work_dir())?;
        Ok(scratch)
    }

    /// OS 사용자 임시 디렉터리 아래에 만듭니다.
    pub fn create_in_user_temp() -> io::Result<ScratchDir> {
        ScratchDir::create(&stellar_local_ipc::os::user_temp_dir()?)
    }

    /// root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// CLI 작업 디렉터리.
    pub fn work_dir(&self) -> PathBuf {
        self.root.join("work")
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// 세션의 Stellar MCP bridge 실행 정보(XIX.1.8 결합 token 포함).
#[derive(Debug)]
pub struct McpServerLaunch {
    /// `stellar-mcp` helper 절대 경로.
    pub command: PathBuf,
    /// helper 인자.
    pub args: Vec<String>,
    /// 세션 결합 token.
    pub binding: BindingToken,
}

/// MCP 설정 파일 이름.
pub const MCP_CONFIG_FILE: &str = "mcp-config.json";

/// MCP 서버 이름(CLI tool 이름 접두어 `mcp__stellar`).
pub const MCP_SERVER_NAME: &str = "stellar";

/// MCP 설정 JSON(`mcpServers.<name>.{type,command,args,env}`, Claude Code `--mcp-config` 형식).
pub fn mcp_config_json(launch: &McpServerLaunch) -> String {
    let server = JsonValue::object(vec![
        ("type", JsonValue::string("stdio")),
        (
            "command",
            JsonValue::string(launch.command.to_string_lossy()),
        ),
        (
            "args",
            JsonValue::Array(launch.args.iter().map(JsonValue::string).collect()),
        ),
        (
            "env",
            JsonValue::object(vec![(
                BINDING_ENV,
                JsonValue::string(launch.binding.to_hex()),
            )]),
        ),
    ]);
    json::write(&JsonValue::object(vec![(
        "mcpServers",
        JsonValue::object(vec![(MCP_SERVER_NAME, server)]),
    )]))
}

fn write_private_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents)
}

fn base_command(program: &Path, args: &[OsString], env: &ChildEnv, cwd: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(env.vars().iter().map(|(k, v)| (k, v)))
        .current_dir(cwd)
        .process_group(0);
    command
}

/// 짧은 확인 명령(버전·연결 상태) 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureOutcome {
    /// 종료 상태. timeout이면 None.
    pub exit: Option<ExitKind>,
    /// timeout으로 끝냈는지.
    pub timed_out: bool,
    /// stdout 앞부분(`max_stdout` byte까지, 나머지는 읽고 버림).
    pub stdout: Vec<u8>,
}

/// 짧은 확인 명령을 실행합니다. stdin은 닫고 stderr는 버리며, 끝나면 process group을 정리합니다.
pub fn run_capture(
    program: &Path,
    args: &[String],
    env: &ChildEnv,
    cwd: &Path,
    timeout: Duration,
    max_stdout: usize,
) -> io::Result<CaptureOutcome> {
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    let mut child = base_command(program, &args, env, cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let pgid = child.id();
    let mut stdout = child.stdout.take().expect("piped stdout");
    let reader = thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = max_stdout.saturating_sub(kept.len());
                    kept.extend_from_slice(&buf[..n.min(room)]);
                }
            }
        }
        kept
    });
    let start = Instant::now();
    let mut timed_out = false;
    let exit = loop {
        if let Some(status) = child.try_wait()? {
            break Some(ExitKind::of(status));
        }
        if start.elapsed() >= timeout {
            timed_out = true;
            signal_group(pgid, sys::SIGKILL);
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(5));
    };
    signal_group(pgid, sys::SIGKILL);
    let stdout = reader.join().unwrap_or_default();
    Ok(CaptureOutcome {
        exit,
        timed_out,
        stdout,
    })
}

/// 실행기가 받은 출력을 넘기는 곳. reader 스레드에서 호출되며 오래 막지 않아야 합니다.
pub trait OutputSink: Send + Sync {
    /// 받은 byte(받은 그대로).
    fn on_output(&self, stream: RawStream, bytes: &[u8]);
    /// stream 끝(EOF).
    fn on_closed(&self, _stream: RawStream) {}
}

/// 세션 프로세스가 끝난 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessEndReason {
    /// CLI가 스스로 끝남.
    Exited,
    /// 시작 timeout(첫 stdout byte 없음).
    StartupTimeout,
    /// 중지 요청.
    Cancelled,
}

/// 세션 프로세스 종료 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessEnd {
    /// 이유.
    pub reason: ProcessEndReason,
    /// 종료 상태.
    pub exit: Option<ExitKind>,
    /// 받은 stdout byte 수.
    pub stdout_bytes: u64,
    /// 받은 stderr byte 수.
    pub stderr_bytes: u64,
}

/// 세션 실행 입력.
pub struct SessionLaunch<'a> {
    /// 확인을 마친 실행 대상.
    pub target: &'a VerifiedTarget,
    /// launch profile.
    pub profile: &'a LaunchProfile,
    /// 허용 목록 환경.
    pub env: ChildEnv,
    /// scratch 디렉터리를 만들 곳(앱은 OS 사용자 임시 디렉터리).
    pub scratch_base: PathBuf,
    /// Stellar MCP bridge(세션 명령에 `{mcp_config}`가 있으면 필수).
    pub mcp: Option<McpServerLaunch>,
    /// 첫 사용자 메시지(그대로 전달).
    pub first_message: String,
    /// 출력 수신처.
    pub sink: Arc<dyn OutputSink>,
}

/// 실행기 오류.
#[derive(Debug)]
pub enum LaunchError {
    /// 실행 대상이 확인 뒤 바뀜.
    TargetChanged,
    /// profile에 MCP 설정 자리가 있는데 bridge 정보가 없음.
    MissingMcp,
    /// OS 오류.
    Io(io::Error),
}

impl From<io::Error> for LaunchError {
    fn from(error: io::Error) -> Self {
        LaunchError::Io(error)
    }
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::TargetChanged => f.write_str("확인 뒤 실행 파일이 바뀌었습니다"),
            LaunchError::MissingMcp => f.write_str("Stellar MCP 설정이 없습니다"),
            LaunchError::Io(e) => write!(f, "CLI를 실행하지 못했습니다: {}", e.kind()),
        }
    }
}

impl std::error::Error for LaunchError {}

/// 사용자 메시지 전달 실패.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// 이 입력 방식은 첫 메시지 뒤 입력을 받지 않습니다.
    NotAccepting,
    /// 세션이 이미 끝났습니다.
    Ended,
}

#[derive(Debug, Default)]
struct Control {
    cancel: AtomicBool,
    first_output: AtomicBool,
}

/// 실행 중인 세션 프로세스.
pub struct SessionProcess {
    pid: u32,
    control: Arc<Control>,
    input: Option<mpsc::Sender<Vec<u8>>>,
    input_mode: InputMode,
    writer: Option<JoinHandle<()>>,
    supervisor: Option<JoinHandle<ProcessEnd>>,
    scratch_root: PathBuf,
}

impl std::fmt::Debug for SessionProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionProcess")
            .field("pid", &self.pid)
            .finish()
    }
}

/// 다른 스레드에서 세션 중지를 요청하는 handle.
#[derive(Debug, Clone)]
pub struct CancelHandle(Arc<Control>);

impl CancelHandle {
    /// 중지를 요청합니다(SIGTERM 뒤 유예 초과 시 SIGKILL).
    pub fn cancel(&self) {
        self.0.cancel.store(true, Ordering::SeqCst);
    }
}

/// stream JSON user message 한 줄(Claude Code `--input-format stream-json`).
pub fn stream_json_user_line(text: &str) -> Vec<u8> {
    let line = JsonValue::object(vec![
        ("type", JsonValue::string("user")),
        (
            "message",
            JsonValue::object(vec![
                ("role", JsonValue::string("user")),
                ("content", JsonValue::string(text)),
            ]),
        ),
    ]);
    let mut bytes = json::write(&line).into_bytes();
    bytes.push(b'\n');
    bytes
}

fn spawn_reader(
    stream: RawStream,
    mut source: impl Read + Send + 'static,
    sink: Arc<dyn OutputSink>,
    control: Arc<Control>,
    counter: Arc<AtomicU64>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match source.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if stream == RawStream::Stdout {
                        control.first_output.store(true, Ordering::SeqCst);
                    }
                    counter.fetch_add(n as u64, Ordering::Relaxed);
                    sink.on_output(stream, &buf[..n]);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        sink.on_closed(stream);
    })
}

impl SessionProcess {
    /// 세션 프로세스를 시작합니다. 확인을 마친 실행 대상([`crate::preflight::PreflightPass`])으로만 부릅니다.
    pub(crate) fn spawn(launch: SessionLaunch<'_>) -> Result<SessionProcess, LaunchError> {
        if !launch.target.still_unchanged() {
            return Err(LaunchError::TargetChanged);
        }
        let scratch = ScratchDir::create(&launch.scratch_base)?;
        let scratch_root = scratch.root().to_path_buf();
        let needs_mcp = launch
            .profile
            .session_command
            .iter()
            .any(|a| a == PLACEHOLDER_MCP_CONFIG);
        let config_path = scratch.root().join(MCP_CONFIG_FILE);
        if needs_mcp {
            let mcp = launch.mcp.as_ref().ok_or(LaunchError::MissingMcp)?;
            write_private_file(&config_path, mcp_config_json(mcp).as_bytes())?;
        }
        let args: Vec<OsString> = launch
            .profile
            .session_command
            .iter()
            .map(|a| {
                if a == PLACEHOLDER_MCP_CONFIG {
                    config_path.clone().into_os_string()
                } else {
                    OsString::from(a)
                }
            })
            .collect();
        let mut child: Child = base_command(
            launch.target.path(),
            &args,
            &launch.env,
            &scratch.work_dir(),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
        let pid = child.id();
        let control = Arc::new(Control::default());
        let stdout_count = Arc::new(AtomicU64::new(0));
        let stderr_count = Arc::new(AtomicU64::new(0));
        let readers = vec![
            spawn_reader(
                RawStream::Stdout,
                child.stdout.take().expect("piped stdout"),
                launch.sink.clone(),
                control.clone(),
                stdout_count.clone(),
            ),
            spawn_reader(
                RawStream::Stderr,
                child.stderr.take().expect("piped stderr"),
                launch.sink.clone(),
                control.clone(),
                stderr_count.clone(),
            ),
        ];

        let mut stdin = child.stdin.take().expect("piped stdin");
        no_sigpipe(&stdin);
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let once = launch.profile.input == InputMode::StdinTextOnce;
        let writer = thread::spawn(move || {
            for chunk in rx {
                if stdin.write_all(&chunk).and_then(|_| stdin.flush()).is_err() {
                    break;
                }
                if once {
                    break;
                }
            }
            drop(stdin);
        });
        let first = match launch.profile.input {
            InputMode::StreamJsonUser => stream_json_user_line(&launch.first_message),
            InputMode::StdinTextOnce => launch.first_message.clone().into_bytes(),
        };
        let _ = tx.send(first);

        let timeouts = launch.profile.timeouts;
        let sup_control = control.clone();
        let supervisor = thread::spawn(move || {
            let start = Instant::now();
            let startup = Duration::from_millis(timeouts.startup_ms);
            let grace = Duration::from_millis(timeouts.terminate_grace_ms);
            let mut reason = ProcessEndReason::Exited;
            let mut term_sent: Option<Instant> = None;
            let exit = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break Some(ExitKind::of(status)),
                    Ok(None) => {}
                    Err(_) => {
                        signal_group(pid, sys::SIGKILL);
                        break child.wait().ok().map(ExitKind::of);
                    }
                }
                let now = Instant::now();
                if term_sent.is_none() {
                    if sup_control.cancel.load(Ordering::SeqCst) {
                        reason = ProcessEndReason::Cancelled;
                    } else if !sup_control.first_output.load(Ordering::SeqCst)
                        && now.duration_since(start) >= startup
                    {
                        reason = ProcessEndReason::StartupTimeout;
                    }
                    if reason != ProcessEndReason::Exited {
                        signal_group(pid, sys::SIGTERM);
                        term_sent = Some(now);
                    }
                } else if let Some(at) = term_sent {
                    if now.duration_since(at) >= grace {
                        signal_group(pid, sys::SIGKILL);
                    }
                }
                thread::sleep(Duration::from_millis(5));
            };
            // CLI가 띄운 하위 프로세스까지 정리합니다(실행 흔적 0).
            signal_group(pid, sys::SIGKILL);
            for reader in readers {
                let _ = reader.join();
            }
            drop(scratch);
            ProcessEnd {
                reason,
                exit,
                stdout_bytes: stdout_count.load(Ordering::Relaxed),
                stderr_bytes: stderr_count.load(Ordering::Relaxed),
            }
        });

        Ok(SessionProcess {
            pid,
            control,
            input: if once { None } else { Some(tx) },
            input_mode: launch.profile.input,
            writer: Some(writer),
            supervisor: Some(supervisor),
            scratch_root,
        })
    }

    /// CLI 프로세스 id(= process group id).
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// 세션 scratch 디렉터리(끝나면 지워짐).
    pub fn scratch_root(&self) -> &Path {
        &self.scratch_root
    }

    /// 중지 handle.
    pub fn cancel_handle(&self) -> CancelHandle {
        CancelHandle(self.control.clone())
    }

    /// 중지를 요청합니다.
    pub fn cancel(&self) {
        self.control.cancel.store(true, Ordering::SeqCst);
    }

    /// 사용자 메시지를 그대로 전달합니다(stream 입력 profile만).
    pub fn send_user_message(&self, text: &str) -> Result<(), SendError> {
        if self.input_mode != InputMode::StreamJsonUser {
            return Err(SendError::NotAccepting);
        }
        let input = self.input.as_ref().ok_or(SendError::Ended)?;
        input
            .send(stream_json_user_line(text))
            .map_err(|_| SendError::Ended)
    }

    /// 끝났는지.
    pub fn is_finished(&self) -> bool {
        self.supervisor.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// 끝날 때까지 기다립니다. stdin을 닫고 reader·writer를 정리한 뒤 결과를 돌려줍니다.
    pub fn wait(mut self) -> ProcessEnd {
        let end = self
            .supervisor
            .take()
            .expect("supervisor")
            .join()
            .expect("supervisor 스레드 panic");
        self.input = None;
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        end
    }
}

impl Drop for SessionProcess {
    fn drop(&mut self) {
        if self.supervisor.is_some() {
            self.cancel();
            self.input = None;
            if let Some(sup) = self.supervisor.take() {
                let _ = sup.join();
            }
            if let Some(writer) = self.writer.take() {
                let _ = writer.join();
            }
        }
    }
}

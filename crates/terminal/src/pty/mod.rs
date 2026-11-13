//! macOS POSIX PTY 실행기 (XVIII.2.1, XVIII.2.3).
//!
//! **PTY 생성**: `posix_openpt(O_RDWR | O_NOCTTY | O_CLOEXEC)` → `grantpt` → `unlockpt` →
//! `ptsname_r` → slave를 `open(O_RDWR | O_NOCTTY | O_CLOEXEC)`. master·slave fd는 만들 때부터
//! close-on-exec라, 다른 thread가 같은 순간에 만든 child process에 exec 뒤까지 새지 않습니다. 초기
//! 크기는 master에 `TIOCSWINSZ`로 설정합니다.
//!
//! **child 실행**: `std::process::Command`로 argv 배열을 그대로 exec합니다(셸 문자열 해석 없음).
//! 환경은 모두 지운 뒤 요청 환경(허용 목록 적용 결과)만 넣고, cwd는 검증된 canonical 경로입니다.
//! stdin·stdout·stderr는 slave 복제본입니다. fork 뒤 exec 전(`pre_exec`)에 `setsid()`로 새 세션과
//! process group의 leader가 되고, `TIOCSCTTY`로 slave를 controlling terminal로 만듭니다. 이
//! closure는 fork된 child에서 돌기 때문에 async-signal-safe 시스템 호출(`setsid`, `ioctl`)만
//! 부릅니다.
//!
//! **출력 끝(EOF)과 slave 보관**: macOS에서는 child가 slave의 **마지막** 참조를 닫는 순간 master가
//! 아직 읽지 않은 출력이 버려질 수 있습니다(`/bin/echo` 400회 실측에서 9~12회 유실). 그래서 부모가
//! slave fd 하나를 계속 쥐고 있다가, leader가 끝나면 출력 queue(`TIOCOUTQ`)가 빌 때까지(상한
//! [`DRAIN_TIMEOUT`]) 기다린 뒤 닫습니다(같은 실측에서 유실 0). 부모가 slave를 닫은 뒤에 master
//! 읽기가 EOF가 됩니다. 사용자가 세션을 닫는 경우([`TerminalProcess::terminate`])에는 기다리지 않고
//! 바로 닫습니다.
//!
//! **크기**: 초기 크기와 resize는 master에 `TIOCSWINSZ`로 전달합니다. 커널이 새 크기를 저장하고
//! foreground process group에 `SIGWINCH`를 보내며, child는 `TIOCGWINSZ`(예: `stty size`)로 새 크기를
//! 읽습니다. [`TerminalProcess::size`]는 master의 `TIOCGWINSZ` 값입니다.
//!
//! **종료**: 세션 leader의 process group(leader pid와 같음)에 `SIGHUP`(터미널 hangup)과
//! `SIGCONT`(정지된 job 깨우기)를 보내고, 유예 시간 안에 끝나지 않으면 `SIGKILL`을 보낸 뒤 leader를
//! 회수합니다. group 신호는 leader를 **회수하기 전에만** 보냅니다(회수 전에는 pid가 예약되어 pgid가
//! 다른 group에 재사용될 수 없음).

mod sys;

use std::ffi::{c_int, CStr};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::launch::{
    ExitReport, LaunchError, LaunchRequest, TerminalIoError, TerminalLauncher, TerminalProcess,
};
use crate::size::TerminalSize;
use crate::state::TerminalExitStatus;

/// 종료 요청(`SIGHUP`) 뒤 `SIGKILL`까지의 기본 유예 시간.
pub const DEFAULT_TERMINATE_GRACE: Duration = Duration::from_millis(1_500);

/// leader가 스스로 끝난 뒤 남은 출력을 reader가 읽어 가기를 기다리는 상한.
pub const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

/// 종료 대기 중 상태 확인 간격.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// macOS POSIX PTY 실행기.
///
/// `Clone`한 실행기는 spawn 계측 카운터를 공유합니다.
#[derive(Debug, Clone)]
pub struct PtyLauncher {
    grace: Duration,
    spawn_count: Arc<AtomicU64>,
}

impl PtyLauncher {
    /// 기본 유예 시간의 실행기.
    pub fn new() -> PtyLauncher {
        PtyLauncher::with_grace(DEFAULT_TERMINATE_GRACE)
    }

    /// 종료 유예 시간을 지정한 실행기.
    pub fn with_grace(grace: Duration) -> PtyLauncher {
        PtyLauncher {
            grace,
            spawn_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 이 실행기가 child 생성을 시도한 횟수(PTY 생성 실패 제외).
    pub fn spawn_count(&self) -> u64 {
        self.spawn_count.load(Ordering::SeqCst)
    }
}

impl Default for PtyLauncher {
    fn default() -> Self {
        PtyLauncher::new()
    }
}

impl TerminalLauncher for PtyLauncher {
    fn launch(&mut self, request: &LaunchRequest) -> Result<Box<dyn TerminalProcess>, LaunchError> {
        let pair = PtyPair::open(request.size()).map_err(LaunchError::Pty)?;
        self.spawn_count.fetch_add(1, Ordering::SeqCst);
        let process = PtyProcess::spawn(pair, request, self.grace).map_err(LaunchError::Spawn)?;
        Ok(Box::new(process))
    }
}

/// 열린 PTY master·slave.
struct PtyPair {
    master: OwnedFd,
    slave: OwnedFd,
}

impl PtyPair {
    fn open(size: TerminalSize) -> Result<PtyPair, TerminalIoError> {
        // SAFETY: posix_openpt는 정수 flag만 받고 새 fd(또는 -1)를 돌려줍니다.
        let raw = unsafe { sys::posix_openpt(sys::O_RDWR | sys::O_NOCTTY | sys::O_CLOEXEC) };
        if raw < 0 {
            return Err(TerminalIoError::last_os_error("posix_openpt"));
        }
        // SAFETY: raw는 방금 열린, 다른 곳이 소유하지 않는 fd입니다.
        let master = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: master는 열린 PTY master fd입니다. grantpt는 fd만 받습니다.
        if unsafe { sys::grantpt(master.as_raw_fd()) } != 0 {
            return Err(TerminalIoError::last_os_error("grantpt"));
        }
        // SAFETY: 위와 같습니다.
        if unsafe { sys::unlockpt(master.as_raw_fd()) } != 0 {
            return Err(TerminalIoError::last_os_error("unlockpt"));
        }
        let mut name = [0 as std::ffi::c_char; 128];
        // SAFETY: buffer는 name 배열 전체이고 길이를 함께 넘깁니다. ptsname_r는 성공하면 NUL로
        // 끝나는 경로를 buffer 안에 쓰고, 공간이 모자라면 ERANGE를 돌려줍니다.
        let rc = unsafe { sys::ptsname_r(master.as_raw_fd(), name.as_mut_ptr(), name.len()) };
        if rc != 0 {
            return Err(TerminalIoError::from_io(
                "ptsname_r",
                &io::Error::from_raw_os_error(rc),
            ));
        }
        // c_char(i8)를 byte로 바꿔 NUL까지 안전하게 읽습니다(NUL이 없으면 오류).
        let bytes = name.map(|c| c as u8);
        let path = CStr::from_bytes_until_nul(&bytes).map_err(|_| TerminalIoError {
            operation: "ptsname_r",
            kind: io::ErrorKind::InvalidData,
            os_code: None,
        })?;
        // SAFETY: path는 NUL로 끝나는 유효한 C 문자열이고 O_CREAT가 없으므로 open의 추가 인자는
        // 읽히지 않습니다.
        let raw = unsafe { sys::open(path.as_ptr(), sys::O_RDWR | sys::O_NOCTTY | sys::O_CLOEXEC) };
        if raw < 0 {
            return Err(TerminalIoError::last_os_error("open slave"));
        }
        // SAFETY: raw는 방금 열린, 다른 곳이 소유하지 않는 fd입니다.
        let slave = unsafe { OwnedFd::from_raw_fd(raw) };
        set_winsize(&master, size)?;
        Ok(PtyPair { master, slave })
    }
}

/// master에 크기를 설정합니다(foreground process group에 `SIGWINCH`가 갑니다).
fn set_winsize(master: &impl AsRawFd, size: TerminalSize) -> Result<(), TerminalIoError> {
    let winsize = sys::Winsize {
        ws_row: size.rows(),
        ws_col: size.cols(),
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCSWINSZ는 struct winsize 포인터 하나를 읽습니다. winsize는 호출 동안 살아 있습니다.
    let rc = unsafe {
        sys::ioctl(
            master.as_raw_fd(),
            sys::TIOCSWINSZ,
            &winsize as *const sys::Winsize,
        )
    };
    if rc != 0 {
        return Err(TerminalIoError::last_os_error("ioctl(TIOCSWINSZ)"));
    }
    Ok(())
}

/// master의 현재 크기(`TIOCGWINSZ`).
fn get_winsize(master: &impl AsRawFd) -> Result<TerminalSize, TerminalIoError> {
    let mut winsize = sys::Winsize::default();
    // SAFETY: TIOCGWINSZ는 struct winsize 포인터 하나에 값을 씁니다. winsize는 호출 동안 살아 있습니다.
    let rc = unsafe {
        sys::ioctl(
            master.as_raw_fd(),
            sys::TIOCGWINSZ,
            &mut winsize as *mut sys::Winsize,
        )
    };
    if rc != 0 {
        return Err(TerminalIoError::last_os_error("ioctl(TIOCGWINSZ)"));
    }
    TerminalSize::new(winsize.ws_row, winsize.ws_col).map_err(|_| TerminalIoError {
        operation: "ioctl(TIOCGWINSZ)",
        kind: io::ErrorKind::InvalidData,
        os_code: None,
    })
}

/// slave 출력 queue가 비었는지(`TIOCOUTQ`). 조회가 실패하면(revoke 등) 더 기다릴 것이 없으므로
/// 빈 것으로 봅니다.
fn output_drained(slave: &impl AsRawFd) -> bool {
    let mut pending: c_int = 0;
    // SAFETY: TIOCOUTQ는 int 포인터 하나에 queue 길이를 씁니다. pending은 호출 동안 살아 있습니다.
    let rc = unsafe { sys::ioctl(slave.as_raw_fd(), sys::TIOCOUTQ, &mut pending as *mut c_int) };
    rc != 0 || pending <= 0
}

/// leader 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leader {
    /// 아직 회수하지 않음(pid 예약, group 신호 가능).
    Unreaped,
    /// 회수함. 남은 출력을 읽어 가기를 기다리는 중일 수 있습니다.
    Reaped {
        report: ExitReport,
        drain_deadline: Instant,
    },
}

/// PTY에 붙은 터미널 child(세션 leader).
#[derive(Debug)]
pub struct PtyProcess {
    child: Child,
    pid: i32,
    master: File,
    /// 출력 유실을 막으려고 부모가 쥐고 있는 slave(끝나면 닫음).
    slave: Option<OwnedFd>,
    grace: Duration,
    leader: Leader,
    /// 출력 정리까지 끝난 최종 결과.
    finished: Option<ExitReport>,
    kill_deadline: Option<Instant>,
}

impl PtyProcess {
    fn spawn(
        pair: PtyPair,
        request: &LaunchRequest,
        grace: Duration,
    ) -> Result<PtyProcess, TerminalIoError> {
        let PtyPair { master, slave } = pair;
        let stdio = |slave: &OwnedFd| {
            slave
                .try_clone()
                .map(Stdio::from)
                .map_err(|err| TerminalIoError::from_io("dup slave", &err))
        };
        let mut command = Command::new(request.program());
        if let Some(arg0) = request.arg0() {
            command.arg0(arg0);
        }
        command
            .args(request.args())
            .current_dir(request.cwd())
            .env_clear()
            .envs(request.env().iter().map(|(name, value)| (name, value)))
            .stdin(stdio(&slave)?)
            .stdout(stdio(&slave)?)
            .stderr(stdio(&slave)?);
        // SAFETY: closure는 fork된 child에서 exec 전에 실행됩니다. async-signal-safe인 setsid와
        // ioctl만 호출하고 메모리 할당·lock을 하지 않습니다(`last_os_error`는 errno만 읽음). 이
        // 시점에 fd 0은 slave 복제본입니다.
        unsafe {
            command.pre_exec(|| {
                if sys::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if sys::ioctl(0, sys::TIOCSCTTY, 0 as c_int) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let spawned = command.spawn();
        // Command가 들고 있는 slave 복제본을 닫습니다(원본 하나는 출력 정리용으로 보관).
        drop(command);
        let child = spawned.map_err(|err| TerminalIoError::from_io("spawn", &err))?;
        let pid = i32::try_from(child.id()).map_err(|_| TerminalIoError {
            operation: "spawn",
            kind: io::ErrorKind::InvalidData,
            os_code: None,
        })?;
        Ok(PtyProcess {
            child,
            pid,
            master: File::from(master),
            slave: Some(slave),
            grace,
            leader: Leader::Unreaped,
            finished: None,
            kill_deadline: None,
        })
    }

    /// leader의 process group 전체에 신호를 보냅니다. leader를 회수하기 전에만 보냅니다.
    fn signal_group(&self, signal: c_int) {
        if self.leader != Leader::Unreaped || self.pid <= 1 {
            return;
        }
        // SAFETY: kill(2)는 정수 인자만 받습니다. pid > 1이고 leader를 아직 회수하지 않았으므로
        // -pid는 이 세션 leader가 만든 process group만 가리킵니다.
        let _ = unsafe { sys::kill(-self.pid, signal) };
    }

    /// 비차단으로 leader를 회수해 봅니다.
    fn try_reap(&mut self) -> Result<(), TerminalIoError> {
        if self.leader != Leader::Unreaped {
            return Ok(());
        }
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.mark_reaped(status);
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(err) => Err(TerminalIoError::from_io("waitpid", &err)),
        }
    }

    fn mark_reaped(&mut self, status: ExitStatus) {
        self.leader = Leader::Reaped {
            report: ExitReport {
                status: exit_status(status),
                remaining_processes: 0,
            },
            drain_deadline: Instant::now() + DRAIN_TIMEOUT,
        };
    }

    /// 보관한 slave를 닫아 출력을 끝내고 최종 결과를 기록합니다.
    fn finish(&mut self, report: ExitReport) -> ExitReport {
        self.slave = None;
        self.finished = Some(report);
        report
    }
}

fn exit_status(status: ExitStatus) -> TerminalExitStatus {
    match (status.code(), status.signal()) {
        (Some(code), _) => TerminalExitStatus::Exited(code),
        (None, Some(signal)) => TerminalExitStatus::Signaled(signal),
        // exit code도 신호도 없는 상태는 wait 결과로 나오지 않지만, 비정상 종료로 기록합니다.
        (None, None) => TerminalExitStatus::Exited(-1),
    }
}

impl TerminalProcess for PtyProcess {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn poll_exit(&mut self) -> Result<Option<ExitReport>, TerminalIoError> {
        if let Some(report) = self.finished {
            return Ok(Some(report));
        }
        self.try_reap()?;
        match self.leader {
            Leader::Unreaped => Ok(None),
            Leader::Reaped {
                report,
                drain_deadline,
            } => {
                let drained = self.slave.as_ref().is_none_or(output_drained);
                if drained || Instant::now() >= drain_deadline {
                    Ok(Some(self.finish(report)))
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn begin_terminate(&mut self) {
        if self.finished.is_some() || self.kill_deadline.is_some() {
            return;
        }
        self.signal_group(sys::SIGHUP);
        self.signal_group(sys::SIGCONT);
        self.kill_deadline = Some(Instant::now() + self.grace);
    }

    fn terminate(&mut self) -> Result<ExitReport, TerminalIoError> {
        if let Some(report) = self.finished {
            return Ok(report);
        }
        self.begin_terminate();
        let deadline = self.kill_deadline.unwrap_or_else(Instant::now);
        while self.leader == Leader::Unreaped && Instant::now() < deadline {
            self.try_reap()?;
            if self.leader == Leader::Unreaped {
                thread::sleep(POLL_INTERVAL);
            }
        }
        if self.leader == Leader::Unreaped {
            self.signal_group(sys::SIGKILL);
            let status = self
                .child
                .wait()
                .map_err(|err| TerminalIoError::from_io("waitpid", &err))?;
            self.mark_reaped(status);
        }
        match self.leader {
            // 닫는 세션의 남은 출력은 기다리지 않습니다.
            Leader::Reaped { report, .. } => Ok(self.finish(report)),
            Leader::Unreaped => Err(TerminalIoError {
                operation: "waitpid",
                kind: io::ErrorKind::Other,
                os_code: None,
            }),
        }
    }

    fn write_input(&mut self, bytes: &[u8]) -> Result<(), TerminalIoError> {
        self.master
            .write_all(bytes)
            .map_err(|err| TerminalIoError::from_io("write", &err))
    }

    fn output_reader(&self) -> Result<Box<dyn Read + Send>, TerminalIoError> {
        let reader = self
            .master
            .try_clone()
            .map_err(|err| TerminalIoError::from_io("dup master", &err))?;
        Ok(Box::new(reader))
    }

    fn resize(&mut self, size: TerminalSize) -> Result<(), TerminalIoError> {
        set_winsize(&self.master, size)
    }

    fn size(&self) -> Result<TerminalSize, TerminalIoError> {
        get_winsize(&self.master)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::test_support::{OutputCollector, TempDir};
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    pub(crate) const TIMEOUT: Duration = Duration::from_secs(10);

    pub(crate) fn request_with_env(
        program: &str,
        args: &[&str],
        cwd: &Path,
        env: Vec<(OsString, OsString)>,
    ) -> LaunchRequest {
        LaunchRequest::new(
            PathBuf::from(program),
            args.iter().map(OsString::from).collect(),
            None,
            cwd.to_path_buf(),
            env,
            TerminalSize::DEFAULT,
        )
    }

    pub(crate) fn request(program: &str, args: &[&str], cwd: &Path) -> LaunchRequest {
        request_with_env(
            program,
            args,
            cwd,
            vec![(OsString::from("PATH"), OsString::from("/usr/bin:/bin"))],
        )
    }

    fn launch(request: &LaunchRequest) -> Box<dyn TerminalProcess> {
        PtyLauncher::with_grace(Duration::from_millis(500))
            .launch(request)
            .unwrap()
    }

    pub(crate) fn wait_exit(process: &mut dyn TerminalProcess) -> ExitReport {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(report) = process.poll_exit().unwrap() {
                return report;
            }
            assert!(Instant::now() < deadline, "child did not exit");
            thread::sleep(POLL_INTERVAL);
        }
    }

    /// 실행해 끝날 때까지 기다리고 (종료 결과, 전체 출력)을 돌려줍니다.
    fn run_to_end(request: &LaunchRequest) -> (ExitReport, String) {
        let mut process = launch(request);
        let output = OutputCollector::start(process.output_reader().unwrap());
        let report = wait_exit(process.as_mut());
        (report, output.wait_eof(TIMEOUT))
    }

    #[test]
    fn echo_output_fixture() {
        let dir = TempDir::new();
        // 출력이 빠지지 않는지 여러 번 확인합니다(slave 보관·drain, 모듈 문서).
        for _ in 0..20 {
            let (report, text) = run_to_end(&request(
                "/bin/echo",
                &["hello", "stellar terminal", "'quoted' $HOME"],
                dir.path(),
            ));
            assert_eq!(report.status, TerminalExitStatus::Exited(0));
            // PTY line discipline(ONLCR)이 개행을 CRLF로 바꿉니다.
            assert_eq!(text, "hello stellar terminal 'quoted' $HOME\r\n");
        }
    }

    #[test]
    fn child_gets_only_requested_environment_and_cwd() {
        let dir = TempDir::new();
        let env = vec![
            (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
            (OsString::from("STELLAR_TEST"), OsString::from("a b=c")),
        ];
        let (report, text) = run_to_end(&request_with_env("/usr/bin/env", &[], dir.path(), env));
        assert_eq!(report.status, TerminalExitStatus::Exited(0));
        let mut lines: Vec<&str> = text
            .lines()
            .map(|line| line.trim_end_matches('\r'))
            .collect();
        lines.sort_unstable();
        assert_eq!(lines, ["PATH=/usr/bin:/bin", "STELLAR_TEST=a b=c"]);

        let (_, text) = run_to_end(&request("/bin/pwd", &[], dir.path()));
        assert_eq!(text, format!("{}\r\n", dir.path().display()));
    }

    #[test]
    fn child_is_session_leader_with_pty_as_controlling_terminal() {
        let dir = TempDir::new();
        // `tty`는 stdin이 터미널이면 그 이름을 출력합니다. `/dev/tty`를 열 수 있으면 controlling
        // terminal이 있다는 뜻입니다.
        let (report, text) = run_to_end(&request(
            "/bin/sh",
            &["-c", "/usr/bin/tty; exec 3</dev/tty && echo ctty-ok"],
            dir.path(),
        ));
        assert_eq!(report.status, TerminalExitStatus::Exited(0));
        assert!(text.starts_with("/dev/ttys"), "{text:?}");
        assert!(text.contains("ctty-ok"), "{text:?}");
    }

    #[test]
    fn interactive_input_and_terminate() {
        let dir = TempDir::new();
        let mut process = launch(&request("/bin/cat", &[], dir.path()));
        let mut output = OutputCollector::start(process.output_reader().unwrap());
        process.write_input(b"ping\n").unwrap();
        // tty echo와 cat 출력이 함께 옵니다.
        output.wait_for("ping\r\nping\r\n", TIMEOUT);
        assert_eq!(process.poll_exit().unwrap(), None);
        let report = process.terminate().unwrap();
        assert_eq!(report.status, TerminalExitStatus::Signaled(sys::SIGHUP));
        // 종료 뒤 다시 호출해도 같은 결과이고, 출력은 EOF로 끝납니다.
        assert_eq!(process.terminate().unwrap(), report);
        assert_eq!(process.poll_exit().unwrap(), Some(report));
        output.wait_eof(TIMEOUT);
    }

    #[test]
    fn missing_program_is_a_spawn_failure() {
        let dir = TempDir::new();
        let mut launcher = PtyLauncher::new();
        let err = launcher
            .launch(&request("/nonexistent/stellar-shell", &[], dir.path()))
            .unwrap_err();
        assert!(
            matches!(err, LaunchError::Spawn(io) if io.kind == io::ErrorKind::NotFound),
            "{err:?}"
        );
        assert_eq!(launcher.spawn_count(), 1);
    }

    fn request_sized(
        program: &str,
        args: &[&str],
        cwd: &Path,
        size: TerminalSize,
    ) -> LaunchRequest {
        LaunchRequest::new(
            PathBuf::from(program),
            args.iter().map(OsString::from).collect(),
            None,
            cwd.to_path_buf(),
            vec![(OsString::from("PATH"), OsString::from("/usr/bin:/bin"))],
            size,
        )
    }

    #[test]
    fn initial_size_fixture() {
        let dir = TempDir::new();
        for (rows, cols) in [(24, 80), (33, 101), (1, 1), (200, 500)] {
            let size = TerminalSize::new(rows, cols).unwrap();
            let (report, text) =
                run_to_end(&request_sized("/bin/stty", &["size"], dir.path(), size));
            assert_eq!(report.status, TerminalExitStatus::Exited(0));
            assert_eq!(text, format!("{rows} {cols}\r\n"));
        }
    }

    #[test]
    fn resize_fixture_columns_and_rows() {
        let dir = TempDir::new();
        let mut process = launch(&request(
            "/bin/sh",
            &["-c", "read line; /bin/stty size"],
            dir.path(),
        ));
        let output = OutputCollector::start(process.output_reader().unwrap());
        assert_eq!(process.size().unwrap(), TerminalSize::DEFAULT);
        let size = TerminalSize::new(50, 132).unwrap();
        process.resize(size).unwrap();
        assert_eq!(process.size().unwrap(), size);
        // child가 새 크기를 읽습니다.
        process.write_input(b"\n").unwrap();
        let report = wait_exit(process.as_mut());
        assert_eq!(report.status, TerminalExitStatus::Exited(0));
        assert!(output.wait_eof(TIMEOUT).ends_with("50 132\r\n"));
    }

    #[test]
    fn resize_delivers_sigwinch_to_foreground_job() {
        let dir = TempDir::new();
        let mut process = launch(&request(
            "/bin/sh",
            &[
                "-c",
                "trap '/bin/stty size; exit 0' WINCH; echo ready; while :; do /bin/sleep 0.05; done",
            ],
            dir.path(),
        ));
        let mut output = OutputCollector::start(process.output_reader().unwrap());
        output.wait_for("ready", TIMEOUT);
        process.resize(TerminalSize::new(40, 100).unwrap()).unwrap();
        output.wait_for("40 100\r\n", TIMEOUT);
        let report = wait_exit(process.as_mut());
        assert_eq!(report.status, TerminalExitStatus::Exited(0));
    }
}

//! macOS POSIX PTY 실행기 (XVIII.2.1, XVIII.2.3, XVIII.2.4).
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
//! **프로세스 tree 정리**(XVIII.2.4): 정리 대상은 leader의 process group만이 아니라 **세션 전체**
//! (job control로 다른 process group에 있는 job, `nohup`처럼 `SIGHUP`을 무시하는 프로세스, leader가
//! 먼저 끝난 뒤 남은 background 프로세스 포함)입니다.
//!
//! 1. leader 종료는 `waitid(WEXITED | WNOHANG | WNOWAIT)`로 **회수하지 않고** 확인합니다. leader를
//!    회수하기 전까지 그 pid는 예약되어 있으므로, pgid·세션 id(= leader pid)가 다른 프로세스에
//!    재사용될 수 없습니다. 그래서 group 신호와 세션 열거는 회수 전에만 합니다.
//! 2. 세션 열거: `proc_listpids(PROC_ALL_PIDS)`의 각 pid에 `getsid(pid) == leader pid`인 것을
//!    고릅니다(이 프로세스·pid 1 이하 제외, 상한 2^18개).
//! 3. hangup: leader group과 세션 프로세스에 `SIGHUP`+`SIGCONT` → 유예 시간([`PtyLauncher::with_grace`],
//!    기본 1.5초) → 남은 프로세스에 `SIGKILL`(정리 중 새로 생긴 프로세스 포함 반복) → 세션이 비면
//!    leader 회수. `SIGKILL` 뒤 [`KILL_WAIT`] 안에도 끝나지 않는 프로세스는 수를
//!    [`ExitReport::remaining_processes`]로 보고합니다.
//! 4. 경로별: child 스스로 종료·외부 신호로 종료 → [`TerminalProcess::poll_exit`]가 비차단 단계로
//!    정리. 닫기·취소·앱 종료 → [`TerminalProcess::terminate`](차단, 유예 + [`KILL_WAIT`] 상한).
//!    정리하지 않은 `PtyProcess`를 drop하면 `terminate`를 실행합니다.

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

/// `SIGKILL` 뒤 세션이 빌 때까지 기다리는 상한.
pub const KILL_WAIT: Duration = Duration::from_secs(2);

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
    fn launch(&mut self, request: LaunchRequest) -> Result<Box<dyn TerminalProcess>, LaunchError> {
        let pair = PtyPair::open(request.size()).map_err(LaunchError::Pty)?;
        self.spawn_count.fetch_add(1, Ordering::SeqCst);
        let process = PtyProcess::spawn(pair, &request, self.grace).map_err(LaunchError::Spawn)?;
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
        let master = open_master()?;
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

/// `posix_openpt` 일시 실패 재시도 횟수.
const OPEN_MASTER_ATTEMPTS: u32 = 8;

/// 다시 시도할 만한 일시 오류: `EINTR`, `EBUSY`, `EAGAIN`과 커널 내부 값 `-6`(`EREDRIVEOPEN`,
/// "open을 다시 하라"는 XNU 내부 errno). 마지막 값은 테스트 반복 중 `/dev/ptmx` clone open에서
/// 드물게 사용자 공간으로 새어 나오는 것을 실측했습니다.
fn transient_open_error(code: Option<i32>) -> bool {
    matches!(code, Some(4 | 16 | 35 | -6))
}

/// PTY master를 엽니다(일시 오류는 짧게 다시 시도).
fn open_master() -> Result<OwnedFd, TerminalIoError> {
    let mut attempt = 1;
    loop {
        // SAFETY: posix_openpt는 정수 flag만 받고 새 fd(또는 -1)를 돌려줍니다.
        let raw = unsafe { sys::posix_openpt(sys::O_RDWR | sys::O_NOCTTY | sys::O_CLOEXEC) };
        if raw >= 0 {
            // SAFETY: raw는 방금 열린, 다른 곳이 소유하지 않는 fd입니다.
            return Ok(unsafe { OwnedFd::from_raw_fd(raw) });
        }
        let err = TerminalIoError::last_os_error("posix_openpt");
        if attempt >= OPEN_MASTER_ATTEMPTS || !transient_open_error(err.os_code) {
            return Err(err);
        }
        attempt += 1;
        thread::sleep(Duration::from_millis(1));
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
    /// 실행 중(회수 전).
    Running,
    /// 끝났지만 회수 전(zombie). pid와 세션 id가 예약되어 있어 세션 정리를 안전하게 할 수 있습니다.
    Exited,
    /// 회수함. 남은 출력을 reader가 읽어 가기를 기다리는 중일 수 있습니다.
    Reaped {
        report: ExitReport,
        drain_deadline: Instant,
    },
}

/// 세션 정리 진행 상황.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Cleanup {
    /// `SIGHUP`·`SIGCONT`를 보낸 시각.
    hangup_at: Option<Instant>,
    /// `SIGKILL`을 처음 보낸 시각.
    kill_at: Option<Instant>,
}

/// PTY에 붙은 터미널 세션(세션 leader와 같은 세션의 모든 프로세스).
#[derive(Debug)]
pub struct PtyProcess {
    child: Child,
    pid: i32,
    master: File,
    /// 출력 유실을 막으려고 부모가 쥐고 있는 slave(끝나면 닫음).
    slave: Option<OwnedFd>,
    grace: Duration,
    leader: Leader,
    cleanup: Cleanup,
    /// 출력 정리까지 끝난 최종 결과.
    finished: Option<ExitReport>,
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
            leader: Leader::Running,
            cleanup: Cleanup::default(),
            finished: None,
        })
    }

    /// leader를 아직 회수하지 않았는지(이때만 group 신호와 세션 열거가 안전합니다).
    fn unreaped(&self) -> bool {
        matches!(self.leader, Leader::Running | Leader::Exited)
    }

    /// 회수하지 않고 leader 종료 여부를 봅니다(`waitid(WNOWAIT)`).
    fn refresh_leader(&mut self) -> Result<(), TerminalIoError> {
        if self.leader != Leader::Running {
            return Ok(());
        }
        if peek_exited(self.pid)? {
            self.leader = Leader::Exited;
        }
        Ok(())
    }

    /// leader를 뺀 같은 세션의 프로세스. leader를 회수하기 전에만 열거합니다.
    fn members(&self) -> Result<Vec<i32>, TerminalIoError> {
        if !self.unreaped() {
            return Ok(Vec::new());
        }
        session_members(self.pid)
    }

    /// leader의 process group 전체에 신호를 보냅니다. leader를 회수하기 전에만 보냅니다.
    fn signal_group(&self, signal: c_int) {
        if !self.unreaped() || self.pid <= 1 {
            return;
        }
        // SAFETY: kill(2)는 정수 인자만 받습니다. pid > 1이고 leader를 아직 회수하지 않았으므로
        // -pid는 이 세션 leader가 만든 process group만 가리킵니다.
        let _ = unsafe { sys::kill(-self.pid, signal) };
    }

    /// 터미널 hangup: leader group과 다른 group의 세션 프로세스에 `SIGHUP`+`SIGCONT`(한 번만).
    fn hangup(&mut self, members: &[i32]) {
        if self.cleanup.hangup_at.is_some() {
            return;
        }
        self.signal_group(sys::SIGHUP);
        self.signal_group(sys::SIGCONT);
        for &pid in members {
            signal_pid(pid, sys::SIGHUP);
            signal_pid(pid, sys::SIGCONT);
        }
        self.cleanup.hangup_at = Some(Instant::now());
    }

    /// 남은 세션 프로세스 전부에 `SIGKILL`.
    fn kill_all(&mut self, members: &[i32]) {
        if self.leader == Leader::Running {
            self.signal_group(sys::SIGKILL);
        }
        for &pid in members {
            signal_pid(pid, sys::SIGKILL);
        }
        self.cleanup.kill_at.get_or_insert_with(Instant::now);
    }

    /// leader를 회수합니다(이미 끝난 zombie이거나 `SIGKILL` 뒤라 곧 돌아옵니다).
    fn reap(&mut self, remaining_processes: u32) -> Result<(), TerminalIoError> {
        let status = self
            .child
            .wait()
            .map_err(|err| TerminalIoError::from_io("waitpid", &err))?;
        self.leader = Leader::Reaped {
            report: ExitReport {
                status: exit_status(status),
                remaining_processes,
            },
            drain_deadline: Instant::now() + DRAIN_TIMEOUT,
        };
        Ok(())
    }

    /// 보관한 slave를 닫아 출력을 끝내고 최종 결과를 기록합니다.
    fn finish(&mut self, report: ExitReport) -> ExitReport {
        self.slave = None;
        self.finished = Some(report);
        report
    }

    /// leader가 스스로 끝난 뒤의 비차단 정리 한 단계.
    fn advance_exited_cleanup(&mut self) -> Result<(), TerminalIoError> {
        let members = self.members()?;
        if members.is_empty() {
            return self.reap(0);
        }
        let now = Instant::now();
        match self.cleanup {
            Cleanup {
                hangup_at: None, ..
            } => self.hangup(&members),
            Cleanup {
                hangup_at: Some(hangup_at),
                kill_at: None,
            } => {
                if now >= hangup_at + self.grace {
                    self.kill_all(&members);
                }
            }
            Cleanup {
                kill_at: Some(kill_at),
                ..
            } => {
                if now >= kill_at + KILL_WAIT {
                    // 끝나지 않는 프로세스(예: 커널 대기 상태)는 수만 보고하고 leader를 회수합니다.
                    return self.reap(count(&members));
                }
                // 정리 중 새로 생긴 프로세스까지 다시 보냅니다.
                self.kill_all(&members);
            }
        }
        Ok(())
    }
}

/// `waitid(P_PID, pid, WEXITED | WNOHANG | WNOWAIT)`: 끝났으면 `true`(회수하지 않음).
fn peek_exited(pid: i32) -> Result<bool, TerminalIoError> {
    let id = u32::try_from(pid).map_err(|_| TerminalIoError {
        operation: "waitid",
        kind: io::ErrorKind::InvalidInput,
        os_code: None,
    })?;
    loop {
        let mut info = sys::SigInfo::zeroed();
        // SAFETY: info는 호출 동안 살아 있는 siginfo_t 크기의 구조체입니다. WNOWAIT라 child 상태를
        // 소비하지 않으므로 이후 std의 wait가 정상적으로 회수합니다.
        let rc = unsafe {
            sys::waitid(
                sys::P_PID,
                id,
                &mut info,
                sys::WEXITED | sys::WNOHANG | sys::WNOWAIT,
            )
        };
        if rc == 0 {
            // WNOHANG에서 아직 바뀐 상태가 없으면 si_pid가 0입니다.
            let exited = info.si_pid != 0
                && matches!(
                    info.si_code,
                    sys::CLD_EXITED | sys::CLD_KILLED | sys::CLD_DUMPED
                );
            return Ok(exited);
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(TerminalIoError::from_io("waitid", &err));
        }
    }
}

/// 열거할 pid 수 상한.
const MAX_LISTED_PIDS: usize = 1 << 18;

/// 세션 id가 `sid`인 프로세스(세션 leader `sid` 자신과 이 프로세스 제외).
///
/// 호출자는 leader를 회수하기 전에만 부릅니다. 회수 전에는 leader pid가 예약되어 있어 같은 값의 세션
/// id를 가진 다른 세션이 생길 수 없습니다.
fn session_members(sid: i32) -> Result<Vec<i32>, TerminalIoError> {
    // SAFETY: buffer가 null이고 크기가 0이면 proc_listpids는 필요한 byte 수만 돌려줍니다.
    let needed = unsafe { sys::proc_listpids(sys::PROC_ALL_PIDS, 0, std::ptr::null_mut(), 0) };
    if needed <= 0 {
        return Err(TerminalIoError::last_os_error("proc_listpids"));
    }
    let slots = (usize::try_from(needed).unwrap_or(0) / std::mem::size_of::<c_int>() + 64)
        .min(MAX_LISTED_PIDS);
    let mut pids: Vec<c_int> = vec![0; slots];
    let bytes = c_int::try_from(slots * std::mem::size_of::<c_int>()).unwrap_or(c_int::MAX);
    // SAFETY: buffer는 slots개 c_int 공간이고 그 byte 크기를 함께 넘깁니다. proc_listpids는 그
    // 안에만 씁니다.
    let written =
        unsafe { sys::proc_listpids(sys::PROC_ALL_PIDS, 0, pids.as_mut_ptr().cast(), bytes) };
    if written < 0 {
        return Err(TerminalIoError::last_os_error("proc_listpids"));
    }
    let listed = (usize::try_from(written).unwrap_or(0) / std::mem::size_of::<c_int>()).min(slots);
    pids.truncate(listed);
    let own = i32::try_from(std::process::id()).unwrap_or(0);
    pids.retain(|&pid| {
        // SAFETY: getsid는 정수 pid만 받고 실패하면 -1입니다(없는 프로세스·zombie 등).
        pid > 1 && pid != sid && pid != own && unsafe { sys::getsid(pid) } == sid
    });
    Ok(pids)
}

/// 프로세스 하나에 신호를 보냅니다(`pid > 1`만).
fn signal_pid(pid: i32, signal: c_int) {
    if pid <= 1 {
        return;
    }
    // SAFETY: kill(2)는 정수 인자만 받습니다. pid는 방금 세션 열거로 찾은 이 세션의 프로세스입니다.
    let _ = unsafe { sys::kill(pid, signal) };
}

fn count(members: &[i32]) -> u32 {
    u32::try_from(members.len()).unwrap_or(u32::MAX)
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
        self.refresh_leader()?;
        if self.leader == Leader::Exited {
            self.advance_exited_cleanup()?;
        }
        match self.leader {
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
            Leader::Running | Leader::Exited => Ok(None),
        }
    }

    fn begin_terminate(&mut self) {
        if self.finished.is_some() || !self.unreaped() || self.cleanup.hangup_at.is_some() {
            return;
        }
        // 열거에 실패해도 leader group에는 신호를 보냅니다.
        let members = self.members().unwrap_or_default();
        self.hangup(&members);
    }

    fn terminate(&mut self) -> Result<ExitReport, TerminalIoError> {
        if let Some(report) = self.finished {
            return Ok(report);
        }
        self.begin_terminate();
        let hangup_deadline = self.cleanup.hangup_at.unwrap_or_else(Instant::now) + self.grace;
        // 1단계: hangup 뒤 스스로 끝나기를 기다립니다.
        while self.unreaped() {
            self.refresh_leader()?;
            let members = self.members().unwrap_or_default();
            if self.leader == Leader::Exited && members.is_empty() {
                self.reap(0)?;
                break;
            }
            if Instant::now() >= hangup_deadline {
                break;
            }
            thread::sleep(POLL_INTERVAL);
        }
        // 2단계: 남은 세션 프로세스를 강제 종료합니다.
        let kill_deadline = Instant::now() + KILL_WAIT;
        while self.unreaped() {
            self.refresh_leader()?;
            let members = self.members().unwrap_or_default();
            if self.leader == Leader::Exited && members.is_empty() {
                self.reap(0)?;
                break;
            }
            if Instant::now() >= kill_deadline {
                if self.leader == Leader::Exited {
                    self.reap(count(&members))?;
                    break;
                }
                return Err(TerminalIoError {
                    operation: "terminate",
                    kind: io::ErrorKind::TimedOut,
                    os_code: None,
                });
            }
            self.kill_all(&members);
            thread::sleep(POLL_INTERVAL);
        }
        match self.leader {
            // 닫는 세션의 남은 출력은 기다리지 않습니다.
            Leader::Reaped { report, .. } => Ok(self.finish(report)),
            Leader::Running | Leader::Exited => Err(TerminalIoError {
                operation: "terminate",
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

impl Drop for PtyProcess {
    /// 정리되지 않은 세션은 drop할 때 프로세스 tree를 종료합니다(앱 종료 등).
    fn drop(&mut self) {
        if self.finished.is_none() {
            let _ = self.terminate();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::capability::{TerminalCapability, TerminalTrust};
    use crate::test_support::{pty_exclusive, pty_shared, OutputCollector, TempDir};
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    pub(crate) const TIMEOUT: Duration = Duration::from_secs(10);

    /// 테스트 실행기용 token(신뢰된 가짜 프로젝트 창).
    pub(crate) fn test_capability() -> TerminalCapability {
        TerminalCapability::grant(crate::registry::tests::window(99), TerminalTrust::Trusted)
            .unwrap()
    }

    pub(crate) fn request_with_env(
        program: &str,
        args: &[&str],
        cwd: &Path,
        env: Vec<(OsString, OsString)>,
    ) -> LaunchRequest {
        LaunchRequest::new(
            test_capability(),
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

    fn launch(request: LaunchRequest) -> Box<dyn TerminalProcess> {
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
    fn run_to_end(request: LaunchRequest) -> (ExitReport, String) {
        let mut process = launch(request);
        let output = OutputCollector::start(process.output_reader().unwrap());
        let report = wait_exit(process.as_mut());
        (report, output.wait_eof(TIMEOUT))
    }

    #[test]
    fn echo_output_fixture() {
        let _pty = pty_shared();
        let dir = TempDir::new();
        // 출력이 빠지지 않는지 여러 번 확인합니다(slave 보관·drain, 모듈 문서).
        for _ in 0..20 {
            let (report, text) = run_to_end(request(
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
        let _pty = pty_shared();
        let dir = TempDir::new();
        let env = vec![
            (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
            (OsString::from("STELLAR_TEST"), OsString::from("a b=c")),
        ];
        let (report, text) = run_to_end(request_with_env("/usr/bin/env", &[], dir.path(), env));
        assert_eq!(report.status, TerminalExitStatus::Exited(0));
        let mut lines: Vec<&str> = text
            .lines()
            .map(|line| line.trim_end_matches('\r'))
            .collect();
        lines.sort_unstable();
        assert_eq!(lines, ["PATH=/usr/bin:/bin", "STELLAR_TEST=a b=c"]);

        let (_, text) = run_to_end(request("/bin/pwd", &[], dir.path()));
        assert_eq!(text, format!("{}\r\n", dir.path().display()));
    }

    #[test]
    fn child_is_session_leader_with_pty_as_controlling_terminal() {
        let _pty = pty_shared();
        let dir = TempDir::new();
        // `tty`는 stdin이 터미널이면 그 이름을 출력합니다. `/dev/tty`를 열 수 있으면 controlling
        // terminal이 있다는 뜻입니다.
        let (report, text) = run_to_end(request(
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
        let _pty = pty_shared();
        let dir = TempDir::new();
        let mut process = launch(request("/bin/cat", &[], dir.path()));
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
    fn only_transient_master_open_errors_are_retried() {
        for code in [4, 16, 35, -6] {
            assert!(transient_open_error(Some(code)), "{code}");
        }
        for code in [None, Some(1), Some(2), Some(6), Some(24)] {
            assert!(!transient_open_error(code), "{code:?}");
        }
    }

    #[test]
    fn missing_program_is_a_spawn_failure() {
        let _pty = pty_shared();
        let dir = TempDir::new();
        let mut launcher = PtyLauncher::new();
        let err = launcher
            .launch(request("/nonexistent/stellar-shell", &[], dir.path()))
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
            test_capability(),
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
        let _pty = pty_shared();
        let dir = TempDir::new();
        for (rows, cols) in [(24, 80), (33, 101), (1, 1), (200, 500)] {
            let size = TerminalSize::new(rows, cols).unwrap();
            let (report, text) =
                run_to_end(request_sized("/bin/stty", &["size"], dir.path(), size));
            assert_eq!(report.status, TerminalExitStatus::Exited(0));
            assert_eq!(text, format!("{rows} {cols}\r\n"));
        }
    }

    #[test]
    fn resize_fixture_columns_and_rows() {
        let _pty = pty_shared();
        let dir = TempDir::new();
        let mut process = launch(request(
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
        let _pty = pty_shared();
        let dir = TempDir::new();
        let mut process = launch(request(
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

    extern "C" {
        fn isatty(fd: c_int) -> c_int;
    }

    /// 세 갈래 프로세스 tree를 만드는 셸 스크립트 앞부분. 각 pid를 `pid:<n>` 줄로 출력합니다.
    ///
    /// - leader와 같은 process group의 background job
    /// - job control(`set -m`)로 다른 process group에 있는 job
    /// - `SIGHUP`을 무시하는(`trap '' HUP` 후 exec) 다른 group의 job. 무시를 설정한 뒤
    ///   `hup-ignored`를 출력합니다([`wait_tree_ready`]).
    pub(crate) const TREE_SCRIPT: &str = "/bin/sleep 300 & echo pid:$!; set -m; \
         /bin/sleep 300 & echo pid:$!; (trap '' HUP; echo hup-ignored; exec /bin/sleep 300) & \
         echo pid:$!; echo ready";

    /// tree가 모두 준비될 때까지(무시 설정 포함) 기다리고 pid 세 개를 돌려줍니다.
    pub(crate) fn wait_tree_ready(output: &mut OutputCollector) -> Vec<i32> {
        output.wait_for("ready", TIMEOUT);
        let pids = pids_in(&output.wait_for("hup-ignored", TIMEOUT));
        assert_eq!(pids.len(), 3, "{:?}", output.text());
        pids
    }

    /// 출력의 `pid:<n>` 줄.
    pub(crate) fn pids_in(text: &str) -> Vec<i32> {
        text.lines()
            .filter_map(|line| line.trim_end_matches('\r').strip_prefix("pid:"))
            .filter_map(|pid| pid.trim().parse().ok())
            .collect()
    }

    /// pid가 아직 있는지(`kill(pid, 0)`이 `ESRCH`가 아니면 있음).
    pub(crate) fn alive(pid: i32) -> bool {
        // SAFETY: 신호 0은 존재·권한만 확인합니다.
        let rc = unsafe { sys::kill(pid, 0) };
        rc == 0 || io::Error::last_os_error().raw_os_error() != Some(3)
    }

    /// 모든 pid가 사라질 때까지 기다립니다(고아 프로세스 0 확인).
    pub(crate) fn wait_gone(pids: &[i32]) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let left: Vec<i32> = pids.iter().copied().filter(|&pid| alive(pid)).collect();
            if left.is_empty() {
                return;
            }
            assert!(Instant::now() < deadline, "남은 프로세스: {left:?}");
            thread::sleep(POLL_INTERVAL);
        }
    }

    /// 테스트가 실패해도 fixture 프로세스를 남기지 않습니다.
    pub(crate) struct KillOnDrop(pub(crate) Vec<i32>);

    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            for &pid in &self.0 {
                if alive(pid) {
                    signal_pid(pid, sys::SIGKILL);
                }
            }
        }
    }

    /// 이 프로세스에서 열린 tty fd 수(PTY master·slave 포함).
    pub(crate) fn tty_fd_count() -> usize {
        let Ok(entries) = std::fs::read_dir("/dev/fd") else {
            return 0;
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().to_str()?.parse::<c_int>().ok())
            // SAFETY: isatty는 fd 번호만 받습니다(닫힌 fd면 0).
            .filter(|&fd| unsafe { isatty(fd) } == 1)
            .count()
    }

    fn tree_request(tail: &str, cwd: &Path) -> LaunchRequest {
        request("/bin/sh", &["-c", &format!("{TREE_SCRIPT}; {tail}")], cwd)
    }

    #[test]
    fn child_exit_cleans_up_whole_session() {
        let _pty = pty_shared();
        let dir = TempDir::new();
        let mut process = launch(tree_request("read line; exit 0", dir.path()));
        let leader = i32::try_from(process.pid()).unwrap();
        let mut output = OutputCollector::start(process.output_reader().unwrap());
        let pids = wait_tree_ready(&mut output);
        let _guard = KillOnDrop(pids.clone());
        process.write_input(b"\n").unwrap();
        let report = wait_exit(process.as_mut());
        output.wait_eof(TIMEOUT);
        assert_eq!(
            report,
            ExitReport {
                status: TerminalExitStatus::Exited(0),
                remaining_processes: 0
            }
        );
        wait_gone(&pids);
        assert!(!alive(leader));
    }

    #[test]
    fn external_signal_to_leader_cleans_up_session() {
        let _pty = pty_shared();
        let dir = TempDir::new();
        let mut process = launch(tree_request("while :; do /bin/sleep 1; done", dir.path()));
        let leader = i32::try_from(process.pid()).unwrap();
        let mut output = OutputCollector::start(process.output_reader().unwrap());
        let pids = wait_tree_ready(&mut output);
        let _guard = KillOnDrop(pids.clone());
        assert!(session_members(leader).unwrap().len() >= 3);
        signal_pid(leader, sys::SIGKILL);
        let report = wait_exit(process.as_mut());
        assert_eq!(report.status, TerminalExitStatus::Signaled(sys::SIGKILL));
        assert_eq!(report.remaining_processes, 0);
        wait_gone(&pids);
    }

    #[test]
    fn terminate_kills_other_groups_and_sighup_ignoring_jobs() {
        let _pty = pty_shared();
        let dir = TempDir::new();
        let mut process = launch(tree_request("while :; do /bin/sleep 1; done", dir.path()));
        let mut output = OutputCollector::start(process.output_reader().unwrap());
        let pids = wait_tree_ready(&mut output);
        let _guard = KillOnDrop(pids.clone());
        let started = Instant::now();
        let report = process.terminate().unwrap();
        assert_eq!(report.remaining_processes, 0);
        assert!(matches!(report.status, TerminalExitStatus::Signaled(_)));
        // SIGHUP을 무시하는 job 때문에 유예 시간 뒤 SIGKILL까지 갑니다(상한 안).
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_millis(500), "{elapsed:?}");
        assert!(elapsed < Duration::from_millis(500) + KILL_WAIT + Duration::from_secs(1));
        wait_gone(&pids);
    }

    #[test]
    fn dropping_unfinished_process_terminates_tree() {
        let _pty = pty_shared();
        let dir = TempDir::new();
        let process = launch(tree_request("while :; do /bin/sleep 1; done", dir.path()));
        let leader = i32::try_from(process.pid()).unwrap();
        let mut output = OutputCollector::start(process.output_reader().unwrap());
        let pids = wait_tree_ready(&mut output);
        let _guard = KillOnDrop(pids.clone());
        // 앱이 registry 없이 handle만 버려도 tree를 정리합니다.
        drop(process);
        wait_gone(&pids);
        assert!(!alive(leader));
        output.wait_eof(TIMEOUT);
    }

    #[test]
    fn tty_fd_count_sees_pty_fds() {
        let _pty = pty_exclusive();
        let before = tty_fd_count();
        let dir = TempDir::new();
        let mut process = launch(request("/bin/sleep", &["300"], dir.path()));
        // master + 부모가 보관한 slave.
        assert_eq!(tty_fd_count(), before + 2);
        process.terminate().unwrap();
        assert_eq!(tty_fd_count(), before + 1);
        drop(process);
        assert_eq!(tty_fd_count(), before);
    }
}

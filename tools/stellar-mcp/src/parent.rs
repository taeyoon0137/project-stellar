//! 부모 프로세스 종료 감지 (XIX.1.9).
//!
//! MCP stdio transport에서 client는 서버를 subprocess로 실행하고, 정상 종료는 stdin을 닫는 것으로
//! 알립니다(MCP 2025-11-25 Lifecycle "Shutdown"). stdin EOF는 [`crate::server::serve`]가 처리합니다.
//! client가 강제 종료되면 stdin 쓰기 끝을 다른 프로세스가 물려받아 EOF가 오지 않을 수 있으므로,
//! 브리지는 부모 프로세스 종료도 직접 감시합니다.
//!
//! ## 방법(macOS `kqueue(2)`, `getpid(2)`)
//!
//! 1. 시작 시 `getppid()`로 부모 pid를 기록합니다. 1(launchd)이면 감시할 부모가 없으므로
//!    [`Watch::NoParent`]입니다.
//! 2. `kqueue()`에 `EVFILT_PROC`·`NOTE_EXIT`로 그 pid를 등록합니다. 등록이 `ESRCH`이면 부모가 이미
//!    종료된 것입니다([`Watch::ParentGone`]).
//! 3. 등록 뒤 `getppid()`가 여전히 같은지 다시 확인합니다. 부모가 등록 전에 죽었다면 브리지는
//!    이미 launchd로 재부모화되어 값이 바뀌었으므로, pid 재사용으로 남의 프로세스를 감시하는 경우를
//!    배제합니다.
//! 4. 감시 thread가 `kevent()`에서 대기하다 `NOTE_EXIT`를 받으면 콜백을 한 번 호출합니다.
//!
//! libc crate 없이 필요한 함수만 최소 `extern "C"`로 선언합니다(macOS `<sys/event.h>`,
//! `<unistd.h>`).

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

mod sys {
    use std::ffi::c_void;

    /// `struct kevent`(`<sys/event.h>`).
    #[repr(C)]
    pub struct KEvent {
        pub ident: usize,
        pub filter: i16,
        pub flags: u16,
        pub fflags: u32,
        pub data: isize,
        pub udata: *mut c_void,
    }

    extern "C" {
        /// `int kqueue(void)`.
        pub fn kqueue() -> i32;
        /// `int kevent(int kq, const struct kevent *changelist, int nchanges,
        /// struct kevent *eventlist, int nevents, const struct timespec *timeout)`.
        pub fn kevent(
            kq: i32,
            changelist: *const KEvent,
            nchanges: i32,
            eventlist: *mut KEvent,
            nevents: i32,
            timeout: *const c_void,
        ) -> i32;
        /// `pid_t getppid(void)`.
        pub fn getppid() -> i32;
    }

    /// `EVFILT_PROC`.
    pub const EVFILT_PROC: i16 = -5;
    /// `EV_ADD`.
    pub const EV_ADD: u16 = 0x0001;
    /// `EV_ONESHOT`.
    pub const EV_ONESHOT: u16 = 0x0010;
    /// `EV_ERROR`.
    pub const EV_ERROR: u16 = 0x4000;
    /// `NOTE_EXIT`.
    pub const NOTE_EXIT: u32 = 0x8000_0000;
}

/// launchd pid. 부모가 죽은 프로세스는 이 pid로 재부모화됩니다.
pub const LAUNCHD_PID: i32 = 1;

/// 감시 시작 결과.
#[derive(Debug)]
pub enum Watch {
    /// 부모를 감시하는 중입니다. 값은 감시 대상 pid입니다.
    Watching(i32),
    /// 부모가 이미 종료되었습니다. 콜백은 호출하지 않았으므로 호출자가 종료합니다.
    ParentGone,
    /// 부모가 launchd(pid 1)라 감시하지 않습니다. 종료 신호는 stdin EOF뿐입니다.
    NoParent,
}

/// 현재 프로세스의 부모 pid.
pub fn parent_pid() -> i32 {
    // SAFETY: getppid는 인자가 없고 항상 성공하며 메모리를 읽거나 쓰지 않습니다.
    unsafe { sys::getppid() }
}

/// 부모 프로세스가 종료되면 `on_exit`를 한 번 호출하는 감시 thread를 시작합니다.
pub fn watch_parent<F>(on_exit: F) -> io::Result<Watch>
where
    F: FnOnce() + Send + 'static,
{
    let ppid = parent_pid();
    if ppid <= LAUNCHD_PID {
        return Ok(Watch::NoParent);
    }
    match watch_pid(ppid, on_exit)? {
        false => Ok(Watch::ParentGone),
        // 등록 전에 부모가 죽어 재부모화되었다면 등록한 pid는 부모가 아닙니다.
        true if parent_pid() != ppid => Ok(Watch::ParentGone),
        true => Ok(Watch::Watching(ppid)),
    }
}

/// `pid` 종료를 감시하는 thread를 시작합니다. 감시를 시작하면 `Ok(true)`, 프로세스가 이미 없으면
/// `Ok(false)`입니다.
///
/// 종료를 감지하면 `on_exit`를 한 번 호출하고 thread가 끝납니다.
pub fn watch_pid<F>(pid: i32, on_exit: F) -> io::Result<bool>
where
    F: FnOnce() + Send + 'static,
{
    let ident = usize::try_from(pid)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "pid는 양수여야 합니다"))?;
    // SAFETY: kqueue는 인자가 없고 실패하면 -1을 반환합니다.
    let raw = unsafe { sys::kqueue() };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: raw는 방금 만든, 이 함수만 소유한 열린 kqueue descriptor입니다.
    let kq = unsafe { OwnedFd::from_raw_fd(raw) };
    let change = sys::KEvent {
        ident,
        filter: sys::EVFILT_PROC,
        flags: sys::EV_ADD | sys::EV_ONESHOT,
        fflags: sys::NOTE_EXIT,
        data: 0,
        udata: std::ptr::null_mut(),
    };
    // SAFETY: change는 유효한 kevent 하나를 가리키고 eventlist는 비어 있습니다(nevents 0).
    let rc = unsafe {
        sys::kevent(
            kq.as_raw_fd(),
            &change,
            1,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
        )
    };
    if rc < 0 {
        let error = io::Error::last_os_error();
        // ESRCH: 감시할 프로세스가 없습니다(kqueue(2) ERRORS).
        return if error.raw_os_error() == Some(ESRCH) {
            Ok(false)
        } else {
            Err(error)
        };
    }
    std::thread::Builder::new()
        .name("stellar-mcp-parent-watch".into())
        .spawn(move || {
            if wait_exit(&kq) {
                on_exit();
            }
        })?;
    Ok(true)
}

/// `<errno.h>` `ESRCH`.
const ESRCH: i32 = 3;
/// `<errno.h>` `EINTR`.
const EINTR: i32 = 4;

/// 등록한 `NOTE_EXIT`가 올 때까지 기다립니다. 감시를 계속할 수 없으면 `false`입니다.
fn wait_exit(kq: &OwnedFd) -> bool {
    loop {
        let mut event = sys::KEvent {
            ident: 0,
            filter: 0,
            flags: 0,
            fflags: 0,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        // SAFETY: event는 쓰기 가능한 kevent 하나이고 changelist는 비어 있습니다. timeout null은
        // 무기한 대기입니다.
        let rc = unsafe {
            sys::kevent(
                kq.as_raw_fd(),
                std::ptr::null(),
                0,
                &mut event,
                1,
                std::ptr::null(),
            )
        };
        if rc < 0 {
            if io::Error::last_os_error().raw_os_error() == Some(EINTR) {
                continue;
            }
            return false;
        }
        if rc == 1 && event.flags & sys::EV_ERROR == 0 && event.fflags & sys::NOTE_EXIT != 0 {
            return true;
        }
        if rc == 1 && event.flags & sys::EV_ERROR != 0 {
            return false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn kevent_layout_matches_the_c_struct() {
        // struct kevent: uintptr_t, int16_t, uint16_t, uint32_t, intptr_t, void* (arm64: 32 byte).
        assert_eq!(std::mem::size_of::<sys::KEvent>(), 32);
        assert_eq!(std::mem::align_of::<sys::KEvent>(), 8);
    }

    #[test]
    fn watched_process_exit_calls_back_once() {
        let mut child = Command::new("/bin/sleep")
            .arg("30")
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let pid = i32::try_from(child.id()).unwrap();
        let (tx, rx) = mpsc::channel();
        assert!(watch_pid(pid, move || tx.send(()).unwrap()).unwrap());
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "살아 있는 동안 콜백 없음"
        );
        child.kill().unwrap();
        child.wait().unwrap();
        rx.recv_timeout(Duration::from_secs(5))
            .expect("종료 후 콜백 호출");
        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "콜백은 한 번"
        );
    }

    #[test]
    fn already_reaped_process_is_reported_gone() {
        let mut child = Command::new("/usr/bin/true").spawn().unwrap();
        let pid = i32::try_from(child.id()).unwrap();
        child.wait().unwrap();
        assert!(!watch_pid(pid, || {}).unwrap());
    }

    #[test]
    fn invalid_pid_is_rejected() {
        assert_eq!(
            watch_pid(-1, || {}).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn test_runner_parent_is_watched() {
        // cargo test runner는 이 프로세스의 부모이며 살아 있습니다.
        let (tx, rx) = mpsc::channel::<()>();
        let watch = watch_parent(move || {
            let _ = tx.send(());
        })
        .unwrap();
        match watch {
            Watch::Watching(pid) => assert_eq!(pid, parent_pid()),
            Watch::NoParent => assert_eq!(parent_pid(), LAUNCHD_PID),
            Watch::ParentGone => panic!("살아 있는 부모를 종료로 판정"),
        }
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    }
}

//! macOS libSystem 최소 FFI 선언(PTY·세션·신호·프로세스 목록).
//!
//! 새 crate 의존 없이 필요한 C 함수와 상수만 선언합니다. 상수 값은 macOS SDK 헤더
//! (`sys/fcntl.h`, `sys/ttycom.h`, `sys/signal.h`, `sys/wait.h`, `sys/proc_info.h`)와 같아야 하며
//! 테스트로 고정합니다. 가변 인자
//! 함수(`open`, `ioctl`)는 Apple arm64 호출 규약에서 고정 인자와 전달 방식이 다르므로 반드시 `...`로
//! 선언합니다.

use std::ffi::{c_char, c_int, c_long, c_ulong, c_void};

/// `O_RDWR`.
pub(crate) const O_RDWR: c_int = 0x0002;
/// `O_NOCTTY`: 여는 프로세스의 controlling terminal로 만들지 않음.
pub(crate) const O_NOCTTY: c_int = 0x0002_0000;
/// `O_CLOEXEC`: fd를 만들 때 원자적으로 close-on-exec 설정.
pub(crate) const O_CLOEXEC: c_int = 0x0100_0000;

const IOC_VOID: u32 = 0x2000_0000;
const IOC_OUT: u32 = 0x4000_0000;
const IOC_IN: u32 = 0x8000_0000;
const IOCPARM_MASK: u32 = 0x1fff;

/// `<sys/ioccom.h>`의 `_IOC(inout, group, num, len)`.
const fn ioc(inout: u32, group: u8, num: u8, len: usize) -> c_ulong {
    (inout | (((len as u32) & IOCPARM_MASK) << 16) | ((group as u32) << 8) | num as u32) as c_ulong
}

/// `TIOCSCTTY = _IO('t', 97)`: controlling terminal로 만듦.
pub(crate) const TIOCSCTTY: c_ulong = ioc(IOC_VOID, b't', 97, 0);
/// `TIOCSWINSZ = _IOW('t', 103, struct winsize)`.
pub(crate) const TIOCSWINSZ: c_ulong = ioc(IOC_IN, b't', 103, std::mem::size_of::<Winsize>());
/// `TIOCGWINSZ = _IOR('t', 104, struct winsize)`.
pub(crate) const TIOCGWINSZ: c_ulong = ioc(IOC_OUT, b't', 104, std::mem::size_of::<Winsize>());
/// `TIOCOUTQ = _IOR('t', 115, int)`: 출력 queue 길이.
pub(crate) const TIOCOUTQ: c_ulong = ioc(IOC_OUT, b't', 115, std::mem::size_of::<c_int>());

/// `SIGHUP`.
pub(crate) const SIGHUP: c_int = 1;
/// `SIGKILL`.
pub(crate) const SIGKILL: c_int = 9;
/// `SIGCONT`(macOS 값).
pub(crate) const SIGCONT: c_int = 19;

/// `idtype_t::P_PID`.
pub(crate) const P_PID: c_int = 1;
/// `WNOHANG`.
pub(crate) const WNOHANG: c_int = 0x01;
/// `WEXITED`.
pub(crate) const WEXITED: c_int = 0x04;
/// `WNOWAIT`: 상태를 보고하되 회수하지 않음(zombie 유지).
pub(crate) const WNOWAIT: c_int = 0x20;
/// `CLD_EXITED`.
pub(crate) const CLD_EXITED: c_int = 1;
/// `CLD_KILLED`.
pub(crate) const CLD_KILLED: c_int = 2;
/// `CLD_DUMPED`.
pub(crate) const CLD_DUMPED: c_int = 3;
/// `PROC_ALL_PIDS`(`proc_listpids` 종류).
pub(crate) const PROC_ALL_PIDS: u32 = 1;

/// `siginfo_t`(macOS `sys/signal.h`).
#[repr(C)]
#[derive(Debug)]
pub(crate) struct SigInfo {
    pub(crate) si_signo: c_int,
    pub(crate) si_errno: c_int,
    pub(crate) si_code: c_int,
    pub(crate) si_pid: c_int,
    pub(crate) si_uid: u32,
    pub(crate) si_status: c_int,
    pub(crate) si_addr: *mut c_void,
    pub(crate) si_value: usize,
    pub(crate) si_band: c_long,
    pub(crate) pad: [c_ulong; 7],
}

impl SigInfo {
    /// 0으로 채운 값.
    pub(crate) fn zeroed() -> SigInfo {
        SigInfo {
            si_signo: 0,
            si_errno: 0,
            si_code: 0,
            si_pid: 0,
            si_uid: 0,
            si_status: 0,
            si_addr: std::ptr::null_mut(),
            si_value: 0,
            si_band: 0,
            pad: [0; 7],
        }
    }
}

/// `struct winsize`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Winsize {
    pub(crate) ws_row: u16,
    pub(crate) ws_col: u16,
    pub(crate) ws_xpixel: u16,
    pub(crate) ws_ypixel: u16,
}

extern "C" {
    pub(crate) fn posix_openpt(oflag: c_int) -> c_int;
    pub(crate) fn grantpt(fd: c_int) -> c_int;
    pub(crate) fn unlockpt(fd: c_int) -> c_int;
    pub(crate) fn ptsname_r(fd: c_int, buffer: *mut c_char, buflen: usize) -> c_int;
    pub(crate) fn open(path: *const c_char, oflag: c_int, ...) -> c_int;
    pub(crate) fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    pub(crate) fn setsid() -> c_int;
    pub(crate) fn kill(pid: c_int, sig: c_int) -> c_int;
    pub(crate) fn getsid(pid: c_int) -> c_int;
    pub(crate) fn waitid(idtype: c_int, id: u32, infop: *mut SigInfo, options: c_int) -> c_int;
    pub(crate) fn proc_listpids(
        kind: u32,
        typeinfo: u32,
        buffer: *mut c_void,
        buffersize: c_int,
    ) -> c_int;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_macos_sdk_headers() {
        assert_eq!(std::mem::size_of::<Winsize>(), 8);
        assert_eq!(TIOCSCTTY, 0x2000_7461);
        assert_eq!(TIOCSWINSZ, 0x8008_7467);
        assert_eq!(TIOCGWINSZ, 0x4008_7468);
        assert_eq!(TIOCOUTQ, 0x4004_7473);
        assert_eq!((O_RDWR, O_NOCTTY, O_CLOEXEC), (2, 0x20000, 0x0100_0000));
        assert_eq!((SIGHUP, SIGKILL, SIGCONT), (1, 9, 19));
        assert_eq!(std::mem::size_of::<SigInfo>(), 104);
        assert_eq!((P_PID, WNOHANG, WEXITED, WNOWAIT), (1, 1, 4, 0x20));
        assert_eq!((CLD_EXITED, CLD_KILLED, CLD_DUMPED), (1, 2, 3));
        assert_eq!(PROC_ALL_PIDS, 1);
    }
}

//! OS 사용자 식별과 사용자별 디렉터리 (XIX.1.2, XIX.1.3).
//!
//! libc crate 없이 필요한 함수만 최소 `extern "C"`로 선언합니다(macOS `<unistd.h>`). 모든 호출은
//! 인자·반환값을 검사하는 안전한 wrapper로만 노출합니다.

use std::ffi::OsString;
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

mod sys {
    /// `<poll.h>` `struct pollfd`.
    #[repr(C)]
    pub struct PollFd {
        /// 기다릴 fd.
        pub fd: i32,
        /// 기다릴 event.
        pub events: i16,
        /// 일어난 event(커널이 씀).
        pub revents: i16,
    }

    extern "C" {
        /// `uid_t geteuid(void)`.
        pub fn geteuid() -> u32;
        /// `size_t confstr(int name, char *buf, size_t len)`.
        pub fn confstr(name: i32, buf: *mut u8, len: usize) -> usize;
        /// `int getpeereid(int socket, uid_t *euid, gid_t *egid)`.
        pub fn getpeereid(socket: i32, euid: *mut u32, egid: *mut u32) -> i32;
        /// `int poll(struct pollfd fds[], nfds_t nfds, int timeout)`(`nfds_t`는 `unsigned int`).
        pub fn poll(fds: *mut PollFd, nfds: u32, timeout: i32) -> i32;
    }

    /// `<unistd.h>` `_CS_DARWIN_USER_TEMP_DIR`.
    pub const CS_DARWIN_USER_TEMP_DIR: i32 = 65537;
    /// `<poll.h>` `POLLIN`.
    pub const POLLIN: i16 = 0x0001;
}

/// 현재 프로세스의 effective 사용자 id.
pub fn current_uid() -> u32 {
    // SAFETY: geteuid는 인자가 없고 항상 성공하며 메모리를 읽거나 쓰지 않습니다.
    unsafe { sys::geteuid() }
}

/// 연결된 Unix domain socket 상대 프로세스의 effective 사용자 id(XIX.1.3).
///
/// 커널이 연결 시점에 기록한 자격 증명이므로 상대가 위조할 수 없습니다.
pub fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let mut uid: u32 = u32::MAX;
    let mut gid: u32 = u32::MAX;
    // SAFETY: fd는 `stream`이 소유한 열린 socket이고(호출 동안 살아 있음), 두 포인터는 쓰기 가능한
    // 지역 변수를 가리킵니다.
    let rc = unsafe { sys::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    if rc == 0 {
        Ok(uid)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `fds` 중 하나가 읽을 수 있게 되거나(EOF·연결 끊김·오류 포함) `timeout`이 지날 때까지 기다립니다
/// (`poll(2)`, IPC I/O 스레드의 연결 다중화).
///
/// 반환값은 `fds`와 같은 순서의 준비 여부입니다. `timeout`이 `None`이면 무기한 기다리고, 1 ms 단위로
/// 올림합니다. 신호로 중단되면(`EINTR`) 모두 `false`입니다.
pub(crate) fn poll_readable(fds: &[RawFd], timeout: Option<Duration>) -> io::Result<Vec<bool>> {
    let mut entries: Vec<sys::PollFd> = fds
        .iter()
        .map(|&fd| sys::PollFd {
            fd,
            events: sys::POLLIN,
            revents: 0,
        })
        .collect();
    let nfds = u32::try_from(entries.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "poll fd가 너무 많습니다"))?;
    let timeout_ms = match timeout {
        None => -1,
        Some(d) => {
            let ms = d.as_nanos().div_ceil(1_000_000);
            i32::try_from(ms).unwrap_or(i32::MAX)
        }
    };
    // SAFETY: entries는 nfds개의 초기화된 `struct pollfd`이고 호출 동안 살아 있습니다. poll은 각
    // 항목의 revents만 씁니다.
    let rc = unsafe { sys::poll(entries.as_mut_ptr(), nfds, timeout_ms) };
    if rc < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(vec![false; fds.len()]);
        }
        return Err(error);
    }
    // POLLIN 외에 POLLHUP·POLLERR·POLLNVAL도 다음 read·accept가 바로 결과를 내는 상태입니다.
    Ok(entries.iter().map(|e| e.revents != 0).collect())
}

/// 현재 사용자의 임시 디렉터리(`_CS_DARWIN_USER_TEMP_DIR`).
///
/// 환경 변수(`TMPDIR` 등)에 의존하지 않으므로, 환경을 비우고 실행된 브리지와 GUI 앱이 같은
/// 경로를 얻습니다. OS가 사용자 전용(0700)으로 만듭니다.
pub fn user_temp_dir() -> io::Result<PathBuf> {
    // SAFETY: buf가 null이고 len이 0이면 confstr는 아무것도 쓰지 않고 필요한 길이(NUL 포함)만
    // 반환합니다.
    let needed = unsafe { sys::confstr(sys::CS_DARWIN_USER_TEMP_DIR, std::ptr::null_mut(), 0) };
    if needed <= 1 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "사용자 임시 디렉터리를 얻을 수 없습니다",
        ));
    }
    let mut buf = vec![0u8; needed];
    // SAFETY: buf는 buf.len() byte 쓰기 가능한 메모리이고, confstr는 len을 넘지 않게(NUL 포함)
    // 씁니다.
    let written =
        unsafe { sys::confstr(sys::CS_DARWIN_USER_TEMP_DIR, buf.as_mut_ptr(), buf.len()) };
    if written != needed {
        return Err(io::Error::other("사용자 임시 디렉터리 길이가 바뀌었습니다"));
    }
    buf.truncate(written - 1); // 끝 NUL 제거.
    if buf.contains(&0) || buf.first() != Some(&b'/') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "사용자 임시 디렉터리가 절대 경로가 아닙니다",
        ));
    }
    Ok(PathBuf::from(OsString::from_vec(buf)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn current_uid_owns_files_this_process_creates() {
        let path = std::env::temp_dir().join(format!("smcp-uid-{}", std::process::id()));
        std::fs::write(&path, b"x").unwrap();
        let owner = std::fs::metadata(&path).unwrap().uid();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(owner, current_uid());
    }

    #[test]
    fn user_temp_dir_is_private_absolute_directory() {
        let dir = user_temp_dir().unwrap();
        assert!(dir.is_absolute());
        let meta = std::fs::metadata(&dir).unwrap();
        assert!(meta.is_dir());
        assert_eq!(meta.uid(), current_uid());
        assert_eq!(meta.mode() & 0o077, 0, "다른 사용자 접근 불가");
    }

    #[test]
    fn poll_readable_reports_data_eof_and_timeout() {
        use std::io::Write;
        let (a, mut b) = UnixStream::pair().unwrap();
        let (c, d) = UnixStream::pair().unwrap();
        let fds = [a.as_raw_fd(), c.as_raw_fd()];
        let started = std::time::Instant::now();
        assert_eq!(
            poll_readable(&fds, Some(Duration::from_millis(20))).unwrap(),
            vec![false, false]
        );
        assert!(started.elapsed() >= Duration::from_millis(20));
        b.write_all(b"x").unwrap();
        assert_eq!(poll_readable(&fds, None).unwrap(), vec![true, false]);
        // 상대가 닫으면(EOF) 읽을 수 있는 상태입니다.
        drop(d);
        assert_eq!(
            poll_readable(&fds, Some(Duration::ZERO)).unwrap(),
            vec![true, true]
        );
    }

    #[test]
    fn peer_uid_of_same_user_socket_pair_is_current_uid() {
        let (a, b) = UnixStream::pair().unwrap();
        assert_eq!(peer_uid(&a).unwrap(), current_uid());
        assert_eq!(peer_uid(&b).unwrap(), current_uid());
    }
}

//! OS 사용자 식별과 사용자별 디렉터리 (XIX.1.2).
//!
//! libc crate 없이 필요한 함수만 최소 `extern "C"`로 선언합니다(macOS `<unistd.h>`). 모든 호출은
//! 인자·반환값을 검사하는 안전한 wrapper로만 노출합니다.

use std::ffi::OsString;
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

mod sys {
    extern "C" {
        /// `uid_t geteuid(void)`.
        pub fn geteuid() -> u32;
        /// `size_t confstr(int name, char *buf, size_t len)`.
        pub fn confstr(name: i32, buf: *mut u8, len: usize) -> usize;
    }

    /// `<unistd.h>` `_CS_DARWIN_USER_TEMP_DIR`.
    pub const CS_DARWIN_USER_TEMP_DIR: i32 = 65537;
}

/// 현재 프로세스의 effective 사용자 id.
pub fn current_uid() -> u32 {
    // SAFETY: geteuid는 인자가 없고 항상 성공하며 메모리를 읽거나 쓰지 않습니다.
    unsafe { sys::geteuid() }
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
}

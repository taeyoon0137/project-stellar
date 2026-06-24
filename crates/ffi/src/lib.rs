//! stellar-ffi — C ABI 경계.
//!
//! runtime handle은 opaque pointer가 아니라 정수형 registry handle로 노출합니다.
//! 0은 무효 handle을 뜻합니다.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// 버전 문자열(NUL 종료에 의존하지 않는 바이트 시퀀스).
const VERSION: &str = "0.0.0";

/// registry에 등록되는 runtime 인스턴스(II.4.2에서 handle별 last error 추가).
struct Runtime;

struct RegistryInner {
    next: u64,
    live: HashMap<u64, Runtime>,
}

fn registry() -> &'static Mutex<RegistryInner> {
    static REGISTRY: OnceLock<Mutex<RegistryInner>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        Mutex::new(RegistryInner {
            next: 1,
            live: HashMap::new(),
        })
    })
}

/// 새 runtime을 생성하고 정수 handle을 반환합니다. 실패 시 `0`.
#[no_mangle]
pub extern "C" fn stellar_runtime_create() -> u64 {
    let Ok(mut reg) = registry().lock() else {
        return 0;
    };
    let handle = reg.next;
    reg.next = reg.next.wrapping_add(1);
    reg.live.insert(handle, Runtime);
    handle
}

/// runtime을 파괴합니다. 성공 `0`, 알 수 없는/이미 파괴된 handle은 `-1`(double free 방지).
#[no_mangle]
pub extern "C" fn stellar_runtime_destroy(handle: u64) -> i32 {
    let Ok(mut reg) = registry().lock() else {
        return -1;
    };
    if reg.live.remove(&handle).is_some() {
        0
    } else {
        -1
    }
}

/// 버전 문자열을 호출자 buffer에 UTF-8 바이트로 씁니다.
///
/// 반환: 쓴 바이트 수. `buf`가 null이면 필요한 길이. 무효 handle은 `-1`.
///
/// # Safety
/// `buf`는 `buf_len` 바이트 쓰기가 유효한 포인터이거나 null이어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_runtime_version(
    handle: u64,
    buf: *mut u8,
    buf_len: usize,
) -> isize {
    {
        let Ok(reg) = registry().lock() else {
            return -1;
        };
        if !reg.live.contains_key(&handle) {
            return -1;
        }
    }
    let bytes = VERSION.as_bytes();
    if buf.is_null() {
        return bytes.len() as isize;
    }
    let n = bytes.len().min(buf_len);
    let out = unsafe { std::slice::from_raw_parts_mut(buf, buf_len) };
    out[..n].copy_from_slice(&bytes[..n]);
    n as isize
}

/// 현재 살아 있는 runtime handle 수(누수 검증용).
#[no_mangle]
pub extern "C" fn stellar_runtime_live_count() -> u64 {
    registry()
        .lock()
        .map(|reg| reg.live.len() as u64)
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_version_destroy_cycle() {
        let h = stellar_runtime_create();
        assert!(h != 0);
        let mut buf = [0u8; 16];
        let n = unsafe { stellar_runtime_version(h, buf.as_mut_ptr(), buf.len()) };
        assert_eq!(&buf[..n as usize], VERSION.as_bytes());
        assert_eq!(stellar_runtime_destroy(h), 0);
        // double free → -1
        assert_eq!(stellar_runtime_destroy(h), -1);
        // 파괴 후 version → -1
        assert_eq!(
            unsafe { stellar_runtime_version(h, buf.as_mut_ptr(), buf.len()) },
            -1
        );
    }

    #[test]
    fn handles_are_distinct_and_freed() {
        let before = stellar_runtime_live_count();
        let a = stellar_runtime_create();
        let b = stellar_runtime_create();
        assert_ne!(a, b);
        assert_eq!(stellar_runtime_live_count(), before + 2);
        assert_eq!(stellar_runtime_destroy(a), 0);
        assert_eq!(stellar_runtime_destroy(b), 0);
        assert_eq!(stellar_runtime_live_count(), before);
    }
}

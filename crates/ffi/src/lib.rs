//! stellar-ffi — C ABI 경계.
//!
//! runtime handle은 opaque pointer가 아니라 정수형 registry handle로 노출합니다(0=무효).
//! 모든 작업 함수는 [`StellarStatus`]를 반환하고, 오류 세부는 전역이 아니라
//! handle별 `stellar_last_error`로 읽습니다.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// 버전 문자열(NUL 종료에 의존하지 않는 바이트 시퀀스).
const VERSION: &str = "0.0.0";

/// FFI 호출 상태 코드.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum StellarStatus {
    /// 성공.
    Ok = 0,
    /// 알 수 없는/이미 파괴된 handle.
    InvalidHandle = 1,
    /// null 포인터 인자.
    NullPointer = 2,
    /// 출력 buffer가 너무 작음.
    BufferTooSmall = 3,
    /// last error가 없음.
    NoError = 4,
    /// 내부 오류(예: 잠금 실패).
    Internal = 5,
}

/// registry에 등록되는 runtime 인스턴스. last error는 handle별로 보관합니다.
struct Runtime {
    last_error: Option<(StellarStatus, String)>,
}

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

/// `out` 포인터가 null이 아니면 값을 씁니다.
///
/// # Safety
/// `out`은 null이거나 `usize` 쓰기가 유효한 포인터여야 합니다.
unsafe fn write_len(out: *mut usize, value: usize) {
    if !out.is_null() {
        unsafe { *out = value };
    }
}

/// 새 runtime을 생성해 handle을 `out_handle`에 씁니다.
///
/// # Safety
/// `out_handle`은 `u64` 쓰기가 유효한 포인터여야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_runtime_create(out_handle: *mut u64) -> StellarStatus {
    if out_handle.is_null() {
        return StellarStatus::NullPointer;
    }
    let Ok(mut reg) = registry().lock() else {
        return StellarStatus::Internal;
    };
    let handle = reg.next;
    reg.next = reg.next.wrapping_add(1);
    reg.live.insert(handle, Runtime { last_error: None });
    unsafe { *out_handle = handle };
    StellarStatus::Ok
}

/// runtime을 파괴합니다. 알 수 없는 handle은 `InvalidHandle`(double free 방지).
#[no_mangle]
pub extern "C" fn stellar_runtime_destroy(handle: u64) -> StellarStatus {
    let Ok(mut reg) = registry().lock() else {
        return StellarStatus::Internal;
    };
    if reg.live.remove(&handle).is_some() {
        StellarStatus::Ok
    } else {
        StellarStatus::InvalidHandle
    }
}

/// 버전 문자열을 호출자 buffer에 UTF-8로 씁니다. `buf`가 null이면 필요한 길이만 보고합니다.
///
/// buffer가 작으면 해당 handle의 last error를 설정하고 `BufferTooSmall`을 반환합니다.
///
/// # Safety
/// `buf`는 null이거나 `buf_len` 바이트 쓰기가 유효해야 하고, `out_written`은 null이거나 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_runtime_version(
    handle: u64,
    buf: *mut u8,
    buf_len: usize,
    out_written: *mut usize,
) -> StellarStatus {
    let bytes = VERSION.as_bytes();
    let Ok(mut reg) = registry().lock() else {
        return StellarStatus::Internal;
    };
    let Some(rt) = reg.live.get_mut(&handle) else {
        return StellarStatus::InvalidHandle;
    };
    if buf.is_null() {
        unsafe { write_len(out_written, bytes.len()) };
        return StellarStatus::Ok;
    }
    if buf_len < bytes.len() {
        rt.last_error = Some((
            StellarStatus::BufferTooSmall,
            format!("version needs {} bytes, buffer {}", bytes.len(), buf_len),
        ));
        return StellarStatus::BufferTooSmall;
    }
    let out = unsafe { std::slice::from_raw_parts_mut(buf, buf_len) };
    out[..bytes.len()].copy_from_slice(bytes);
    unsafe { write_len(out_written, bytes.len()) };
    rt.last_error = None;
    StellarStatus::Ok
}

/// 해당 handle의 마지막 오류 메시지를 호출자 buffer에 씁니다.
///
/// 오류가 없으면 `NoError`. 전역 last error는 사용하지 않습니다.
///
/// # Safety
/// `buf`는 null이거나 `buf_len` 바이트 쓰기가 유효해야 하고, `out_written`은 null이거나 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_last_error(
    handle: u64,
    buf: *mut u8,
    buf_len: usize,
    out_written: *mut usize,
) -> StellarStatus {
    let Ok(reg) = registry().lock() else {
        return StellarStatus::Internal;
    };
    let Some(rt) = reg.live.get(&handle) else {
        return StellarStatus::InvalidHandle;
    };
    let Some((_code, message)) = &rt.last_error else {
        unsafe { write_len(out_written, 0) };
        return StellarStatus::NoError;
    };
    let bytes = message.as_bytes();
    if buf.is_null() {
        unsafe { write_len(out_written, bytes.len()) };
        return StellarStatus::Ok;
    }
    if buf_len < bytes.len() {
        return StellarStatus::BufferTooSmall;
    }
    let out = unsafe { std::slice::from_raw_parts_mut(buf, buf_len) };
    out[..bytes.len()].copy_from_slice(bytes);
    unsafe { write_len(out_written, bytes.len()) };
    StellarStatus::Ok
}

/// 현재 살아 있는 runtime handle 수(누수 검증용 진단 helper).
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

    fn create() -> u64 {
        let mut h = 0u64;
        let s = unsafe { stellar_runtime_create(&mut h) };
        assert_eq!(s, StellarStatus::Ok);
        assert!(h != 0);
        h
    }

    #[test]
    fn create_version_destroy_cycle() {
        let h = create();
        let mut buf = [0u8; 16];
        let mut written = 0usize;
        let s = unsafe { stellar_runtime_version(h, buf.as_mut_ptr(), buf.len(), &mut written) };
        assert_eq!(s, StellarStatus::Ok);
        assert_eq!(&buf[..written], VERSION.as_bytes());
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
        // double free
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::InvalidHandle);
    }

    #[test]
    fn null_handle_out_is_rejected() {
        assert_eq!(
            unsafe { stellar_runtime_create(std::ptr::null_mut()) },
            StellarStatus::NullPointer
        );
    }

    #[test]
    fn last_error_is_per_handle() {
        let h1 = create();
        let h2 = create();
        let mut written = 0usize;
        let mut ebuf = [0u8; 64];

        // h1에 BufferTooSmall 오류 유발(buffer 1).
        let mut tiny1 = [0u8; 1];
        let s1 =
            unsafe { stellar_runtime_version(h1, tiny1.as_mut_ptr(), tiny1.len(), &mut written) };
        assert_eq!(s1, StellarStatus::BufferTooSmall);

        // h2는 아직 오류 없음(전역이 아님을 증명).
        let e2 = unsafe { stellar_last_error(h2, ebuf.as_mut_ptr(), ebuf.len(), &mut written) };
        assert_eq!(e2, StellarStatus::NoError);

        // h1은 자신의 오류를 보유(메시지에 "buffer 1").
        let e1 = unsafe { stellar_last_error(h1, ebuf.as_mut_ptr(), ebuf.len(), &mut written) };
        assert_eq!(e1, StellarStatus::Ok);
        assert!(std::str::from_utf8(&ebuf[..written])
            .unwrap()
            .contains("buffer 1"));

        // h2에 다른 크기로 오류 유발(buffer 2).
        let mut tiny2 = [0u8; 2];
        let s2 =
            unsafe { stellar_runtime_version(h2, tiny2.as_mut_ptr(), tiny2.len(), &mut written) };
        assert_eq!(s2, StellarStatus::BufferTooSmall);
        let e2b = unsafe { stellar_last_error(h2, ebuf.as_mut_ptr(), ebuf.len(), &mut written) };
        assert_eq!(e2b, StellarStatus::Ok);
        assert!(std::str::from_utf8(&ebuf[..written])
            .unwrap()
            .contains("buffer 2"));

        // h1은 여전히 자신의 오류(섞이지 않음).
        let e1b = unsafe { stellar_last_error(h1, ebuf.as_mut_ptr(), ebuf.len(), &mut written) };
        assert_eq!(e1b, StellarStatus::Ok);
        assert!(std::str::from_utf8(&ebuf[..written])
            .unwrap()
            .contains("buffer 1"));

        assert_eq!(stellar_runtime_destroy(h1), StellarStatus::Ok);
        assert_eq!(stellar_runtime_destroy(h2), StellarStatus::Ok);
    }
}

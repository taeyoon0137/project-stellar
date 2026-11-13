//! stellar-ffi — C ABI 경계.
//!
//! runtime handle은 opaque pointer가 아니라 정수형 registry handle로 노출합니다(0=무효).
//! 모든 작업 함수는 [`StellarStatus`]를 반환하고, 오류 세부는 전역이 아니라
//! handle별 `stellar_last_error`로 읽습니다.
//! 모든 FFI 경계는 [`ffi_guard`]로 감싸 panic이 경계를 넘지 않고 `InternalPanic`으로 변환됩니다.

/// 양 플랫폼 공통 입력 이벤트와 native timestamp 변환 (XII.3.1).
pub mod input;
/// macOS `NSEvent` → 공통 입력 이벤트 adapter (XII.3.2).
pub mod input_macos;
/// 셸→코어 입력 대기열과 연속 move/wheel coalescing (XII.3.4).
pub mod input_queue;

use std::collections::{HashMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use stellar_core_types::clock::MonotonicClock;

use crate::input::{InputTimeDomain, NativeTimestamp};
use crate::input_macos::{convert_mac_event, StellarMacEvent};
use crate::input_queue::{InputQueue, PushOutcome};

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
    /// 입력 바이트가 유효한 UTF-8이 아님.
    InvalidUtf8 = 6,
    /// FFI 경계에서 잡힌 panic.
    InternalPanic = 7,
    /// 입력 값이 규칙에 맞지 않음(세부는 `stellar_last_error`).
    InvalidArgument = 8,
    /// 고정 용량 큐가 가득 참(입력은 버려지고 상태만 반환).
    QueueFull = 9,
}

/// 셸→코어 입력 이벤트 대기열 용량(이벤트 수).
///
/// 소비자(DocumentThread, XII.1.2)가 생기기 전에도 메모리가 무한히 늘지 않도록 고정합니다.
pub const INPUT_QUEUE_CAPACITY: usize = 4096;

/// 코어 단조 시계: 프로세스에서 처음 읽은 시점 이후 경과 nanoseconds(`Instant` 기반).
///
/// 모든 runtime이 같은 기준점을 공유하므로 runtime 간 입력 시각을 비교할 수 있습니다.
/// 기준점은 첫 `stellar_runtime_create`에서 고정합니다. 그래야 runtime 생성 뒤에 발생한
/// native 입력이 기준점 이전(0으로 고정)으로 눌리지 않습니다.
struct InstantClock;

impl MonotonicClock for InstantClock {
    fn now_nanos(&self) -> u64 {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        let elapsed = ORIGIN.get_or_init(Instant::now).elapsed().as_nanos();
        u64::try_from(elapsed).unwrap_or(u64::MAX)
    }
}

/// 셸이 전달한 drawable(backing) 픽셀 크기와 backing scale.
#[derive(Debug, Clone, Copy)]
struct Drawable {
    pixel_width: u32,
    pixel_height: u32,
    scale: f64,
}

/// registry에 등록되는 runtime 인스턴스. last error는 handle별로 보관합니다.
struct Runtime {
    last_error: Option<(StellarStatus, String)>,
    label: Option<String>,
    events: VecDeque<(u32, Vec<u8>)>,
    drawable: Option<Drawable>,
    /// 셸이 제출한 입력 이벤트(FIFO, 용량 [`INPUT_QUEUE_CAPACITY`], 연속 move/wheel 병합).
    input: InputQueue,
    /// native 입력 시각 → 코어 단조 domain 변환기(첫 입력에서 기준점을 잡음).
    input_domain: Option<InputTimeDomain>,
}

impl Runtime {
    fn new() -> Runtime {
        Runtime {
            last_error: None,
            label: None,
            events: VecDeque::new(),
            drawable: None,
            input: InputQueue::new(INPUT_QUEUE_CAPACITY),
            input_domain: None,
        }
    }
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

/// FFI 경계 guard: panic을 catch해 `InternalPanic`으로 변환합니다(프로세스 종료 방지).
fn ffi_guard<F: FnOnce() -> StellarStatus>(body: F) -> StellarStatus {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(status) => status,
        Err(_) => StellarStatus::InternalPanic,
    }
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
    ffi_guard(move || {
        if out_handle.is_null() {
            return StellarStatus::NullPointer;
        }
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let handle = reg.next;
        reg.next = reg.next.wrapping_add(1);
        reg.live.insert(handle, Runtime::new());
        // 코어 단조 시계 기준점을 첫 runtime 생성 시점으로 고정합니다(입력 시각 domain).
        let _ = InstantClock.now_nanos();
        unsafe { *out_handle = handle };
        StellarStatus::Ok
    })
}

/// runtime을 파괴합니다. 알 수 없는 handle은 `InvalidHandle`(double free 방지).
#[no_mangle]
pub extern "C" fn stellar_runtime_destroy(handle: u64) -> StellarStatus {
    ffi_guard(move || {
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        if reg.live.remove(&handle).is_some() {
            StellarStatus::Ok
        } else {
            StellarStatus::InvalidHandle
        }
    })
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
    ffi_guard(move || {
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
    })
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
    ffi_guard(move || {
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
    })
}

/// FFI 입력 문자열 규칙: UTF-8 byte 포인터 + length. NUL 종료에 의존하지 않습니다.
///
/// `len`이 0이면 빈 문자열(ptr 무시). 유효하지 않은 UTF-8이면 `InvalidUtf8`,
/// `len > 0`인데 ptr이 null이면 `NullPointer`.
///
/// # Safety
/// `len > 0`이면 `ptr`은 `len` 바이트 읽기가 유효한 포인터여야 합니다.
unsafe fn decode_utf8_owned(ptr: *const u8, len: usize) -> Result<String, StellarStatus> {
    if len == 0 {
        return Ok(String::new());
    }
    if ptr.is_null() {
        return Err(StellarStatus::NullPointer);
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_string()),
        Err(_) => Err(StellarStatus::InvalidUtf8),
    }
}

/// handle에 UTF-8 label을 설정합니다(입력은 ptr+len, NUL 종료 비의존).
///
/// # Safety
/// `len > 0`이면 `ptr`은 `len` 바이트 읽기가 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_runtime_set_label(
    handle: u64,
    ptr: *const u8,
    len: usize,
) -> StellarStatus {
    ffi_guard(move || {
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        match unsafe { decode_utf8_owned(ptr, len) } {
            Ok(label) => {
                rt.label = Some(label);
                StellarStatus::Ok
            }
            Err(status) => {
                rt.last_error = Some((status, "invalid label bytes".to_string()));
                status
            }
        }
    })
}

/// handle의 label을 호출자 buffer에 UTF-8 바이트로 씁니다(출력은 caller buffer).
///
/// # Safety
/// `buf`는 null이거나 `buf_len` 바이트 쓰기가 유효해야 하고, `out_written`은 null이거나 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_runtime_label(
    handle: u64,
    buf: *mut u8,
    buf_len: usize,
    out_written: *mut usize,
) -> StellarStatus {
    ffi_guard(move || {
        let Ok(reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let bytes = rt.label.as_deref().unwrap_or("").as_bytes();
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
    })
}

/// Core→Shell 이벤트를 handle 큐에 넣습니다(코어 내부용 Rust API).
///
/// 이벤트는 pull 방식 [`stellar_runtime_poll_event`]로만 읽으며, callback이 없으므로
/// 재진입이 발생하지 않습니다.
pub fn enqueue_event(handle: u64, kind: u32, payload: &[u8]) -> StellarStatus {
    let Ok(mut reg) = registry().lock() else {
        return StellarStatus::Internal;
    };
    let Some(rt) = reg.live.get_mut(&handle) else {
        return StellarStatus::InvalidHandle;
    };
    rt.events.push_back((kind, payload.to_vec()));
    StellarStatus::Ok
}

/// 큐에서 다음 이벤트를 pull 방식으로 읽습니다(FIFO). callback을 호출하지 않습니다.
///
/// 빈 큐는 `NoError`. `buf`가 null이면 다음 이벤트의 kind·필요 길이만 보고하고 pop하지 않습니다.
/// buffer가 작으면 `BufferTooSmall`(pop하지 않음). 성공 시 이벤트를 pop합니다.
///
/// # Safety
/// `out_kind`/`out_written`은 null이거나 유효한 포인터, `buf`는 null이거나 `buf_len` 쓰기가 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_runtime_poll_event(
    handle: u64,
    out_kind: *mut u32,
    buf: *mut u8,
    buf_len: usize,
    out_written: *mut usize,
) -> StellarStatus {
    ffi_guard(move || {
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let Some((front_kind, front_payload)) = rt.events.front() else {
            unsafe { write_len(out_written, 0) };
            return StellarStatus::NoError;
        };
        let kind = *front_kind;
        let need = front_payload.len();

        if buf.is_null() {
            if !out_kind.is_null() {
                unsafe { *out_kind = kind };
            }
            unsafe { write_len(out_written, need) };
            return StellarStatus::Ok;
        }
        if buf_len < need {
            return StellarStatus::BufferTooSmall;
        }

        let (kind, payload) = rt.events.pop_front().expect("front exists");
        if !out_kind.is_null() {
            unsafe { *out_kind = kind };
        }
        let out = unsafe { std::slice::from_raw_parts_mut(buf, buf_len) };
        out[..payload.len()].copy_from_slice(&payload);
        unsafe { write_len(out_written, payload.len()) };
        StellarStatus::Ok
    })
}

/// 셸이 계산한 drawable(backing) 픽셀 크기와 backing scale을 코어에 전달합니다.
///
/// 렌더링은 하지 않고 값만 보관합니다(II.5.4 — 전달 구조만). 픽셀 크기는 셸에서
/// `point 크기 × backing scale`로 계산해 넘깁니다.
#[no_mangle]
pub extern "C" fn stellar_runtime_set_drawable_size(
    handle: u64,
    pixel_width: u32,
    pixel_height: u32,
    scale: f64,
) -> StellarStatus {
    ffi_guard(move || {
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        rt.drawable = Some(Drawable {
            pixel_width,
            pixel_height,
            scale,
        });
        StellarStatus::Ok
    })
}

/// 마지막으로 전달된 drawable 픽셀 크기와 scale을 읽습니다(검증·진단용).
///
/// 아직 설정되지 않았으면 `NoError`. 각 out 포인터는 null이면 무시합니다.
///
/// # Safety
/// `out_pixel_width`/`out_pixel_height`/`out_scale`은 null이거나 각각 유효한 쓰기 포인터여야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_runtime_drawable_size(
    handle: u64,
    out_pixel_width: *mut u32,
    out_pixel_height: *mut u32,
    out_scale: *mut f64,
) -> StellarStatus {
    ffi_guard(move || {
        let Ok(reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let Some(drawable) = rt.drawable else {
            return StellarStatus::NoError;
        };
        if !out_pixel_width.is_null() {
            unsafe { *out_pixel_width = drawable.pixel_width };
        }
        if !out_pixel_height.is_null() {
            unsafe { *out_pixel_height = drawable.pixel_height };
        }
        if !out_scale.is_null() {
            unsafe { *out_scale = drawable.scale };
        }
        StellarStatus::Ok
    })
}

/// macOS 셸이 받은 `NSEvent` 원시 필드를 공통 입력 이벤트로 변환해 입력 대기열에 넣습니다(XII.3.2).
///
/// - 첫 입력에서 `receive_uptime`(셸 수신 시각)과 코어 단조 시계로 시간 domain 기준점을 잡습니다.
/// - 공통 집합에 없는 이벤트(enter/exit, gesture, 표에 없는 key 등)는 넣지 않고 `out_queued = 0`.
/// - 대기열에 반영되면 `out_queued = 1`입니다. 끝 항목과 병합된 연속 move/wheel도 반영으로 셉니다(XII.3.4).
/// - 규칙 위반은 `InvalidArgument`, 대기열이 가득 차면 `QueueFull`(둘 다 last error 설정, 입력은 버림).
///
/// # Safety
/// `event`는 `StellarMacEvent` 읽기가 유효한 포인터여야 하고, `out_queued`는 null이거나 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_runtime_submit_mac_event(
    handle: u64,
    event: *const StellarMacEvent,
    out_queued: *mut u32,
) -> StellarStatus {
    ffi_guard(move || {
        if event.is_null() {
            return StellarStatus::NullPointer;
        }
        let raw = unsafe { *event };
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let write_queued = |count: u32| {
            if !out_queued.is_null() {
                unsafe { *out_queued = count };
            }
        };
        write_queued(0);
        let domain = match rt.input_domain.as_mut() {
            Some(domain) => domain,
            None => {
                let anchor = InputTimeDomain::new(
                    NativeTimestamp::MacEventSeconds(raw.receive_uptime),
                    &InstantClock,
                );
                match anchor {
                    Ok(domain) => rt.input_domain.insert(domain),
                    Err(error) => {
                        rt.last_error = Some((
                            StellarStatus::InvalidArgument,
                            format!("입력 시각 기준점 오류: {error}"),
                        ));
                        return StellarStatus::InvalidArgument;
                    }
                }
            }
        };
        match convert_mac_event(&raw, domain) {
            Ok(None) => StellarStatus::Ok,
            Ok(Some(input)) => match rt.input.push(input) {
                PushOutcome::Queued | PushOutcome::Coalesced => {
                    write_queued(1);
                    StellarStatus::Ok
                }
                PushOutcome::Full => {
                    rt.last_error = Some((
                        StellarStatus::QueueFull,
                        format!("입력 대기열이 가득 찼습니다({INPUT_QUEUE_CAPACITY})"),
                    ));
                    StellarStatus::QueueFull
                }
            },
            Err(error) => {
                rt.last_error = Some((StellarStatus::InvalidArgument, error.to_string()));
                StellarStatus::InvalidArgument
            }
        }
    })
}

/// 입력 대기열의 다음 공통 입력 이벤트를 직렬화 바이트(`InputEvent::encode`)로 읽습니다(FIFO).
///
/// 빈 대기열은 `NoError`. `buf`가 null이면 필요한 길이만 보고하고 꺼내지 않습니다.
/// buffer가 작으면 `BufferTooSmall`(꺼내지 않음). 성공 시 이벤트를 꺼냅니다.
///
/// # Safety
/// `buf`는 null이거나 `buf_len` 바이트 쓰기가 유효해야 하고, `out_written`은 null이거나 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_runtime_poll_input(
    handle: u64,
    buf: *mut u8,
    buf_len: usize,
    out_written: *mut usize,
) -> StellarStatus {
    ffi_guard(move || {
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let Some(front) = rt.input.front() else {
            unsafe { write_len(out_written, 0) };
            return StellarStatus::NoError;
        };
        let bytes = front.encode();
        if buf.is_null() {
            unsafe { write_len(out_written, bytes.len()) };
            return StellarStatus::Ok;
        }
        if buf_len < bytes.len() {
            return StellarStatus::BufferTooSmall;
        }
        rt.input.pop();
        let out = unsafe { std::slice::from_raw_parts_mut(buf, buf_len) };
        out[..bytes.len()].copy_from_slice(&bytes);
        unsafe { write_len(out_written, bytes.len()) };
        StellarStatus::Ok
    })
}

/// 현재 살아 있는 runtime handle 수(누수 검증용 진단 helper).
#[no_mangle]
pub extern "C" fn stellar_runtime_live_count() -> u64 {
    catch_unwind(|| {
        registry()
            .lock()
            .map(|reg| reg.live.len() as u64)
            .unwrap_or(u64::MAX)
    })
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

    #[test]
    fn label_uses_ptr_len_not_nul_termination() {
        let h = create();
        // 중간 NUL이 있어도 len 기반으로 보존됩니다(NUL 종료 비의존).
        let input = b"ab\0cd";
        assert_eq!(
            unsafe { stellar_runtime_set_label(h, input.as_ptr(), input.len()) },
            StellarStatus::Ok
        );
        let mut buf = [0u8; 16];
        let mut written = 0usize;
        let r = unsafe { stellar_runtime_label(h, buf.as_mut_ptr(), buf.len(), &mut written) };
        assert_eq!(r, StellarStatus::Ok);
        assert_eq!(written, 5);
        assert_eq!(&buf[..written], input);
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
    }

    #[test]
    fn empty_label_is_ok() {
        let h = create();
        assert_eq!(
            unsafe { stellar_runtime_set_label(h, std::ptr::null(), 0) },
            StellarStatus::Ok
        );
        let mut written = 7usize;
        let r = unsafe { stellar_runtime_label(h, std::ptr::null_mut(), 0, &mut written) };
        assert_eq!(r, StellarStatus::Ok);
        assert_eq!(written, 0);
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
    }

    #[test]
    fn invalid_utf8_label_is_rejected() {
        let h = create();
        let bad = [0xFFu8, 0xFE];
        assert_eq!(
            unsafe { stellar_runtime_set_label(h, bad.as_ptr(), bad.len()) },
            StellarStatus::InvalidUtf8
        );
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
    }

    #[test]
    fn ffi_guard_converts_panic_to_status() {
        // 테스트 전용 panic이 경계를 넘지 않고 InternalPanic으로 변환됨(프로세스 생존).
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {})); // 패닉 메시지 출력 억제
        let status = ffi_guard(|| panic!("forced panic for test"));
        std::panic::set_hook(prev);
        assert_eq!(status, StellarStatus::InternalPanic);
    }

    #[test]
    fn operations_do_not_panic_on_edge_inputs() {
        // release abort 정책에서도 정상 경로는 panic 없이 상태를 반환합니다.
        let mut written = 0usize;
        assert_eq!(
            stellar_runtime_destroy(999_999),
            StellarStatus::InvalidHandle
        );
        assert_eq!(
            unsafe { stellar_runtime_version(999_999, std::ptr::null_mut(), 0, &mut written) },
            StellarStatus::InvalidHandle
        );
        assert_eq!(
            unsafe { stellar_last_error(999_999, std::ptr::null_mut(), 0, &mut written) },
            StellarStatus::InvalidHandle
        );
    }

    #[test]
    fn poll_event_empty_single_multiple() {
        let h = create();
        let mut kind = 0u32;
        let mut written = 0usize;
        let mut buf = [0u8; 16];

        // 빈 큐 → NoError
        assert_eq!(
            unsafe {
                stellar_runtime_poll_event(h, &mut kind, buf.as_mut_ptr(), buf.len(), &mut written)
            },
            StellarStatus::NoError
        );

        // 단일 이벤트
        assert_eq!(enqueue_event(h, 7, b"x"), StellarStatus::Ok);
        let s = unsafe {
            stellar_runtime_poll_event(h, &mut kind, buf.as_mut_ptr(), buf.len(), &mut written)
        };
        assert_eq!(s, StellarStatus::Ok);
        assert_eq!(kind, 7);
        assert_eq!(&buf[..written], b"x");

        // 비워진 뒤 다시 NoError
        assert_eq!(
            unsafe {
                stellar_runtime_poll_event(h, &mut kind, buf.as_mut_ptr(), buf.len(), &mut written)
            },
            StellarStatus::NoError
        );

        // 여러 이벤트 FIFO
        enqueue_event(h, 1, b"a");
        enqueue_event(h, 2, b"bb");
        enqueue_event(h, 3, b"ccc");
        for (expect_kind, expect_payload) in [
            (1u32, b"a".as_slice()),
            (2, b"bb".as_slice()),
            (3, b"ccc".as_slice()),
        ] {
            let s = unsafe {
                stellar_runtime_poll_event(h, &mut kind, buf.as_mut_ptr(), buf.len(), &mut written)
            };
            assert_eq!(s, StellarStatus::Ok);
            assert_eq!(kind, expect_kind);
            assert_eq!(&buf[..written], expect_payload);
        }
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
    }

    #[test]
    fn drawable_size_round_trip() {
        let h = create();
        // 설정 전에는 NoError.
        let mut w = 0u32;
        let mut ht = 0u32;
        let mut scale = 0.0f64;
        assert_eq!(
            unsafe { stellar_runtime_drawable_size(h, &mut w, &mut ht, &mut scale) },
            StellarStatus::NoError
        );

        // point 800x600 @ scale 2.0 → pixel 1600x1200.
        assert_eq!(
            stellar_runtime_set_drawable_size(h, 1600, 1200, 2.0),
            StellarStatus::Ok
        );
        assert_eq!(
            unsafe { stellar_runtime_drawable_size(h, &mut w, &mut ht, &mut scale) },
            StellarStatus::Ok
        );
        assert_eq!((w, ht), (1600, 1200));
        assert_eq!(scale, 2.0);

        // null out 포인터는 무시(상태만 Ok).
        assert_eq!(
            unsafe {
                stellar_runtime_drawable_size(
                    h,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            },
            StellarStatus::Ok
        );

        // 알 수 없는 handle.
        assert_eq!(
            stellar_runtime_set_drawable_size(999_999, 1, 1, 1.0),
            StellarStatus::InvalidHandle
        );
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
    }

    #[test]
    fn poll_event_buffer_too_small_keeps_event() {
        let h = create();
        assert_eq!(enqueue_event(h, 9, b"hello"), StellarStatus::Ok);
        let mut kind = 0u32;
        let mut written = 0usize;
        let mut small = [0u8; 2];
        // 작은 buffer → BufferTooSmall(이벤트 보존)
        assert_eq!(
            unsafe {
                stellar_runtime_poll_event(
                    h,
                    &mut kind,
                    small.as_mut_ptr(),
                    small.len(),
                    &mut written,
                )
            },
            StellarStatus::BufferTooSmall
        );
        // 충분한 buffer → Ok
        let mut buf = [0u8; 8];
        let s = unsafe {
            stellar_runtime_poll_event(h, &mut kind, buf.as_mut_ptr(), buf.len(), &mut written)
        };
        assert_eq!(s, StellarStatus::Ok);
        assert_eq!(kind, 9);
        assert_eq!(&buf[..written], b"hello");
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
    }

    #[test]
    fn header_declares_every_extern_fn_and_shared_constant() {
        // 생성 헤더(템플릿)가 Rust C ABI 표면·상수와 어긋나지 않는지 검사합니다.
        let header = include_str!("../include/stellar_ffi.h");
        let source = include_str!("lib.rs");
        let mut names = Vec::new();
        for line in source.lines() {
            let line = line.trim_start();
            let rest = line
                .strip_prefix("pub unsafe extern \"C\" fn ")
                .or_else(|| line.strip_prefix("pub extern \"C\" fn "));
            if let Some(rest) = rest {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                names.push(name);
            }
        }
        assert!(names.len() >= 12, "extern fn 수집 실패: {names:?}");
        for name in &names {
            assert!(
                header.contains(&format!(" {name}(")),
                "헤더에 {name} 선언 없음"
            );
        }
        assert!(header.contains(&format!(
            "#define STELLAR_INPUT_QUEUE_CAPACITY {INPUT_QUEUE_CAPACITY}\n"
        )));
        assert!(header.contains(&format!(
            "#define STELLAR_MAC_EVENT_VERSION {}\n",
            crate::input_macos::MAC_EVENT_VERSION
        )));
        assert!(header.contains("STELLAR_INVALID_ARGUMENT = 8"));
        assert!(header.contains("STELLAR_QUEUE_FULL = 9"));
        assert!(header.contains(&format!(
            "sizeof(StellarMacEvent) == {}",
            std::mem::size_of::<StellarMacEvent>()
        )));
    }

    mod mac_input {
        use super::*;
        use crate::input::{InputEvent, InputEventKind, PointerButton, PointerPhase};
        use crate::input_macos::{event_type, MAC_EVENT_VERSION};

        fn mac_event(event_type: u32, timestamp: f64) -> StellarMacEvent {
            StellarMacEvent {
                version: MAC_EVENT_VERSION,
                event_type,
                timestamp,
                receive_uptime: timestamp,
                x: 12.5,
                y: 8.0,
                click_count: 2,
                ..StellarMacEvent::default()
            }
        }

        fn submit(h: u64, event: &StellarMacEvent) -> (StellarStatus, u32) {
            let mut queued = 7u32;
            let status = unsafe { stellar_runtime_submit_mac_event(h, event, &mut queued) };
            (status, queued)
        }

        fn poll(h: u64) -> Result<InputEvent, StellarStatus> {
            let mut need = 0usize;
            let status =
                unsafe { stellar_runtime_poll_input(h, std::ptr::null_mut(), 0, &mut need) };
            if status != StellarStatus::Ok {
                return Err(status);
            }
            let mut buf = vec![0u8; need];
            let mut written = 0usize;
            let status =
                unsafe { stellar_runtime_poll_input(h, buf.as_mut_ptr(), buf.len(), &mut written) };
            if status != StellarStatus::Ok {
                return Err(status);
            }
            Ok(InputEvent::decode(&buf[..written]).expect("decode"))
        }

        fn last_error(h: u64) -> String {
            let mut buf = [0u8; 256];
            let mut written = 0usize;
            let status =
                unsafe { stellar_last_error(h, buf.as_mut_ptr(), buf.len(), &mut written) };
            assert_eq!(status, StellarStatus::Ok);
            String::from_utf8(buf[..written].to_vec()).expect("utf8")
        }

        #[test]
        fn submitted_mac_events_are_polled_in_order() {
            let h = create();
            assert_eq!(poll(h), Err(StellarStatus::NoError));
            let down = mac_event(event_type::LEFT_MOUSE_DOWN, 50.0);
            let up = mac_event(event_type::LEFT_MOUSE_UP, 50.25);
            assert_eq!(submit(h, &down), (StellarStatus::Ok, 1));
            assert_eq!(submit(h, &up), (StellarStatus::Ok, 1));
            let first = poll(h).expect("down");
            let second = poll(h).expect("up");
            for (event, phase) in [(&first, PointerPhase::Down), (&second, PointerPhase::Up)] {
                let InputEventKind::Pointer(pointer) = event.kind() else {
                    panic!("pointer가 아님");
                };
                assert_eq!(pointer.phase, phase);
                assert_eq!(pointer.button, PointerButton::Primary);
                assert_eq!(pointer.click_count, 2);
            }
            // 같은 domain: 두 이벤트 간격은 native 간격(0.25초)과 같습니다.
            assert_eq!(
                second.timestamp().get() - first.timestamp().get(),
                250_000_000
            );
            assert_eq!(poll(h), Err(StellarStatus::NoError));
            assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
        }

        #[test]
        fn ten_thousand_mac_drags_poll_as_latest_one_between_down_and_up() {
            // XII.3.4: down → 10,000 drag → up 제출이 down, 최신 drag 1개, up 순서로 읽힙니다.
            let h = create();
            let down = mac_event(event_type::LEFT_MOUSE_DOWN, 10.0);
            assert_eq!(submit(h, &down), (StellarStatus::Ok, 1));
            for i in 0..10_000u32 {
                let mut drag =
                    mac_event(event_type::LEFT_MOUSE_DRAGGED, 10.0 + f64::from(i) * 1e-4);
                drag.x = f64::from(i);
                drag.y = f64::from(i) * 0.5;
                assert_eq!(submit(h, &drag), (StellarStatus::Ok, 1));
            }
            let up = mac_event(event_type::LEFT_MOUSE_UP, 11.5);
            assert_eq!(submit(h, &up), (StellarStatus::Ok, 1));

            let events: Vec<InputEvent> = std::iter::from_fn(|| poll(h).ok()).collect();
            let pointers: Vec<_> = events
                .iter()
                .map(|event| match event.kind() {
                    InputEventKind::Pointer(pointer) => *pointer,
                    other => panic!("pointer가 아님: {other:?}"),
                })
                .collect();
            let phases: Vec<PointerPhase> = pointers.iter().map(|p| p.phase).collect();
            assert_eq!(
                phases,
                [PointerPhase::Down, PointerPhase::Move, PointerPhase::Up]
            );
            assert_eq!(pointers[1].position.x(), 9_999.0);
            assert_eq!(pointers[1].position.y(), 4_999.5);
            assert_eq!(pointers[1].button, PointerButton::Primary);
            assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
        }

        #[test]
        fn ignored_and_invalid_mac_events() {
            let h = create();
            // 공통 집합 밖 이벤트는 넣지 않습니다.
            assert_eq!(
                submit(h, &mac_event(event_type::MOUSE_ENTERED, 1.0)),
                (StellarStatus::Ok, 0)
            );
            assert_eq!(poll(h), Err(StellarStatus::NoError));
            // 규칙 위반은 InvalidArgument + handle별 last error.
            let mut bad = mac_event(event_type::LEFT_MOUSE_DOWN, 1.0);
            bad.click_count = 0;
            assert_eq!(submit(h, &bad), (StellarStatus::InvalidArgument, 0));
            assert!(last_error(h).contains("click count"));
            assert_eq!(poll(h), Err(StellarStatus::NoError));
            // null 이벤트·알 수 없는 handle.
            assert_eq!(
                unsafe {
                    stellar_runtime_submit_mac_event(h, std::ptr::null(), std::ptr::null_mut())
                },
                StellarStatus::NullPointer
            );
            let event = mac_event(event_type::MOUSE_MOVED, 1.0);
            assert_eq!(
                unsafe { stellar_runtime_submit_mac_event(999_999, &event, std::ptr::null_mut()) },
                StellarStatus::InvalidHandle
            );
            assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
        }

        #[test]
        fn invalid_receive_uptime_does_not_anchor() {
            let h = create();
            let mut event = mac_event(event_type::MOUSE_MOVED, 1.0);
            event.receive_uptime = f64::NAN;
            assert_eq!(submit(h, &event), (StellarStatus::InvalidArgument, 0));
            assert!(last_error(h).contains("기준점"));
            // 올바른 수신 시각이 오면 그때 기준점을 잡습니다.
            event.receive_uptime = 1.0;
            assert_eq!(submit(h, &event), (StellarStatus::Ok, 1));
            assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
        }

        #[test]
        fn input_queue_is_bounded_and_poll_keeps_event_on_small_buffer() {
            let h = create();
            let mut event = mac_event(event_type::KEY_DOWN, 1.0);
            event.key_code = 0x00;
            for _ in 0..INPUT_QUEUE_CAPACITY {
                assert_eq!(submit(h, &event).0, StellarStatus::Ok);
            }
            assert_eq!(submit(h, &event), (StellarStatus::QueueFull, 0));
            assert!(last_error(h).contains("가득"));
            // 작은 buffer는 꺼내지 않습니다.
            let mut small = [0u8; 2];
            let mut written = 0usize;
            assert_eq!(
                unsafe {
                    stellar_runtime_poll_input(h, small.as_mut_ptr(), small.len(), &mut written)
                },
                StellarStatus::BufferTooSmall
            );
            let mut drained = 0usize;
            while poll(h).is_ok() {
                drained += 1;
            }
            assert_eq!(drained, INPUT_QUEUE_CAPACITY);
            // 비운 뒤에는 다시 받습니다.
            assert_eq!(submit(h, &event), (StellarStatus::Ok, 1));
            assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
        }
    }
}

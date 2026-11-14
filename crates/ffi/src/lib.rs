//! stellar-ffi — C ABI 경계.
//!
//! runtime handle은 opaque pointer가 아니라 정수형 registry handle로 노출합니다(0=무효).
//! 모든 작업 함수는 [`StellarStatus`]를 반환하고, 오류 세부는 전역이 아니라
//! handle별 `stellar_last_error`로 읽습니다.
//! 모든 FFI 경계는 [`ffi_guard`]로 감싸 panic이 경계를 넘지 않고 `InternalPanic`으로 변환됩니다.
//!
//! 프로젝트·문서 요청(`stellar_project_*`, `stellar_document_*`)은 `stellar-runtime`의
//! [`AppRuntime`]으로 넘깁니다. 요청은 기다리지 않고 request id를 돌려주며, 완료는
//! [`stellar_runtime_poll_event`]로 pull하는 event와 [`stellar_runtime_state`]·
//! [`stellar_document_info`]로 읽는 상태 snapshot으로만 셸에 돌아옵니다(ADR-0014). Rust 타입은
//! 경계를 넘지 않고 정수 handle·고정 POD 구조체·버전형 바이트만 씁니다. `AppRuntime`(document
//! thread)은 handle의 첫 프로젝트·문서 요청에서 시작합니다.

pub mod canvas;
mod component_library;
mod git_names;
/// 양 플랫폼 공통 입력 이벤트와 native timestamp 변환 (XII.3.1).
pub mod input;
/// macOS `NSEvent` → 공통 입력 이벤트 adapter (XII.3.2).
pub mod input_macos;
/// 셸→코어 입력 대기열과 연속 move/wheel coalescing (XII.3.4).
pub mod input_queue;
mod sidebar;
/// 창 session C ABI (XII.2.2).
pub mod window_session;

use std::collections::{HashMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use stellar_core_types::clock::MonotonicClock;
use stellar_core_types::ids::StableId;
use stellar_runtime::app::{
    AppRuntime, DocumentSessionId, EventPoll, ProjectSessionId, RequestError, RequestId,
};

use crate::input::{InputTimeDomain, NativeTimestamp};
use crate::input_macos::{convert_mac_event, StellarMacEvent};
use crate::input_queue::{InputQueue, PushOutcome};
use stellar_runtime::input_trace::InputReceipt;

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
    /// native 입력 시각 → 코어 단조 domain 변환기(검증을 통과한 첫 입력에서 기준점을 잡음).
    input_domain: Option<InputTimeDomain>,
    /// 프로젝트·문서 요청을 처리하는 앱 runtime(첫 요청에서 시작).
    app: Option<AppRuntime>,
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
            app: None,
        }
    }

    /// 앱 runtime을 돌려줍니다. 아직 없으면 시작합니다(실패 시 last error 설정).
    fn app(&mut self) -> Result<&AppRuntime, StellarStatus> {
        if self.app.is_none() {
            match AppRuntime::start() {
                Ok(app) => self.app = Some(app),
                Err(error) => {
                    self.last_error = Some((
                        StellarStatus::Internal,
                        format!("코어 runtime을 시작하지 못했습니다: {error}"),
                    ));
                    return Err(StellarStatus::Internal);
                }
            }
        }
        self.app.as_ref().ok_or(StellarStatus::Internal)
    }

    /// 요청 접수 결과를 상태 코드로 바꾸고 request id를 씁니다(실패 시 0과 last error).
    ///
    /// # Safety
    /// `out_request`는 null이거나 `u64` 쓰기가 유효한 포인터여야 합니다.
    unsafe fn finish_request(
        &mut self,
        result: Result<RequestId, RequestError>,
        out_request: *mut u64,
    ) -> StellarStatus {
        let (status, request) = match result {
            Ok(request) => (StellarStatus::Ok, request.0),
            Err(error) => {
                let status = match error {
                    RequestError::QueueFull => StellarStatus::QueueFull,
                    RequestError::Stopped => StellarStatus::Internal,
                    RequestError::InvalidPath { .. } => StellarStatus::InvalidArgument,
                };
                self.last_error = Some((status, error.to_string()));
                (status, 0)
            }
        };
        if !out_request.is_null() {
            unsafe { *out_request = request };
        }
        status
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

/// 열린 프로젝트의 신뢰를 읽거나 사용자가 확인한 결정을 기록합니다.
/// action 0은 조회, 1은 등록, 2는 철회입니다.
///
/// # Safety
/// `out_request`는 null이거나 `u64` 쓰기가 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_project_trust(
    handle: u64,
    project: u64,
    action: u32,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        let decision = match action {
            0 => None,
            1 => Some(true),
            2 => Some(false),
            _ => return StellarStatus::InvalidArgument,
        };
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.project_trust(ProjectSessionId(project), decision),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
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
        let removed = reg.live.remove(&handle);
        // registry lock을 놓은 뒤 파괴합니다. 앱 runtime 종료는 document thread join을 기다리므로
        // lock을 쥔 채 기다리면 다른 handle의 호출이 막힙니다.
        drop(reg);
        match removed {
            Some(runtime) => {
                drop(runtime);
                StellarStatus::Ok
            }
            None => StellarStatus::InvalidHandle,
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
            // 코어 내부 event가 없으면 앱 runtime의 완료 event를 같은 규칙으로 읽습니다.
            return unsafe { poll_app_event(rt, out_kind, buf, buf_len, out_written) };
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
/// - 검증을 통과한 첫 입력에서 `receive_uptime`(셸 수신 시각)과 코어 단조 시계로 시간 domain 기준점을
///   잡습니다. 거부된 이벤트는 기준점을 잡지 않습니다. 수신 시각보다 10초 넘게 늦은 `timestamp`는 수신
///   시각으로 자릅니다([`convert_mac_event`]).
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
        // T0 후보(XII.3.5): 잠금과 변환 전에 코어 경계 수신 시각을 기록합니다.
        let received = InstantClock.now_nanos();
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
        // 기준점이 없으면 이 이벤트의 수신 시각으로 임시 domain을 만들고, 이벤트가 검증을 통과했을
        // 때만 runtime에 남깁니다. 거부될 이벤트가 기준점을 영구히 정하지 않게 합니다.
        let anchored = rt.input_domain.is_some();
        let mut domain = match rt.input_domain.take() {
            Some(domain) => domain,
            None => {
                let anchor = InputTimeDomain::new(
                    NativeTimestamp::MacEventSeconds(raw.receive_uptime),
                    &InstantClock,
                );
                match anchor {
                    Ok(domain) => domain,
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
        let converted = convert_mac_event(&raw, &mut domain);
        if anchored || converted.is_ok() {
            rt.input_domain = Some(domain);
        }
        match converted {
            Ok(None) => StellarStatus::Ok,
            Ok(Some(input)) => {
                let receipt = InputReceipt::new(input.timestamp().get(), received);
                match rt.input.push_received(input, receipt) {
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
                }
            }
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

/// 앱 runtime 완료 event를 `stellar_runtime_poll_event` 규칙으로 읽습니다.
///
/// # Safety
/// [`stellar_runtime_poll_event`]와 같습니다.
unsafe fn poll_app_event(
    rt: &Runtime,
    out_kind: *mut u32,
    buf: *mut u8,
    buf_len: usize,
    out_written: *mut usize,
) -> StellarStatus {
    let Some(app) = rt.app.as_ref() else {
        unsafe { write_len(out_written, 0) };
        return StellarStatus::NoError;
    };
    let capacity = (!buf.is_null()).then_some(buf_len);
    match app.poll_event(capacity) {
        EventPoll::Empty => {
            unsafe { write_len(out_written, 0) };
            StellarStatus::NoError
        }
        EventPoll::Pending { kind, len } => {
            if !buf.is_null() {
                return StellarStatus::BufferTooSmall;
            }
            if !out_kind.is_null() {
                unsafe { *out_kind = kind };
            }
            unsafe { write_len(out_written, len) };
            StellarStatus::Ok
        }
        EventPoll::Taken { kind, payload } => {
            if !out_kind.is_null() {
                unsafe { *out_kind = kind };
            }
            let out = unsafe { std::slice::from_raw_parts_mut(buf, buf_len) };
            out[..payload.len()].copy_from_slice(&payload);
            unsafe { write_len(out_written, payload.len()) };
            StellarStatus::Ok
        }
    }
}

/// `StellarRuntimeState` 레이아웃 버전. 2: `window_session_count` 추가(XII.2.2, 48 byte).
pub const RUNTIME_STATE_VERSION: u32 = 2;

/// `StellarDocumentInfo` 레이아웃 버전.
pub const DOCUMENT_INFO_VERSION: u32 = 1;

/// 앱 runtime 상태 snapshot 요약(고정 48 byte, C ABI).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct StellarRuntimeState {
    /// [`RUNTIME_STATE_VERSION`](코어가 씀).
    pub version: u32,
    /// 0.
    pub reserved: u32,
    /// 상태 snapshot 게시 세대(성공한 요청마다 1 증가, 0 = 아직 없음).
    pub generation: u64,
    /// 열린 프로젝트 session 수.
    pub project_count: u64,
    /// 열린 문서 session 수.
    pub document_count: u64,
    /// 완료 event를 아직 꺼내지 않은 요청 수(실행 중 포함).
    pub outstanding: u64,
    /// 열린 창 session 수(XII.2.2).
    pub window_session_count: u64,
}

/// 열린 문서 session 정보(고정 56 byte, C ABI).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct StellarDocumentInfo {
    /// [`DOCUMENT_INFO_VERSION`](코어가 씀).
    pub version: u32,
    /// 0.
    pub reserved: u32,
    /// 문서 session handle.
    pub document: u64,
    /// 소속 프로젝트 session handle.
    pub project: u64,
    /// snapshot 게시 시점의 문서 revision.
    pub revision: u64,
    /// snapshot 게시 시점의 node 수.
    pub node_count: u64,
    /// 문서 StableId 16 byte(UUID 바이트 순서).
    pub document_id: [u8; 16],
}

/// 프로젝트 폴더 열기를 요청합니다(XII.6.2 코어 측). 기다리지 않고 request id를 씁니다.
///
/// 경로는 UTF-8 ptr+len 절대 경로입니다. 결과는 `STELLAR_EVENT_PROJECT_OPENED` 또는
/// `STELLAR_EVENT_REQUEST_FAILED` event로 옵니다. 빈 경로·상대 경로·NUL 포함은
/// `InvalidArgument`, 처리 대기 요청이 가득 차면 `QueueFull`입니다(요청 없음, `out_request = 0`).
///
/// # Safety
/// `path_len > 0`이면 `path`는 `path_len` 바이트 읽기가 유효해야 하고, `out_request`는 null이거나
/// 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_project_open(
    handle: u64,
    path: *const u8,
    path_len: usize,
    out_request: *mut u64,
) -> StellarStatus {
    unsafe {
        project_path_request(
            handle,
            path,
            path_len,
            out_request,
            AppRuntime::open_project,
        )
    }
}

/// 새 폴더로 프로젝트 생성 요청. 결과는 PROJECT_CREATED 또는 REQUEST_FAILED입니다.
///
/// # Safety
/// path와 out_request의 조건은 stellar_project_open과 같습니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_project_create(
    handle: u64,
    path: *const u8,
    path_len: usize,
    out_request: *mut u64,
) -> StellarStatus {
    unsafe {
        project_path_request(
            handle,
            path,
            path_len,
            out_request,
            AppRuntime::create_project,
        )
    }
}

/// 이름으로 새 프로젝트 컴포넌트 원본을 만드는 비동기 요청.
/// # Safety
/// name은 name_len개의 읽을 수 있는 UTF-8 바이트, out_request는 쓸 수 있는 u64여야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_component_create(
    handle: u64,
    session: u64,
    name: *const u8,
    name_len: usize,
    out_request: *mut u64,
) -> StellarStatus {
    unsafe { component_create_request(handle, session, name, name_len, None, out_request) }
}

/// 이름 sheet가 표시한 선택과 같은 Frame subtree를 독립 원본으로 만듭니다.
/// # Safety
/// view와 node는 각각 16바이트 UUID입니다. 이름과 출력은 stellar_window_component_create와 같습니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_component_from_selection(
    handle: u64,
    session: u64,
    name: *const u8,
    name_len: usize,
    view: *const [u8; 16],
    node: *const [u8; 16],
    revision: u64,
    out_request: *mut u64,
) -> StellarStatus {
    if view.is_null() || node.is_null() {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        return StellarStatus::NullPointer;
    }
    let selection = unsafe {
        (
            StableId::from_bytes(*view),
            StableId::from_bytes(*node),
            revision,
        )
    };
    unsafe {
        component_create_request(
            handle,
            session,
            name,
            name_len,
            Some(selection),
            out_request,
        )
    }
}

unsafe fn component_create_request(
    handle: u64,
    session: u64,
    name: *const u8,
    name_len: usize,
    selection: Option<(StableId, StableId, u64)>,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if out_request.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe {
            *out_request = 0;
        }
        if name_len == 0 || name_len > 255 {
            return StellarStatus::InvalidArgument;
        }
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let name = match unsafe { decode_utf8_owned(name, name_len) } {
            Ok(name) => name,
            Err(status) => return status,
        };
        let result = match rt.app() {
            Ok(app) => {
                let session = stellar_runtime::app::WindowSessionHandle(session);
                match selection {
                    Some((view, node, revision)) => app.create_component_from_selection(
                        session,
                        name,
                        view,
                        stellar_core_types::revision::DocumentRevision(revision),
                        node,
                    ),
                    None => app.create_component(session, name),
                }
            }
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

unsafe fn project_path_request(
    handle: u64,
    path: *const u8,
    path_len: usize,
    out_request: *mut u64,
    submit: fn(&AppRuntime, &std::path::Path) -> Result<RequestId, RequestError>,
) -> StellarStatus {
    ffi_guard(move || {
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let root = match unsafe { decode_utf8_owned(path, path_len) } {
            Ok(root) => root,
            Err(status) => {
                rt.last_error = Some((status, "invalid project path bytes".to_string()));
                return status;
            }
        };
        let result = match rt.app() {
            Ok(app) => submit(app, std::path::Path::new(&root)),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 프로젝트 session 닫기를 요청합니다. 그 아래 열린 문서도 닫습니다.
///
/// 결과는 `STELLAR_EVENT_PROJECT_CLOSED` 또는 `STELLAR_EVENT_REQUEST_FAILED` event로 옵니다.
///
/// # Safety
/// `out_request`는 null이거나 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_project_close(
    handle: u64,
    project: u64,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.close_project(ProjectSessionId(project)),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 프로젝트 session 아래에서 문서 열기를 요청합니다. `document_id`는 StableId 16 byte입니다.
///
/// 결과는 `STELLAR_EVENT_DOCUMENT_OPENED` 또는 `STELLAR_EVENT_REQUEST_FAILED` event로 옵니다.
///
/// # Safety
/// `document_id`는 16 byte 읽기가 유효한 포인터여야 하고, `out_request`는 null이거나 유효해야
/// 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_document_open(
    handle: u64,
    project: u64,
    document_id: *const u8,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if document_id.is_null() {
            return StellarStatus::NullPointer;
        }
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(unsafe { std::slice::from_raw_parts(document_id, 16) });
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.open_document(ProjectSessionId(project), StableId::from_bytes(bytes)),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 문서 session 닫기를 요청합니다.
///
/// 결과는 `STELLAR_EVENT_DOCUMENT_CLOSED` 또는 `STELLAR_EVENT_REQUEST_FAILED` event로 옵니다.
///
/// # Safety
/// `out_request`는 null이거나 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_document_close(
    handle: u64,
    document: u64,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.close_document(DocumentSessionId(document)),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 최신 상태 snapshot 요약을 씁니다. document thread를 기다리지 않습니다.
///
/// 아직 요청이 없었으면 세대 0·개수 0입니다.
///
/// # Safety
/// `out_state`는 `StellarRuntimeState` 쓰기가 유효한 포인터여야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_runtime_state(
    handle: u64,
    out_state: *mut StellarRuntimeState,
) -> StellarStatus {
    ffi_guard(move || {
        if out_state.is_null() {
            return StellarStatus::NullPointer;
        }
        let Ok(reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let mut state = StellarRuntimeState {
            version: RUNTIME_STATE_VERSION,
            ..StellarRuntimeState::default()
        };
        if let Some(app) = rt.app.as_ref() {
            let snapshot = app.state();
            state.generation = snapshot.generation;
            state.project_count = snapshot.projects.len() as u64;
            state.document_count = snapshot.documents.len() as u64;
            state.outstanding = app.outstanding() as u64;
            state.window_session_count = snapshot.windows.len() as u64;
        }
        unsafe { *out_state = state };
        StellarStatus::Ok
    })
}

/// 열린 문서 session의 최신 snapshot 정보를 씁니다. document thread를 기다리지 않습니다.
///
/// 열려 있지 않은 문서 session은 `InvalidArgument`(last error 설정)입니다.
///
/// # Safety
/// `out_info`는 `StellarDocumentInfo` 쓰기가 유효한 포인터여야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_document_info(
    handle: u64,
    document: u64,
    out_info: *mut StellarDocumentInfo,
) -> StellarStatus {
    ffi_guard(move || {
        if out_info.is_null() {
            return StellarStatus::NullPointer;
        }
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let snapshot = rt.app.as_ref().map(AppRuntime::state);
        let Some(summary) = snapshot
            .as_ref()
            .and_then(|state| state.document(DocumentSessionId(document)))
        else {
            rt.last_error = Some((
                StellarStatus::InvalidArgument,
                format!("열려 있지 않은 문서 session입니다: {document}"),
            ));
            return StellarStatus::InvalidArgument;
        };
        let info = StellarDocumentInfo {
            version: DOCUMENT_INFO_VERSION,
            reserved: 0,
            document: summary.document.0,
            project: summary.project.0,
            revision: summary.revision.0,
            node_count: summary.node_count,
            document_id: summary.document_id.to_bytes(),
        };
        unsafe { *out_info = info };
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

    #[test]
    fn header_declares_session_constants_and_layouts() {
        use stellar_runtime::app::{
            event_kind, RequestFailure, EVENT_SCHEMA_VERSION, EVENT_TEXT_MAX,
        };

        let header = include_str!("../include/stellar_ffi.h");
        let define = |name: &str, value: String| {
            let line = format!("#define {name} {value}\n");
            assert!(header.contains(&line), "헤더에 {line:?} 없음");
        };
        define(
            "STELLAR_EVENT_SCHEMA_VERSION",
            EVENT_SCHEMA_VERSION.to_string(),
        );
        define("STELLAR_EVENT_TEXT_MAX", EVENT_TEXT_MAX.to_string());
        for (name, kind) in [
            ("STELLAR_EVENT_PROJECT_OPENED", event_kind::PROJECT_OPENED),
            ("STELLAR_EVENT_PROJECT_TRUST", event_kind::PROJECT_TRUST),
            ("STELLAR_EVENT_GIT_NAMES", event_kind::GIT_NAMES),
            (
                "STELLAR_EVENT_COMPONENT_CREATED",
                event_kind::COMPONENT_CREATED,
            ),
            ("STELLAR_EVENT_PROJECT_CREATED", event_kind::PROJECT_CREATED),
            ("STELLAR_EVENT_PROJECT_CLOSED", event_kind::PROJECT_CLOSED),
            ("STELLAR_EVENT_DOCUMENT_OPENED", event_kind::DOCUMENT_OPENED),
            ("STELLAR_EVENT_DOCUMENT_CLOSED", event_kind::DOCUMENT_CLOSED),
            ("STELLAR_EVENT_REQUEST_FAILED", event_kind::REQUEST_FAILED),
            (
                "STELLAR_EVENT_WINDOW_SESSION_OPENED",
                event_kind::WINDOW_SESSION_OPENED,
            ),
            (
                "STELLAR_EVENT_WINDOW_SESSION_CLOSED",
                event_kind::WINDOW_SESSION_CLOSED,
            ),
            (
                "STELLAR_EVENT_WINDOW_VIEW_CHANGED",
                event_kind::WINDOW_VIEW_CHANGED,
            ),
            (
                "STELLAR_EVENT_HISTORY_REPLAYED",
                event_kind::HISTORY_REPLAYED,
            ),
            (
                "STELLAR_EVENT_WINDOW_SESSION_SAVED",
                event_kind::WINDOW_SESSION_SAVED,
            ),
        ] {
            define(name, format!("{kind:#06x}"));
        }
        define(
            "STELLAR_EVENT_CAPACITY",
            AppRuntime::DEFAULT_EVENT_CAPACITY.to_string(),
        );
        let failures = [
            ("UNKNOWN_PROJECT", RequestFailure::UnknownProject),
            ("UNKNOWN_DOCUMENT", RequestFailure::UnknownDocument),
            (
                "DOCUMENT_IN_OTHER_PROJECT",
                RequestFailure::DocumentInOtherProject,
            ),
            ("NOT_A_REPOSITORY", RequestFailure::NotARepository),
            ("NOT_A_STELLAR_PROJECT", RequestFailure::NotAStellarProject),
            ("MANIFEST_UNREADABLE", RequestFailure::ManifestUnreadable),
            ("MANIFEST_CORRUPT", RequestFailure::ManifestCorrupt),
            ("MANIFEST_INVALID", RequestFailure::ManifestInvalid),
            ("INVALID_PROJECT_ID", RequestFailure::InvalidProjectId),
            ("UNTRUSTED_SYMLINK", RequestFailure::UntrustedSymlink),
            ("PATH_UNRESOLVED", RequestFailure::PathUnresolved),
            (
                "UNKNOWN_WINDOW_SESSION",
                RequestFailure::UnknownWindowSession,
            ),
            (
                "WINDOW_SESSION_LOAD_FAILED",
                RequestFailure::WindowSessionLoadFailed,
            ),
            ("UNKNOWN_VIEW", RequestFailure::UnknownView),
            ("NO_ACTIVE_VIEW", RequestFailure::NoActiveView),
            ("SAVE_STALE", RequestFailure::SaveStale),
            ("SAVE_FAILED", RequestFailure::SaveFailed),
            ("HISTORY_UNAVAILABLE", RequestFailure::HistoryUnavailable),
            ("INVALID_VIEWPORT", RequestFailure::InvalidViewport),
            ("TRUST_UNAVAILABLE", RequestFailure::TrustUnavailable),
            ("PROJECT_CREATE_FAILED", RequestFailure::ProjectCreateFailed),
            ("GIT_NAMES_UNAVAILABLE", RequestFailure::GitNamesUnavailable),
        ];
        assert_eq!(
            failures.len(),
            RequestFailure::ALL.len(),
            "모든 실패 코드 노출"
        );
        for (name, failure) in failures {
            define(
                &format!("STELLAR_FAILURE_{name}"),
                failure.code().to_string(),
            );
        }
        define(
            "STELLAR_RUNTIME_STATE_VERSION",
            RUNTIME_STATE_VERSION.to_string(),
        );
        define(
            "STELLAR_DOCUMENT_INFO_VERSION",
            DOCUMENT_INFO_VERSION.to_string(),
        );
        assert!(header.contains(&format!(
            "sizeof(StellarRuntimeState) == {}",
            std::mem::size_of::<StellarRuntimeState>()
        )));
        assert!(header.contains(&format!(
            "sizeof(StellarDocumentInfo) == {}",
            std::mem::size_of::<StellarDocumentInfo>()
        )));
        define(
            "STELLAR_WINDOW_SESSION_INFO_VERSION",
            window_session::WINDOW_SESSION_INFO_VERSION.to_string(),
        );
        define(
            "STELLAR_WINDOW_VIEW_INFO_VERSION",
            window_session::WINDOW_VIEW_INFO_VERSION.to_string(),
        );
        assert!(header.contains(&format!(
            "sizeof(StellarWindowViewInfo) == {}",
            std::mem::size_of::<window_session::StellarWindowViewInfo>()
        )));
        assert!(header.contains(&format!(
            "sizeof(StellarWindowSessionInfo) == {}",
            std::mem::size_of::<window_session::StellarWindowSessionInfo>()
        )));
        let commands = [
            ("UNDO", stellar_runtime::app_command::AppCommand::Undo),
            ("REDO", stellar_runtime::app_command::AppCommand::Redo),
            (
                "SAVE_NOW",
                stellar_runtime::app_command::AppCommand::SaveNow,
            ),
            ("ZOOM_IN", stellar_runtime::app_command::AppCommand::ZoomIn),
            (
                "ZOOM_OUT",
                stellar_runtime::app_command::AppCommand::ZoomOut,
            ),
            (
                "ZOOM_ACTUAL_SIZE",
                stellar_runtime::app_command::AppCommand::ZoomToActualSize,
            ),
        ];
        assert_eq!(
            commands.len(),
            stellar_runtime::app_command::AppCommand::ALL.len(),
            "모든 명령 코드 노출"
        );
        for (name, command) in commands {
            define(
                &format!("STELLAR_COMMAND_{name}"),
                command.code().to_string(),
            );
        }
        for function in [
            "stellar_window_session_open",
            "stellar_window_session_close",
            "stellar_window_session_info",
            "stellar_window_session_perform",
            "stellar_window_session_select",
            "stellar_window_session_move_frame",
            "stellar_window_layer_rename",
            "stellar_window_session_resize_frame",
            "stellar_window_session_set_frame_background",
            "stellar_window_session_insert_frame",
            "stellar_window_session_delete_selection",
            "stellar_window_session_duplicate_selection",
            "stellar_window_session_insert_rectangle",
            "stellar_command_code",
            "stellar_window_view_info",
        ] {
            assert!(
                header.contains(&format!("StellarStatus {function}(")),
                "헤더에 {function} 없음"
            );
        }
    }

    #[test]
    fn session_struct_layouts_are_fixed() {
        use std::mem::{align_of, offset_of, size_of};
        assert_eq!(size_of::<StellarRuntimeState>(), 48);
        assert_eq!(offset_of!(StellarRuntimeState, window_session_count), 40);
        assert_eq!(align_of::<StellarRuntimeState>(), 8);
        assert_eq!(offset_of!(StellarRuntimeState, version), 0);
        assert_eq!(offset_of!(StellarRuntimeState, reserved), 4);
        assert_eq!(offset_of!(StellarRuntimeState, generation), 8);
        assert_eq!(offset_of!(StellarRuntimeState, project_count), 16);
        assert_eq!(offset_of!(StellarRuntimeState, document_count), 24);
        assert_eq!(offset_of!(StellarRuntimeState, outstanding), 32);
        assert_eq!(size_of::<StellarDocumentInfo>(), 56);
        assert_eq!(offset_of!(StellarDocumentInfo, version), 0);
        assert_eq!(offset_of!(StellarDocumentInfo, reserved), 4);
        assert_eq!(offset_of!(StellarDocumentInfo, document), 8);
        assert_eq!(offset_of!(StellarDocumentInfo, project), 16);
        assert_eq!(offset_of!(StellarDocumentInfo, revision), 24);
        assert_eq!(offset_of!(StellarDocumentInfo, node_count), 32);
        assert_eq!(offset_of!(StellarDocumentInfo, document_id), 40);
    }

    mod project_session {
        use super::*;
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::{Duration, Instant};

        use stellar_runtime::app::{event_kind, RequestFailure, RuntimeEvent};

        const MANIFEST: &str = "schema_version: \"1\"\nproject:\n  id: \"0195c530-3acd-7bc1-8414-c213f61cb965\"\n  name: FFI 프로젝트\ntarget_profiles: []\ndependencies: {}\n";

        struct TempDir {
            path: PathBuf,
        }

        impl TempDir {
            fn new(tag: &str) -> TempDir {
                static COUNTER: AtomicUsize = AtomicUsize::new(0);
                let n = COUNTER.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "stellar-ffi-session-{tag}-{}-{n}",
                    std::process::id()
                ));
                let _ = std::fs::remove_dir_all(&path);
                std::fs::create_dir_all(&path).unwrap();
                TempDir { path }
            }

            fn project() -> TempDir {
                let dir = TempDir::new("project");
                std::fs::write(dir.path.join("stellar.yaml"), MANIFEST).unwrap();
                std::fs::create_dir(dir.path.join(".git")).unwrap();
                dir
            }

            fn path_bytes(&self) -> Vec<u8> {
                self.path.to_str().unwrap().as_bytes().to_vec()
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.path);
            }
        }

        fn open_project(h: u64, path: &[u8]) -> (StellarStatus, u64) {
            let mut request = 99u64;
            let status =
                unsafe { stellar_project_open(h, path.as_ptr(), path.len(), &mut request) };
            (status, request)
        }

        /// 다음 event를 C ABI로 읽습니다(길이 조회 → buffer). 최대 5초 기다립니다.
        fn next_event(h: u64) -> (u32, Vec<u8>) {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let mut kind = 0u32;
                let mut need = 0usize;
                let status = unsafe {
                    stellar_runtime_poll_event(h, &mut kind, std::ptr::null_mut(), 0, &mut need)
                };
                if status == StellarStatus::Ok {
                    let mut buf = vec![0u8; need];
                    let mut written = 0usize;
                    let mut taken_kind = 0u32;
                    let status = unsafe {
                        stellar_runtime_poll_event(
                            h,
                            &mut taken_kind,
                            buf.as_mut_ptr(),
                            buf.len(),
                            &mut written,
                        )
                    };
                    assert_eq!(status, StellarStatus::Ok);
                    assert_eq!(taken_kind, kind);
                    buf.truncate(written);
                    return (kind, buf);
                }
                assert_eq!(status, StellarStatus::NoError);
                assert!(Instant::now() < deadline, "event가 오지 않았습니다");
                std::thread::yield_now();
            }
        }

        fn next_runtime_event(h: u64) -> RuntimeEvent {
            let (kind, payload) = next_event(h);
            RuntimeEvent::decode(kind, &payload).expect("event payload")
        }

        fn state(h: u64) -> StellarRuntimeState {
            let mut state = StellarRuntimeState::default();
            assert_eq!(
                unsafe { stellar_runtime_state(h, &mut state) },
                StellarStatus::Ok
            );
            state
        }

        fn last_error(h: u64) -> String {
            let mut buf = [0u8; 512];
            let mut written = 0usize;
            let status =
                unsafe { stellar_last_error(h, buf.as_mut_ptr(), buf.len(), &mut written) };
            assert_eq!(status, StellarStatus::Ok);
            String::from_utf8(buf[..written].to_vec()).unwrap()
        }

        #[test]
        fn create_via_abi_reopens_in_an_editable_window_and_preserves_existing_folder() {
            use crate::window_session::{
                stellar_window_session_open, stellar_window_view_info, StellarWindowViewInfo,
            };
            // 실제 앱과 같은 프로세스 환경에서 사용자 전역 Git 설정·서명·hook과 분리합니다.
            const FIXTURE_HOME: &str = "STELLAR_CREATE_ABI_FIXTURE_HOME";
            let parent = TempDir::new("new-project");
            if std::env::var_os(FIXTURE_HOME).is_none() {
                // 사용자가 Git 자체의 전역 설정 경로를 지정한 경우 해당 identity를 그대로 씁니다.
                let config = parent.path.join("my-git-config");
                std::fs::write(&config, "[user]\n name = Create Fixture\n email = create@example.invalid\n[commit]\n gpgsign = false\n[core]\n hooksPath = /dev/null\n").unwrap();
                let output = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "tests::project_session::create_via_abi_reopens_in_an_editable_window_and_preserves_existing_folder", "--nocapture"])
                    .env_clear()
                    .env("HOME", &parent.path)
                    .env("XDG_CONFIG_HOME", &parent.path)
                    .env("GIT_CONFIG_GLOBAL", &config)
                    .env(FIXTURE_HOME, "1")
                    .output().unwrap();
                let stdout = String::from_utf8_lossy(&output.stdout);
                assert!(
                    output.status.success() && stdout.contains("1 passed"),
                    "{stdout} {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                return;
            }
            let path = parent.path.join("새 프로젝트");
            let bytes = path.as_os_str().as_encoded_bytes();
            let h = create();
            let mut request = 0;
            let create_at = |request: &mut u64| unsafe {
                stellar_project_create(h, bytes.as_ptr(), bytes.len(), request)
            };
            assert_eq!(create_at(&mut request), StellarStatus::Ok);
            let RuntimeEvent::ProjectCreated {
                request: done,
                git_state,
            } = next_runtime_event(h)
            else {
                panic!("create event expected");
            };
            assert_eq!(done.0, request);
            assert_eq!(git_state, 0, "Git가 선택한 신원으로 최초 커밋 생성");
            let author = std::process::Command::new("git")
                .args(["log", "-1", "--format=%an <%ae>"])
                .current_dir(&path)
                .output()
                .unwrap();
            assert!(author.status.success());
            assert_eq!(
                String::from_utf8(author.stdout).unwrap().trim(),
                "Create Fixture <create@example.invalid>"
            );
            assert_eq!(state(h).project_count, 0);
            assert_eq!(open_project(h, bytes).0, StellarStatus::Ok);
            let RuntimeEvent::ProjectOpened { project, name, .. } = next_runtime_event(h) else {
                panic!("open event expected");
            };
            assert_eq!(name, "새 프로젝트");
            assert_eq!(
                unsafe { stellar_window_session_open(h, project.0, &mut request) },
                StellarStatus::Ok
            );
            let RuntimeEvent::WindowSessionOpened {
                session,
                document_id,
                generation,
                ..
            } = next_runtime_event(h)
            else {
                panic!("window event expected");
            };
            assert_ne!(document_id, StableId::NIL);
            assert_eq!(generation, 1);
            let mut info = StellarWindowViewInfo::default();
            assert_eq!(
                unsafe { stellar_window_view_info(h, session.0, &mut info) },
                StellarStatus::Ok
            );
            assert_eq!((info.has_view, info.tab_count), (1, 1));
            assert_eq!(create_at(&mut request), StellarStatus::Ok);
            assert!(matches!(
                next_runtime_event(h),
                RuntimeEvent::RequestFailed {
                    failure: RequestFailure::ProjectCreateFailed,
                    ..
                }
            ));
            assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
        }

        #[test]
        fn project_and_document_lifecycle_through_the_c_abi() {
            let fixture = TempDir::project();
            let h = create();
            // 요청 전: app runtime 미시작, 세대 0.
            let before = state(h);
            assert_eq!(before.version, RUNTIME_STATE_VERSION);
            assert_eq!(
                (before.generation, before.project_count, before.outstanding),
                (0, 0, 0)
            );

            let (status, request) = open_project(h, &fixture.path_bytes());
            assert_eq!(status, StellarStatus::Ok);
            assert_eq!(request, 1);
            let RuntimeEvent::ProjectOpened {
                request: got,
                project,
                name,
                root,
                ..
            } = next_runtime_event(h)
            else {
                panic!("프로젝트 열기 실패");
            };
            assert_eq!(got.0, request);
            assert_eq!(name, "FFI 프로젝트");
            assert_eq!(Path::new(&root), fixture.path.as_path());
            let after = state(h);
            assert_eq!((after.generation, after.project_count), (1, 1));
            assert_eq!(after.outstanding, 0);

            // 문서 열기 → 정보 조회.
            let doc = StableId::new_v7(7, 0x0abc, 7);
            let mut doc_request = 0u64;
            assert_eq!(
                unsafe {
                    stellar_document_open(h, project.0, doc.to_bytes().as_ptr(), &mut doc_request)
                },
                StellarStatus::Ok
            );
            let RuntimeEvent::DocumentOpened {
                document,
                document_id,
                already_open,
                ..
            } = next_runtime_event(h)
            else {
                panic!("문서 열기 실패");
            };
            assert_eq!(document_id, doc);
            assert!(!already_open);
            let mut info = StellarDocumentInfo::default();
            assert_eq!(
                unsafe { stellar_document_info(h, document.0, &mut info) },
                StellarStatus::Ok
            );
            assert_eq!(
                info,
                StellarDocumentInfo {
                    version: DOCUMENT_INFO_VERSION,
                    reserved: 0,
                    document: document.0,
                    project: project.0,
                    revision: 0,
                    node_count: 0,
                    document_id: doc.to_bytes(),
                }
            );
            assert_eq!(state(h).document_count, 1);

            // 프로젝트 닫기 → 문서도 닫힘, 정보 조회는 InvalidArgument.
            let mut close_request = 0u64;
            assert_eq!(
                unsafe { stellar_project_close(h, project.0, &mut close_request) },
                StellarStatus::Ok
            );
            assert_eq!(
                next_runtime_event(h),
                RuntimeEvent::ProjectClosed {
                    request: stellar_runtime::app::RequestId(close_request),
                    project,
                    closed_documents: 1,
                }
            );
            assert_eq!(
                unsafe { stellar_document_info(h, document.0, &mut info) },
                StellarStatus::InvalidArgument
            );
            assert!(last_error(h).contains("문서 session"));
            let mut doc_close = 0u64;
            assert_eq!(
                unsafe { stellar_document_close(h, document.0, &mut doc_close) },
                StellarStatus::Ok
            );
            let RuntimeEvent::RequestFailed { failure, .. } = next_runtime_event(h) else {
                panic!("실패 event가 아님");
            };
            assert_eq!(failure, RequestFailure::UnknownDocument);
            let last = state(h);
            assert_eq!((last.project_count, last.document_count), (0, 0));
            assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
        }

        #[test]
        fn invalid_project_folder_reports_a_failure_event() {
            let empty = TempDir::new("no-repo");
            let h = create();
            let (status, request) = open_project(h, &empty.path_bytes());
            assert_eq!(status, StellarStatus::Ok);
            let (kind, payload) = next_event(h);
            assert_eq!(kind, event_kind::REQUEST_FAILED);
            // payload: version u32, request u64, failure u32, message.
            assert_eq!(&payload[4..12], &request.to_le_bytes());
            assert_eq!(
                u32::from_le_bytes(payload[12..16].try_into().unwrap()),
                RequestFailure::NotARepository.code()
            );
            assert_eq!(state(h).generation, 0, "실패는 상태를 바꾸지 않음");
            assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
        }

        #[test]
        fn request_arguments_are_validated_at_the_boundary() {
            let h = create();
            let (status, request) = open_project(h, b"relative/project");
            assert_eq!((status, request), (StellarStatus::InvalidArgument, 0));
            assert!(last_error(h).contains("절대"));
            let (status, _) = open_project(h, &[0xFF, 0xFE]);
            assert_eq!(status, StellarStatus::InvalidUtf8);
            assert_eq!(
                unsafe { stellar_project_open(h, std::ptr::null(), 4, std::ptr::null_mut()) },
                StellarStatus::NullPointer
            );
            assert_eq!(
                unsafe { stellar_document_open(h, 1, std::ptr::null(), std::ptr::null_mut()) },
                StellarStatus::NullPointer
            );
            assert_eq!(
                unsafe { stellar_runtime_state(h, std::ptr::null_mut()) },
                StellarStatus::NullPointer
            );
            assert_eq!(
                unsafe { stellar_document_info(h, 1, std::ptr::null_mut()) },
                StellarStatus::NullPointer
            );
            // 알 수 없는 handle.
            let mut out = StellarRuntimeState::default();
            assert_eq!(
                unsafe { stellar_runtime_state(999_999, &mut out) },
                StellarStatus::InvalidHandle
            );
            assert_eq!(
                unsafe { stellar_project_close(999_999, 1, std::ptr::null_mut()) },
                StellarStatus::InvalidHandle
            );
            assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
        }

        #[test]
        fn poll_keeps_runtime_events_on_small_buffers_after_local_events() {
            let empty = TempDir::new("poll");
            let h = create();
            let (status, request) = open_project(h, &empty.path_bytes());
            assert_eq!(status, StellarStatus::Ok);
            // 코어 내부 event는 runtime 완료 event보다 먼저 읽힙니다.
            assert_eq!(enqueue_event(h, 7, b"local"), StellarStatus::Ok);
            let (kind, payload) = next_event(h);
            assert_eq!((kind, payload.as_slice()), (7, b"local".as_slice()));
            // runtime event가 준비될 때까지 기다린 뒤 작은 buffer는 꺼내지 않습니다.
            let deadline = Instant::now() + Duration::from_secs(5);
            let need = loop {
                let mut need = 0usize;
                let status = unsafe {
                    stellar_runtime_poll_event(
                        h,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        0,
                        &mut need,
                    )
                };
                if status == StellarStatus::Ok {
                    break need;
                }
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            };
            let mut small = vec![0u8; need - 1];
            let mut written = 0usize;
            assert_eq!(
                unsafe {
                    stellar_runtime_poll_event(
                        h,
                        std::ptr::null_mut(),
                        small.as_mut_ptr(),
                        small.len(),
                        &mut written,
                    )
                },
                StellarStatus::BufferTooSmall
            );
            assert_eq!(state(h).outstanding, 1, "꺼내지 않았으므로 남아 있음");
            let event = next_runtime_event(h);
            assert_eq!(event.request().0, request);
            assert_eq!(state(h).outstanding, 0);
            assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
        }

        #[test]
        fn unpolled_requests_are_bounded() {
            let empty = TempDir::new("bounded");
            let path = empty.path_bytes();
            let h = create();
            for _ in 0..AppRuntime::DEFAULT_EVENT_CAPACITY {
                assert_eq!(open_project(h, &path).0, StellarStatus::Ok);
            }
            let (status, request) = open_project(h, &path);
            assert_eq!((status, request), (StellarStatus::QueueFull, 0));
            assert!(last_error(h).contains("가득"));
            assert_eq!(
                state(h).outstanding,
                AppRuntime::DEFAULT_EVENT_CAPACITY as u64
            );
            // 하나를 꺼내면 다시 받습니다.
            next_event(h);
            assert_eq!(open_project(h, &path).0, StellarStatus::Ok);
            // 완료 event를 남긴 채 파괴해도 누수 없이 끝납니다.
            assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
        }

        #[test]
        fn create_destroy_cycles_with_a_started_core_do_not_leak() {
            let empty = TempDir::new("cycles");
            let path = empty.path_bytes();
            for _ in 0..50 {
                let h = create();
                assert_eq!(open_project(h, &path).0, StellarStatus::Ok);
                assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
                assert_eq!(stellar_runtime_destroy(h), StellarStatus::InvalidHandle);
            }
        }
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
        fn submitted_events_carry_t0_into_document_thread_hit_test_trace() {
            // XII.3.5: 셸 이벤트 수신 때 찍은 T0 후보가 병합을 거쳐 document thread hit-test request
            // trace까지 이어집니다(같은 프로세스 단조 시계).
            use std::sync::Arc;
            use stellar_runtime::document_thread::DocumentThread;
            use stellar_runtime::input_trace::TraceClock;

            let h = create();
            let before = InstantClock.now_nanos();
            assert_eq!(
                submit(h, &mac_event(event_type::LEFT_MOUSE_DOWN, 20.0)).0,
                StellarStatus::Ok
            );
            for i in 0..100u32 {
                let drag = mac_event(event_type::LEFT_MOUSE_DRAGGED, 20.0 + f64::from(i) * 1e-3);
                assert_eq!(submit(h, &drag).0, StellarStatus::Ok);
            }
            assert_eq!(
                submit(h, &mac_event(event_type::LEFT_MOUSE_UP, 20.2)).0,
                StellarStatus::Ok
            );
            let after = InstantClock.now_nanos();
            let queued: Vec<_> = {
                let mut reg = registry().lock().expect("registry");
                let rt = reg.live.get_mut(&h).expect("runtime");
                std::iter::from_fn(|| rt.input.pop_received()).collect()
            };
            assert_eq!(queued.len(), 3);
            let merged: Vec<u32> = queued.iter().map(|(_, r)| r.merged()).collect();
            assert_eq!(merged, [0, 99, 0]);
            let received: Vec<u64> = queued.iter().map(|(_, r)| r.received()).collect();
            assert!(received.windows(2).all(|pair| pair[0] <= pair[1]));
            assert!(before <= received[0] && received[2] <= after);

            let clock: TraceClock = Arc::new(InstantClock);
            let doc = DocumentThread::spawn().expect("document thread");
            let sender = doc.sender();
            for (event, receipt) in queued {
                let class = event.queue_class();
                let trace = sender
                    .submit_hit_test(class, receipt, &clock, |_ctx| ())
                    .expect("submit")
                    .ticket
                    .wait()
                    .expect("hit test")
                    .trace;
                assert_eq!(trace.class, class);
                assert_eq!(
                    trace.t0_candidates(),
                    [receipt.event_time(), receipt.received()]
                );
                let [started, finished] = trace.t1_candidates();
                assert!(receipt.received() <= trace.enqueued);
                assert!(trace.enqueued <= started && started <= finished);
                assert_eq!(trace.queue_latency_nanos(), started - receipt.received());
            }
            doc.shutdown().expect("shutdown");
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

        /// 병합되지 않는 key down 이벤트(native 시각 `timestamp`, 수신 시각 `receive_uptime`).
        fn key_at(timestamp: f64, receive_uptime: f64) -> StellarMacEvent {
            StellarMacEvent {
                key_code: 0x00,
                receive_uptime,
                ..mac_event(event_type::KEY_DOWN, timestamp)
            }
        }

        fn poll_timestamp(h: u64) -> u64 {
            poll(h).expect("입력 이벤트").timestamp().get()
        }

        #[test]
        fn one_future_timestamp_does_not_latch_later_inputs() {
            // 보안 리뷰 FR-3: 수신 시각보다 터무니없이 미래인 timestamp 하나가 이후 정상 입력의 시각을
            // 그 값으로 고정하지 않습니다. 이벤트는 받은 뒤에 일어날 수 없으므로 수신 시각에서 자릅니다.
            let h = create();
            assert_eq!(submit(h, &key_at(100.0, 100.0)), (StellarStatus::Ok, 1));
            assert_eq!(submit(h, &key_at(1.0e9, 100.1)), (StellarStatus::Ok, 1));
            assert_eq!(submit(h, &key_at(100.2, 100.2)), (StellarStatus::Ok, 1));
            assert_eq!(submit(h, &key_at(105.0, 105.0)), (StellarStatus::Ok, 1));
            let t: Vec<u64> = (0..4).map(|_| poll_timestamp(h)).collect();
            assert_eq!(
                t[1] - t[0],
                100_000_000,
                "미래 timestamp는 수신 시각에서 자름"
            );
            assert_eq!(t[2] - t[0], 200_000_000);
            assert_eq!(t[3] - t[2], 4_800_000_000, "이후 정상 입력 간격 보존");
            assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
        }

        #[test]
        fn rejected_first_event_does_not_set_the_time_anchor() {
            // 보안 리뷰 FR-4: 거부된 첫 이벤트의 수신 시각이 기준점을 영구히 정하지 않습니다.
            let h = create();
            let mut bad_version = key_at(1.0e6, 1.0e6);
            bad_version.version = 99;
            assert_eq!(submit(h, &bad_version), (StellarStatus::InvalidArgument, 0));
            let mut bad_click = mac_event(event_type::LEFT_MOUSE_DOWN, 2.0e6);
            bad_click.click_count = 0;
            assert_eq!(submit(h, &bad_click), (StellarStatus::InvalidArgument, 0));
            assert_eq!(submit(h, &key_at(100.0, 100.0)), (StellarStatus::Ok, 1));
            assert_eq!(submit(h, &key_at(200.0, 200.0)), (StellarStatus::Ok, 1));
            let a = poll_timestamp(h);
            let b = poll_timestamp(h);
            assert!(a > 0, "첫 정상 이벤트가 기준점(코어 현재 시각)에 놓임");
            assert_eq!(b - a, 100_000_000_000, "100초 간격 보존");
            assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
        }

        #[test]
        fn invalid_receive_uptime_is_rejected_after_the_anchor() {
            // 기준점을 잡은 뒤에도 수신 시각은 유한한 음이 아닌 값이어야 합니다.
            let h = create();
            assert_eq!(submit(h, &key_at(1.0, 1.0)), (StellarStatus::Ok, 1));
            for receive in [f64::NAN, f64::INFINITY, -1.0] {
                assert_eq!(
                    submit(h, &key_at(1.5, receive)),
                    (StellarStatus::InvalidArgument, 0),
                    "receive_uptime {receive}"
                );
            }
            assert_eq!(submit(h, &key_at(2.0, 2.0)), (StellarStatus::Ok, 1));
            let first = poll_timestamp(h);
            assert_eq!(poll_timestamp(h) - first, 1_000_000_000);
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

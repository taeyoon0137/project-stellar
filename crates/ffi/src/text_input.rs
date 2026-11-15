//! 캔버스 Text 편집 ABI(ADR-0067 §6). 조합 상태는 runtime 창 session이 소유하고, 셸은 token만 들고 있습니다.
//! 시작·조합·확정·취소·대체는 요청 ID를 돌려주는 비동기 요청이고, 본문 조회만 셸 worker에서 결과를 기다립니다.
use crate::{decode_utf8_owned, ffi_guard, registry, StellarStatus};
use stellar_core_types::{ids::StableId, revision::DocumentRevision, value::Vec2};
use stellar_runtime::app::{
    text_input::{TextRange, TextTarget},
    AppRuntime, RequestError, RequestId, WindowSessionHandle,
};

fn with_app(
    handle: u64,
    out_request: *mut u64,
    run: impl FnOnce(&AppRuntime) -> Result<RequestId, RequestError>,
) -> StellarStatus {
    let Ok(mut reg) = registry().lock() else {
        return StellarStatus::Internal;
    };
    let Some(rt) = reg.live.get_mut(&handle) else {
        return StellarStatus::InvalidHandle;
    };
    let result = match rt.app() {
        Ok(app) => run(app),
        Err(status) => return status,
    };
    unsafe { rt.finish_request(result, out_request) }
}

/// IME 조합을 시작합니다. new_text가 0이면 target은 선택한 Text이고 start..end는 문서 Unicode scalar 범위입니다.
/// new_text가 1이면 target은 자유 배치 부모 Frame, (x, y)는 부모 local 원점이고 범위는 0..0이어야 합니다.
/// out_request는 이후 조합 요청의 token입니다. out_node는 조합 대상(새 Text는 확정 때 만들 ID)입니다.
/// # Safety
/// view, target은 16 byte를 읽을 수 있고 out_node는 16 byte, out_request는 정렬된 u64 출력이어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_text_begin(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    target: *const [u8; 16],
    revision: u64,
    new_text: u32,
    x: f32,
    y: f32,
    start: u32,
    end: u32,
    out_node: *mut [u8; 16],
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if view.is_null() || target.is_null() || out_node.is_null() || out_request.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe {
            out_node.write([0; 16]);
            out_request.write(0);
        }
        let Some(range) = TextRange::new(start, end) else {
            return StellarStatus::InvalidArgument;
        };
        let target_id = StableId::from_bytes(unsafe { *target });
        let target = match new_text {
            0 => TextTarget::Node(target_id),
            1 => match Vec2::new(x, y) {
                Ok(origin) => TextTarget::New {
                    parent: target_id,
                    origin,
                },
                Err(_) => return StellarStatus::InvalidArgument,
            },
            _ => return StellarStatus::InvalidArgument,
        };
        let view = StableId::from_bytes(unsafe { *view });
        with_app(handle, out_request, |app| {
            app.begin_canvas_text(
                WindowSessionHandle(session),
                view,
                DocumentRevision(revision),
                target,
                range,
            )
            .map(|(request, node)| {
                unsafe { out_node.write(node.to_bytes()) };
                request
            })
        })
    })
}

/// 조합 중 본문(UTF-8)을 바꿉니다. 문서와 history는 바뀌지 않고 canvas 미리보기만 바뀝니다.
/// # Safety
/// len>0이면 text는 len byte를 읽을 수 있어야 하고 out_request는 정렬된 u64 출력이어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_text_mark(
    handle: u64,
    session: u64,
    token: u64,
    text: *const u8,
    len: usize,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if out_request.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { out_request.write(0) };
        let value = match unsafe { decode_utf8_owned(text, len) } {
            Ok(s) => s,
            Err(status) => return status,
        };
        with_app(handle, out_request, |app| {
            app.mark_canvas_text(WindowSessionHandle(session), token, value)
        })
    })
}

/// 입력기가 확정한 본문을 조합 범위에 기록하고 조합을 끝냅니다. 본문이 같으면 history를 늘리지 않습니다.
/// # Safety
/// len>0이면 text는 len byte를 읽을 수 있어야 하고 out_request는 정렬된 u64 출력이어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_text_commit(
    handle: u64,
    session: u64,
    token: u64,
    text: *const u8,
    len: usize,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if out_request.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { out_request.write(0) };
        let value = match unsafe { decode_utf8_owned(text, len) } {
            Ok(s) => s,
            Err(status) => return status,
        };
        with_app(handle, out_request, |app| {
            app.commit_canvas_text(WindowSessionHandle(session), token, value)
        })
    })
}

/// 지정한 조합만 버립니다. 이미 끝났거나 더 새 조합이 있으면 아무것도 바꾸지 않습니다.
/// # Safety
/// out_request는 정렬된 u64 출력이어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_text_cancel(
    handle: u64,
    session: u64,
    token: u64,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if out_request.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { out_request.write(0) };
        with_app(handle, out_request, |app| {
            app.cancel_canvas_text(WindowSessionHandle(session), token)
        })
    })
}

/// 조합 없는 입력 한 번을 선택한 Text의 scalar 범위 편집으로 기록합니다.
/// # Safety
/// view, node는 16 byte를 읽을 수 있고, len>0이면 text는 len byte를 읽을 수 있어야 하며 out_request는 정렬된
/// u64 출력이어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_text_replace(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    node: *const [u8; 16],
    revision: u64,
    start: u32,
    end: u32,
    text: *const u8,
    len: usize,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if view.is_null() || node.is_null() || out_request.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { out_request.write(0) };
        let Some(range) = TextRange::new(start, end) else {
            return StellarStatus::InvalidArgument;
        };
        let value = match unsafe { decode_utf8_owned(text, len) } {
            Ok(s) => s,
            Err(status) => return status,
        };
        let view = StableId::from_bytes(unsafe { *view });
        let node = StableId::from_bytes(unsafe { *node });
        with_app(handle, out_request, |app| {
            app.replace_canvas_text(
                WindowSessionHandle(session),
                view,
                DocumentRevision(revision),
                node,
                range,
                value,
            )
        })
    })
}

/// 편집 가능한 Text의 저장된 UTF-8 본문을 씁니다. 셸 worker 전용입니다(document thread 결과를 기다림).
/// out_len에는 항상 본문 길이를 씁니다. buf가 null이거나 capacity가 모자라면 복사하지 않고 각각 OK,
/// BUFFER_TOO_SMALL을 돌려줍니다. revision·View가 다르거나 편집할 수 없으면 INVALID_ARGUMENT입니다.
/// # Safety
/// view, node는 16 byte를 읽을 수 있고 out_len은 usize 출력, buf는 null 또는 capacity byte 쓰기가 가능해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_text_content(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    node: *const [u8; 16],
    revision: u64,
    buf: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> StellarStatus {
    ffi_guard(move || {
        if view.is_null() || node.is_null() || out_len.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { out_len.write(0) };
        let view = StableId::from_bytes(unsafe { *view });
        let node = StableId::from_bytes(unsafe { *node });
        let ticket = {
            let Ok(reg) = registry().lock() else {
                return StellarStatus::Internal;
            };
            let Some(rt) = reg.live.get(&handle) else {
                return StellarStatus::InvalidHandle;
            };
            let Some(app) = rt.app.as_ref() else {
                return StellarStatus::InvalidArgument;
            };
            match app.canvas_text_content(
                WindowSessionHandle(session),
                view,
                DocumentRevision(revision),
                node,
            ) {
                Ok(ticket) => ticket,
                Err(stellar_runtime::document_thread::SubmitError::QueueFull { .. }) => {
                    return StellarStatus::QueueFull
                }
                Err(stellar_runtime::document_thread::SubmitError::Stopped) => {
                    return StellarStatus::Internal
                }
            }
        };
        let Ok(Some(text)) = ticket.wait() else {
            return StellarStatus::InvalidArgument;
        };
        let bytes = text.as_bytes();
        unsafe { out_len.write(bytes.len()) };
        if buf.is_null() {
            return StellarStatus::Ok;
        }
        if capacity < bytes.len() {
            return StellarStatus::BufferTooSmall;
        }
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len()) };
        StellarStatus::Ok
    })
}

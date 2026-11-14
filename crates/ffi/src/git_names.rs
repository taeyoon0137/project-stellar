//! 창의 저장 노드 이름 변경 조회 및 선택 Stage C ABI.
use crate::{ffi_guard, registry, write_len, StellarStatus};
use stellar_runtime::app::{RequestId, WindowSessionHandle};

/// action 0은 새로고침, 1은 표시 세대의 행 하나를 stage합니다.
/// # Safety
/// `out_request`는 null이거나 u64를 쓸 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_git_names_request(
    handle: u64,
    session: u64,
    action: u32,
    generation: u64,
    row: u32,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if out_request.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { *out_request = 0 }
        let chosen = match action {
            0 if generation == 0 && row == 0 => None,
            1 if generation != 0 => Some((RequestId(generation), row as usize)),
            _ => return StellarStatus::InvalidArgument,
        };
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let Some(app) = rt.app.as_ref() else {
            return StellarStatus::InvalidArgument;
        };
        let result = app.request_git_names(WindowSessionHandle(session), chosen);
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 코어에 보관된 해당 세대의 한 행만 읽습니다. null buf는 필요한 길이만 반환합니다.
/// # Safety
/// `out_len`은 size_t를 쓸 수 있어야 합니다. null이 아닌 `buf`에는 capacity 바이트를 쓸 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_git_names_row(
    handle: u64,
    session: u64,
    generation: u64,
    row: u32,
    buf: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> StellarStatus {
    ffi_guard(move || {
        if out_len.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { write_len(out_len, 0) }
        let Ok(reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let Some(item) = rt.app.as_ref().and_then(|app| {
            app.git_name_row(
                WindowSessionHandle(session),
                RequestId(generation),
                row as usize,
            )
        }) else {
            return StellarStatus::NoError;
        };
        let mut bytes =
            Vec::with_capacity(45 + item.head.len() + item.index.len() + item.working.len());
        bytes.extend_from_slice(&item.document.to_bytes());
        bytes.extend_from_slice(&item.node.to_bytes());
        bytes.push(u8::from(item.staged));
        for text in [&item.head, &item.index, &item.working] {
            bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
            bytes.extend_from_slice(text.as_bytes());
        }
        unsafe { write_len(out_len, bytes.len()) }
        if buf.is_null() {
            return StellarStatus::Ok;
        }
        if capacity < bytes.len() {
            return StellarStatus::BufferTooSmall;
        }
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len()) }
        StellarStatus::Ok
    })
}

/// 같은 창과 요청에만 취소를 보냅니다. 이미 완료된 요청은 무시합니다.
#[no_mangle]
pub extern "C" fn stellar_git_names_cancel(
    handle: u64,
    session: u64,
    request: u64,
) -> StellarStatus {
    ffi_guard(move || {
        let Ok(reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let Some(app) = rt.app.as_ref() else {
            return StellarStatus::InvalidArgument;
        };
        app.cancel_git_names(WindowSessionHandle(session), RequestId(request));
        StellarStatus::Ok
    })
}

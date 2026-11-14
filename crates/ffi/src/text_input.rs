//! 창별 native 입력기가 소유한 조합. 호출자는 시작/조회 작업을 worker에서 수행합니다.
use crate::{decode_utf8_owned, ffi_guard, input, registry, StellarStatus};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use stellar_core_types::{ids::StableId, revision::DocumentRevision};
use stellar_runtime::{
    app::{
        text_input::{CanvasTextInput, TextRange},
        WindowSessionHandle,
    },
    document_thread::SubmitError,
};

struct WindowInput {
    token: u64,
    composition: Option<Arc<Mutex<CanvasTextInput>>>,
}

#[derive(Default)]
pub(super) struct TextInputs {
    next: u64,
    windows: HashMap<u64, WindowInput>,
}

impl TextInputs {
    pub(super) fn close(&mut self, session: u64) {
        self.windows.remove(&session);
    }

    fn reserve(&mut self, session: u64) -> Option<u64> {
        let token = self.next.checked_add(1)?;
        self.next = token;
        self.windows.insert(
            session,
            WindowInput {
                token,
                composition: None,
            },
        );
        Some(token)
    }

    fn active(&self, session: u64, token: u64) -> Option<&Arc<Mutex<CanvasTextInput>>> {
        let current = self.windows.get(&session)?;
        if current.token != token {
            return None;
        }
        current.composition.as_ref()
    }
}

/// 현재 View의 자유 배치 부모 Frame에 확정한 UTF-8 본문과 local 원점을 저장합니다.
/// # Safety
/// view, parent는 16바이트를 읽을 수 있고 text는 len바이트, out_request는 정렬된 u64 출력이어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_insert_text(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    parent: *const [u8; 16],
    revision: u64,
    x: f32,
    y: f32,
    text: *const u8,
    len: usize,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if out_request.is_null() || view.is_null() || parent.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { out_request.write(0) };
        let Ok(origin) = stellar_core_types::value::Vec2::new(x, y) else {
            return StellarStatus::InvalidArgument;
        };
        if len == 0 {
            return StellarStatus::InvalidArgument;
        }
        let content = match unsafe { decode_utf8_owned(text, len) } {
            Ok(value) => value,
            Err(status) => return status,
        };
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let request = match rt.app() {
            Ok(app) => app.insert_canvas_text(
                WindowSessionHandle(session),
                StableId::from_bytes(unsafe { *view }),
                DocumentRevision(revision),
                StableId::from_bytes(unsafe { *parent }),
                origin,
                content,
            ),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(request, out_request) }
    })
}

/// window worker에서 현재 선택한 Text의 조합을 시작합니다. range는 문서의 Unicode scalar 단위입니다.
/// # Safety
/// view와 node는 각각 16 byte를 읽을 수 있고 out_token은 정렬된 u64 출력을 허용해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_text_begin(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    node: *const [u8; 16],
    revision: u64,
    start: u32,
    end: u32,
    out_token: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if out_token.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { out_token.write(0) };
        if view.is_null() || node.is_null() {
            return StellarStatus::NullPointer;
        }
        let Some(range) = TextRange::new(start, end) else {
            return StellarStatus::InvalidArgument;
        };
        let (token, ticket) = {
            let Ok(mut reg) = registry().lock() else {
                return StellarStatus::Internal;
            };
            let Some(rt) = reg.live.get_mut(&handle) else {
                return StellarStatus::InvalidHandle;
            };
            let Some(app) = rt.app.as_ref() else {
                return StellarStatus::InvalidArgument;
            };
            if app.state().window(WindowSessionHandle(session)).is_none() {
                return StellarStatus::InvalidArgument;
            }
            let view = StableId::from_bytes(unsafe { *view });
            let node = StableId::from_bytes(unsafe { *node });
            let ticket = match app.begin_canvas_text(
                WindowSessionHandle(session),
                view,
                DocumentRevision(revision),
                node,
                range,
            ) {
                Ok(ticket) => ticket,
                Err(SubmitError::QueueFull { .. }) => return StellarStatus::QueueFull,
                Err(SubmitError::Stopped) => return StellarStatus::Internal,
            };
            let Some(token) = rt.text_inputs.reserve(session) else {
                return StellarStatus::Internal;
            };
            (token, ticket)
        };
        let result = ticket.wait();
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let Some(slot) = rt.text_inputs.windows.get_mut(&session) else {
            return StellarStatus::InvalidArgument;
        };
        if slot.token != token {
            return StellarStatus::InvalidArgument;
        }
        match result {
            Ok(Some(edit)) => {
                slot.composition = Some(Arc::new(Mutex::new(edit)));
                unsafe { out_token.write(token) };
                StellarStatus::Ok
            }
            result => {
                rt.text_inputs.close(session);
                if result.is_err() {
                    StellarStatus::Internal
                } else {
                    StellarStatus::InvalidArgument
                }
            }
        }
    })
}

fn active(
    handle: u64,
    session: u64,
    token: u64,
) -> Result<Arc<Mutex<CanvasTextInput>>, StellarStatus> {
    let reg = registry().lock().map_err(|_| StellarStatus::Internal)?;
    let rt = reg.live.get(&handle).ok_or(StellarStatus::InvalidHandle)?;
    rt.text_inputs
        .active(session, token)
        .cloned()
        .ok_or(StellarStatus::InvalidArgument)
}

/// native marked text와 그 안의 UTF-16 상대 선택을 갱신합니다.
/// # Safety
/// len>0이면 text는 len byte 읽을 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_text_mark(
    handle: u64,
    session: u64,
    token: u64,
    text: *const u8,
    len: usize,
    start: u32,
    count: u32,
) -> StellarStatus {
    ffi_guard(move || {
        let value = match unsafe { decode_utf8_owned(text, len) } {
            Ok(s) => s,
            Err(status) => return status,
        };
        let range = match input::TextRange::from_utf16(&value, start, count) {
            Ok(r) => r,
            Err(_) => return StellarStatus::InvalidArgument,
        };
        let Ok(scalar_start) = u32::try_from(value[..range.start() as usize].chars().count())
        else {
            return StellarStatus::InvalidArgument;
        };
        let Ok(scalar_end) = u32::try_from(value[..range.end() as usize].chars().count()) else {
            return StellarStatus::InvalidArgument;
        };
        let edit = match active(handle, session, token) {
            Ok(e) => e,
            Err(s) => return s,
        };
        let Ok(mut edit) = edit.lock() else {
            return StellarStatus::Internal;
        };
        if edit.set_marked(
            value,
            TextRange::new(scalar_start, scalar_end).expect("validated UTF-16 range"),
        ) {
            StellarStatus::Ok
        } else {
            StellarStatus::InvalidArgument
        }
    })
}

/// 현재 조합을 포함하는 UTF-8 본문을 caller buffer로 조회합니다. null buffer는 길이만 반환합니다.
/// # Safety
/// out_len은 usize 쓰기를 허용하고, buf는 null 또는 capacity byte를 쓸 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_text_preview(
    handle: u64,
    session: u64,
    token: u64,
    buf: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> StellarStatus {
    ffi_guard(move || {
        if out_len.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { out_len.write(0) };
        let edit = match active(handle, session, token) {
            Ok(e) => e,
            Err(s) => return s,
        };
        let Ok(edit) = edit.lock() else {
            return StellarStatus::Internal;
        };
        let Ok(preview) = edit.preview() else {
            return StellarStatus::InvalidArgument;
        };
        let text = preview.content().as_bytes();
        unsafe { out_len.write(text.len()) };
        if buf.is_null() {
            return StellarStatus::Ok;
        }
        if capacity < text.len() {
            return StellarStatus::BufferTooSmall;
        }
        unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), buf, text.len()) };
        StellarStatus::Ok
    })
}

/// 최종 입력을 비동기로 접수합니다. 재시도 가능한 큐 오류에는 조합을 유지합니다.
/// # Safety
/// len>0이면 text는 len byte 읽기 가능하며 out_request는 null 또는 정렬된 u64 쓰기가 가능해야 합니다.
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
        if !out_request.is_null() {
            unsafe { out_request.write(0) };
        }
        let value = match unsafe { decode_utf8_owned(text, len) } {
            Ok(s) => s,
            Err(status) => return status,
        };
        let edit = match active(handle, session, token) {
            Ok(e) => e,
            Err(s) => return s,
        };
        let Ok(edit) = edit.lock() else {
            return StellarStatus::Internal;
        };
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        if rt.text_inputs.active(session, token).is_none() {
            return StellarStatus::InvalidArgument;
        }
        let Some(app) = rt.app.as_ref() else {
            return StellarStatus::InvalidArgument;
        };
        let result = app.commit_canvas_text(&edit, value);
        if result.is_ok() {
            rt.text_inputs.close(session);
        }
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 지정한 조합만 버립니다. 늦은 취소는 더 최근 입력에 영향을 주지 않습니다.
#[no_mangle]
pub extern "C" fn stellar_text_cancel(handle: u64, session: u64, token: u64) -> StellarStatus {
    ffi_guard(move || {
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        if rt.text_inputs.active(session, token).is_none() {
            return StellarStatus::InvalidArgument;
        }
        rt.text_inputs.close(session);
        StellarStatus::Ok
    })
}

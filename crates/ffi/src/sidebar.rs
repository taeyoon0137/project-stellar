//! 셸 worker가 읽는 창·View·revision별 한정된 트리 페이지.
use crate::{ffi_guard, registry, StellarStatus};
use stellar_core_types::{ids::StableId, revision::DocumentRevision};
use stellar_runtime::app::WindowSessionHandle;
use stellar_runtime::document_thread::SubmitError;

#[repr(C)]
pub struct StellarLayerQuery {
    view: [u8; 16],
    parent: [u8; 16],
    after: [u8; 16],
    revision: u64,
}
#[repr(C)]
#[derive(Default)]
pub struct StellarLayerInfo {
    page: [u8; 16],
    primary: [u8; 16],
    reveal_child: [u8; 16],
    count: u32,
    more: u32,
    page_name_len: usize,
    view_name_len: usize,
    text_len: usize,
}
#[repr(C)]
pub struct StellarLayerRow {
    id: [u8; 16],
    kind: u16,
    shape: u8,
    flags: u8,
    name_len: usize,
    selection_order: usize,
}

/// 셸의 background queue에서만 호출합니다. registry 잠금을 푼 뒤 작업 결과를 기다립니다.
/// # Safety
/// `query`는 읽기 가능, `info`와 `rows`는 각각 1개와 64개 정렬된 구조체를 쓸 수 있어야 합니다.
/// `text`는 null 또는 `capacity` 바이트를 쓸 수 있어야 합니다. 출력 버퍼는 서로 겹치지 않아야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_layer_page(
    handle: u64,
    session: u64,
    query: *const StellarLayerQuery,
    info: *mut StellarLayerInfo,
    rows: *mut StellarLayerRow,
    text: *mut u8,
    capacity: usize,
) -> StellarStatus {
    unsafe {
        stellar_window_layer_page_with_history(
            handle,
            session,
            query,
            info,
            rows,
            text,
            capacity,
            std::ptr::null_mut(),
        )
    }
}

/// 행의 선택과 같은 시점의 history sequence를 함께 반환합니다.
/// # Safety
/// 버퍼는 `stellar_window_layer_page`와 같은 계약입니다. `out_history`는 null이거나
/// 다른 출력과 겹치지 않는 정렬된 u64 하나를 쓸 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_layer_page_with_history(
    handle: u64,
    session: u64,
    query: *const StellarLayerQuery,
    info: *mut StellarLayerInfo,
    rows: *mut StellarLayerRow,
    text: *mut u8,
    capacity: usize,
    out_history: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if query.is_null() || info.is_null() || rows.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { info.write(StellarLayerInfo::default()) };
        let query = unsafe { &*query };
        let optional = |bytes| (bytes != [0; 16]).then(|| StableId::from_bytes(bytes));
        let ticket = {
            let Ok(reg) = registry().lock() else {
                return StellarStatus::Internal;
            };
            let Some(app) = reg.live.get(&handle).and_then(|rt| rt.app.as_ref()) else {
                return StellarStatus::InvalidHandle;
            };
            match app.sidebar_page(
                WindowSessionHandle(session),
                StableId::from_bytes(query.view),
                DocumentRevision(query.revision),
                optional(query.parent),
                optional(query.after),
            ) {
                Ok(ticket) => ticket,
                Err(SubmitError::QueueFull { .. }) => return StellarStatus::QueueFull,
                Err(SubmitError::Stopped) => return StellarStatus::InvalidHandle,
            }
        };
        let page = match ticket.wait() {
            Ok(Some(page)) => page,
            Ok(None) => return StellarStatus::NoError,
            Err(_) => return StellarStatus::Internal,
        };
        if !out_history.is_null() {
            unsafe { out_history.write(page.history_sequence) };
        }
        let names = [&page.page_name, &page.view_name]
            .into_iter()
            .chain(page.rows.iter().map(|r| &r.name));
        let Some(total) = names
            .clone()
            .try_fold(0usize, |sum, name| sum.checked_add(name.len()))
        else {
            return StellarStatus::Internal;
        };
        unsafe {
            info.write(StellarLayerInfo {
                page: page.page.to_bytes(),
                primary: page.primary.unwrap_or(StableId::NIL).to_bytes(),
                reveal_child: page.reveal_child.unwrap_or(StableId::NIL).to_bytes(),
                more: u32::from(page.more),
                count: page.rows.len() as u32,
                page_name_len: page.page_name.len(),
                view_name_len: page.view_name.len(),
                text_len: total,
            })
        };
        if total > capacity || (total > 0 && text.is_null()) {
            return StellarStatus::BufferTooSmall;
        }
        let mut offset = 0;
        for name in names {
            if !name.is_empty() {
                unsafe {
                    std::ptr::copy_nonoverlapping(name.as_ptr(), text.add(offset), name.len())
                };
                offset += name.len();
            }
        }
        for (i, row) in page.rows.iter().enumerate() {
            let flags =
                u8::from(row.visible) | u8::from(row.locked) << 1 | u8::from(row.has_children) << 2;
            unsafe {
                rows.add(i).write(StellarLayerRow {
                    id: row.id.to_bytes(),
                    kind: row.kind,
                    shape: row.shape,
                    flags,
                    name_len: row.name.len(),
                    selection_order: row.selection_order.map_or(0, |order| order + 1),
                })
            };
        }
        StellarStatus::Ok
    })
}

/// 표시한 레이어 이름 변경을 문서 스레드에 접수합니다.
/// # Safety
/// 두 ID는 16바이트씩, name은 name_len바이트 읽을 수 있어야 합니다.
/// out_request는 null이거나 u64 하나를 쓸 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_layer_rename(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    node: *const [u8; 16],
    revision: u64,
    name: *const u8,
    name_len: usize,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if view.is_null() || node.is_null() {
            return StellarStatus::NullPointer;
        }
        let name = match unsafe { crate::decode_utf8_owned(name, name_len) } {
            Ok(name) => name,
            Err(status) => return status,
        };
        let view = StableId::from_bytes(unsafe { *view });
        let node = StableId::from_bytes(unsafe { *node });
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.rename_layer(
                WindowSessionHandle(session),
                view,
                DocumentRevision(revision),
                node,
                name,
            ),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

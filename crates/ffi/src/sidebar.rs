//! 셸 worker가 읽는 창·View·revision별 한정된 트리 페이지.
use crate::{ffi_guard, registry, StellarStatus};
use stellar_core_types::{ids::StableId, revision::DocumentRevision};
use stellar_runtime::app::sidebar::LayerFlag;
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
/// 행의 선택 순서와 같은 시점의 창 history sequence를 `out_history`에 함께 씁니다.
/// # Safety
/// `query`는 읽기 가능, `info`와 `rows`는 각각 1개와 64개 정렬된 구조체를 쓸 수 있어야 합니다.
/// `text`는 null 또는 `capacity` 바이트를 쓸 수 있어야 합니다. `out_history`는 정렬된 u64 하나를 쓸 수
/// 있어야 합니다. 출력 버퍼는 서로 겹치지 않아야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_layer_page(
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
        if query.is_null() || info.is_null() || rows.is_null() || out_history.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe {
            info.write(StellarLayerInfo::default());
            out_history.write(0);
        }
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
        unsafe { out_history.write(page.history_sequence) };
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
            let flags = u8::from(row.visible)
                | u8::from(row.locked) << 1
                | u8::from(row.has_children) << 2
                | u8::from(row.can_hide) << 3
                | u8::from(row.can_lock) << 4;
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

/// 레이어 하나를 활성 View의 표시 순서로 이동합니다. parent가 null이면 View root 직속입니다.
/// # Safety
/// view와 node는 16바이트를 읽을 수 있어야 하고, parent는 null 또는 16바이트를 읽을 수 있어야 합니다.
/// out_request는 null이거나 u64 하나를 쓸 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_layer_move(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    revision: u64,
    node: *const [u8; 16],
    parent: *const [u8; 16],
    index: usize,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if view.is_null() || node.is_null() {
            return StellarStatus::NullPointer;
        }
        let view = StableId::from_bytes(unsafe { *view });
        let node = StableId::from_bytes(unsafe { *node });
        let parent = (!parent.is_null()).then(|| StableId::from_bytes(unsafe { *parent }));
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.move_layer(
                WindowSessionHandle(session),
                view,
                DocumentRevision(revision),
                node,
                parent,
                index,
            ),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 표시한 레이어 행들의 표시(`flag` 1) 또는 잠금(`flag` 2)을 한 transaction에 토글합니다.
/// # Safety
/// view는 16바이트, nodes는 `count * 16`바이트를 읽을 수 있어야 합니다.
/// out_request는 null이거나 u64 하나를 쓸 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_layer_set_flag(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    revision: u64,
    nodes: *const [u8; 16],
    count: usize,
    flag: u32,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if view.is_null() || nodes.is_null() {
            return StellarStatus::NullPointer;
        }
        let flag = match flag {
            1 => LayerFlag::Visible,
            2 => LayerFlag::Locked,
            _ => return StellarStatus::InvalidArgument,
        };
        if count == 0 || count > isize::MAX as usize / 16 {
            return StellarStatus::InvalidArgument;
        }
        let view = StableId::from_bytes(unsafe { *view });
        let nodes = unsafe { std::slice::from_raw_parts(nodes, count) }
            .iter()
            .copied()
            .map(StableId::from_bytes)
            .collect();
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.toggle_layer_flag(
                WindowSessionHandle(session),
                view,
                DocumentRevision(revision),
                nodes,
                flag,
            ),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 머리 줄 Page › View 팝업의 한 행. View마다 하나이고, View 없는 Page는 `view`가 0입니다.
#[repr(C)]
pub struct StellarViewOutlineRow {
    page: [u8; 16],
    view: [u8; 16],
    page_name_len: usize,
    view_name_len: usize,
}

/// 셸 worker 전용. 창 작업 문서의 Page·View를 정의 순서로 씁니다. text는 행 순서로 Page 이름(그 Page의 첫 행만)과
/// View 이름입니다. 공간이 부족하면 `BufferTooSmall`과 함께 필요한 행 수·text 길이를 씁니다.
/// # Safety
/// `rows`는 null 또는 `row_capacity`개, `text`는 null 또는 `text_capacity`바이트를 쓸 수 있어야 하며 서로 겹치지 않아야
/// 합니다. `out_rows`·`out_text_len`은 각각 usize 하나를 쓸 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_view_outline(
    handle: u64,
    session: u64,
    rows: *mut StellarViewOutlineRow,
    row_capacity: usize,
    text: *mut u8,
    text_capacity: usize,
    out_rows: *mut usize,
    out_text_len: *mut usize,
) -> StellarStatus {
    ffi_guard(move || {
        if out_rows.is_null() || out_text_len.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe {
            out_rows.write(0);
            out_text_len.write(0);
        }
        let ticket = {
            let Ok(reg) = registry().lock() else {
                return StellarStatus::Internal;
            };
            let Some(app) = reg.live.get(&handle).and_then(|rt| rt.app.as_ref()) else {
                return StellarStatus::InvalidHandle;
            };
            match app.view_outline(WindowSessionHandle(session)) {
                Ok(ticket) => ticket,
                Err(SubmitError::QueueFull { .. }) => return StellarStatus::QueueFull,
                Err(SubmitError::Stopped) => return StellarStatus::InvalidHandle,
            }
        };
        let pages = match ticket.wait() {
            Ok(Some(pages)) => pages,
            Ok(None) => return StellarStatus::NoError,
            Err(_) => return StellarStatus::Internal,
        };
        let mut items = Vec::new();
        let mut names: Vec<&str> = Vec::new();
        for page in &pages {
            if page.views.is_empty() {
                items.push((page.id, StableId::NIL, page.name.len(), 0));
                names.push(&page.name);
            }
            for (index, (view, name)) in page.views.iter().enumerate() {
                let page_name = if index == 0 { page.name.as_str() } else { "" };
                items.push((page.id, *view, page_name.len(), name.len()));
                names.push(page_name);
                names.push(name);
            }
        }
        let Some(total) = names
            .iter()
            .try_fold(0usize, |sum, name| sum.checked_add(name.len()))
        else {
            return StellarStatus::Internal;
        };
        unsafe {
            out_rows.write(items.len());
            out_text_len.write(total);
        }
        if items.len() > row_capacity
            || total > text_capacity
            || (!items.is_empty() && rows.is_null())
            || (total > 0 && text.is_null())
        {
            return StellarStatus::BufferTooSmall;
        }
        for (i, (page, view, page_name_len, view_name_len)) in items.into_iter().enumerate() {
            unsafe {
                rows.add(i).write(StellarViewOutlineRow {
                    page: page.to_bytes(),
                    view: view.to_bytes(),
                    page_name_len,
                    view_name_len,
                })
            };
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
        StellarStatus::Ok
    })
}

/// 창 session에서 작업 문서의 View 탭을 열고 active로 만듭니다(이미 열려 있으면 active만).
/// # Safety
/// view는 16바이트를 읽을 수 있어야 하고, out_request는 null이거나 u64 하나를 쓸 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_open_view(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if view.is_null() {
            return StellarStatus::NullPointer;
        }
        let view = StableId::from_bytes(unsafe { *view });
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.open_view_tab(WindowSessionHandle(session), view),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

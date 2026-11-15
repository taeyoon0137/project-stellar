//! 창 session C ABI (XII.2.2, ADR-0028).
//!
//! 셸의 창 controller 하나가 창 session handle 하나를 소유합니다. 요청은 다른 프로젝트·문서 요청과 같이
//! 기다리지 않고 request id를 돌려주며, 완료는 `stellar_runtime_poll_event`의 event
//! (`STELLAR_EVENT_WINDOW_SESSION_*`)와 [`stellar_window_session_info`]의 상태 snapshot으로만 돌아옵니다.
//!
//! 활성 창의 Undo·Redo·Save Now(XII.6.4)는 [`stellar_window_session_perform`]에 명령 코드
//! (`STELLAR_COMMAND_*`)를 넘겨 그 창 session에만 보냅니다. 선택 변경([`stellar_window_session_select`])은 그 창
//! history에 기록됩니다.
//!
//! 명령 코드는 코어가 정하고 셸과 공유합니다(XII.6.5). 셸은 메뉴 명령 식별자(공유 이름, 예: `edit.undo`)로
//! [`stellar_command_code`]에 물어 코드를 얻고, 단축키는 각 운영체제 관례대로 셸 메뉴에 둡니다.

use stellar_core_types::ids::StableId;
use stellar_runtime::app::{
    AppRuntime, ProjectSessionId, SelectionEdit, SelectionEditTarget, WindowSessionHandle,
};
use stellar_runtime::app_command::AppCommand;

use crate::{ffi_guard, registry, StellarStatus};

/// `StellarWindowSessionInfo` 레이아웃 버전.
pub const WINDOW_SESSION_INFO_VERSION: u32 = 2;

/// `StellarWindowViewInfo` 레이아웃 버전.
pub const WINDOW_VIEW_INFO_VERSION: u32 = 1;

/// 창 view state 정보(고정 56 byte, C ABI, XII.4.3): 열린 탭 수, active view와 그 viewport(scene camera 입력).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[repr(C)]
pub struct StellarWindowViewInfo {
    /// [`WINDOW_VIEW_INFO_VERSION`](코어가 씀).
    pub version: u32,
    /// active 탭이 있으면 1(없으면 아래 viewport·view 값은 0).
    pub has_view: u32,
    /// 열린 탭 수.
    pub tab_count: u32,
    /// 0.
    pub reserved: u32,
    /// 화면 중심의 문서 x.
    pub center_x: f64,
    /// 화면 중심의 문서 y.
    pub center_y: f64,
    /// 배율(1.0 = 100%).
    pub zoom: f64,
    /// active view StableId 16 byte.
    pub active_view: [u8; 16],
}

/// 확정된 선택의 Inspector snapshot(고정 104 byte, C ABI). viewport 밖에서 선택한 레이어도 포함합니다.
///
/// 값과 편집 가능 여부는 core가 문서에서 읽습니다. 셸은 캔버스 투영에서 Inspector 값을 읽지 않습니다.
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct StellarSelectedNode {
    /// 문서 revision.
    pub revision: u64,
    /// 같은 snapshot의 창 history sequence. Inspector 편집 요청에 그대로 넘깁니다.
    pub history_sequence: u64,
    /// 선택한 node 수.
    pub selection_count: u64,
    /// 활성 View ID.
    pub view: [u8; 16],
    /// 단일 선택 node ID(단일 선택이 아니면 0).
    pub node: [u8; 16],
    /// 단일 선택의 NodeKind 저장 tag(단일 선택이 아니면 0).
    pub kind: u16,
    /// Shape이면 ShapeKind 저장 tag, 아니면 0.
    pub shape: u8,
    /// bit 0: 아래 Frame 값이 유효함, bit 1: Inspector 편집을 받을 수 있음.
    pub flags: u8,
    /// Frame 고정 너비.
    pub width: f32,
    /// Frame 고정 높이.
    pub height: f32,
    /// Frame 원본 모서리 반경 TL·TR·BR·BL.
    pub radii: [f32; 4],
    /// Frame 단색 배경(색공간 0 = 없음).
    pub background: crate::canvas::StellarCanvasColor,
}

/// `StellarSelectedNode.flags`: Frame 값이 유효함.
pub const SELECTED_FRAME: u8 = 1;
/// `StellarSelectedNode.flags`: Inspector 편집을 받을 수 있음.
pub const SELECTED_EDITABLE: u8 = 2;

/// 한 게시 snapshot에서 활성 View·선택·revision·history와 단일 선택의 Inspector 값을 읽습니다.
/// 활성 View가 없으면 `STELLAR_NO_ERROR`와 0 값, 열려 있지 않은 창이면 `STELLAR_INVALID_ARGUMENT`입니다.
/// # Safety
/// `out`은 `StellarSelectedNode` 하나를 쓸 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_selected_node(
    handle: u64,
    session: u64,
    out: *mut StellarSelectedNode,
) -> StellarStatus {
    ffi_guard(move || {
        if out.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { out.write(StellarSelectedNode::default()) };
        let Ok(reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let state = rt.app.as_ref().map(AppRuntime::state);
        let Some(w) = state
            .as_ref()
            .and_then(|s| s.window(WindowSessionHandle(session)))
        else {
            return StellarStatus::InvalidArgument;
        };
        let Some(view) = w.active_view else {
            return StellarStatus::NoError;
        };
        let mut info = StellarSelectedNode {
            revision: w.revision.0,
            history_sequence: w.history_sequence,
            selection_count: u64::from(w.selection_len),
            view: view.to_bytes(),
            ..StellarSelectedNode::default()
        };
        if let Some(selected) = w.selected {
            info.node = selected.node.to_bytes();
            info.kind = selected.kind;
            info.shape = selected.shape;
            if let Some(frame) = selected.frame {
                let (tl, tr, br, bl) = frame.corners.components();
                info.flags = SELECTED_FRAME | if frame.editable { SELECTED_EDITABLE } else { 0 };
                info.width = frame.size.width();
                info.height = frame.size.height();
                info.radii = [tl, tr, br, bl];
                info.background = frame.background.into();
            }
        }
        unsafe { out.write(info) };
        StellarStatus::Ok
    })
}

/// 열린 창 session 정보(고정 88 byte, C ABI).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct StellarWindowSessionInfo {
    /// [`WINDOW_SESSION_INFO_VERSION`](코어가 씀).
    pub version: u32,
    /// 작업 문서가 있으면 1, 없으면 0(저장한 적 없는 프로젝트).
    pub has_document: u32,
    /// 창 session handle.
    pub session: u64,
    /// 소속 프로젝트 session handle.
    pub project: u64,
    /// snapshot 게시 시점의 작업 문서 revision.
    pub revision: u64,
    /// loaded generation 번호(저장한 적 없으면 0).
    pub generation: u64,
    /// undo stack 깊이.
    pub undo_depth: u32,
    /// redo stack 깊이.
    pub redo_depth: u32,
    /// 선택된 node 수.
    pub selection_count: u32,
    /// 저장되지 않은 문서 변경이면 1(선택·viewport 제외). v1의 reserved 위치.
    pub has_unsaved_changes: u32,
    /// 창 편집 session id 16 byte(UUID 바이트 순서).
    pub window_id: [u8; 16],
    /// 작업 문서 StableId 16 byte(없으면 0).
    pub document_id: [u8; 16],
}

/// 프로젝트 session 아래에서 창 session 열기를 요청합니다. 기다리지 않고 request id를 씁니다.
///
/// 결과는 `STELLAR_EVENT_WINDOW_SESSION_OPENED` 또는 `STELLAR_EVENT_REQUEST_FAILED` event로 옵니다.
///
/// # Safety
/// `out_request`는 null이거나 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_open(
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
            Ok(app) => app.open_window_session(ProjectSessionId(project)),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 창 session 닫기를 요청합니다. 그 창의 문서 state·history·선택을 저장하지 않고 폐기합니다.
///
/// 결과는 `STELLAR_EVENT_WINDOW_SESSION_CLOSED` 또는 `STELLAR_EVENT_REQUEST_FAILED` event로 옵니다.
///
/// # Safety
/// `out_request`는 null이거나 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_close(
    handle: u64,
    session: u64,
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
            Ok(app) => app.close_window_session(WindowSessionHandle(session)),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 공유 command 이름(UTF-8 ptr+len, 예: `edit.undo`)의 코어 명령 코드를 씁니다(XII.6.5). runtime handle이 필요 없는
/// 순수 조회입니다.
///
/// 코어가 구현하는 이름이면 `Ok`와 코드(`STELLAR_COMMAND_*`), 코어 명령이 아닌 이름(셸 전용·미구현)이면
/// `STELLAR_NO_ERROR`와 코드 0입니다. 유효하지 않은 UTF-8은 `InvalidUtf8`, null `out_code`는 `NullPointer`입니다.
///
/// # Safety
/// `name_len > 0`이면 `name`은 `name_len` 바이트 읽기가 유효해야 하고, `out_code`는 `u32` 쓰기가 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_command_code(
    name: *const u8,
    name_len: usize,
    out_code: *mut u32,
) -> StellarStatus {
    ffi_guard(move || {
        if out_code.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { *out_code = 0 };
        let text = match unsafe { crate::decode_utf8_owned(name, name_len) } {
            Ok(text) => text,
            Err(status) => return status,
        };
        match AppCommand::from_name(&text) {
            Some(command) => {
                unsafe { *out_code = command.code() };
                StellarStatus::Ok
            }
            None => StellarStatus::NoError,
        }
    })
}

/// 창 session에 명령(`STELLAR_COMMAND_*`)을 보냅니다(XII.6.4). 기다리지 않고 request id를 씁니다.
///
/// Undo·Redo는 `STELLAR_EVENT_HISTORY_REPLAYED`, Save Now는 `STELLAR_EVENT_WINDOW_SESSION_SAVED`, 실패는
/// `STELLAR_EVENT_REQUEST_FAILED`(없는 창, `SAVE_STALE`, `SAVE_FAILED`, `HISTORY_UNAVAILABLE`)로 옵니다. 모르는 명령
/// 코드는 `InvalidArgument`(요청 없음, `out_request = 0`)입니다.
///
/// # Safety
/// `out_request`는 null이거나 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_perform(
    handle: u64,
    session: u64,
    command: u32,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let Some(command) = AppCommand::from_code(command) else {
            rt.last_error = Some((
                StellarStatus::InvalidArgument,
                format!("알 수 없는 명령 코드입니다: {command}"),
            ));
            if !out_request.is_null() {
                unsafe { *out_request = 0 };
            }
            return StellarStatus::InvalidArgument;
        };
        let result = match rt.app() {
            Ok(app) => app.perform(WindowSessionHandle(session), command),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 실제 canvas 크기(logical point)로 active View의 root Frame이 화면에 들어오도록 카메라를 맞춥니다.
/// 새 창의 첫 표시와 Zoom to Fit이 씁니다. 완료는 `WINDOW_VIEW_CHANGED` 또는 `REQUEST_FAILED`입니다.
///
/// # Safety
/// `out_request`는 null이거나 `u64` 쓰기가 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_fit_root(
    handle: u64,
    session: u64,
    width: f64,
    height: f64,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe {
                *out_request = 0;
            }
        }
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return StellarStatus::InvalidArgument;
        }
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.fit_root_frame(WindowSessionHandle(session), width, height),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 창의 active viewport 내용을 화면 point 단위로 이동합니다(Hand 도구 드래그). 문서·history는 바꾸지 않습니다.
/// 완료는 `STELLAR_EVENT_WINDOW_VIEW_CHANGED` 또는 `STELLAR_EVENT_REQUEST_FAILED`입니다. 스크롤은
/// `stellar_window_session_submit_mac_event`로 코어가 해석합니다.
///
/// # Safety
/// `out_request`는 null이거나 u64 쓰기가 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_pan(
    handle: u64,
    session: u64,
    dx: f64,
    dy: f64,
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
            Ok(app) => app.pan_viewport(WindowSessionHandle(session), dx, dy),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 화면 중앙 기준 클릭으로 한 단계 줌. fit_factor > 0이면 영역 중심을 가운데 두고 해당 배율을 곱합니다.
/// # Safety
/// out_request는 null이거나 요청 ID 하나를 쓸 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_zoom_at(
    handle: u64,
    session: u64,
    x: f64,
    y: f64,
    fit_factor: f64,
    zoom_out: u32,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { out_request.write(0) };
        }
        if !x.is_finite()
            || !y.is_finite()
            || !fit_factor.is_finite()
            || fit_factor < 0.0
            || zoom_out > 1
        {
            return StellarStatus::InvalidArgument;
        }
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.zoom_viewport_at(
                WindowSessionHandle(session),
                [x, y],
                (fit_factor > 0.0).then_some(fit_factor),
                zoom_out != 0,
            ),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 표시된 선택의 문서 편집 기준(기존 창·뷰 ABI와 독립).
#[derive(Debug, Default, Clone, Copy)]
#[repr(C)]
pub struct StellarSelectionTarget {
    session: u64,
    revision: u64,
    history_sequence: u64,
    selection_count: u64,
    view: [u8; 16],
}

impl StellarSelectionTarget {
    pub(crate) fn runtime_target(self) -> SelectionEditTarget {
        SelectionEditTarget {
            session: WindowSessionHandle(self.session),
            view: StableId::from_bytes(self.view),
            revision: stellar_core_types::revision::DocumentRevision(self.revision),
            history_sequence: self.history_sequence,
        }
    }
}

/// 한 번 게시된 snapshot에서 선택과 활성 View를 함께 읽습니다.
/// # Safety
/// `out`은 대상 구조체 하나를 쓸 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_selection_target(
    handle: u64,
    session: u64,
    out: *mut StellarSelectionTarget,
) -> StellarStatus {
    ffi_guard(move || {
        if out.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { out.write(StellarSelectionTarget::default()) };
        let Ok(reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let state = rt.app.as_ref().map(AppRuntime::state);
        let Some(summary) = state
            .as_ref()
            .and_then(|s| s.window(WindowSessionHandle(session)))
        else {
            return StellarStatus::InvalidArgument;
        };
        let Some(view) = summary.active_view else {
            return StellarStatus::NoError;
        };
        unsafe {
            out.write(StellarSelectionTarget {
                session,
                revision: summary.revision.0,
                history_sequence: summary.history_sequence,
                selection_count: u64::from(summary.selection_len),
                view: view.to_bytes(),
            })
        };
        StellarStatus::Ok
    })
}

/// 주어진 선택 snapshot에서 형제 순서 이동 가능 방향과 흐름 부모 여부를 읽습니다.
/// # Safety
/// `target`은 읽을 수 있는 구조체 하나를 가리켜야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_layer_order_flags(
    handle: u64,
    target: *const StellarSelectionTarget,
) -> u32 {
    std::panic::catch_unwind(|| {
        let target = unsafe { target.as_ref() }?;
        let reg = registry().lock().ok()?;
        let state = reg.live.get(&handle)?.app.as_ref()?.state();
        let window = state.window(WindowSessionHandle(target.session))?;
        (window.active_view?.to_bytes() == target.view
            && window.revision.0 == target.revision
            && window.history_sequence == target.history_sequence
            && target.selection_count == 1
            && window.selection_len == 1)
            .then_some(window.order_flags)
    })
    .ok()
    .flatten()
    .unwrap_or(0)
}

/// 표시된 선택에 문서 명령을 접수합니다. Layers 요청은 대상 ID도 함께 고정합니다.
/// # Safety
/// `target`은 구조체 하나, `ids`는 null 또는 `count * 16`바이트를 읽을 수 있어야 합니다.
/// `out_request`는 null 또는 쓰기가 가능한 u64 포인터입니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_edit_selection(
    handle: u64,
    target: *const StellarSelectionTarget,
    action: u32,
    ids: *const [u8; 16],
    count: usize,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        let Some(target) = (unsafe { target.as_ref() }) else {
            return StellarStatus::NullPointer;
        };
        use stellar_runtime::app::SiblingOrder;
        let edit = match action {
            1 => SelectionEdit::Delete,
            2 => SelectionEdit::Duplicate,
            3 => SelectionEdit::Reorder(SiblingOrder::Forward),
            4 => SelectionEdit::Reorder(SiblingOrder::Backward),
            5 => SelectionEdit::Reorder(SiblingOrder::Front),
            6 => SelectionEdit::Reorder(SiblingOrder::Back),
            _ => return StellarStatus::InvalidArgument,
        };
        if ids.is_null() && count != 0 {
            return StellarStatus::NullPointer;
        }
        if count > isize::MAX as usize / 16 {
            return StellarStatus::InvalidArgument;
        }
        let ids = if ids.is_null() {
            None
        } else {
            if count == 0 || count as u64 != target.selection_count {
                return StellarStatus::InvalidArgument;
            }
            Some(
                unsafe { std::slice::from_raw_parts(ids, count) }
                    .iter()
                    .copied()
                    .map(StableId::from_bytes)
                    .collect(),
            )
        };
        let target = target.runtime_target();
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.edit_selection(target, edit, ids),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 표시된 창의 선택을 복사합니다. 작은 출력 buffer에는 필요한 byte 수를 반환합니다.
/// # Safety
/// `target`은 구조체 하나, `out_len`은 usize 하나, null이 아닌 `bytes`는 capacity byte를 쓸 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_copy_selection(
    handle: u64,
    target: *const StellarSelectionTarget,
    bytes: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> StellarStatus {
    ffi_guard(move || {
        if out_len.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { out_len.write(0) };
        let Some(target) = (unsafe { target.as_ref() }) else {
            return StellarStatus::NullPointer;
        };
        unsafe {
            copy_selection_bytes(handle, bytes, capacity, out_len, |app| {
                app.copy_displayed_subtree(target.runtime_target())
            })
        }
    })
}

unsafe fn copy_selection_bytes(
    handle: u64,
    bytes: *mut u8,
    capacity: usize,
    out_len: *mut usize,
    capture: impl FnOnce(
        &AppRuntime,
    ) -> Result<
        stellar_runtime::document_thread::JobTicket<Option<Vec<u8>>>,
        stellar_runtime::document_thread::SubmitError,
    >,
) -> StellarStatus {
    let ticket = {
        let Ok(reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(app) = reg.live.get(&handle).and_then(|rt| rt.app.as_ref()) else {
            return StellarStatus::InvalidHandle;
        };
        match capture(app) {
            Ok(ticket) => ticket,
            Err(stellar_runtime::document_thread::SubmitError::QueueFull { .. }) => {
                return StellarStatus::QueueFull
            }
            Err(_) => return StellarStatus::InvalidHandle,
        }
    };
    let data = match ticket.wait() {
        Ok(Some(data)) => data,
        Ok(None) => return StellarStatus::NoError,
        Err(_) => return StellarStatus::Internal,
    };
    unsafe { out_len.write(data.len()) };
    if data.len() > capacity || bytes.is_null() {
        return StellarStatus::BufferTooSmall;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(data.as_ptr(), bytes, data.len());
    }
    StellarStatus::Ok
}

/// 표시한 창·선택에서 같은 문서의 subtree를 붙여넣습니다. 목적 부모는 실행 시점의 선택에서 계산합니다.
/// 길이는 1 byte~16 MiB이며 복사 전에 검사합니다. 반환 후 입력 buffer를 해제해도 됩니다.
/// # Safety
/// `target`은 구조체 하나, `bytes`는 len byte를 읽을 수 있고 `out_request`는 null 또는 쓰기 가능해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_paste_selection(
    handle: u64,
    target: *const StellarSelectionTarget,
    bytes: *const u8,
    len: usize,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { out_request.write(0) };
        }
        if len == 0 || len > 16 * 1024 * 1024 {
            return StellarStatus::InvalidArgument;
        }
        let Some(target) = (unsafe { target.as_ref() }) else {
            return StellarStatus::NullPointer;
        };
        if bytes.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe {
            paste_selection_bytes(handle, bytes, len, out_request, |app, data| {
                app.paste_displayed_subtree(target.runtime_target(), data)
            })
        }
    })
}

unsafe fn paste_selection_bytes(
    handle: u64,
    bytes: *const u8,
    len: usize,
    out_request: *mut u64,
    paste: impl FnOnce(
        &AppRuntime,
        Vec<u8>,
    )
        -> Result<stellar_runtime::app::RequestId, stellar_runtime::app::RequestError>,
) -> StellarStatus {
    let Ok(mut reg) = registry().lock() else {
        return StellarStatus::Internal;
    };
    let Some(rt) = reg.live.get_mut(&handle) else {
        return StellarStatus::InvalidHandle;
    };
    let result = match rt.app() {
        Ok(app) => paste(
            app,
            unsafe { std::slice::from_raw_parts(bytes, len) }.to_vec(),
        ),
        Err(status) => return status,
    };
    unsafe { rt.finish_request(result, out_request) }
}

/// 표시된 Frame을 부모 좌표계 delta만큼 이동하도록 요청합니다.
///
/// # Safety
/// `view`와 `node`는 각각 16 byte를 읽을 수 있고 `out_request`는 null 또는 쓰기 가능해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_move_frame(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    node: *const [u8; 16],
    revision: u64,
    dx: f32,
    dy: f32,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if view.is_null() || node.is_null() {
            return StellarStatus::NullPointer;
        }
        let Ok(delta) = stellar_core_types::value::Vec2::new(dx, dy) else {
            return StellarStatus::InvalidArgument;
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
            Ok(app) => app.move_canvas_frame(
                WindowSessionHandle(session),
                view,
                stellar_core_types::revision::DocumentRevision(revision),
                node,
                delta,
            ),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// Inspector가 Frame 속성 하나에 쓰는 값(고정 24 byte). 길이 key는 `length`, `BACKGROUND`는 `color`를 씁니다.
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct StellarFrameValue {
    /// width·height·모서리 longhand의 비음수 논리 픽셀 길이.
    pub length: f32,
    /// 배경 색(색공간 0 = 배경 제거).
    pub color: crate::canvas::StellarCanvasColor,
}

/// 선택 Frame의 Inspector 속성 하나를 바꿉니다. `property_key`는 CSS longhand
/// `WIDTH`(0x0108)·`HEIGHT`(0x0109)·모서리 4개(0x010a–0x010d)·`BACKGROUND`(0x0104)입니다.
/// 표시한 View·revision·`history_sequence`가 실행 시점과 다르거나 편집할 수 없는 Frame이면
/// `REQUEST_FAILED`로 끝납니다.
///
/// # Safety
/// `view`와 `node`는 각각 16 byte를 읽을 수 있고 `out_request`는 null 또는 쓰기 가능해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_set_frame_property(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    node: *const [u8; 16],
    revision: u64,
    history_sequence: u64,
    property_key: u32,
    value: StellarFrameValue,
    out_request: *mut u64,
) -> StellarStatus {
    use stellar_runtime::app::FrameValue;
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if view.is_null() || node.is_null() {
            return StellarStatus::NullPointer;
        }
        let key = stellar_core_types::property::PropertyKey(property_key);
        let value = if key == FrameValue::BACKGROUND_KEY {
            let Ok(color) = value.color.decode() else {
                return StellarStatus::InvalidArgument;
            };
            FrameValue::Background(color)
        } else if FrameValue::LENGTH_KEYS.contains(&key) {
            match stellar_core_types::value::FiniteF32::new(value.length) {
                Ok(length) if length.get() >= 0.0 => FrameValue::Length(length),
                _ => return StellarStatus::InvalidArgument,
            }
        } else {
            return StellarStatus::InvalidArgument;
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
            Ok(app) => app.set_canvas_frame_property(
                SelectionEditTarget {
                    session: WindowSessionHandle(session),
                    view,
                    revision: stellar_core_types::revision::DocumentRevision(revision),
                    history_sequence,
                },
                node,
                key,
                value,
            ),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 창에 표시한 자유 배치 부모 Frame의 local 영역에 자식 Frame을 생성합니다.
///
/// # Safety
/// `view`와 `parent`는 각각 16 byte를 읽을 수 있고 `out_request`는 null 또는 쓰기 가능해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_insert_frame(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    parent: *const [u8; 16],
    revision: u64,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    out_request: *mut u64,
) -> StellarStatus {
    unsafe {
        insert_canvas_frame(
            handle,
            session,
            view,
            parent,
            revision,
            x,
            y,
            width,
            height,
            out_request,
        )
    }
}

/// 새 도형의 채우기·선 스타일(고정 44 byte). 셸이 사용자 앱 설정에서 채웁니다(사용자 결정 D7).
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct StellarShapeStyle {
    /// Rectangle·Ellipse·Polygon 채우기. 색공간 0은 허용하지 않습니다.
    pub fill: crate::canvas::StellarCanvasColor,
    /// Line 선 색. 색공간 0은 허용하지 않습니다.
    pub stroke: crate::canvas::StellarCanvasColor,
    /// Line 선 두께(유한, 0보다 큼).
    pub stroke_width: f32,
}

/// 부모 local 공간의 시작·끝 두 점으로 Shape를 생성합니다(ShapeKind 저장 tag).
/// Fill·Stroke modifier는 `style` 값으로 만듭니다. core는 기본 색을 정하지 않습니다.
/// # Safety
/// `view`와 `parent`는 각각 16 byte를 읽을 수 있고 `out_request`는 null 또는 쓰기 가능해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_insert_shape(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    parent: *const [u8; 16],
    revision: u64,
    shape: u8,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    style: StellarShapeStyle,
    out_request: *mut u64,
) -> StellarStatus {
    use stellar_core_types::value::{FiniteF32, Vec2};
    use stellar_runtime::app::{ShapeKind, ShapeStyle};
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if view.is_null() || parent.is_null() {
            return StellarStatus::NullPointer;
        }
        let (Ok(kind), Ok(start), Ok(end)) = (
            ShapeKind::from_tag(shape),
            Vec2::new(x0, y0),
            Vec2::new(x1, y1),
        ) else {
            return StellarStatus::InvalidArgument;
        };
        let (Ok(Some(fill)), Ok(Some(stroke)), Ok(stroke_width)) = (
            style.fill.decode(),
            style.stroke.decode(),
            FiniteF32::new(style.stroke_width),
        ) else {
            return StellarStatus::InvalidArgument;
        };
        if stroke_width.get() <= 0.0 {
            return StellarStatus::InvalidArgument;
        }
        let style = ShapeStyle {
            fill,
            stroke,
            stroke_width,
        };
        let view = StableId::from_bytes(unsafe { *view });
        let parent = StableId::from_bytes(unsafe { *parent });
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.insert_canvas_shape(
                WindowSessionHandle(session),
                view,
                stellar_core_types::revision::DocumentRevision(revision),
                parent,
                kind,
                [start, end],
                style,
            ),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

#[allow(clippy::too_many_arguments)]
unsafe fn insert_canvas_frame(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    parent: *const [u8; 16],
    revision: u64,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    out_request: *mut u64,
) -> StellarStatus {
    use stellar_core_types::value::{Rect, Size2, Vec2};
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { *out_request = 0 };
        }
        if view.is_null() || parent.is_null() {
            return StellarStatus::NullPointer;
        }
        let (Ok(origin), Ok(size)) = (Vec2::new(x, y), Size2::new(width, height)) else {
            return StellarStatus::InvalidArgument;
        };
        if size.is_empty() {
            return StellarStatus::InvalidArgument;
        }
        let view = StableId::from_bytes(unsafe { *view });
        let parent = StableId::from_bytes(unsafe { *parent });
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.insert_canvas_frame(
                WindowSessionHandle(session),
                view,
                stellar_core_types::revision::DocumentRevision(revision),
                parent,
                Rect::new(origin, size),
            ),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 창 session의 선택을 `ids`(StableId 16 byte × `count`, 선택 순서)로 바꾸고 그 창 history에 기록합니다.
///
/// 문서에 없거나 숨김 node는 거릅니다. 결과는 `STELLAR_EVENT_WINDOW_VIEW_CHANGED` 또는 `REQUEST_FAILED`입니다.
/// `count`가 0이면 선택을 비웁니다.
///
/// # Safety
/// `count > 0`이면 `ids`는 `count * 16` 바이트 읽기가 유효해야 하고, `out_request`는 null이거나 유효해야
/// 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_select(
    handle: u64,
    session: u64,
    ids: *const u8,
    count: usize,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if count > 0 && ids.is_null() {
            return StellarStatus::NullPointer;
        }
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let Some(len) = count.checked_mul(16) else {
            rt.last_error = Some((
                StellarStatus::InvalidArgument,
                "선택 id 수가 너무 큽니다".to_string(),
            ));
            return StellarStatus::InvalidArgument;
        };
        let bytes = if count == 0 {
            &[][..]
        } else {
            unsafe { std::slice::from_raw_parts(ids, len) }
        };
        let selection: Vec<StableId> = bytes
            .chunks_exact(16)
            .map(|chunk| {
                let mut id = [0u8; 16];
                id.copy_from_slice(chunk);
                StableId::from_bytes(id)
            })
            .collect();
        let result = match rt.app() {
            Ok(app) => app.set_selection(WindowSessionHandle(session), selection),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// 열린 창 session의 view state(탭 수, active view, viewport)를 씁니다(XII.4.3). document thread를 기다리지 않습니다.
///
/// 열려 있지 않은 창 session은 `InvalidArgument`(last error 설정)입니다.
///
/// # Safety
/// `out_info`는 `StellarWindowViewInfo` 쓰기가 유효한 포인터여야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_view_info(
    handle: u64,
    session: u64,
    out_info: *mut StellarWindowViewInfo,
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
            .and_then(|state| state.window(WindowSessionHandle(session)))
        else {
            rt.last_error = Some((
                StellarStatus::InvalidArgument,
                format!("열려 있지 않은 창 session입니다: {session}"),
            ));
            return StellarStatus::InvalidArgument;
        };
        let mut info = StellarWindowViewInfo {
            version: WINDOW_VIEW_INFO_VERSION,
            tab_count: summary.tab_count,
            ..StellarWindowViewInfo::default()
        };
        if let (Some(view), Some(viewport)) = (summary.active_view, summary.viewport) {
            info.has_view = 1;
            info.center_x = viewport.center_x();
            info.center_y = viewport.center_y();
            info.zoom = viewport.zoom();
            info.active_view = view.to_bytes();
        }
        unsafe { *out_info = info };
        StellarStatus::Ok
    })
}

/// 열린 창 session의 최신 snapshot 정보를 씁니다. document thread를 기다리지 않습니다.
///
/// 열려 있지 않은 창 session은 `InvalidArgument`(last error 설정)입니다.
///
/// # Safety
/// `out_info`는 `StellarWindowSessionInfo` 쓰기가 유효한 포인터여야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_session_info(
    handle: u64,
    session: u64,
    out_info: *mut StellarWindowSessionInfo,
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
            .and_then(|state| state.window(WindowSessionHandle(session)))
        else {
            rt.last_error = Some((
                StellarStatus::InvalidArgument,
                format!("열려 있지 않은 창 session입니다: {session}"),
            ));
            return StellarStatus::InvalidArgument;
        };
        let info = StellarWindowSessionInfo {
            version: WINDOW_SESSION_INFO_VERSION,
            has_document: u32::from(summary.document_id.is_some()),
            session: summary.session.0,
            project: summary.project.0,
            revision: summary.revision.0,
            generation: summary.generation,
            undo_depth: summary.undo_depth,
            redo_depth: summary.redo_depth,
            selection_count: summary.selection_len,
            has_unsaved_changes: u32::from(summary.has_unsaved_changes),
            window_id: summary.window_id.stable_id().to_bytes(),
            document_id: summary.document_id.map_or([0; 16], |id| id.to_bytes()),
        };
        unsafe { *out_info = info };
        StellarStatus::Ok
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use stellar_runtime::app::{RequestFailure, RequestId, RuntimeEvent};

    use crate::{
        stellar_project_close, stellar_project_open, stellar_runtime_create,
        stellar_runtime_destroy, stellar_runtime_poll_event, stellar_runtime_state,
        StellarRuntimeState,
    };

    /// 저장 프로젝트 fixture(`crates/runtime/tests/fixtures/window-session-project`)를 복사한 임시 프로젝트.
    pub(crate) struct FixtureProject {
        pub(crate) path: PathBuf,
    }

    impl FixtureProject {
        pub(crate) fn copy(tag: &str) -> FixtureProject {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-ffi-window-{tag}-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            let source = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../runtime/tests/fixtures/window-session-project");
            copy_tree(&source, &path);
            std::fs::create_dir(path.join(".git")).unwrap();
            FixtureProject { path }
        }
    }

    impl Drop for FixtureProject {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    /// 다음 event를 C ABI로 읽어 해석합니다(최대 5초).
    pub(crate) fn next_event(h: u64) -> RuntimeEvent {
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
                let status = unsafe {
                    stellar_runtime_poll_event(
                        h,
                        &mut kind,
                        buf.as_mut_ptr(),
                        buf.len(),
                        &mut written,
                    )
                };
                assert_eq!(status, StellarStatus::Ok);
                return RuntimeEvent::decode(kind, &buf[..written]).expect("event payload");
            }
            assert_eq!(status, StellarStatus::NoError);
            assert!(Instant::now() < deadline, "event가 오지 않았습니다");
            std::thread::yield_now();
        }
    }

    pub(crate) fn create() -> u64 {
        let mut h = 0u64;
        assert_eq!(unsafe { stellar_runtime_create(&mut h) }, StellarStatus::Ok);
        h
    }

    pub(crate) fn open_project(h: u64, root: &Path) -> u64 {
        let bytes = root.to_str().unwrap().as_bytes();
        let mut request = 0u64;
        assert_eq!(
            unsafe { stellar_project_open(h, bytes.as_ptr(), bytes.len(), &mut request) },
            StellarStatus::Ok
        );
        match next_event(h) {
            RuntimeEvent::ProjectOpened { project, .. } => project.0,
            other => panic!("프로젝트 열기 실패: {other:?}"),
        }
    }

    pub(crate) fn open_window(h: u64, project: u64) -> u64 {
        let mut request = 0u64;
        assert_eq!(
            unsafe { stellar_window_session_open(h, project, &mut request) },
            StellarStatus::Ok
        );
        match next_event(h) {
            RuntimeEvent::WindowSessionOpened {
                request: got,
                session,
                ..
            } => {
                assert_eq!(got, RequestId(request));
                session.0
            }
            other => panic!("창 session 열기 실패: {other:?}"),
        }
    }

    pub(crate) fn info(h: u64, session: u64) -> Result<StellarWindowSessionInfo, StellarStatus> {
        let mut info = StellarWindowSessionInfo::default();
        match unsafe { stellar_window_session_info(h, session, &mut info) } {
            StellarStatus::Ok => Ok(info),
            status => Err(status),
        }
    }

    fn state(h: u64) -> StellarRuntimeState {
        let mut state = StellarRuntimeState::default();
        assert_eq!(
            unsafe { stellar_runtime_state(h, &mut state) },
            StellarStatus::Ok
        );
        state
    }

    #[test]
    fn frame_property_and_selected_node_reject_invalid_input_before_submitting() {
        let id = [0u8; 16];
        let mut node = StellarSelectedNode::default();
        assert_eq!(
            unsafe { stellar_window_selected_node(0, 0, std::ptr::null_mut()) },
            StellarStatus::NullPointer
        );
        assert_eq!(
            unsafe { stellar_window_selected_node(0, 0, &mut node) },
            StellarStatus::InvalidHandle
        );
        let length = |length| StellarFrameValue {
            length,
            ..StellarFrameValue::default()
        };
        for (view, key, value, expected) in [
            (
                std::ptr::null(),
                0x0108,
                length(1.0),
                StellarStatus::NullPointer,
            ),
            (
                &id as *const _,
                0x0101,
                length(1.0),
                StellarStatus::InvalidArgument,
            ),
            (
                &id as *const _,
                0x0108,
                length(-1.0),
                StellarStatus::InvalidArgument,
            ),
            (
                &id as *const _,
                0x010a,
                length(f32::NAN),
                StellarStatus::InvalidArgument,
            ),
            (
                &id as *const _,
                0x0108,
                length(1.0),
                StellarStatus::InvalidHandle,
            ),
        ] {
            let mut request = 99;
            assert_eq!(
                unsafe {
                    stellar_window_session_set_frame_property(
                        0,
                        0,
                        view,
                        &id,
                        0,
                        0,
                        key,
                        value,
                        &mut request,
                    )
                },
                expected
            );
            assert_eq!(request, 0);
        }
    }

    #[test]
    fn copy_selection_rejects_invalid_pointers_and_handle() {
        let target = StellarSelectionTarget::default();
        let mut len = 99;
        assert_eq!(
            unsafe {
                stellar_window_copy_selection(
                    0,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    0,
                    &mut len,
                )
            },
            StellarStatus::NullPointer
        );
        assert_eq!(
            unsafe {
                stellar_window_copy_selection(
                    0,
                    &target,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                )
            },
            StellarStatus::NullPointer
        );
        assert_eq!(
            unsafe { stellar_window_copy_selection(0, &target, std::ptr::null_mut(), 0, &mut len) },
            StellarStatus::InvalidHandle
        );
        assert_eq!(len, 0);
    }

    #[test]
    fn info_layout_is_fixed() {
        assert_eq!(std::mem::size_of::<StellarWindowSessionInfo>(), 88);
        assert_eq!(std::mem::align_of::<StellarWindowSessionInfo>(), 8);
        assert_eq!(
            std::mem::offset_of!(StellarWindowSessionInfo, has_unsaved_changes),
            52
        );
        assert_eq!(
            std::mem::offset_of!(StellarWindowSessionInfo, window_id),
            56
        );
        assert_eq!(std::mem::size_of::<StellarSelectionTarget>(), 48);
        assert_eq!(std::mem::offset_of!(StellarSelectionTarget, view), 32);
        assert_eq!(std::mem::size_of::<StellarSelectedNode>(), 104);
        assert_eq!(std::mem::align_of::<StellarSelectedNode>(), 8);
        assert_eq!(
            std::mem::offset_of!(StellarSelectedNode, selection_count),
            16
        );
        assert_eq!(std::mem::offset_of!(StellarSelectedNode, view), 24);
        assert_eq!(std::mem::offset_of!(StellarSelectedNode, node), 40);
        assert_eq!(std::mem::offset_of!(StellarSelectedNode, kind), 56);
        assert_eq!(std::mem::offset_of!(StellarSelectedNode, flags), 59);
        assert_eq!(std::mem::offset_of!(StellarSelectedNode, width), 60);
        assert_eq!(std::mem::offset_of!(StellarSelectedNode, radii), 68);
        assert_eq!(std::mem::offset_of!(StellarSelectedNode, background), 84);
        assert_eq!(std::mem::size_of::<StellarFrameValue>(), 24);
        assert_eq!(std::mem::size_of::<StellarShapeStyle>(), 44);
        assert_eq!(std::mem::offset_of!(StellarShapeStyle, stroke), 20);
        assert_eq!(std::mem::offset_of!(StellarShapeStyle, stroke_width), 40);
        assert_eq!(std::mem::offset_of!(StellarFrameValue, color), 4);
        assert_eq!(WINDOW_SESSION_INFO_VERSION, 2);
        assert_eq!(std::mem::size_of::<StellarWindowViewInfo>(), 56);
        assert_eq!(std::mem::offset_of!(StellarWindowViewInfo, center_x), 16);
        assert_eq!(std::mem::offset_of!(StellarWindowViewInfo, active_view), 40);
    }

    fn view_info(h: u64, session: u64) -> StellarWindowViewInfo {
        let mut info = StellarWindowViewInfo::default();
        assert_eq!(
            unsafe { stellar_window_view_info(h, session, &mut info) },
            StellarStatus::Ok
        );
        info
    }

    #[test]
    fn zoom_commands_update_the_view_info_of_their_window() {
        // XII.4.3(C ABI): 줌 명령은 그 창 viewport만 바꾸고 view 정보로 읽습니다.
        let first = FixtureProject::copy("zoom-a");
        let second = FixtureProject::copy("zoom-b");
        let h = create();
        let a = open_window(h, open_project(h, &first.path));
        let b = open_window(h, open_project(h, &second.path));
        let initial = view_info(h, a);
        assert_eq!(initial.version, WINDOW_VIEW_INFO_VERSION);
        assert_eq!(
            (initial.has_view, initial.tab_count, initial.zoom),
            (1, 1, 1.0)
        );
        assert_eq!(
            initial.active_view,
            StableId::new_v7(3, 0, 3).to_bytes(),
            "첫 View"
        );
        for command in [AppCommand::ZoomIn, AppCommand::ZoomIn, AppCommand::ZoomOut] {
            assert!(matches!(
                perform(h, a, command.code()),
                RuntimeEvent::WindowViewChanged { session, .. } if session.0 == a
            ));
        }
        assert_eq!(view_info(h, a).zoom, 2.0);
        assert_eq!(view_info(h, b).zoom, 1.0, "B viewport 그대로");
        assert!(matches!(
            perform(h, a, AppCommand::ZoomToActualSize.code()),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        assert_eq!(view_info(h, a).zoom, 1.0);
        let mut info = StellarWindowViewInfo::default();
        assert_eq!(
            unsafe { stellar_window_view_info(h, 999, &mut info) },
            StellarStatus::InvalidArgument
        );
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
    }

    #[test]
    fn pan_moves_only_its_window_and_rejects_invalid_deltas() {
        let fixture = FixtureProject::copy("pan");
        let h = create();
        let project = open_project(h, &fixture.path);
        let a = open_window(h, project);
        let b = open_window(h, project);
        perform(h, a, AppCommand::ZoomIn.code());
        let before = info(h, a).unwrap();
        let other = view_info(h, b);
        let mut request = 0;
        assert_eq!(
            unsafe { stellar_window_session_pan(h, a, 12.0, -8.0, &mut request) },
            StellarStatus::Ok
        );
        assert!(
            matches!(next_event(h), RuntimeEvent::WindowViewChanged { request: id, session, .. }
            if id.0 == request && session.0 == a)
        );
        let moved = view_info(h, a);
        assert_eq!(
            (moved.center_x, moved.center_y, moved.zoom),
            (-6.0, 4.0, 2.0)
        );
        assert_eq!(view_info(h, b), other);
        assert_eq!(info(h, a).unwrap(), before);
        for delta in [f64::NAN, f64::INFINITY] {
            assert_eq!(
                unsafe { stellar_window_session_pan(h, a, delta, 0.0, &mut request) },
                StellarStatus::Ok
            );
            assert!(matches!(
                next_event(h),
                RuntimeEvent::RequestFailed {
                    failure: RequestFailure::InvalidViewport,
                    ..
                }
            ));
            assert_eq!(view_info(h, a), moved);
        }
        assert_eq!(
            unsafe { stellar_window_session_pan(h, 999, 1.0, 0.0, &mut request) },
            StellarStatus::Ok
        );
        assert!(matches!(next_event(h), RuntimeEvent::RequestFailed { .. }));
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
        assert_eq!(
            unsafe { stellar_window_session_pan(h, a, 1.0, 0.0, &mut request) },
            StellarStatus::InvalidHandle
        );
    }

    #[test]
    fn two_window_sessions_open_and_close_through_the_c_abi() {
        let first = FixtureProject::copy("a");
        let second = FixtureProject::copy("b");
        let h = create();
        let project_a = open_project(h, &first.path);
        let project_b = open_project(h, &second.path);
        let a = open_window(h, project_a);
        let b = open_window(h, project_b);
        assert_ne!(a, b);
        assert_eq!(state(h).window_session_count, 2);

        let info_a = info(h, a).unwrap();
        let info_b = info(h, b).unwrap();
        assert_eq!(info_a.version, WINDOW_SESSION_INFO_VERSION);
        assert_eq!((info_a.project, info_b.project), (project_a, project_b));
        assert_eq!((info_a.has_document, info_a.generation), (1, 1));
        assert_eq!(info_a.document_id, info_b.document_id, "같은 fixture 문서");
        assert_ne!(info_a.window_id, info_b.window_id, "창마다 다른 id");
        assert_eq!((info_a.undo_depth, info_a.redo_depth), (0, 0));

        let mut request = 0u64;
        assert_eq!(
            unsafe { stellar_window_session_close(h, a, &mut request) },
            StellarStatus::Ok
        );
        assert_eq!(
            next_event(h),
            RuntimeEvent::WindowSessionClosed {
                request: RequestId(request),
                session: WindowSessionHandle(a),
            }
        );
        assert_eq!(info(h, a), Err(StellarStatus::InvalidArgument));
        assert!(info(h, b).is_ok());
        assert_eq!(state(h).window_session_count, 1);

        // 프로젝트를 닫으면 남은 창 session도 닫힙니다.
        assert_eq!(
            unsafe { stellar_project_close(h, project_b, &mut request) },
            StellarStatus::Ok
        );
        assert!(matches!(next_event(h), RuntimeEvent::ProjectClosed { .. }));
        assert_eq!(info(h, b), Err(StellarStatus::InvalidArgument));
        assert_eq!(state(h).window_session_count, 0);

        // 닫은 창 session을 다시 닫으면 실패 event입니다.
        assert_eq!(
            unsafe { stellar_window_session_close(h, a, &mut request) },
            StellarStatus::Ok
        );
        let RuntimeEvent::RequestFailed { failure, .. } = next_event(h) else {
            panic!("실패 event가 아님");
        };
        assert_eq!(failure, RequestFailure::UnknownWindowSession);
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
    }

    fn perform(h: u64, session: u64, command: u32) -> RuntimeEvent {
        let mut request = 0u64;
        assert_eq!(
            unsafe { stellar_window_session_perform(h, session, command, &mut request) },
            StellarStatus::Ok
        );
        let event = next_event(h);
        assert_eq!(event.request(), RequestId(request));
        event
    }

    #[test]
    fn undo_redo_and_save_now_go_to_the_given_window_session() {
        // 검증 기준(XII.6.4, C ABI): 창별 history와 save 호출.
        let first = FixtureProject::copy("cmd-a");
        let second = FixtureProject::copy("cmd-b");
        let h = create();
        let a = open_window(h, open_project(h, &first.path));
        let b = open_window(h, open_project(h, &second.path));
        // 선택할 node: fixture 문서의 첫 View root(10)의 자식 11.
        let child = StableId::new_v7(11, 0, 11).to_bytes();
        let mut request = 0u64;
        assert_eq!(
            unsafe { stellar_window_session_select(h, a, child.as_ptr(), 1, &mut request) },
            StellarStatus::Ok
        );
        assert!(matches!(
            next_event(h),
            RuntimeEvent::WindowViewChanged { session, .. } if session.0 == a
        ));
        assert_eq!(
            (
                info(h, a).unwrap().undo_depth,
                info(h, a).unwrap().selection_count
            ),
            (1, 1)
        );
        assert_eq!(info(h, b).unwrap().undo_depth, 0);

        let undo = perform(h, a, AppCommand::Undo.code());
        assert!(matches!(
            undo,
            RuntimeEvent::HistoryReplayed {
                redo: false,
                applied: true,
                ..
            }
        ));
        let after = info(h, a).unwrap();
        assert_eq!((after.selection_count, after.redo_depth), (0, 1));
        assert!(matches!(
            perform(h, b, AppCommand::Undo.code()),
            RuntimeEvent::HistoryReplayed { applied: false, .. }
        ));
        assert!(matches!(
            perform(h, a, AppCommand::Redo.code()),
            RuntimeEvent::HistoryReplayed {
                redo: true,
                applied: true,
                ..
            }
        ));
        assert_eq!(info(h, a).unwrap().selection_count, 1);
        // Save Now: 선택만 바뀌어 저장할 문서 변경이 없습니다(generation 1 그대로).
        assert!(matches!(
            perform(h, a, AppCommand::SaveNow.code()),
            RuntimeEvent::WindowSessionSaved { session, generation: 1, written: false, .. }
                if session.0 == a
        ));

        // 모르는 명령 코드는 요청을 만들지 않습니다.
        let mut request = 9u64;
        assert_eq!(
            unsafe { stellar_window_session_perform(h, a, 999, &mut request) },
            StellarStatus::InvalidArgument
        );
        assert_eq!(request, 0);
        assert_eq!(
            unsafe { stellar_window_session_select(h, a, std::ptr::null(), 1, &mut request) },
            StellarStatus::NullPointer
        );
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
    }

    #[test]
    fn paste_selection_rejects_invalid_buffers_before_reading() {
        let target = StellarSelectionTarget::default();
        let byte = [0u8];
        for (target, bytes, len, expected) in [
            (
                &target as *const _,
                byte.as_ptr(),
                0,
                StellarStatus::InvalidArgument,
            ),
            (
                &target as *const _,
                byte.as_ptr(),
                16 * 1024 * 1024 + 1,
                StellarStatus::InvalidArgument,
            ),
            (
                std::ptr::null(),
                byte.as_ptr(),
                1,
                StellarStatus::NullPointer,
            ),
            (
                &target as *const _,
                std::ptr::null(),
                1,
                StellarStatus::NullPointer,
            ),
            (
                &target as *const _,
                byte.as_ptr(),
                1,
                StellarStatus::InvalidHandle,
            ),
        ] {
            let mut request = 99;
            assert_eq!(
                unsafe { stellar_window_paste_selection(0, target, bytes, len, &mut request) },
                expected
            );
            assert_eq!(request, 0);
        }
    }

    #[test]
    fn paste_selection_ffi_rejects_invalid_bytes_without_changing_history() {
        let fixture = FixtureProject::copy("paste-invalid-bytes");
        let h = create();
        let session = open_window(h, open_project(h, &fixture.path));
        let before = info(h, session).unwrap();
        let mut target = StellarSelectionTarget::default();
        assert_eq!(
            unsafe { stellar_window_selection_target(h, session, &mut target) },
            StellarStatus::Ok
        );
        let mut copied_len = 99;
        assert_eq!(
            unsafe {
                stellar_window_copy_selection(h, &target, std::ptr::null_mut(), 0, &mut copied_len)
            },
            StellarStatus::NoError
        );
        assert_eq!(copied_len, 0);
        let mut bytes = vec![0u8];
        let mut request = 0;
        assert_eq!(
            unsafe {
                stellar_window_paste_selection(
                    h,
                    &target,
                    bytes.as_ptr(),
                    bytes.len(),
                    &mut request,
                )
            },
            StellarStatus::Ok
        );
        assert_ne!(request, 0);
        bytes.clear();
        drop(bytes);
        assert!(matches!(next_event(h), RuntimeEvent::RequestFailed {
            request: got, failure: RequestFailure::UnknownView, ..
        } if got == RequestId(request)));
        let after = info(h, session).unwrap();
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.undo_depth, before.undo_depth);
        assert_eq!(after.selection_count, before.selection_count);
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
    }

    fn code_of(name: &str) -> (StellarStatus, u32) {
        let mut code = 77u32;
        let status = unsafe { stellar_command_code(name.as_ptr(), name.len(), &mut code) };
        (status, code)
    }

    #[test]
    fn command_codes_are_looked_up_by_shared_name() {
        // 검증 기준(XII.6.5, C ABI): command ID는 코어가 공유 이름으로 알려 줍니다.
        for command in AppCommand::ALL {
            assert_eq!(code_of(command.name()), (StellarStatus::Ok, command.code()));
        }
        for name in ["app.quit", "edit.copy", "view.toggle_sidebar", ""] {
            assert_eq!(code_of(name), (StellarStatus::NoError, 0), "{name}");
        }
        let bad = [0xFFu8, 0xFE];
        let mut code = 5u32;
        assert_eq!(
            unsafe { stellar_command_code(bad.as_ptr(), bad.len(), &mut code) },
            StellarStatus::InvalidUtf8
        );
        assert_eq!(code, 0);
        assert_eq!(
            unsafe { stellar_command_code(bad.as_ptr(), bad.len(), std::ptr::null_mut()) },
            StellarStatus::NullPointer
        );
    }

    #[test]
    fn null_info_pointer_and_unknown_handle_are_rejected() {
        let h = create();
        assert_eq!(
            unsafe { stellar_window_session_info(h, 1, std::ptr::null_mut()) },
            StellarStatus::NullPointer
        );
        assert_eq!(info(h, 1), Err(StellarStatus::InvalidArgument));
        let mut request = 7u64;
        assert_eq!(
            unsafe { stellar_window_session_open(0, 1, &mut request) },
            StellarStatus::InvalidHandle
        );
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
    }
}

//! 컴포넌트 라이브러리 조회와 원본 편집을 위한 네이티브 셸 API.
use crate::{ffi_guard, registry, StellarStatus};
use stellar_runtime::app::WindowSessionHandle;

/// 게시된 창 문서의 단일 인스턴스가 참조하는 정의 ID를 반환합니다. 파일은 읽지 않습니다.
/// # Safety
/// out은 16바이트 쓰기가 가능해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_selected_definition(
    handle: u64,
    session: u64,
    out: *mut [u8; 16],
) -> StellarStatus {
    ffi_guard(move || {
        if out.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { out.write([0; 16]) };
        let Ok(reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(app) = reg.live.get(&handle).and_then(|rt| rt.app.as_ref()) else {
            return StellarStatus::InvalidHandle;
        };
        let state = app.state();
        let Some(window) = state.window(WindowSessionHandle(session)) else {
            return StellarStatus::InvalidHandle;
        };
        let Some(id) = window.selected_definition else {
            return StellarStatus::NoError;
        };
        unsafe { out.write(id.to_bytes()) };
        StellarStatus::Ok
    })
}

/// 비차단 macOS 변경 신호 fd를 생성합니다. 호출자는 worker에서 기다리고 fd를 닫습니다.
#[cfg(target_os = "macos")]
#[no_mangle]
pub extern "C" fn stellar_window_component_watch_fd(handle: u64, session: u64) -> i32 {
    use std::os::fd::IntoRawFd;
    std::panic::catch_unwind(|| {
        let start = {
            let reg = registry().lock().ok()?;
            reg.live
                .get(&handle)?
                .app
                .as_ref()?
                .component_file_watch(WindowSessionHandle(session))
        };
        Some(start()?.into_raw_fd())
    })
    .ok()
    .flatten()
    .unwrap_or(-1)
}

/// 선택된 정의를 표시한 View에 배치합니다. 완료는 WINDOW_VIEW_CHANGED 또는 REQUEST_FAILED입니다.
/// # Safety
/// view와 component는 각각 16바이트 UUID이고 hash는 71바이트 UTF-8, out_request는 쓸 수 있는 u64입니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_place_component(
    handle: u64,
    session: u64,
    view: *const [u8; 16],
    revision: u64,
    component: *const [u8; 16],
    hash: *const u8,
    hash_len: usize,
    x: f32,
    y: f32,
    out_request: *mut u64,
) -> StellarStatus {
    use stellar_core_types::{ids::StableId, revision::DocumentRevision, value::Vec2};
    use stellar_runtime::app::InstancePlacement;
    ffi_guard(move || {
        if out_request.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe {
            *out_request = 0;
        }
        if view.is_null() || component.is_null() || hash.is_null() {
            return StellarStatus::NullPointer;
        }
        if hash_len != 71 {
            return StellarStatus::InvalidArgument;
        }
        let Ok(position) = Vec2::new(x, y) else {
            return StellarStatus::InvalidArgument;
        };
        let hash = unsafe { std::slice::from_raw_parts(hash, hash_len) };
        let Some(source_hash) = std::str::from_utf8(hash).ok().and_then(|s| s.parse().ok()) else {
            return StellarStatus::InvalidArgument;
        };
        let input = InstancePlacement {
            component: StableId::from_bytes(unsafe { *component }),
            source_hash,
            view: StableId::from_bytes(unsafe { *view }),
            revision: DocumentRevision(revision),
            position,
        };
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.place_component_instance(WindowSessionHandle(session), input),
            Err(status) => return status,
        };
        unsafe { rt.finish_request(result, out_request) }
    })
}

/// worker에서 원본 라이브러리를 읽고 현재 창의 캔버스 갱신을 요청합니다.
/// # Safety
/// out_request는 null 또는 u64 쓰기가 가능해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_reload_component_canvas(
    handle: u64,
    session: u64,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out_request.is_null() {
            unsafe { out_request.write(0) };
        }
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let Some(app) = rt.app.as_ref() else {
            return StellarStatus::InvalidHandle;
        };
        let result = app.reload_canvas_components(WindowSessionHandle(session));
        unsafe { rt.finish_request(result, out_request) }
    })
}

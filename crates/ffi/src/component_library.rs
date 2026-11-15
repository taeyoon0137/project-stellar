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

/// worker 전용. 성공하거나 편집을 거부한 이유를 출력합니다. 버퍼 크기 조사로 쓰기를 재실행하지 않습니다.
/// # Safety
/// id는 읽을 수 있는 16바이트, hash는 hash_len byte UTF-8, error는 적어도 EVENT_TEXT_MAX개의
/// 바이트를 쓸 수 있어야 하고 out_len은 쓸 수 있는 size_t여야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_component_resize(
    handle: u64,
    session: u64,
    id: *const u8,
    hash: *const u8,
    hash_len: usize,
    width: f32,
    height: f32,
    error: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> StellarStatus {
    unsafe {
        edit_component_root(
            handle,
            session,
            id,
            hash,
            hash_len,
            (error, capacity, out_len),
            |p, id, hash| p.resize_component_root(id, hash, width, height),
        )
    }
}

/// 프로젝트 root Frame의 저장 채우기를 기존 hash와 비교한 뒤 교체합니다.
/// # Safety
/// 입력·출력 버퍼 조건은 stellar_window_component_resize와 같습니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_component_set_fill(
    handle: u64,
    session: u64,
    id: *const u8,
    hash: *const u8,
    hash_len: usize,
    fill: crate::canvas::StellarCanvasColor,
    error: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> StellarStatus {
    unsafe {
        edit_component_root(
            handle,
            session,
            id,
            hash,
            hash_len,
            (error, capacity, out_len),
            |p, id, hash| {
                let color = fill
                    .decode()
                    .map_err(|_| "잘못된 색공간 또는 RGBA".to_owned())?;
                p.set_component_root_background(id, hash, color)
            },
        )
    }
}

unsafe fn edit_component_root(
    handle: u64,
    session: u64,
    id: *const u8,
    hash: *const u8,
    hash_len: usize,
    (error, capacity, out_len): (*mut u8, usize, *mut usize),
    edit: impl FnOnce(
        &stellar_runtime::app::ProjectSummary,
        stellar_core_types::ids::StableId,
        &str,
    ) -> Result<(), String>,
) -> StellarStatus {
    ffi_guard(move || {
        if out_len.is_null() || id.is_null() || hash.is_null() || error.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe {
            *out_len = 0;
        }
        if capacity < stellar_runtime::app::EVENT_TEXT_MAX {
            return StellarStatus::BufferTooSmall;
        }
        if hash_len != 71 {
            return StellarStatus::InvalidArgument;
        }
        let Ok(hash) = (unsafe { std::str::from_utf8(std::slice::from_raw_parts(hash, hash_len)) })
        else {
            return StellarStatus::InvalidArgument;
        };
        let id =
            stellar_core_types::ids::StableId::from_bytes(unsafe { id.cast::<[u8; 16]>().read() });
        let state = {
            let Ok(reg) = registry().lock() else {
                return StellarStatus::Internal;
            };
            let Some(app) = reg.live.get(&handle).and_then(|rt| rt.app.as_ref()) else {
                return StellarStatus::InvalidHandle;
            };
            app.state()
        };
        let Some(project) = state
            .window(WindowSessionHandle(session))
            .and_then(|w| state.project(w.project))
        else {
            return StellarStatus::InvalidHandle;
        };
        if let Err(message) = edit(project, id, hash) {
            let mut len = message.len().min(stellar_runtime::app::EVENT_TEXT_MAX);
            while !message.is_char_boundary(len) {
                len -= 1;
            }
            unsafe {
                std::ptr::copy_nonoverlapping(message.as_ptr(), error, len);
                *out_len = len;
            }
            return StellarStatus::InvalidArgument;
        }
        StellarStatus::Ok
    })
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

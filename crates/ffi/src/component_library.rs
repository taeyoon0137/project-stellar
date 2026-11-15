//! 컴포넌트 원본 감시와 캔버스 재읽기를 위한 네이티브 셸 API.
use crate::{ffi_guard, registry, StellarStatus};
use stellar_runtime::app::WindowSessionHandle;

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

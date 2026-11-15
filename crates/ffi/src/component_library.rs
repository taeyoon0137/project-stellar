//! 컴포넌트 원본 캔버스 재읽기를 위한 네이티브 셸 API(ADR-0028).
//!
//! 읽기와 감시는 신뢰와 무관합니다(신뢰하지 않은 프로젝트는 프로젝트 밖 target 내용만 읽지 않음, ADR-0061 결정 8).
//! 원본 감시는 코어가 프로젝트 session마다 하나 두며 셸에 fd를 넘기지 않습니다. 감시가 예약한 재읽기
//! 결과는 요청 없는 `WINDOW_VIEW_CHANGED`와 `stellar_runtime_event_ready_fd` 신호로 돌아옵니다.
use crate::{ffi_guard, registry, StellarStatus};
use stellar_runtime::app::WindowSessionHandle;

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

#[cfg(all(test, unix))]
mod tests {
    use std::os::fd::{FromRawFd, OwnedFd};

    use crate::window_session::tests::create;
    use crate::{stellar_runtime_destroy, stellar_runtime_event_ready_fd, StellarStatus};

    #[test]
    fn event_ready_fd_belongs_to_the_caller_and_unknown_handles_fail() {
        let h = create();
        let first = stellar_runtime_event_ready_fd(h);
        let second = stellar_runtime_event_ready_fd(h);
        assert!(first >= 0 && second >= 0 && first != second);
        // 호출자가 소유하므로 닫아도 다음 호출이 새 fd를 받습니다.
        drop(unsafe { OwnedFd::from_raw_fd(first) });
        drop(unsafe { OwnedFd::from_raw_fd(second) });
        assert_eq!(stellar_runtime_event_ready_fd(999_999), -1);
        assert_eq!(stellar_runtime_destroy(h), StellarStatus::Ok);
        let header = include_str!("../include/stellar_ffi.h");
        assert!(header.contains("int32_t stellar_runtime_event_ready_fd(uint64_t handle);"));
        assert!(!header.contains("watch_fd"), "창별 원본 감시 fd는 삭제됨");
    }
}

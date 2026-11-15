//! 자동 저장 flush C ABI (UX.2.11, ADR-0072).
//!
//! 자동 저장 시점(debounce·최대 지연)은 코어 runtime이 정하고 셸은 앱 비활성화만 알립니다. 창 닫기 flush는 기존
//! Save Now(`stellar_window_session_perform`)가 맡습니다. 자동 저장 상태는 `StellarWindowSessionInfo::autosave`
//! (v3)로 읽습니다.

use crate::{ffi_guard, registry, StellarStatus};
use stellar_runtime::app::RequestError;

/// 앱이 비활성화될 때 저장되지 않은 완료 편집을 debounce 없이 저장하도록 접수합니다. 기다리지 않고 완료 event도
/// 없습니다. 앱 runtime을 아직 시작하지 않았으면 저장할 것이 없으므로 `Ok`입니다. 대기열이 차면
/// `QueueFull`이며, 그 편집은 다음 자동 저장이 씁니다.
#[no_mangle]
pub extern "C" fn stellar_runtime_flush_autosave(handle: u64) -> StellarStatus {
    ffi_guard(move || {
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let Some(app) = rt.app.as_ref() else {
            return StellarStatus::Ok;
        };
        match app.flush_autosave() {
            Ok(()) => StellarStatus::Ok,
            Err(error) => {
                let status = match error {
                    RequestError::QueueFull => StellarStatus::QueueFull,
                    _ => StellarStatus::Internal,
                };
                rt.last_error = Some((status, error.to_string()));
                status
            }
        }
    })
}

/// Debug 검사 전용: 이 handle의 앱 runtime을 자동 저장 없이 시작합니다. 검사는 dirty·stale 상태를 시간에 걸쳐
/// 단정하므로, debounce에 따라 저장 시점이 바뀌지 않게 합니다(ADR-0072). 앱 runtime이 이미 시작됐으면
/// `InvalidArgument`입니다. Release에는 이 심벌이 없습니다.
#[cfg(debug_assertions)]
#[no_mangle]
pub extern "C" fn stellar_runtime_check_disable_autosave(handle: u64) -> StellarStatus {
    ffi_guard(move || {
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        if rt.app.is_some() {
            rt.last_error = Some((
                StellarStatus::InvalidArgument,
                "앱 runtime이 이미 시작됐습니다".to_string(),
            ));
            return StellarStatus::InvalidArgument;
        }
        rt.autosave = false;
        StellarStatus::Ok
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{stellar_runtime_create, stellar_runtime_destroy};

    #[test]
    fn flush_needs_a_live_handle_and_is_a_no_op_before_the_app_runtime_starts() {
        assert_eq!(
            stellar_runtime_flush_autosave(u64::MAX),
            StellarStatus::InvalidHandle
        );
        let mut handle = 0;
        assert_eq!(
            unsafe { stellar_runtime_create(&mut handle) },
            StellarStatus::Ok
        );
        assert_eq!(stellar_runtime_flush_autosave(handle), StellarStatus::Ok);
        assert_eq!(stellar_runtime_destroy(handle), StellarStatus::Ok);
    }

    #[test]
    fn check_switch_applies_only_before_the_app_runtime_starts() {
        let mut handle = 0;
        assert_eq!(
            unsafe { stellar_runtime_create(&mut handle) },
            StellarStatus::Ok
        );
        assert_eq!(
            stellar_runtime_check_disable_autosave(handle),
            StellarStatus::Ok
        );
        {
            let mut reg = registry().lock().unwrap();
            let rt = reg.live.get_mut(&handle).unwrap();
            assert!(!rt.autosave);
            rt.app().unwrap();
        }
        assert_eq!(
            stellar_runtime_check_disable_autosave(handle),
            StellarStatus::InvalidArgument
        );
        assert_eq!(stellar_runtime_flush_autosave(handle), StellarStatus::Ok);
        assert_eq!(stellar_runtime_destroy(handle), StellarStatus::Ok);
    }
}

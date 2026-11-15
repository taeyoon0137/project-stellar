//! 자동 저장 flush C ABI (UX.2.11, ADR-0072).
//!
//! 자동 저장 시점(debounce·최대 지연)은 코어 runtime이 정하고 셸은 앱 비활성화만 알립니다. 창 닫기 flush는 기존
//! Save Now(`stellar_window_session_perform`)가 맡습니다. 자동 저장 상태는 `StellarWindowSessionInfo::autosave`
//! (v3)로 읽습니다.
//!
//! 셸의 Debug 검사 실행(`--…-check`)은 dirty·stale 상태를 시간에 걸쳐 단정하므로 앱 runtime을 자동 저장 없이
//! 시작합니다. 이 전환은 C ABI가 아니라 검사 전용 환경 변수 [`CHECK_DISABLE_AUTOSAVE_ENV`]이고 Debug 빌드만 읽습니다.
//! 공개 헤더는 구성마다 같습니다(ADR-0028 "Debug 전용 창 편집 검사 ABI 삭제", ADR-0072 10절).

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

/// 셸 Debug 검사 실행이 앱 runtime을 만들기 전에 `1`로 두는 환경 변수입니다.
pub(crate) const CHECK_DISABLE_AUTOSAVE_ENV: &str = "STELLAR_CHECK_DISABLE_AUTOSAVE";

/// 앱 runtime을 자동 저장 없이 시작할지. Release 빌드는 환경 변수를 읽지 않고 늘 자동 저장을 켭니다.
pub(crate) fn disabled_for_native_checks() -> bool {
    cfg!(debug_assertions)
        && std::env::var_os(CHECK_DISABLE_AUTOSAVE_ENV).is_some_and(|value| value == "1")
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
}

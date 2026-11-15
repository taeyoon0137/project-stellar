//! 앱 데이터의 미저장 체크포인트 제안과 사용자 결정(아키텍처 §22.3, ADR-0028).
use crate::{ffi_guard, registry, StellarStatus};
use stellar_core_types::ids::StableId;
use stellar_runtime::app::{
    recovery::{RecoveryAction, RecoveryCandidate},
    ProjectSummary, WindowSessionHandle,
};

/// [`StellarRecoveryCandidate`] 레이아웃 버전.
pub const RECOVERY_CANDIDATE_VERSION: u32 = 1;
/// 복구본을 이 창 session 메모리에 복원합니다.
pub const RECOVERY_RESTORE: u32 = 1;
/// 복구본을 지웁니다.
pub const RECOVERY_DISCARD: u32 = 2;

/// 복구 제안 하나(고정 56 byte, 코어가 씀). `has_candidate`가 0이면 나머지는 0입니다.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StellarRecoveryCandidate {
    /// [`RECOVERY_CANDIDATE_VERSION`].
    pub version: u32,
    /// 제안할 복구본이 있으면 1.
    pub has_candidate: u32,
    /// 복구본을 남긴 창 편집 session id(UUID 바이트 순서).
    pub window_id: [u8; 16],
    /// 복구본 파일 bytes의 SHA-256 digest. 적용할 때 같은 bytes인지 다시 확인합니다.
    pub content_hash: [u8; 32],
}

fn project(handle: u64, session: u64) -> Result<ProjectSummary, StellarStatus> {
    let reg = registry().lock().map_err(|_| StellarStatus::Internal)?;
    let state = reg
        .live
        .get(&handle)
        .and_then(|rt| rt.app.as_ref())
        .ok_or(StellarStatus::InvalidHandle)?
        .state();
    state
        .window(WindowSessionHandle(session))
        .and_then(|w| state.project(w.project))
        .cloned()
        .ok_or(StellarStatus::InvalidArgument)
}

fn fail(handle: u64, status: StellarStatus, message: String) -> StellarStatus {
    if let Ok(mut reg) = registry().lock() {
        if let Some(rt) = reg.live.get_mut(&handle) {
            rt.last_error = Some((status, message));
        }
    }
    status
}

/// 이 창 session의 프로젝트에 제안할 복구본을 찾습니다. 파일을 읽으므로 UI thread 밖에서 부릅니다.
///
/// 없으면 `has_candidate = 0`과 `OK`입니다. 복구 저장소를 읽지 못하면 `INTERNAL`과 last error입니다.
///
/// # Safety
/// `out_candidate`는 [`StellarRecoveryCandidate`] 쓰기가 유효한 포인터여야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_recovery_probe(
    handle: u64,
    session: u64,
    out_candidate: *mut StellarRecoveryCandidate,
) -> StellarStatus {
    ffi_guard(|| {
        if out_candidate.is_null() {
            return StellarStatus::NullPointer;
        }
        let mut out = StellarRecoveryCandidate {
            version: RECOVERY_CANDIDATE_VERSION,
            ..Default::default()
        };
        let status = match project(handle, session).map(|p| p.recovery_candidate()) {
            Err(status) => status,
            Ok(Err(message)) => fail(handle, StellarStatus::Internal, message),
            Ok(Ok(None)) => StellarStatus::Ok,
            Ok(Ok(Some(candidate))) => {
                out.has_candidate = 1;
                out.window_id = candidate.window.stable_id().to_bytes();
                out.content_hash = *candidate.hash.digest();
                StellarStatus::Ok
            }
        };
        unsafe { *out_candidate = out };
        status
    })
}

/// probe가 돌려준 복구본에 사용자의 결정을 적용합니다. 표시한 것과 같은 bytes일 때만 적용합니다.
///
/// `action`은 [`RECOVERY_RESTORE`] 또는 [`RECOVERY_DISCARD`]입니다. 완료는 `WINDOW_VIEW_CHANGED` 또는
/// `REQUEST_FAILED`입니다. 모르는 action, `has_candidate = 0`, 다른 version은 `INVALID_ARGUMENT`(요청 없음)입니다.
///
/// # Safety
/// `candidate`는 [`StellarRecoveryCandidate`] 읽기, `out_request`는 u64 쓰기가 유효해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_recovery_apply(
    handle: u64,
    session: u64,
    candidate: *const StellarRecoveryCandidate,
    action: u32,
    out_request: *mut u64,
) -> StellarStatus {
    ffi_guard(|| {
        if candidate.is_null() || out_request.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { *out_request = 0 };
        let candidate = unsafe { *candidate };
        let action = match action {
            RECOVERY_RESTORE => RecoveryAction::Restore,
            RECOVERY_DISCARD => RecoveryAction::Discard,
            _ => return StellarStatus::InvalidArgument,
        };
        if candidate.version != RECOVERY_CANDIDATE_VERSION || candidate.has_candidate != 1 {
            return StellarStatus::InvalidArgument;
        }
        let candidate = RecoveryCandidate::from_digest(
            StableId::from_bytes(candidate.window_id),
            &candidate.content_hash,
        );
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let Some(app) = &rt.app else {
            return StellarStatus::InvalidHandle;
        };
        let result = app.recover_checkpoint(WindowSessionHandle(session), candidate, action);
        unsafe { rt.finish_request(result, out_request) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_layout_is_fixed() {
        assert_eq!(std::mem::size_of::<StellarRecoveryCandidate>(), 56);
        assert_eq!(std::mem::align_of::<StellarRecoveryCandidate>(), 4);
        assert_eq!(std::mem::offset_of!(StellarRecoveryCandidate, window_id), 8);
        assert_eq!(
            std::mem::offset_of!(StellarRecoveryCandidate, content_hash),
            24
        );
    }
}

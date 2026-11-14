//! 앱 데이터의 미저장 체크포인트 제안과 사용자 결정.
use crate::{ffi_guard, registry, StellarStatus};
use stellar_runtime::app::{recovery::RecoveryCandidate, ProjectSummary, WindowSessionHandle};

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
unsafe fn output(bytes: &[u8], out: *mut u8, capacity: usize, len: *mut usize) -> StellarStatus {
    if len.is_null() || out.is_null() {
        return StellarStatus::NullPointer;
    }
    unsafe { *len = bytes.len() };
    if bytes.len() > capacity {
        return StellarStatus::BufferTooSmall;
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len()) };
    StellarStatus::Ok
}
/// 반환 UTF-8의 첫 바이트는 0(없음), 1(복원 가능), 2(사본만), 3(오류). 1/2는 창 ID와 hash입니다.
/// # Safety
/// out은 capacity만큼 쓰기 가능하고 out_len은 size_t 출력입니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_recovery_probe(
    handle: u64,
    session: u64,
    out: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> StellarStatus {
    ffi_guard(|| {
        let p = match project(handle, session) {
            Ok(p) => p,
            Err(e) => return e,
        };
        let result = match p.recovery_candidate() {
            Ok(None) => "0".into(),
            Ok(Some(c)) => format!(
                "{}{} {}",
                if c.matches_base { "1" } else { "2" },
                c.window,
                c.hash
            ),
            Err(e) => format!("3{e}"),
        };
        unsafe { output(result.as_bytes(), out, capacity, out_len) }
    })
}
/// ticket이 가리키는 동일 bytes만 복원합니다. path 길이가 0이면 현재 session, 아니면 새 폴더입니다.
/// # Safety
/// ticket/path는 해당 길이만큼 유효하며 request는 쓰기 가능한 출력입니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_recovery_apply(
    handle: u64,
    session: u64,
    ticket: *const u8,
    ticket_len: usize,
    path: *const u8,
    path_len: usize,
    request: *mut u64,
) -> StellarStatus {
    ffi_guard(|| {
        if request.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe {
            *request = 0;
        };
        let ticket = match unsafe { crate::decode_utf8_owned(ticket, ticket_len) } {
            Ok(t) => t,
            Err(e) => return e,
        };
        let Some((id, hash)) = ticket.split_once(' ') else {
            return StellarStatus::InvalidArgument;
        };
        let Some(candidate) = id
            .parse()
            .ok()
            .and_then(|id| RecoveryCandidate::from_ticket(id, hash).ok())
        else {
            return StellarStatus::InvalidArgument;
        };
        let destination = if path_len > 0 {
            let value = match unsafe { crate::decode_utf8_owned(path, path_len) } {
                Ok(v) => v,
                Err(e) => return e,
            };
            Some(std::path::PathBuf::from(value))
        } else {
            None
        };
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let Some(app) = &rt.app else {
            return StellarStatus::InvalidHandle;
        };
        let result = app.recover_checkpoint(WindowSessionHandle(session), candidate, destination);
        unsafe { rt.finish_request(result, request) }
    })
}

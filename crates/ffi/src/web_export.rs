//! 저장된 View·Frame의 읽기 전용 HTML 투영(UX.7.4).

use stellar_runtime::app::{export_saved_web, AppRuntime, WindowSessionHandle};

use crate::{ffi_guard, registry, StellarStatus};

#[repr(C)]
#[derive(Default)]
pub struct StellarWebExportInfo {
    pub html_len: usize,
    pub error_len: usize,
    pub diagnostic_count: usize,
    pub text_len: usize,
    pub has_html: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct StellarWebDiagnostic {
    pub node: [u8; 16],
    pub error: u32,
    pub code_len: u32,
    pub message_len: u32,
}

/// 현재 창의 활성 View 또는 단일 선택 Frame을 저장 generation에서 내보냅니다.
/// 파일 읽기와 코드 생성이 있으므로 호출자는 worker thread를 사용해야 합니다.
/// text는 HTML(있으면), 오류 메시지(있으면), 진단 code·message 순서입니다.
///
/// # Safety
/// `info`는 유효한 쓰기 포인터여야 합니다. `rows`·`text`는 각각 선언한 capacity만큼
/// 쓸 수 있어야 하며 모든 출력 영역은 겹치지 않아야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_export_web(
    handle: u64,
    session: u64,
    info: *mut StellarWebExportInfo,
    rows: *mut StellarWebDiagnostic,
    row_capacity: usize,
    text: *mut u8,
    text_capacity: usize,
) -> StellarStatus {
    ffi_guard(move || {
        if info.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { info.write(StellarWebExportInfo::default()) };
        let input = {
            let Ok(reg) = registry().lock() else {
                return StellarStatus::Internal;
            };
            let Some(rt) = reg.live.get(&handle) else {
                return StellarStatus::InvalidHandle;
            };
            let Some(state) = rt.app.as_ref().map(AppRuntime::state) else {
                return StellarStatus::InvalidArgument;
            };
            let Some(window) = state.window(WindowSessionHandle(session)) else {
                return StellarStatus::InvalidArgument;
            };
            let Some(project) = state.project(window.project) else {
                return StellarStatus::InvalidArgument;
            };
            let (Some(document), Some(view)) = (window.document_id, window.active_view) else {
                return StellarStatus::InvalidArgument;
            };
            let frame = if window.selection_len == 1 {
                window.primary_selection
            } else {
                None
            };
            (
                project.canonical_root.clone(),
                document,
                view,
                frame,
                window.generation,
            )
        };
        let result = export_saved_web(&input.0, input.1, input.2, input.3, input.4);
        let mut bytes = Vec::new();
        let mut out_rows = Vec::new();
        let mut has_html = 0;
        let mut html_len = 0;
        let mut error_len = 0;
        match result {
            Ok(report) => {
                if let Some(html) = report.html {
                    has_html = 1;
                    html_len = html.len();
                    bytes.extend_from_slice(html.as_bytes());
                }
                for diagnostic in report.diagnostics {
                    let (Ok(code_len), Ok(message_len)) = (
                        u32::try_from(diagnostic.code.len()),
                        u32::try_from(diagnostic.message.len()),
                    ) else {
                        return StellarStatus::Internal;
                    };
                    out_rows.push(StellarWebDiagnostic {
                        node: diagnostic.node.to_bytes(),
                        error: u32::from(diagnostic.error),
                        code_len,
                        message_len,
                    });
                    bytes.extend_from_slice(diagnostic.code.as_bytes());
                    bytes.extend_from_slice(diagnostic.message.as_bytes());
                }
            }
            Err(error) => {
                error_len = error.len();
                bytes.extend_from_slice(error.as_bytes());
            }
        }
        unsafe {
            info.write(StellarWebExportInfo {
                html_len,
                error_len,
                diagnostic_count: out_rows.len(),
                text_len: bytes.len(),
                has_html,
            });
        }
        if out_rows.len() > row_capacity || bytes.len() > text_capacity {
            return StellarStatus::BufferTooSmall;
        }
        if (!out_rows.is_empty() && rows.is_null()) || (!bytes.is_empty() && text.is_null()) {
            return StellarStatus::NullPointer;
        }
        unsafe {
            if !out_rows.is_empty() {
                std::ptr::copy_nonoverlapping(out_rows.as_ptr(), rows, out_rows.len());
            }
            if !bytes.is_empty() {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), text, bytes.len());
            }
        }
        StellarStatus::Ok
    })
}

//! 현재 프로젝트 라이브러리의 읽기 전용 셸 payload.
use crate::{ffi_guard, registry, StellarStatus};
use stellar_runtime::app::WindowSessionHandle;

fn text(out: &mut Vec<u8>, string: &str) {
    out.extend_from_slice(&(string.len() as u32).to_le_bytes());
    out.extend_from_slice(string.as_bytes());
}

/// registry mutex를 푼 뒤 파일을 읽습니다. UI는 worker에서 호출합니다.
/// # Safety
/// out_len은 쓸 수 있는 size_t, null이 아닌 buf는 capacity 바이트를 쓸 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_component_library(
    handle: u64,
    session: u64,
    buf: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> StellarStatus {
    ffi_guard(move || {
        if out_len.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { *out_len = 0 }
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
        // tag 0: legacy chunk 개수, 행 수, 행들. tag 1: 오류 한 줄. 문자열은 u32le 길이 + UTF-8.
        let mut bytes = Vec::new();
        match project.component_library() {
            Ok(library) => {
                bytes.push(0);
                bytes.extend_from_slice(&(library.chunk_count as u32).to_le_bytes());
                bytes.extend_from_slice(&(library.entries.len() as u32).to_le_bytes());
                for entry in library.entries {
                    bytes.extend_from_slice(&entry.id.to_bytes());
                    bytes.extend_from_slice(&entry.root.to_bytes());
                    bytes.extend_from_slice(&(entry.node_count as u32).to_le_bytes());
                    bytes.extend_from_slice(&entry.size.width().to_le_bytes());
                    bytes.extend_from_slice(&entry.size.height().to_le_bytes());
                    text(&mut bytes, &entry.name);
                    let Some(filename) = entry.file.file_name().and_then(|name| name.to_str())
                    else {
                        return StellarStatus::Internal;
                    };
                    text(&mut bytes, filename);
                    text(&mut bytes, &entry.hash.to_string());
                }
            }
            Err(message) => {
                bytes.push(1);
                text(&mut bytes, &message);
            }
        }
        unsafe {
            *out_len = bytes.len();
        }
        if buf.is_null() {
            return StellarStatus::Ok;
        }
        if capacity < bytes.len() {
            return StellarStatus::BufferTooSmall;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len());
        }
        StellarStatus::Ok
    })
}

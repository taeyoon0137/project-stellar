//! 컴포넌트 라이브러리 조회와 원본 편집을 위한 네이티브 셸 API.
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
    unsafe { read_component_library(handle, session, buf, capacity, out_len, false) }
}

/// 라이브러리의 각 행에 root 채우기(u8 색공간, 0이면 없음, 있으면 f32le RGBA)를 덧붙입니다.
/// # Safety
/// 출력 버퍼 조건은 stellar_window_component_library와 같습니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_component_library_with_fill(
    handle: u64,
    session: u64,
    buf: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> StellarStatus {
    unsafe { read_component_library(handle, session, buf, capacity, out_len, true) }
}

unsafe fn read_component_library(
    handle: u64,
    session: u64,
    buf: *mut u8,
    capacity: usize,
    out_len: *mut usize,
    with_fill: bool,
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
                    if with_fill {
                        if let Some(color) = entry.background {
                            bytes.extend_from_slice(&color.to_bytes());
                        } else {
                            bytes.push(0);
                        }
                    }
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

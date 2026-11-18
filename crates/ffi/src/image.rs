//! 창의 검증된 이미지 원본 가져오기와 PNG 표시용 worker 읽기.
use crate::{ffi_guard, registry, window_session::StellarSelectionTarget, StellarStatus};
use stellar_core_types::{
    ids::StableId,
    value::{Size2, Vec2},
};
use stellar_runtime::app::{ImagePlacement, WindowSessionHandle};

/// PNG 선택을 이미지 노드 하나로 배치합니다.
/// # Safety
/// target, parent(16), digest(32), path(len)는 읽기 가능해야 하고 out은 null 또는 request 하나를 쓸 수 있어야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_place_png(
    handle: u64,
    target: *const StellarSelectionTarget,
    parent: *const [u8; 16],
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    path: *const u8,
    len: usize,
    digest: *const [u8; 32],
    out: *mut u64,
) -> StellarStatus {
    ffi_guard(move || {
        if !out.is_null() {
            unsafe { out.write(0) }
        }
        if target.is_null() || parent.is_null() || digest.is_null() || path.is_null() {
            return StellarStatus::NullPointer;
        }
        if len == 0 || len > 4096 {
            return StellarStatus::InvalidArgument;
        }
        let Ok(source) = std::str::from_utf8(unsafe { std::slice::from_raw_parts(path, len) })
        else {
            return StellarStatus::InvalidArgument;
        };
        let Ok(position) = Vec2::new(x, y) else {
            return StellarStatus::InvalidArgument;
        };
        let size = if width == 0.0 && height == 0.0 {
            None
        } else {
            match Size2::new(width, height) {
                Ok(size) if !size.is_empty() => Some(size),
                _ => return StellarStatus::InvalidArgument,
            }
        };
        let input = ImagePlacement {
            target: unsafe { *target }.runtime_target(),
            parent: StableId::from_bytes(unsafe { *parent }),
            position,
            size,
            source: source.into(),
            digest: unsafe { *digest },
        };
        let Ok(mut reg) = registry().lock() else {
            return StellarStatus::Internal;
        };
        let Some(rt) = reg.live.get_mut(&handle) else {
            return StellarStatus::InvalidHandle;
        };
        let result = match rt.app() {
            Ok(app) => app.place_png(input),
            Err(e) => return e,
        };
        unsafe { rt.finish_request(result, out) }
    })
}

/// 읽기는 worker에서만 합니다. null buffer는 hash/size만 확인합니다.
/// # Safety
/// node(16)는 읽기 가능하고, digest(32), len과 null이 아닌 buf(capacity)는 쓰기 가능해야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_canvas_png(
    handle: u64,
    session: u64,
    revision: u64,
    row: u32,
    node: *const [u8; 16],
    digest: *mut [u8; 32],
    buf: *mut u8,
    capacity: usize,
    len: *mut usize,
) -> StellarStatus {
    ffi_guard(move || {
        if node.is_null() || digest.is_null() || len.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe {
            len.write(0);
            digest.write([0; 32]);
        }
        let state = {
            let Ok(reg) = registry().lock() else {
                return StellarStatus::Internal;
            };
            let Some(app) = reg.live.get(&handle).and_then(|r| r.app.as_ref()) else {
                return StellarStatus::InvalidHandle;
            };
            app.state()
        };
        let Some((hash, size, bytes)) = state.canvas_png(
            WindowSessionHandle(session),
            revision,
            row as usize,
            StableId::from_bytes(unsafe { *node }),
            !buf.is_null(),
        ) else {
            return StellarStatus::NoError;
        };
        unsafe {
            len.write(size as usize);
            digest.write(*hash.digest());
        }
        if !buf.is_null() {
            if bytes.len() > capacity {
                return StellarStatus::BufferTooSmall;
            }
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, bytes.len());
            }
        }
        StellarStatus::Ok
    })
}

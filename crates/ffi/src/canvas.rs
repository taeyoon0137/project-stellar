//! 저장된 Free Frame scene과 같은 게시 snapshot의 카메라를 macOS 창으로 복사합니다.

use stellar_core_types::color::Color;
use stellar_core_types::value::Transform2D;
use stellar_runtime::app::WindowSessionHandle;
use stellar_runtime::canvas::CanvasFrame;

use crate::{ffi_guard, registry, StellarStatus};

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct StellarCanvasColor {
    space: u32,
    rgba: [f32; 4],
}

impl From<Option<Color>> for StellarCanvasColor {
    fn from(color: Option<Color>) -> Self {
        color.map_or_else(Self::default, |c| Self {
            space: u32::from(c.space().tag()),
            rgba: [c.r(), c.g(), c.b(), c.alpha()],
        })
    }
}

/// 한 Frame의 그리기 정보. C 레이아웃은 stellar_ffi.h와 공유합니다.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct StellarCanvasFrame {
    id: [u8; 16],
    parent: u32,
    flags: u32,
    world: [f32; 6],
    size: [f32; 2],
    radii: [f32; 4],
    fill: StellarCanvasColor,
    border: StellarCanvasColor,
    border_width: f32,
}

fn affine(t: Transform2D) -> [f32; 6] {
    let (a, b, c, d, x, y) = t.components();
    [a, b, c, d, x, y]
}

impl From<&CanvasFrame> for StellarCanvasFrame {
    fn from(frame: &CanvasFrame) -> Self {
        let size = frame.geometry.rect.size();
        let (tl, tr, br, bl) = frame.geometry.radii.components();
        Self {
            id: frame.id.to_bytes(),
            parent: frame.parent.unwrap_or(u32::MAX),
            flags: u32::from(frame.clip_children) | (u32::from(frame.hit_target) << 1),
            world: affine(frame.world),
            size: [size.width(), size.height()],
            radii: [tl, tr, br, bl],
            fill: frame.background.into(),
            border: frame.border.map(|b| b.color).into(),
            border_width: frame.border.map_or(0.0, |b| b.thickness()),
        }
    }
}

/// 한 조회에서 사용한 View, revision, 카메라 및 목록 길이.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct StellarCanvasInfo {
    revision: u64,
    view: [u8; 16],
    primary: [u8; 16],
    camera: [f32; 6],
    count: u32,
    skipped: u32,
}

/// 현재 창의 active view에 대해 같은 snapshot의 metadata와 선택적 painter 목록을 돌려줍니다.
///
/// # Safety
/// `info`는 쓰기 가능한 포인터이고 `rows`는 null이거나 `capacity`개 항목의 정렬된 쓰기 버퍼여야 합니다.
#[no_mangle]
pub unsafe extern "C" fn stellar_window_canvas(
    handle: u64,
    session: u64,
    width: f32,
    height: f32,
    info: *mut StellarCanvasInfo,
    rows: *mut StellarCanvasFrame,
    capacity: usize,
) -> StellarStatus {
    ffi_guard(move || {
        if info.is_null() {
            return StellarStatus::NullPointer;
        }
        unsafe { info.write(StellarCanvasInfo::default()) };
        if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
            return StellarStatus::InvalidArgument;
        }
        let state = {
            let Ok(reg) = registry().lock() else {
                return StellarStatus::Internal;
            };
            let Some(runtime) = reg.live.get(&handle) else {
                return StellarStatus::InvalidHandle;
            };
            let Some(app) = &runtime.app else {
                return StellarStatus::InvalidArgument;
            };
            app.state()
        };
        let Some(window) = state.window(WindowSessionHandle(session)) else {
            return StellarStatus::InvalidArgument;
        };
        let (Some(scene), Some(view), Some(viewport)) =
            (&window.canvas, window.active_view, window.viewport)
        else {
            return StellarStatus::NoError;
        };
        let Some(camera) = viewport.world_to_view(width, height) else {
            return StellarStatus::InvalidArgument;
        };
        let Ok(count) = u32::try_from(scene.frames.len()) else {
            return StellarStatus::Internal;
        };
        unsafe {
            info.write(StellarCanvasInfo {
                revision: window.revision.0,
                view: view.to_bytes(),
                primary: window.primary_selection.map_or([0; 16], |id| id.to_bytes()),
                camera: affine(camera),
                count,
                skipped: scene.skipped,
            })
        };
        if rows.is_null() {
            return StellarStatus::Ok;
        }
        if capacity < scene.frames.len() {
            return StellarStatus::BufferTooSmall;
        }
        for (index, frame) in scene.frames.iter().enumerate() {
            unsafe {
                rows.add(index).write(frame.into());
            }
        }
        StellarStatus::Ok
    })
}

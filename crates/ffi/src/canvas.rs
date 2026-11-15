//! 저장된 Free Frame scene과 같은 게시 snapshot의 카메라를 macOS 창으로 복사합니다.

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::value::Transform2D;
use stellar_runtime::app::WindowSessionHandle;
use stellar_runtime::canvas::CanvasFrame;

use crate::{ffi_guard, registry, StellarStatus};

/// C ABI의 선택적 비연관 RGBA. 색공간 tag 0은 색 없음입니다.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct StellarCanvasColor {
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

impl StellarCanvasColor {
    pub(crate) fn decode(self) -> Result<Option<Color>, StellarStatus> {
        if self.space == 0 {
            return Ok(None);
        }
        let space = u8::try_from(self.space)
            .ok()
            .and_then(ColorSpace::from_tag)
            .ok_or(StellarStatus::InvalidArgument)?;
        let [r, g, b, a] = self.rgba;
        Color::new(space, r, g, b, a)
            .map(Some)
            .map_err(|_| StellarStatus::InvalidArgument)
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
            flags: u32::from(frame.clip_children)
                | (u32::from(frame.hit_target) << 1)
                | (u32::from(frame.rectangle_shape) << 2)
                | (u32::from(frame.instance_frame) << 3)
                | (u32::from(frame.text_node) << 4)
                | (u32::from(frame.image_asset.is_some()) << 5)
                | (u32::from(frame.path_shape) << 6),
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

#[cfg(test)]
mod color_tests {
    use super::*;

    #[test]
    fn background_preserves_none_and_wide_gamut_and_rejects_invalid_values() {
        let p3 = Color::new(ColorSpace::DisplayP3, 1.25, -0.1, 0.5, 0.375).unwrap();
        assert_eq!(
            StellarCanvasColor::from(Some(p3)).decode().unwrap(),
            Some(p3)
        );
        assert_eq!(StellarCanvasColor::default().decode().unwrap(), None);
        for color in [
            StellarCanvasColor {
                space: 257,
                rgba: [0.0; 4],
            },
            StellarCanvasColor {
                space: 1,
                rgba: [0.0, 0.0, 0.0, 2.0],
            },
            StellarCanvasColor {
                space: 2,
                rgba: [f32::NAN, 0.0, 0.0, 1.0],
            },
        ] {
            assert_eq!(color.decode(), Err(StellarStatus::InvalidArgument));
        }
    }
}

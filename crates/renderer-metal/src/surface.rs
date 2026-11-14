//! CAMetalLayer 연결·설정과 drawable 획득 (XI.8.1).
//!
//! platform 셸(CanvasHost, II.5.4)이 만든 CAMetalLayer를 backend 생성 시 넘겨받아 retain합니다.
//! [`RendererBackend::init`](stellar_renderer_core::backend::RendererBackend::init)에서 device,
//! pixel format, colorspace, drawable 크기를 설정합니다.
//!
//! - **pixel format**: CAMetalLayer는 BGRA8(·sRGB)·RGBA16Float만 받습니다. 다른 format은 설정 전에
//!   `Unsupported`로 거부합니다(잘못된 값은 Objective-C 예외).
//! - **colorspace**: surface 색공간 tag(III.4.1 `ColorSpace::tag`)에 맞춰 sRGB 또는 Display P3를
//!   지정합니다. RGBA16Float는 선형 working 값을 담으므로 extended linear 변형을 씁니다.
//! - **속성 변경**은 명시적 `CATransaction`(animation 끔) 안에서 합니다. render thread에서 바꿔도
//!   implicit transaction이 thread에 남지 않습니다.
//! - **drawable 획득**은 `allowsNextDrawableTimeout`을 항상 켜 둡니다. 가려진 창에서도 무한 대기하지
//!   않고 최대 1초 뒤 nil을 받습니다.
//!
//! ## resize와 frame skip (XI.8.2)
//!
//! - `resize`는 backend가 기억하는 surface 크기를 바꾸고, 0이 아니면 layer `drawableSize`에 넣습니다.
//!   0 크기(최소화)는 layer에 넣지 않고 기억만 합니다. 그 동안 surface frame은 drawable을 요청하지 않고
//!   `Skipped(EmptySurface)`입니다(최소화된 창에서 `nextDrawable` 대기 없음).
//! - drawable이 nil이면(가려짐·timeout) frame을 `Skipped(NoDrawable)`로 건너뜁니다. 오류가 아니므로
//!   frame은 열리지 않고 다음 tick에 다시 시도합니다.
//! - 얻은 drawable 크기가 backend 크기와 다르면(셸이 layer 크기를 직접 바꾼 경우) layer를 backend
//!   크기로 다시 맞추고 그 frame은 `Skipped(NoDrawable)`입니다. attachment 크기 불일치로 Metal에 잘못된
//!   render pass를 넘기지 않습니다.

use std::ffi::c_void;
use std::ptr::NonNull;

use stellar_renderer_core::backend::{BackendError, BackendErrorKind, SurfaceConfig, SurfaceSize};
use stellar_renderer_core::render_graph::{TextureFormat, MAX_TEXTURE_DIMENSION};

use crate::format::{is_layer_format, mtl_pixel_format};
use crate::objc::{class, sel, send, Id, NSUInteger, RawId};
use crate::sys::{
    kCGColorSpaceDisplayP3, kCGColorSpaceExtendedLinearDisplayP3, kCGColorSpaceExtendedLinearSRGB,
    kCGColorSpaceSRGB, CFStringRef, CGColorSpaceCreateWithName, CGColorSpaceRelease, CGSize,
};

/// sRGB 색공간 tag(III.4.1 `ColorSpace::tag`, 저장 포맷 계약).
const COLOR_SPACE_TAG_SRGB: u8 = 0x01;
/// Display P3 색공간 tag.
const COLOR_SPACE_TAG_DISPLAY_P3: u8 = 0x02;

/// CAMetalLayer 설정.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayerOptions {
    /// drawable 최대 수(`maximumDrawableCount`, 2 또는 3).
    pub maximum_drawable_count: u8,
    /// display 주사율 동기화(`displaySyncEnabled`).
    pub display_sync: bool,
    /// drawable을 render target으로만 씁니다(`framebufferOnly`). 거짓이면 blit·sampling이 가능하지만
    /// 느립니다(진단·테스트용).
    pub framebuffer_only: bool,
}

impl LayerOptions {
    /// 기본값: drawable 3개, display sync, framebuffer only.
    pub const DEFAULT: LayerOptions = LayerOptions {
        maximum_drawable_count: 3,
        display_sync: true,
        framebuffer_only: true,
    };

    fn validate(&self) -> Result<(), BackendError> {
        if !(2..=3).contains(&self.maximum_drawable_count) {
            return Err(BackendError::new(
                BackendErrorKind::InvalidUsage,
                "CAMetalLayer drawable 수는 2 또는 3이어야 합니다",
            ));
        }
        Ok(())
    }
}

impl Default for LayerOptions {
    fn default() -> LayerOptions {
        LayerOptions::DEFAULT
    }
}

/// surface 색공간·format에 맞는 CoreGraphics 색공간 이름.
fn color_space_name(tag: u8, format: TextureFormat) -> Option<CFStringRef> {
    let linear = format == TextureFormat::Rgba16Float;
    // SAFETY: CoreGraphics가 정의한 불변 전역 CFString 상수를 읽습니다.
    unsafe {
        match (tag, linear) {
            (COLOR_SPACE_TAG_SRGB, false) => Some(kCGColorSpaceSRGB),
            (COLOR_SPACE_TAG_SRGB, true) => Some(kCGColorSpaceExtendedLinearSRGB),
            (COLOR_SPACE_TAG_DISPLAY_P3, false) => Some(kCGColorSpaceDisplayP3),
            (COLOR_SPACE_TAG_DISPLAY_P3, true) => Some(kCGColorSpaceExtendedLinearDisplayP3),
            _ => None,
        }
    }
}

/// animation 없는 명시적 `CATransaction` 안에서 `f`를 실행합니다.
fn with_transaction<T>(f: impl FnOnce() -> T) -> T {
    let transaction = class!("CATransaction");
    if let Some(class) = transaction {
        // SAFETY: `+begin`은 () -> void, `+setDisableActions:`는 (BOOL) -> void입니다.
        unsafe {
            let _: () = send(class.as_raw(), sel!("begin"), ());
            let _: () = send(class.as_raw(), sel!("setDisableActions:"), (true,));
        }
    }
    let result = f();
    if let Some(class) = transaction {
        // SAFETY: `+commit`은 () -> void이며 위 begin과 짝입니다.
        unsafe {
            let _: () = send(class.as_raw(), sel!("commit"), ());
        }
    }
    result
}

/// surface frame의 resize·skip 통계(진단·benchmark용, XI.11.5). device 재생성에도 누적합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SurfaceStats {
    /// 크기가 실제로 바뀐 resize 횟수.
    pub resizes: u64,
    /// 0 크기 surface(최소화)로 건너뛴 frame 수. drawable을 요청하지 않습니다.
    pub skipped_empty: u64,
    /// drawable을 얻지 못해 건너뛴 frame 수(가려짐·timeout·크기 재동기화).
    pub skipped_no_drawable: u64,
    /// layer drawable 크기가 backend 크기와 달라 layer를 다시 맞춘 횟수(`skipped_no_drawable`에 포함).
    pub drawable_size_resyncs: u64,
}

/// 얻은 drawable과 그 texture.
#[derive(Debug)]
pub(crate) struct Drawable {
    drawable: Id,
    texture: Id,
}

impl Drawable {
    /// drawable texture pixel 크기.
    pub(crate) fn size(&self) -> (u32, u32) {
        // SAFETY: `-width`·`-height`는 () -> NSUInteger입니다.
        let (width, height): (NSUInteger, NSUInteger) = unsafe {
            (
                self.texture.send(sel!("width"), ()),
                self.texture.send(sel!("height"), ()),
            )
        };
        (
            u32::try_from(width).unwrap_or(u32::MAX),
            u32::try_from(height).unwrap_or(u32::MAX),
        )
    }

    /// `id<CAMetalDrawable>`.
    pub(crate) fn drawable(&self) -> &Id {
        &self.drawable
    }

    /// drawable의 `id<MTLTexture>`.
    pub(crate) fn texture(&self) -> &Id {
        &self.texture
    }
}

/// backend가 그리는 CAMetalLayer.
#[derive(Debug)]
pub(crate) struct LayerSurface {
    layer: Id,
    options: LayerOptions,
}

impl LayerSurface {
    /// platform 코드가 넘긴 CAMetalLayer를 retain합니다. CAMetalLayer가 아니면 `InvalidUsage`.
    ///
    /// # Safety
    ///
    /// `layer`는 살아 있는 Objective-C 객체를 가리켜야 합니다.
    pub(crate) unsafe fn from_raw(
        layer: NonNull<c_void>,
        options: LayerOptions,
    ) -> Result<LayerSurface, BackendError> {
        options.validate()?;
        let class = class!("CAMetalLayer").ok_or(BackendError::new(
            BackendErrorKind::Unsupported,
            "CAMetalLayer class가 없습니다",
        ))?;
        // SAFETY: layer는 살아 있는 객체(호출자 보장)이고 `-isKindOfClass:`는 (Class) -> BOOL입니다.
        let is_layer: bool =
            unsafe { send(layer.as_ptr(), sel!("isKindOfClass:"), (class.as_raw(),)) };
        if !is_layer {
            return Err(BackendError::new(
                BackendErrorKind::InvalidUsage,
                "surface 객체가 CAMetalLayer가 아닙니다",
            ));
        }
        // SAFETY: layer는 살아 있는 객체입니다(호출자 보장).
        let layer = unsafe { Id::from_borrowed(layer.as_ptr()) }.ok_or(BackendError::new(
            BackendErrorKind::InvalidUsage,
            "null CAMetalLayer",
        ))?;
        Ok(LayerSurface { layer, options })
    }

    /// 창에 붙지 않은 새 CAMetalLayer(테스트·진단용).
    pub(crate) fn detached(options: LayerOptions) -> Result<LayerSurface, BackendError> {
        options.validate()?;
        let class = class!("CAMetalLayer").ok_or(BackendError::new(
            BackendErrorKind::Unsupported,
            "CAMetalLayer class가 없습니다",
        ))?;
        // SAFETY: `+[CAMetalLayer new]`는 () -> id(+1)입니다.
        let layer = unsafe { Id::from_owned(send(class.as_raw(), sel!("new"), ())) }.ok_or(
            BackendError::new(
                BackendErrorKind::OutOfHostMemory,
                "CAMetalLayer를 만들 수 없습니다",
            ),
        )?;
        Ok(LayerSurface { layer, options })
    }

    /// `CAMetalLayer`.
    #[cfg(test)]
    pub(crate) fn layer(&self) -> &Id {
        &self.layer
    }

    /// device·pixel format·colorspace·drawable 크기를 설정합니다.
    pub(crate) fn configure(
        &self,
        device: &Id,
        surface: &SurfaceConfig,
    ) -> Result<(), BackendError> {
        if !is_layer_format(surface.format) {
            return Err(BackendError::new(
                BackendErrorKind::Unsupported,
                "CAMetalLayer가 지원하지 않는 surface pixel format",
            ));
        }
        let space_name = color_space_name(surface.color_space.tag(), surface.format).ok_or(
            BackendError::new(
                BackendErrorKind::Unsupported,
                "지원하지 않는 surface 색공간",
            ),
        )?;
        check_surface_size(surface.size)?;
        // SAFETY: CGColorSpaceCreateWithName은 Create 규칙(+1) 또는 NULL입니다.
        let space = unsafe { CGColorSpaceCreateWithName(space_name) };
        if space.is_null() {
            return Err(BackendError::new(
                BackendErrorKind::Unsupported,
                "CoreGraphics 색공간을 만들 수 없습니다",
            ));
        }
        let format = mtl_pixel_format(surface.format);
        let options = self.options;
        with_transaction(|| {
            // SAFETY: CAMetalLayer setter 시그니처: setDevice:(id), setPixelFormat:(NSUInteger,
            // 위에서 허용 format 확인), setColorspace:(CGColorSpaceRef, layer가 retain),
            // setFramebufferOnly:/setDisplaySyncEnabled:/setAllowsNextDrawableTimeout:(BOOL),
            // setMaximumDrawableCount:(NSUInteger, 2..=3 확인).
            unsafe {
                let _: () = self.layer.send(sel!("setDevice:"), (device.as_raw(),));
                let _: () = self.layer.send(sel!("setPixelFormat:"), (format,));
                let _: () = self.layer.send(sel!("setColorspace:"), (space,));
                let _: () = self
                    .layer
                    .send(sel!("setFramebufferOnly:"), (options.framebuffer_only,));
                let _: () = self
                    .layer
                    .send(sel!("setDisplaySyncEnabled:"), (options.display_sync,));
                let _: () = self
                    .layer
                    .send(sel!("setAllowsNextDrawableTimeout:"), (true,));
                let _: () = self.layer.send(
                    sel!("setMaximumDrawableCount:"),
                    (NSUInteger::from(options.maximum_drawable_count),),
                );
            }
        });
        // SAFETY: 위 Create로 받은 참조를 한 번 해제합니다(layer는 자기 참조를 retain).
        unsafe { CGColorSpaceRelease(space) };
        self.set_drawable_size(surface.size);
        Ok(())
    }

    /// drawable pixel 크기를 바꿉니다. 0 크기는 layer에 넣지 않습니다(그 동안 frame은 skip).
    pub(crate) fn set_drawable_size(&self, size: SurfaceSize) {
        if size.is_empty() {
            return;
        }
        let size = CGSize {
            width: f64::from(size.width),
            height: f64::from(size.height),
        };
        with_transaction(|| {
            // SAFETY: `-setDrawableSize:`는 (CGSize) -> void입니다.
            unsafe {
                let _: () = self.layer.send(sel!("setDrawableSize:"), (size,));
            }
        });
    }

    /// layer의 현재 drawable 크기.
    #[cfg(test)]
    pub(crate) fn drawable_size(&self) -> CGSize {
        // SAFETY: `-drawableSize`는 () -> CGSize입니다(16 byte, register 반환).
        unsafe { self.layer.send(sel!("drawableSize"), ()) }
    }

    /// 다음 drawable. timeout·가려짐으로 얻지 못하면 `None`.
    pub(crate) fn next_drawable(&self) -> Option<Drawable> {
        // SAFETY: `-nextDrawable`은 () -> id<CAMetalDrawable>(+0, nil 가능),
        // `-texture`는 () -> id<MTLTexture>(+0)입니다.
        unsafe {
            let raw: RawId = self.layer.send(sel!("nextDrawable"), ());
            let drawable = Id::from_borrowed(raw)?;
            let texture = Id::from_borrowed(drawable.send(sel!("texture"), ()))?;
            Some(Drawable { drawable, texture })
        }
    }
}

/// surface 크기가 texture 상한 안이면 통과합니다(0 크기 허용).
pub(crate) fn check_surface_size(size: SurfaceSize) -> Result<(), BackendError> {
    if size.width > MAX_TEXTURE_DIMENSION || size.height > MAX_TEXTURE_DIMENSION {
        return Err(BackendError::new(
            BackendErrorKind::InvalidUsage,
            "surface 크기가 texture 상한을 넘습니다",
        ));
    }
    Ok(())
}

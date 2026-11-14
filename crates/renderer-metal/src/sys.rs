//! Metal·QuartzCore·CoreGraphics C 선언과 상수.
//!
//! 값은 Apple SDK header(`MTLPixelFormat.h`, `MTLRenderPass.h`, `MTLTexture.h` 등)의 enum 값을 그대로
//! 옮깁니다. enum 기반 타입은 모두 `NSUInteger`입니다.

use std::ffi::c_void;

use crate::objc::{NSUInteger, RawId};

/// `CGFloat`(64-bit).
pub(crate) type CGFloat = f64;

/// `CGSize`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CGSize {
    /// 너비.
    pub width: CGFloat,
    /// 높이.
    pub height: CGFloat,
}

/// `MTLClearColor`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MTLClearColor {
    /// red.
    pub red: f64,
    /// green.
    pub green: f64,
    /// blue.
    pub blue: f64,
    /// alpha.
    pub alpha: f64,
}

/// `MTLOrigin`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MTLOrigin {
    /// x.
    pub x: NSUInteger,
    /// y.
    pub y: NSUInteger,
    /// z.
    pub z: NSUInteger,
}

/// `MTLSize`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MTLSize {
    /// 너비.
    pub width: NSUInteger,
    /// 높이.
    pub height: NSUInteger,
    /// 깊이.
    pub depth: NSUInteger,
}

/// `CGColorSpaceRef`.
pub(crate) type CGColorSpaceRef = *const c_void;

/// `CFStringRef`.
pub(crate) type CFStringRef = *const c_void;

#[link(name = "Metal", kind = "framework")]
extern "C" {
    /// 시스템 기본 GPU(+1, 없으면 nil).
    pub(crate) fn MTLCreateSystemDefaultDevice() -> RawId;
}

// CAMetalLayer·CATransaction class를 runtime에 올립니다.
#[link(name = "QuartzCore", kind = "framework")]
extern "C" {}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    pub(crate) static kCGColorSpaceSRGB: CFStringRef;
    pub(crate) static kCGColorSpaceDisplayP3: CFStringRef;
    pub(crate) static kCGColorSpaceExtendedLinearSRGB: CFStringRef;
    pub(crate) static kCGColorSpaceExtendedLinearDisplayP3: CFStringRef;
    pub(crate) fn CGColorSpaceCreateWithName(name: CFStringRef) -> CGColorSpaceRef;
    pub(crate) fn CGColorSpaceRelease(space: CGColorSpaceRef);
}

/// `MTLPixelFormat`.
pub(crate) mod pixel_format {
    use crate::objc::NSUInteger;

    /// `MTLPixelFormatR8Unorm`.
    pub(crate) const R8_UNORM: NSUInteger = 10;
    /// `MTLPixelFormatRGBA8Unorm`.
    pub(crate) const RGBA8_UNORM: NSUInteger = 70;
    /// `MTLPixelFormatRGBA8Unorm_sRGB`.
    pub(crate) const RGBA8_UNORM_SRGB: NSUInteger = 71;
    /// `MTLPixelFormatBGRA8Unorm`.
    pub(crate) const BGRA8_UNORM: NSUInteger = 80;
    /// `MTLPixelFormatBGRA8Unorm_sRGB`.
    pub(crate) const BGRA8_UNORM_SRGB: NSUInteger = 81;
    /// `MTLPixelFormatRGBA16Float`.
    pub(crate) const RGBA16_FLOAT: NSUInteger = 115;
    /// `MTLPixelFormatDepth32Float_Stencil8`.
    pub(crate) const DEPTH32_FLOAT_STENCIL8: NSUInteger = 260;
}

/// `MTLLoadAction`.
pub(crate) mod load_action {
    use crate::objc::NSUInteger;

    /// `MTLLoadActionDontCare`.
    pub(crate) const DONT_CARE: NSUInteger = 0;
    /// `MTLLoadActionLoad`.
    pub(crate) const LOAD: NSUInteger = 1;
    /// `MTLLoadActionClear`.
    pub(crate) const CLEAR: NSUInteger = 2;
}

/// `MTLStoreAction`.
pub(crate) mod store_action {
    use crate::objc::NSUInteger;

    /// `MTLStoreActionDontCare`.
    pub(crate) const DONT_CARE: NSUInteger = 0;
    /// `MTLStoreActionStore`.
    pub(crate) const STORE: NSUInteger = 1;
}

/// `MTLTextureType`.
pub(crate) mod texture_type {
    use crate::objc::NSUInteger;

    /// `MTLTextureType2D`.
    pub(crate) const TYPE_2D: NSUInteger = 2;
    /// `MTLTextureType2DMultisample`.
    pub(crate) const TYPE_2D_MULTISAMPLE: NSUInteger = 4;
}

/// `MTLTextureUsage` bit.
pub(crate) mod texture_usage {
    use crate::objc::NSUInteger;

    /// `MTLTextureUsageShaderRead`.
    pub(crate) const SHADER_READ: NSUInteger = 1;
    /// `MTLTextureUsageRenderTarget`.
    pub(crate) const RENDER_TARGET: NSUInteger = 4;
}

/// `MTLStorageMode`.
pub(crate) mod storage_mode {
    use crate::objc::NSUInteger;

    /// `MTLStorageModePrivate`.
    pub(crate) const PRIVATE: NSUInteger = 2;
}

/// `MTLResourceOptions`.
pub(crate) mod resource_options {
    use crate::objc::NSUInteger;

    /// `MTLResourceStorageModeShared`(CPU·GPU 공유, hazard tracking 기본).
    pub(crate) const STORAGE_SHARED: NSUInteger = 0;
}

/// `MTLCommandBufferStatus`.
pub(crate) mod command_buffer_status {
    use crate::objc::NSUInteger;

    /// `MTLCommandBufferStatusCompleted`.
    pub(crate) const COMPLETED: NSUInteger = 4;
    /// `MTLCommandBufferStatusError`.
    pub(crate) const ERROR: NSUInteger = 5;
}

/// `MTLScissorRect`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MTLScissorRect {
    /// 왼쪽.
    pub x: NSUInteger,
    /// 위.
    pub y: NSUInteger,
    /// 너비.
    pub width: NSUInteger,
    /// 높이.
    pub height: NSUInteger,
}

/// `MTLPrimitiveTypeTriangle`.
pub(crate) const PRIMITIVE_TRIANGLE: NSUInteger = 3;

/// `MTLIndexTypeUInt32`.
pub(crate) const INDEX_TYPE_UINT32: NSUInteger = 1;

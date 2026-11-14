//! backend 중립 [`TextureFormat`] ↔ `MTLPixelFormat` 대응.

use stellar_renderer_core::render_graph::TextureFormat;

use crate::objc::NSUInteger;
use crate::sys::pixel_format;

/// `MTLPixelFormat` 값.
pub(crate) const fn mtl_pixel_format(format: TextureFormat) -> NSUInteger {
    match format {
        TextureFormat::Rgba8Unorm => pixel_format::RGBA8_UNORM,
        TextureFormat::Rgba8UnormSrgb => pixel_format::RGBA8_UNORM_SRGB,
        TextureFormat::Bgra8Unorm => pixel_format::BGRA8_UNORM,
        TextureFormat::Bgra8UnormSrgb => pixel_format::BGRA8_UNORM_SRGB,
        TextureFormat::Rgba16Float => pixel_format::RGBA16_FLOAT,
        TextureFormat::R8Unorm => pixel_format::R8_UNORM,
        TextureFormat::Depth32FloatStencil8 => pixel_format::DEPTH32_FLOAT_STENCIL8,
    }
}

/// pixel 하나의 byte 수(depth·stencil은 readback·upload 대상이 아니라 `None`).
pub(crate) const fn bytes_per_pixel(format: TextureFormat) -> Option<u32> {
    match format {
        TextureFormat::Rgba8Unorm
        | TextureFormat::Rgba8UnormSrgb
        | TextureFormat::Bgra8Unorm
        | TextureFormat::Bgra8UnormSrgb => Some(4),
        TextureFormat::Rgba16Float => Some(8),
        TextureFormat::R8Unorm => Some(1),
        TextureFormat::Depth32FloatStencil8 => None,
    }
}

/// `CAMetalLayer.pixelFormat`에 쓸 수 있는 format이면 참입니다.
///
/// CAMetalLayer는 BGRA8(·sRGB), RGBA16Float, 10-bit 계열만 받습니다. 다른 값을 넣으면 Objective-C
/// 예외가 나므로 설정 전에 이 함수로 거릅니다.
pub(crate) const fn is_layer_format(format: TextureFormat) -> bool {
    matches!(
        format,
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb | TextureFormat::Rgba16Float
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [TextureFormat; 7] = [
        TextureFormat::Rgba8Unorm,
        TextureFormat::Rgba8UnormSrgb,
        TextureFormat::Bgra8Unorm,
        TextureFormat::Bgra8UnormSrgb,
        TextureFormat::Rgba16Float,
        TextureFormat::R8Unorm,
        TextureFormat::Depth32FloatStencil8,
    ];

    #[test]
    fn formats_map_to_distinct_sdk_values() {
        let values: Vec<NSUInteger> = ALL.iter().map(|f| mtl_pixel_format(*f)).collect();
        assert_eq!(values, vec![70, 71, 80, 81, 115, 10, 260]);
        assert_eq!(
            ALL.iter().map(|f| bytes_per_pixel(*f)).collect::<Vec<_>>(),
            vec![Some(4), Some(4), Some(4), Some(4), Some(8), Some(1), None]
        );
        let layer: Vec<TextureFormat> = ALL.into_iter().filter(|f| is_layer_format(*f)).collect();
        assert_eq!(
            layer,
            vec![
                TextureFormat::Bgra8Unorm,
                TextureFormat::Bgra8UnormSrgb,
                TextureFormat::Rgba16Float
            ]
        );
    }
}

//! PNG 내보내기 테스트 (XX.2.2\~XX.2.4).

use stellar_core_types::color::ColorSpace;

use super::*;
use crate::raster::{CancelToken, OutputProfile, RgbaImage};

/// encode·해석·색 정보·encoder 옵션 테스트 (XX.2.2).
mod codec;

/// 2색 4×3 image.
fn sample_image(profile: OutputProfile) -> RgbaImage {
    let mut pixels = Vec::new();
    for y in 0..3u8 {
        for x in 0..4u8 {
            pixels.extend_from_slice(&[x * 60, y * 100, 255 - x * 30, 255 - y * 60]);
        }
    }
    RgbaImage::new(4, 3, profile, pixels).unwrap()
}

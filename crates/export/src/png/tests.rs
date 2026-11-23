//! PNG 내보내기 테스트 (XX.2.2~XX.2.4).

use std::collections::HashMap;
use std::sync::Arc;

use stellar_core_types::color::ColorSpace;

use super::*;
use crate::preflight::ConfirmedExport;
use crate::raster::software::SoftwareTarget;
use crate::raster::tests::{
    capture_of, frame, leaf_frame, quad_png, request, scene, srgb, text_layout, Assets, Glyphs,
    LAYOUT,
};
use crate::raster::{CancelToken, OffscreenError, OutputProfile, RgbaImage};
use crate::request::{ExportBackground, ExportFormat};
use crate::test_fixtures as fx;

/// encode·해석·색 정보·encoder 옵션 테스트 (XX.2.2).
mod codec;

/// worker 실행 테스트 (XX.2.3).
mod job;

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

fn assets() -> Assets {
    let mut map = HashMap::new();
    map.insert(fx::asset(1), quad_png());
    Assets(map)
}

fn software() -> Arc<SoftwareTarget> {
    Arc::new(SoftwareTarget::new(
        Arc::new(assets()),
        Arc::new(Glyphs::default()),
    ))
}

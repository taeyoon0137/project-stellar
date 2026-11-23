//! XI.11.1: common fixture offscreen render → raw RGBA reference harness (Metal).
//!
//! [`MetalHarness`]는 headless Metal backend 하나로 fixture를 그립니다. lowering은 backend 중립
//! `stellar_renderer_core::effects::lower`이고, 이 harness는 encode·submit·readback만 합니다. reference artifact
//! (`<name>.rgba`, `<name>.png`)는 [`artifact_dir`] 아래에 씁니다. 제품 export 기능이 아닙니다.
//!
//! - artifact 위치: `STELLAR_RENDER_CONFORMANCE_OUT`가 있으면 그 경로, 없으면 workspace `target/render-conformance`.

use std::path::PathBuf;

use stellar_renderer_core::backend::{
    BackendError, CheckedBackend, EncodeOutcome, FrameId, RendererBackend,
};
use stellar_renderer_core::effects::artifact::RgbaImage;
use stellar_renderer_core::effects::fixture::{
    fill_mesh, p3, solid, srgb, stroke_mesh, Category, Coverage, CoverageMask, Feature, FillRule,
    Fixture, GlyphOp, ImageData, ImageOp, Mesh, Op, PathBuilder,
};
use stellar_renderer_core::effects::lower::{lower, LowerError};
use stellar_renderer_core::effects::raster::render_reference;
use stellar_renderer_core::placeholder::TextureFilter;
use stellar_renderer_core::render_graph::TextureFormat;
use stellar_renderer_core::stroke::{StrokeJoin, StrokeStyle};

use super::*;
use crate::MetalBackend;

/// artifact 출력 경로 환경 변수.
pub(crate) const OUT_ENV: &str = "STELLAR_RENDER_CONFORMANCE_OUT";

/// artifact 출력 경로.
pub(crate) fn artifact_dir(group: &str) -> PathBuf {
    let base = std::env::var_os(OUT_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/render-conformance")
        });
    base.join(group)
}

/// headless Metal 적합성 harness.
pub(crate) struct MetalHarness {
    backend: CheckedBackend<MetalBackend>,
    next_frame: u64,
}

impl MetalHarness {
    pub(crate) fn new() -> MetalHarness {
        let mut backend = CheckedBackend::new(MetalBackend::headless());
        backend
            .init(&device_config(64, 64, TextureFormat::Bgra8Unorm))
            .expect("init");
        MetalHarness {
            backend,
            next_frame: 1,
        }
    }

    /// fixture를 offscreen render하고 target byte를 읽습니다(RGBA 순서, 8-bit target).
    pub(crate) fn render(&mut self, fixture: &Fixture) -> Result<RgbaImage, LowerError> {
        let lowered = lower(&mut self.backend, fixture)?;
        let frame = FrameId(self.next_frame);
        self.next_frame += 1;
        let result = (|| -> Result<Vec<u8>, BackendError> {
            let EncodeOutcome::Encoded(encoded) = self.backend.encode(&lowered.request(frame))?
            else {
                panic!("offscreen frame은 skip하지 않습니다");
            };
            self.backend.submit(encoded)?;
            self.backend.inner_mut().wait_idle()?;
            self.backend.inner_mut().read_texture(lowered.target)
        })();
        let release = lowered.release(&mut self.backend);
        let pixels = result?;
        release?;
        Ok(RgbaImage {
            width: fixture.width,
            height: fixture.height,
            pixels,
        })
    }
}

/// harness 자체를 검사하는 작은 공통 fixture(분류별 inventory는 XI.11.3).
fn smoke_fixtures() -> Vec<Fixture> {
    let star: Vec<[f32; 2]> = (0..5)
        .map(|i| {
            let a =
                std::f32::consts::PI * 2.0 * (i as f32 * 2.0) / 5.0 - std::f32::consts::FRAC_PI_2;
            [16.0 + 13.0 * a.cos(), 16.0 + 13.0 * a.sin()]
        })
        .collect();
    let star = PathBuilder::new().polygon(&star).build().expect("star");
    let zigzag = PathBuilder::new()
        .move_to(4.0, 26.0)
        .line_to(12.0, 6.0)
        .line_to(20.0, 26.0)
        .line_to(28.0, 6.0)
        .build()
        .expect("zigzag");
    let stroke = StrokeStyle::new(5.0)
        .expect("width")
        .with_join(StrokeJoin::Round);
    let checker = ImageData {
        width: 4,
        height: 4,
        space: stellar_renderer_core::color::ColorSpace::Srgb,
        texels: (0..16)
            .map(|i| {
                if (i % 4 + i / 4) % 2 == 0 {
                    [230, 40, 20, 255]
                } else {
                    [20, 60, 200, 128]
                }
            })
            .collect(),
    };
    let glyph = CoverageMask {
        width: 6,
        height: 8,
        data: (0..48)
            .map(|i| match (i % 6, i / 6) {
                (0 | 5, _) | (_, 0) => 255,
                (1 | 4, _) => 128,
                _ => 0,
            })
            .collect(),
    };
    vec![
        Fixture::new("smoke-solid-rect", Category::Frame, 32, 32)
            .feature(Feature::SolidFill)
            .with(solid(
                Mesh::rect(4.0, 4.0, 28.0, 20.0),
                srgb(0.9, 0.2, 0.1, 1.0),
                Coverage::Disjoint,
            ))
            .with(solid(
                Mesh::rect(10.0, 10.0, 22.0, 30.0),
                srgb(0.1, 0.4, 0.9, 0.5),
                Coverage::Disjoint,
            )),
        Fixture::new("smoke-star-evenodd", Category::Shape, 32, 32)
            .feature(Feature::SolidFill)
            .with(Op::Fill(stellar_renderer_core::effects::fixture::FillOp {
                mesh: fill_mesh(&star, FillRule::EvenOdd).expect("mesh"),
                paint: stellar_renderer_core::effects::fixture::Paint::Solid(srgb(
                    0.2, 0.6, 0.3, 1.0,
                )),
                coverage: Coverage::Disjoint,
            })),
        Fixture::new("smoke-stroke-union", Category::Path, 32, 32)
            .feature(Feature::StrokeUnion)
            .with(solid(
                stroke_mesh(&zigzag, &stroke).expect("stroke"),
                srgb(0.0, 0.0, 0.0, 0.5),
                Coverage::Union,
            )),
        Fixture::new("smoke-image-nearest", Category::Image, 16, 16)
            .feature(Feature::Image)
            .with(Op::Image(ImageOp {
                rect: [0.0, 0.0, 16.0, 16.0],
                uv: [0.0, 0.0, 1.0, 1.0],
                image: std::sync::Arc::new(checker),
                filter: TextureFilter::Nearest,
                opacity: 1.0,
            })),
        Fixture::new("smoke-glyph-p3", Category::Text, 16, 16)
            .feature(Feature::Glyph)
            .feature(Feature::DisplayP3)
            .with(Op::Glyphs(GlyphOp {
                origin: [5, 4],
                mask: std::sync::Arc::new(glyph),
                color: p3(0.1, 0.5, 0.9, 1.0),
            })),
    ]
}

#[test]
fn harness_renders_common_fixtures_to_deterministic_raw_references() {
    // 검증 기준(XI.11.1): 동일 fixture raw image 생성.
    if !metal_available() {
        return;
    }
    let mut harness = MetalHarness::new();
    let dir = artifact_dir("harness-smoke");
    for fixture in smoke_fixtures() {
        let first = harness.render(&fixture).expect("render");
        let second = harness.render(&fixture).expect("render again");
        assert!(first.is_well_formed());
        assert_eq!(first, second, "{}: 같은 fixture는 같은 byte", fixture.name);
        let path = first
            .write_reference(&dir, &fixture.name)
            .expect("write reference");
        let raw = std::fs::read(&path).expect("read raw");
        assert_eq!(RgbaImage::from_raw(&raw).expect("parse raw"), first);
        assert!(dir.join(format!("{}.png", fixture.name)).exists());
        // CPU 기준과 같은 그림입니다(정밀 허용 오차 판정은 XI.11.2).
        let reference = render_reference(&fixture).expect("reference");
        let worst = first
            .pixels
            .iter()
            .zip(reference.pixels.iter())
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        let edge_pixels = first
            .pixels
            .chunks_exact(4)
            .zip(reference.pixels.chunks_exact(4))
            .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 1))
            .count();
        assert!(
            edge_pixels <= (fixture.width * fixture.height / 64) as usize,
            "{}: CPU 기준과 다른 pixel {edge_pixels}개(최대 차이 {worst})",
            fixture.name
        );
    }
}

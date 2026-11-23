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
use stellar_renderer_core::effects::compare::{compare, Comparison, ToleranceConfig};
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

/// GPU image를 fixture CPU 기준과 fixture 허용 오차 class로 비교합니다. 통과하지 못하면 `dir/diff/<name>/`에 diff
/// artifact를 씁니다(XI.11.2).
pub(crate) fn check_against_reference(
    fixture: &Fixture,
    actual: &RgbaImage,
    config: &ToleranceConfig,
    dir: &std::path::Path,
) -> Comparison {
    let expected = render_reference(fixture).expect("CPU reference");
    let tolerance = config.get(fixture.tolerance).unwrap_or_else(|| {
        panic!(
            "{}: 허용 오차 class {} 없음",
            fixture.name, fixture.tolerance
        )
    });
    let comparison = compare(&expected, actual, tolerance).expect("same size");
    if !comparison.passed() {
        let written = comparison
            .write_artifact(&dir.join("diff"), &fixture.name, &expected, actual)
            .expect("diff artifact");
        eprintln!("{}: diff artifact {}", fixture.name, written.display());
    }
    comparison
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
    let config = ToleranceConfig::builtin();
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
        // CPU 기준과 허용 오차(XI.11.2 config) 안입니다.
        let comparison = check_against_reference(&fixture, &first, &config, &dir);
        assert!(
            comparison.passed(),
            "{}: {}",
            fixture.name,
            comparison.summary()
        );
    }
}

#[test]
fn gpu_image_against_altered_reference_fails_with_located_diff_artifact() {
    // 검증 기준(XI.11.2): known different fixture 판정과 초과 pixel 위치 diff artifact(GPU 출력).
    if !metal_available() {
        return;
    }
    let mut harness = MetalHarness::new();
    let base = Fixture::new("altered-reference", Category::Frame, 16, 16).with(solid(
        Mesh::rect(4.0, 4.0, 12.0, 12.0),
        srgb(0.2, 0.4, 0.8, 1.0),
        Coverage::Disjoint,
    ));
    let gpu = harness.render(&base).expect("render");
    let mut altered = base.clone();
    altered.ops.push(solid(
        Mesh::rect(6.0, 6.0, 8.0, 7.0),
        srgb(1.0, 1.0, 0.0, 1.0),
        Coverage::Disjoint,
    ));
    let config = ToleranceConfig::builtin();
    let dir = artifact_dir("harness-altered");
    let comparison = check_against_reference(&altered, &gpu, &config, &dir);
    assert!(!comparison.passed());
    let mut located: Vec<(u32, u32)> = comparison.failures.iter().map(|d| (d.x, d.y)).collect();
    located.sort();
    assert_eq!(located, vec![(6, 6), (7, 6)]);
    let report = std::fs::read_to_string(dir.join("diff/altered-reference/failures.txt"))
        .expect("failures.txt");
    assert!(report.contains("failure x=6 y=6"), "{report}");
    assert!(dir.join("diff/altered-reference/diff.png").exists());
    // 같은 fixture 기준과는 통과합니다(known equal).
    assert!(check_against_reference(&base, &gpu, &config, &dir).passed());
}

/// fixture 묶음을 GPU로 그려 CPU 기준과 비교합니다. 실패한 fixture 이름과 요약을 모아 한 번에 실패합니다.
pub(crate) fn run_group(group: &str, fixtures: &[Fixture]) {
    let mut harness = MetalHarness::new();
    let config = ToleranceConfig::builtin();
    let dir = artifact_dir(group);
    let mut failed = Vec::new();
    for fixture in fixtures {
        let image = harness
            .render(fixture)
            .unwrap_or_else(|error| panic!("{}: {error}", fixture.name));
        image
            .write_reference(&dir, &fixture.name)
            .expect("write reference");
        let comparison = check_against_reference(fixture, &image, &config, &dir);
        if !comparison.passed() {
            failed.push(format!("{}: {}", fixture.name, comparison.summary()));
        }
    }
    assert!(failed.is_empty(), "{group} 실패:\n{}", failed.join("\n"));
}

#[test]
fn blend_reference_images_match_cpu_formula() {
    // 검증 기준(XI.10.2): blend reference image 통과(mode 6개 × 불투명·반투명 backdrop, 중첩, 선형 target).
    if !metal_available() {
        return;
    }
    run_group(
        "blend",
        &stellar_renderer_core::effects::inventory::blend_fixtures(),
    );
}

#[test]
fn gradient_pixel_fixtures_match_cpu_formula() {
    // 검증 기준(XI.10.3): gradient pixel fixture 통과(같은 position tie·범위 밖 stop·pad·투명 stop·P3·도형 안).
    if !metal_available() {
        return;
    }
    run_group(
        "gradient",
        &stellar_renderer_core::effects::inventory::gradient_fixtures(),
    );
}

#[test]
fn effect_fixtures_match_cpu_reference_per_feature() {
    // 검증 기준(XI.10.4): clip·mask·opacity·shadow·blur 기능별 Metal 비교 통과(D3D12는 Windows 재개 때).
    if !metal_available() {
        return;
    }
    run_group(
        "effects",
        &stellar_renderer_core::effects::inventory::effect_fixtures(),
    );
}

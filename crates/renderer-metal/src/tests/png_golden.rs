//! PNG 내보내기 decoded pixel golden 대조: CPU target과 Metal target (XX.2.4, ADR-0103 결정 7, ADR-0116 결정 4).
//!
//! 같은 snapshot·옵션으로 두 target이 PNG까지 만들고, 해석한 pixel을 연관 알파 값으로 비교합니다. 내부 pixel은
//! [`TOLERANCE`], 가장자리 pixel은 모양마다 허용치가 다릅니다. CPU target은 가로 정확 coverage·세로 16 sub-scanline이고
//! Metal target은 축마다 8배 supersampling입니다. 색 정보 chunk는 정확히 같아야 합니다. Windows는 보류(ADR-0006)라 macOS의
//! 두 target을 비교합니다.

use super::offscreen_target::*;
use std::sync::Arc;
use stellar_core_types::value::Size2;
use stellar_export::png::DecodedPng;
use stellar_export::raster::OutputProfile;
use stellar_export::request::ExportBackground;
use stellar_scene::builder::{ImageInput, NodeInput, PathInput, TextInput};
use stellar_scene::geometry::{FillRule, PathCommand, PathGeometry};
use stellar_scene::node::{CornerRadii, ImageCrop, ImageFit, ImageSampling, SceneNode};
use stellar_scene::stack::{Opacity, Paint};

/// 채널별 허용 오차(연관 알파 8-bit 값): 내부 pixel.
const TOLERANCE: u8 = 2;

/// 가장자리 pixel 허용 오차(연관 값). GPU는 축마다 8 sample(coverage 오차 1/8 이하), CPU는 가로 정확·세로 16
/// sub-scanline이라 모양 가장자리의 coverage 차이는 약 1/8(32 code value) 이하입니다.
const EDGE_TOLERANCE: u8 = 40;

/// 경계를 공유하는 그리기(Frame 배경과 그 안쪽 테두리, fill과 stroke)가 있는 scene의 가장자리 허용 오차. CPU target은 그리기마다
/// coverage를 곱해 합성하므로 공유 경계에서 아래 그리기가 비치고 알파가 커집니다(coverage 합성의 경계 번짐). GPU는 sample마다
/// 0/1로 합성해 번짐이 없습니다. coverage가 c인 경계에서 알파는 많아야 `c·(1 − c) ≤ 1/4`(64) 더 크고, 아래 색 비침도 같은
/// 크기라 연관 값 차이는 128 이하입니다.
const SHARED_EDGE_TOLERANCE: u8 = 128;

/// 내부 허용치를 넘는(가장자리) pixel의 최대 비율.
const MAX_EDGE_FRACTION: f64 = 0.12;

/// 연관 알파(encode 값 × 알파)로 옮긴 두 pixel의 채널 차이 최댓값. 알파가 작은 가장자리 pixel의 비연관 색은 coverage
/// 방식(CPU coverage 곱, GPU sample)에 따라 크게 달라도 보이는 결과는 가까우므로 연관 값으로 비교합니다.
fn premultiplied_diff(a: &[u8], b: &[u8]) -> u8 {
    let premultiply = |p: &[u8], i: usize| -> f32 {
        if i == 3 {
            f32::from(p[3])
        } else {
            f32::from(p[i]) * f32::from(p[3]) / 255.0
        }
    };
    (0..4)
        .map(|i| (premultiply(a, i) - premultiply(b, i)).abs().round() as u8)
        .max()
        .unwrap_or(0)
}

/// 허용치를 넘은 pixel(x, y, 기대, 실제).
type Exceeding = Vec<(u32, u32, [u8; 4], [u8; 4])>;

/// 비교 결과: (허용 오차를 넘는 pixel 수, 가장자리 허용치를 쓴 pixel 수, 가장 큰 차이, 처음 위치들).
type Comparison = (usize, usize, u8, Exceeding);

/// 두 해석 결과를 연관 값으로 비교합니다.
fn compare(a: &DecodedPng, b: &DecodedPng, edge_tolerance: u8) -> Comparison {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let (mut over, mut edges, mut max) = (0, 0, 0u8);
    let mut first = Vec::new();
    for (index, (pa, pb)) in a.rgba.chunks(4).zip(b.rgba.chunks(4)).enumerate() {
        let diff = premultiplied_diff(pa, pb);
        max = max.max(diff);
        if diff > TOLERANCE {
            edges += 1;
        }
        if diff > edge_tolerance {
            over += 1;
            if first.len() < 8 {
                let i = index as u32;
                first.push((
                    i % a.width,
                    i / a.width,
                    pa.try_into().unwrap(),
                    pb.try_into().unwrap(),
                ));
            }
        }
    }
    (over, edges, max, first)
}

fn assert_close(name: &str, expected: &DecodedPng, actual: &DecodedPng, edge_tolerance: u8) {
    let (over, edges, max, first) = compare(expected, actual, edge_tolerance);
    let total = f64::from(expected.width * expected.height);
    assert_eq!(
        over, 0,
        "{name}: 가장자리 허용치({edge_tolerance})도 넘는 pixel(최대 차이 {max}): {first:?}"
    );
    assert!(
        (edges as f64) / total <= MAX_EDGE_FRACTION,
        "{name}: 가장자리 pixel {edges}/{total}"
    );
    assert_eq!(actual.color, expected.color, "{name}: 색 정보");
}

/// 경계를 공유하는 그리기가 없는 scene: 둥근 Frame, 겹친 opacity 그룹, path fill, 이미지, text.
fn disjoint_roots() -> Vec<Arc<SceneNode>> {
    let mut root = frame(1, 0.0, 0.0, 48.0, 32.0, srgb(0.95, 0.95, 0.9, 1.0));
    root.corner_radii = CornerRadii::new(6.0, 6.0, 6.0, 6.0).unwrap();
    scene(&root, |b, s| {
        let mut group = frame(2, 4.0, 4.0, 20.0, 12.0, srgb(0.2, 0.6, 1.0, 1.0));
        group.node.paint.opacity = Opacity::new(0.6).unwrap();
        let mut overlap = frame(3, 8.0, 4.0, 18.0, 10.0, srgb(1.0, 0.3, 0.2, 1.0));
        overlap.corner_radii = CornerRadii::new(3.0, 0.0, 3.0, 0.0).unwrap();
        let mut group_scope = b.begin_frame(s, &group).unwrap();
        leaf(b, group_scope.scope_mut(), &overlap);
        b.finish_frame(s, group_scope).unwrap();
        let mut node = NodeInput::new(id(4), translate(28.0, 4.0));
        node.paint
            .fills
            .push(Paint::Solid(srgb(0.0, 0.5, 0.2, 0.8)));
        b.build_path(
            s,
            &PathInput {
                node,
                layout_rect: stellar_core_types::value::Rect::new(
                    p(0.0, 0.0),
                    Size2::new(16.0, 12.0).unwrap(),
                ),
                geometry: PathGeometry::new(vec![
                    PathCommand::MoveTo(p(0.0, 0.0)),
                    PathCommand::LineTo(p(16.0, 0.0)),
                    PathCommand::CubicTo {
                        c1: p(16.0, 8.0),
                        c2: p(12.0, 12.0),
                        to: p(4.0, 12.0),
                    },
                    PathCommand::Close,
                ])
                .unwrap(),
                fill_rule: FillRule::NonZero,
            },
        )
        .unwrap();
        b.build_image(
            s,
            &ImageInput {
                node: NodeInput::new(id(5), translate(4.0, 19.0)),
                asset: asset(),
                size: Size2::new(10.0, 10.0).unwrap(),
                fit: ImageFit::Cover,
                crop: ImageCrop::FULL,
                sampling: ImageSampling::Linear,
                color_space: None,
            },
        )
        .unwrap();
        b.build_text(
            s,
            &TextInput {
                node: NodeInput::new(id(6), translate(18.0, 18.0)),
                layout: LAYOUT,
                size: Size2::new(26.0, 10.0).unwrap(),
            },
        )
        .unwrap();
    })
}

#[test]
fn metal_target_matches_cpu_target_on_shared_scenes() {
    let cases = [
        (
            "srgb_1x",
            1.0,
            ExportBackground::Transparent,
            OutputProfile::Srgb,
        ),
        (
            "p3_2x",
            2.0,
            ExportBackground::Transparent,
            OutputProfile::DisplayP3,
        ),
        (
            "srgb_1_5x_background",
            1.5,
            ExportBackground::Solid(srgb(0.85, 0.9, 1.0, 1.0)),
            OutputProfile::Srgb,
        ),
    ];
    for (name, scale, background, profile) in cases {
        for (scene_name, roots, edge) in [
            ("disjoint", disjoint_roots(), EDGE_TOLERANCE),
            ("shared", shared_roots(), SHARED_EDGE_TOLERANCE),
        ] {
            let name = format!("{scene_name}_{name}");
            let export = confirmed(roots, scale, background);
            let expected = png(&export, &cpu(), profile);
            let actual = png(&export, &metal(), profile);
            write_artifact(&format!("{name}_cpu"), &expected, profile);
            write_artifact(&format!("{name}_metal"), &actual, profile);
            assert_close(&name, &expected, &actual, edge);
        }
    }
}

#[test]
fn metal_target_is_deterministic_and_tiles_without_seams() {
    // 배율 12: 출력 576×384 → tile 경계가 여럿입니다. 두 번 그려 같은 pixel이고 CPU와도 맞습니다.
    let export = confirmed(disjoint_roots(), 12.0, ExportBackground::Transparent);
    let first = png(&export, &metal(), OutputProfile::Srgb);
    let second = png(&export, &metal(), OutputProfile::Srgb);
    assert_eq!(first.rgba, second.rgba);
    let expected = png(&export, &cpu(), OutputProfile::Srgb);
    assert_close("tiled", &expected, &first, EDGE_TOLERANCE);
}

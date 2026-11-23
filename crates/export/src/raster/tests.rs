//! offscreen target 테스트 (XX.2.1).
//!
//! 크기·배율 fixture, coverage·합성·clip·stroke 정렬·이미지·text·selection·미지원 기능·취소를 CPU target으로 검사합니다.
//! 같은 helper를 PNG 테스트([`crate::png`])가 씁니다.

use std::collections::HashMap;
use std::sync::Arc;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::reference::AssetRef;
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::shadow::{BlurRadius, Shadow};
use stellar_core_types::value::{Size2, Transform2D};
use stellar_scene::builder::{
    BorderInput, FrameClipInput, FrameInput, ImageInput, NodeInput, PathInput, SceneBuilder,
    SceneScope, StrokeInput, TextInput,
};
use stellar_scene::geometry::{FillRule, PathCommand, PathGeometry};
use stellar_scene::handle::SnapshotHandle;
use stellar_scene::node::{
    CornerRadii, ImageCrop, ImageFit, ImageSampling, SceneNode, TextLayoutKey,
};
use stellar_scene::revision::SceneRevisionMap;
use stellar_scene::stack::{Opacity, Paint, SceneBlendMode};

use super::software::{unsupported_features, SoftwareTarget};
use super::*;
use crate::capture::{
    AssetRevision, EncodedImage, ExportAssetSource, ExportCapture, FontRevision, ImageMedia,
    ResourcePins, SceneSource,
};
use crate::preflight::ConfirmedExport;
use crate::request::{
    ExportBackground, ExportFormat, ExportRequest, ExportScale, ExportSource, OutputPath,
};
use crate::test_fixtures as fx;
use crate::text::{
    ExportFontFace, ExportTextSource, FontFileHash, GlyphOutline, GlyphOutlineError,
    GlyphOutlineKey, GlyphRun, GlyphVariations, PlacedGlyph, RunSize, TextGlyphLayout,
};

pub(crate) const PINS: ResourcePins = ResourcePins {
    fonts: FontRevision(1),
    assets: AssetRevision(3),
};

pub(crate) const FONT: ExportFontFace =
    ExportFontFace::new(FontFileHash::from_sha256_digest([0xA1; 32]), 0);

pub(crate) const LAYOUT: TextLayoutKey = TextLayoutKey::from_bytes([0x51; 16]);

/// 고정 revision의 asset.
#[derive(Default)]
pub(crate) struct Assets(pub(crate) HashMap<AssetRef, EncodedImage>);

impl ExportAssetSource for Assets {
    fn encoded_image(&self, asset: AssetRef, revision: AssetRevision) -> Option<EncodedImage> {
        (revision == PINS.assets)
            .then(|| self.0.get(&asset).cloned())
            .flatten()
    }
}

/// glyph 1 = 0.6em × 0.7em 사각형(baseline 위), glyph 2 = outline 없음(bitmap).
#[derive(Default)]
pub(crate) struct Glyphs(pub(crate) HashMap<TextLayoutKey, Arc<TextGlyphLayout>>);

impl ExportTextSource for Glyphs {
    fn glyph_layout(
        &self,
        layout: TextLayoutKey,
        fonts: FontRevision,
    ) -> Option<Arc<TextGlyphLayout>> {
        (fonts == PINS.fonts)
            .then(|| self.0.get(&layout).cloned())
            .flatten()
    }

    fn glyph_outline(
        &self,
        key: &GlyphOutlineKey,
        _: FontRevision,
    ) -> Result<GlyphOutline, GlyphOutlineError> {
        if key.font != FONT {
            return Err(GlyphOutlineError::FontUnavailable);
        }
        let size = key.size.px();
        match key.glyph {
            1 => Ok(GlyphOutline::Path(
                PathGeometry::new(vec![
                    PathCommand::MoveTo(fx::p(0.0, -0.7 * size)),
                    PathCommand::LineTo(fx::p(0.6 * size, -0.7 * size)),
                    PathCommand::LineTo(fx::p(0.6 * size, 0.0)),
                    PathCommand::LineTo(fx::p(0.0, 0.0)),
                    PathCommand::Close,
                ])
                .unwrap(),
            )),
            2 => Ok(GlyphOutline::NoOutline),
            _ => Ok(GlyphOutline::Empty),
        }
    }
}

/// 한 run(10px, glyph 목록, 색)의 layout.
pub(crate) fn text_layout(glyphs: &[u32], fill: Option<Color>) -> TextGlyphLayout {
    TextGlyphLayout {
        runs: vec![GlyphRun {
            font: FONT,
            size: RunSize::from_px(10.0).unwrap(),
            variations: GlyphVariations::none(),
            fill,
            glyphs: glyphs
                .iter()
                .enumerate()
                .map(|(i, glyph)| PlacedGlyph {
                    glyph: *glyph,
                    x: (i as i32) * 8 * 64,
                    y: 8 * 64,
                })
                .collect(),
        }],
    }
}

pub(crate) fn srgb(r: f32, g: f32, b: f32, a: f32) -> Color {
    fx::srgb(r, g, b, a)
}

/// 요청(View 전체).
pub(crate) fn request(scale: f32, background: ExportBackground) -> ExportRequest {
    request_for(
        ExportSource::view(fx::view_id()).unwrap(),
        scale,
        background,
    )
}

pub(crate) fn request_for(
    source: ExportSource,
    scale: f32,
    background: ExportBackground,
) -> ExportRequest {
    ExportRequest::new(
        source,
        fx::profile(),
        ExportScale::new(scale).unwrap(),
        background,
        OutputPath::new("/tmp/stellar-export/out.png").unwrap(),
        ExportFormat::Png,
    )
    .unwrap()
}

pub(crate) fn capture_of(roots: Vec<Arc<SceneNode>>, request: ExportRequest) -> ExportCapture {
    let snapshot = SceneRevisionMap::new()
        .generate(DocumentRevision(1), roots)
        .unwrap();
    ExportCapture::new(
        request,
        SceneSource {
            view: fx::view_id(),
            target_profile: fx::profile(),
            snapshot: SnapshotHandle::new(snapshot),
        },
        PINS,
    )
    .unwrap()
}

pub(crate) fn target(assets: Assets, glyphs: Glyphs) -> SoftwareTarget {
    SoftwareTarget::new(Arc::new(assets), Arc::new(glyphs))
}

pub(crate) fn render_with(
    capture: &ExportCapture,
    profile: OutputProfile,
    target: &SoftwareTarget,
) -> Result<RgbaImage, OffscreenError> {
    let export = ConfirmedExport::assume_confirmed(capture.clone());
    let request = OffscreenRequest::new(&export, profile);
    let image = target.render(&request, &CancelToken::new())?;
    check_image(&image, &request)?;
    Ok(image)
}

pub(crate) fn render(capture: &ExportCapture) -> RgbaImage {
    render_with(
        capture,
        OutputProfile::Srgb,
        &target(Assets::default(), Glyphs::default()),
    )
    .unwrap()
}

/// 단색 사각형 Frame(배경만) 입력.
pub(crate) fn frame(id: u64, x: f32, y: f32, w: f32, h: f32, color: Color) -> FrameInput {
    FrameInput {
        node: NodeInput::new(fx::node_id(id), fx::translate(x, y)),
        size: Size2::new(w, h).unwrap(),
        corner_radii: CornerRadii::ZERO,
        background: Some(color),
        border: None,
        clip: FrameClipInput::None,
    }
}

/// 자식 없는 Frame을 scope에 넣습니다.
pub(crate) fn leaf_frame(builder: &mut SceneBuilder, scope: &mut SceneScope, input: &FrameInput) {
    let frame = builder.begin_frame(scope, input).unwrap();
    builder.finish_frame(scope, frame).unwrap();
}

/// root 하나(Frame `input`) 아래 자식을 `children`이 만드는 scene.
pub(crate) fn scene(
    input: &FrameInput,
    children: impl FnOnce(&mut SceneBuilder, &mut SceneScope),
) -> Vec<Arc<SceneNode>> {
    let mut builder = SceneBuilder::new();
    let mut root = builder.root_scope();
    let mut frame = builder.begin_frame(&root, input).unwrap();
    children(&mut builder, frame.scope_mut());
    builder.finish_frame(&mut root, frame).unwrap();
    builder.finish(root)
}

pub(crate) fn transparent() -> Color {
    srgb(0.0, 0.0, 0.0, 0.0)
}

fn px(image: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
    image.pixel(x, y).unwrap()
}

#[test]
fn size_follows_export_bounds_and_scale() {
    // Root 100×80 Frame(world 20,10)과 자식 — capture 영역은 Frame 그리기 영역입니다.
    for (scale, expected) in [
        (1.0, (100, 80)),
        (2.0, (200, 160)),
        (0.5, (50, 40)),
        (0.25, (25, 20)),
    ] {
        let capture = capture_of(
            fx::basic_roots(&fx::Basic::default()),
            request(scale, ExportBackground::Transparent),
        );
        let bounds = capture.bounds();
        let ceil = |v: f32| (f64::from(v) * f64::from(scale)).ceil() as u32;
        assert_eq!(
            expected,
            (ceil(bounds.width()), ceil(bounds.height())),
            "scale {scale}"
        );
        let export = ConfirmedExport::assume_confirmed(capture.clone());
        let request = OffscreenRequest::new(&export, OutputProfile::Srgb);
        assert_eq!(request.size().unwrap(), expected, "scale {scale}");
        let image =
            target(Assets::default(), Glyphs::default()).render(&request, &CancelToken::new());
        // basic fixture의 image asset이 없어 확인 없는 missing image 오류입니다(크기 검사는 위에서 끝남).
        assert!(
            matches!(image, Err(OffscreenError::MissingImage { .. })),
            "{image:?}"
        );
    }
    // 1.5 배율의 소수 크기는 올림입니다(각 변 최소 1).
    let roots = scene(
        &frame(1, 0.0, 0.0, 3.0, 1.0, srgb(1.0, 0.0, 0.0, 1.0)),
        |_, _| {},
    );
    let image = render(&capture_of(
        roots,
        request(1.5, ExportBackground::Transparent),
    ));
    assert_eq!((image.width(), image.height()), (5, 2));
}

#[test]
fn opaque_pixel_aligned_rect_is_exact() {
    let roots = scene(
        &frame(1, 0.0, 0.0, 10.0, 5.0, srgb(1.0, 0.0, 0.0, 1.0)),
        |_, _| {},
    );
    let image = render(&capture_of(
        roots,
        request(2.0, ExportBackground::Transparent),
    ));
    assert_eq!((image.width(), image.height()), (20, 10));
    assert!(image.pixels().chunks(4).all(|p| p == [255, 0, 0, 255]));
}

#[test]
fn fractional_edges_are_antialiased_and_background_is_painted() {
    let roots = scene(&frame(1, 0.0, 0.0, 4.0, 2.0, transparent()), |b, s| {
        leaf_frame(
            b,
            s,
            &frame(2, 0.5, 0.0, 2.0, 2.0, srgb(0.0, 0.0, 1.0, 1.0)),
        );
    });
    let image = render(&capture_of(
        roots.clone(),
        request(1.0, ExportBackground::Transparent),
    ));
    assert_eq!(px(&image, 0, 0), [0, 0, 255, 128]);
    assert_eq!(px(&image, 1, 0), [0, 0, 255, 255]);
    assert_eq!(px(&image, 2, 0), [0, 0, 255, 128]);
    assert_eq!(px(&image, 3, 0), [0, 0, 0, 0]);
    // 흰 배경: 가장자리 pixel은 선형광에서 반씩 섞입니다(sRGB encode(0.5) = 188).
    let white = request(1.0, ExportBackground::Solid(srgb(1.0, 1.0, 1.0, 1.0)));
    let image = render(&capture_of(roots, white));
    assert_eq!(px(&image, 3, 0), [255, 255, 255, 255]);
    assert_eq!(px(&image, 0, 0), [188, 188, 255, 255]);
}

#[test]
fn group_opacity_composites_subtree_once() {
    // 겹치는 두 사각형을 opacity 0.5 Frame 안에 둡니다. 겹친 곳도 알파 0.5입니다(layer 합성).
    let mut group = frame(1, 0.0, 0.0, 4.0, 1.0, transparent());
    group.node.paint.opacity = Opacity::new(0.5).unwrap();
    let roots = scene(&group, |b, s| {
        leaf_frame(
            b,
            s,
            &frame(2, 0.0, 0.0, 3.0, 1.0, srgb(1.0, 0.0, 0.0, 1.0)),
        );
        leaf_frame(
            b,
            s,
            &frame(3, 1.0, 0.0, 3.0, 1.0, srgb(1.0, 0.0, 0.0, 1.0)),
        );
    });
    let image = render(&capture_of(
        roots,
        request(1.0, ExportBackground::Transparent),
    ));
    for x in 0..4 {
        assert_eq!(px(&image, x, 0), [255, 0, 0, 128], "x {x}");
    }
}

#[test]
fn frame_clip_and_inside_border() {
    // 둥근 모서리 clip Frame(10×10, radius 4) 안의 꽉 찬 자식, 테두리 1px(Inside).
    let mut clip = frame(1, 0.0, 0.0, 10.0, 10.0, transparent());
    clip.corner_radii = CornerRadii::new(4.0, 4.0, 4.0, 4.0).unwrap();
    clip.clip = FrameClipInput::Bounds;
    clip.border = Some(BorderInput {
        color: srgb(0.0, 0.0, 0.0, 1.0),
        thickness: 1.0,
    });
    let roots = scene(&clip, |b, s| {
        leaf_frame(
            b,
            s,
            &frame(2, -5.0, -5.0, 20.0, 20.0, srgb(0.0, 1.0, 0.0, 1.0)),
        );
    });
    let image = render(&capture_of(
        roots,
        request(1.0, ExportBackground::Transparent),
    ));
    assert_eq!((image.width(), image.height()), (10, 10));
    // 모서리 바깥은 clip으로 투명, 가운데는 자식 색입니다.
    assert_eq!(px(&image, 0, 0), [0, 0, 0, 0]);
    assert_eq!(px(&image, 5, 5), [0, 255, 0, 255]);
    // 테두리는 Frame 자기 내용(자식 아래)이라 가운데 변에서는 자식이 덮습니다. 자식 없는 Frame으로 다시 봅니다.
    let mut bordered = frame(1, 0.0, 0.0, 6.0, 6.0, srgb(1.0, 1.0, 1.0, 1.0));
    bordered.border = Some(BorderInput {
        color: srgb(0.0, 0.0, 0.0, 1.0),
        thickness: 1.0,
    });
    let image = render(&capture_of(
        scene(&bordered, |_, _| {}),
        request(1.0, ExportBackground::Transparent),
    ));
    // Inside 1px: 바깥 줄은 검정, 그 안은 흰색, 상자 밖으로 나가지 않음(크기 6×6).
    assert_eq!((image.width(), image.height()), (6, 6));
    assert_eq!(px(&image, 0, 3), [0, 0, 0, 255]);
    assert_eq!(px(&image, 1, 3), [255, 255, 255, 255]);
    assert_eq!(px(&image, 5, 3), [0, 0, 0, 255]);
}

#[test]
fn path_fill_rules_and_center_stroke() {
    // 같은 방향 사각형 두 개(바깥 8×8, 안 4×4)를 even-odd로 채우면 가운데가 빕니다.
    let geometry = PathGeometry::new(vec![
        PathCommand::MoveTo(fx::p(0.0, 0.0)),
        PathCommand::LineTo(fx::p(8.0, 0.0)),
        PathCommand::LineTo(fx::p(8.0, 8.0)),
        PathCommand::LineTo(fx::p(0.0, 8.0)),
        PathCommand::Close,
        PathCommand::MoveTo(fx::p(2.0, 2.0)),
        PathCommand::LineTo(fx::p(6.0, 2.0)),
        PathCommand::LineTo(fx::p(6.0, 6.0)),
        PathCommand::LineTo(fx::p(2.0, 6.0)),
        PathCommand::Close,
    ])
    .unwrap();
    let build = |rule: FillRule, stroke: bool| {
        scene(&frame(1, 0.0, 0.0, 10.0, 10.0, transparent()), |b, s| {
            let mut node = NodeInput::new(fx::node_id(2), fx::translate(1.0, 1.0));
            node.paint
                .fills
                .push(Paint::Solid(srgb(1.0, 0.0, 0.0, 1.0)));
            if stroke {
                node.paint.strokes.push(StrokeInput {
                    paint: Paint::Solid(srgb(0.0, 0.0, 1.0, 1.0)),
                    width: 2.0,
                });
            }
            b.build_path(
                s,
                &PathInput {
                    node,
                    layout_rect: fx::rect(0.0, 0.0, 8.0, 8.0),
                    geometry: geometry.clone(),
                    fill_rule: rule,
                },
            )
            .unwrap();
        })
    };
    let evenodd = render(&capture_of(
        build(FillRule::EvenOdd, false),
        request(1.0, ExportBackground::Transparent),
    ));
    let nonzero = render(&capture_of(
        build(FillRule::NonZero, false),
        request(1.0, ExportBackground::Transparent),
    ));
    assert_eq!(px(&evenodd, 5, 5), [0, 0, 0, 0]);
    assert_eq!(px(&nonzero, 5, 5), [255, 0, 0, 255]);
    assert_eq!(px(&evenodd, 2, 2), [255, 0, 0, 255]);
    // 중심 stroke 2px: 바깥 변(world x=1)은 x 0..2, 안쪽 변(world x=3)은 x 2..4를 덮습니다. 구멍 가운데(x 4..6)는
    // stroke도 fill도 없습니다.
    let stroked = render(&capture_of(
        build(FillRule::EvenOdd, true),
        request(1.0, ExportBackground::Transparent),
    ));
    assert_eq!(px(&stroked, 0, 5), [0, 0, 255, 255]);
    assert_eq!(px(&stroked, 1, 5), [0, 0, 255, 255]);
    assert_eq!(px(&stroked, 2, 5), [0, 0, 255, 255]);
    assert_eq!(px(&stroked, 4, 4), [0, 0, 0, 0]);
}

/// 2×2 PNG(왼쪽 위 빨강, 오른쪽 위 초록, 왼쪽 아래 파랑, 오른쪽 아래 반투명 흰색). 참조 구현(Python zlib + 손 PNG
/// writer)이 만든 bytes입니다.
const QUAD_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72, 0xB6, 0x0D,
    0x24, 0x00, 0x00, 0x00, 0x13, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0xF8, 0xCF, 0xC0, 0xF0,
    0x1F, 0x0C, 0x81, 0x34, 0x08, 0x34, 0x00, 0x00, 0x49, 0x49, 0x09, 0x78, 0x9C, 0x51, 0x17, 0x92,
    0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

pub(crate) fn quad_png() -> EncodedImage {
    EncodedImage::new(ImageMedia::Png, Arc::from(QUAD_PNG.to_vec()), 2, 2).unwrap()
}

pub(crate) fn image_scene(
    fit: ImageFit,
    size: (f32, f32),
    sampling: ImageSampling,
) -> Vec<Arc<SceneNode>> {
    scene(&frame(1, 0.0, 0.0, 8.0, 8.0, transparent()), |b, s| {
        b.build_image(
            s,
            &ImageInput {
                node: NodeInput::new(fx::node_id(2), Transform2D::identity()),
                asset: fx::asset(1),
                size: Size2::new(size.0, size.1).unwrap(),
                fit,
                crop: ImageCrop::FULL,
                sampling,
                color_space: None,
            },
        )
        .unwrap();
    })
}

#[test]
fn images_decode_place_and_sample() {
    let assets = || {
        let mut map = HashMap::new();
        map.insert(fx::asset(1), quad_png());
        Assets(map)
    };
    let roots = image_scene(ImageFit::Fill, (8.0, 8.0), ImageSampling::Nearest);
    let image = render_with(
        &capture_of(roots, request(1.0, ExportBackground::Transparent)),
        OutputProfile::Srgb,
        &target(assets(), Glyphs::default()),
    )
    .unwrap();
    assert_eq!(px(&image, 1, 1), [255, 0, 0, 255]);
    assert_eq!(px(&image, 6, 1), [0, 255, 0, 255]);
    assert_eq!(px(&image, 1, 6), [0, 0, 255, 255]);
    assert_eq!(px(&image, 6, 6), [255, 255, 255, 128]);
    // Contain: 4×8 상자에 2×2 원본은 4×4로 가운데(y 2..6)에 놓입니다. 위아래 여백은 투명입니다.
    let roots = image_scene(ImageFit::Contain, (4.0, 8.0), ImageSampling::Nearest);
    let image = render_with(
        &capture_of(roots, request(1.0, ExportBackground::Transparent)),
        OutputProfile::Srgb,
        &target(assets(), Glyphs::default()),
    )
    .unwrap();
    assert_eq!(px(&image, 0, 0), [0, 0, 0, 0]);
    assert_eq!(px(&image, 0, 2), [255, 0, 0, 255]);
    assert_eq!(px(&image, 3, 5), [255, 255, 255, 128]);
    // Linear sampling은 texel 중심 사이를 보간합니다(가운데 pixel은 섞인 색).
    let roots = image_scene(ImageFit::Fill, (8.0, 8.0), ImageSampling::Linear);
    let image = render_with(
        &capture_of(roots, request(1.0, ExportBackground::Transparent)),
        OutputProfile::Srgb,
        &target(assets(), Glyphs::default()),
    )
    .unwrap();
    assert_eq!(px(&image, 0, 0), [255, 0, 0, 255]);
    let mixed = px(&image, 3, 0);
    assert!(
        mixed[0] > 100 && mixed[1] > 100 && mixed[3] == 255,
        "{mixed:?}"
    );
    // JPEG asset은 근사하지 않고 미지원 오류입니다.
    let mut map = HashMap::new();
    map.insert(
        fx::asset(1),
        EncodedImage::new(ImageMedia::Jpeg, Arc::from(b"jpeg".to_vec()), 2, 2).unwrap(),
    );
    let error = render_with(
        &capture_of(
            image_scene(ImageFit::Fill, (8.0, 8.0), ImageSampling::Nearest),
            request(1.0, ExportBackground::Transparent),
        ),
        OutputProfile::Srgb,
        &target(Assets(map), Glyphs::default()),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        OffscreenError::Unsupported {
            feature: UnsupportedFeature::ImageFormat,
            ..
        }
    ));
}

#[test]
fn missing_image_requires_confirmation_to_omit() {
    let capture = capture_of(
        image_scene(ImageFit::Fill, (8.0, 8.0), ImageSampling::Nearest),
        request(1.0, ExportBackground::Transparent),
    );
    let error = render_with(
        &capture,
        OutputProfile::Srgb,
        &target(Assets::default(), Glyphs::default()),
    )
    .unwrap_err();
    assert_eq!(
        error,
        OffscreenError::MissingImage {
            node: fx::node_id(2),
            asset: fx::asset(1)
        }
    );
    // 사용자가 missing image를 확인하면 pixel 없이 진행합니다.
    let preflight = crate::preflight::ExportPreflight::run(
        capture,
        &crate::preflight::PreflightInput {
            capabilities: &crate::preflight::TargetCapabilityReport::Validated {
                profile: fx::profile(),
                findings: Vec::new(),
            },
            font_fallbacks: &[],
            assets: &Assets::default(),
            text: &Glyphs::default(),
            rasterizer: &crate::svg::NoRasterizer,
        },
    )
    .unwrap();
    let confirmation = preflight.confirm();
    let export = preflight.proceed(Some(&confirmation)).unwrap();
    let request = OffscreenRequest::new(&export, OutputProfile::Srgb);
    let image = target(Assets::default(), Glyphs::default())
        .render(&request, &CancelToken::new())
        .unwrap();
    assert!(image.pixels().iter().all(|b| *b == 0));
}

pub(crate) fn text_scene(stroke: bool) -> Vec<Arc<SceneNode>> {
    scene(&frame(1, 0.0, 0.0, 24.0, 12.0, transparent()), |b, s| {
        let mut node = NodeInput::new(fx::node_id(2), fx::translate(2.0, 2.0));
        if stroke {
            node.paint.strokes.push(StrokeInput {
                paint: Paint::Solid(srgb(0.0, 0.0, 0.0, 1.0)),
                width: 1.0,
            });
        }
        b.build_text(
            s,
            &TextInput {
                node,
                layout: LAYOUT,
                size: Size2::new(20.0, 8.0).unwrap(),
            },
        )
        .unwrap();
    })
}

#[test]
fn text_glyph_outlines_are_filled_with_run_color() {
    let mut layouts = HashMap::new();
    layouts.insert(
        LAYOUT,
        Arc::new(text_layout(&[1, 3, 1], Some(srgb(1.0, 0.0, 0.0, 1.0)))),
    );
    let image = render_with(
        &capture_of(
            text_scene(false),
            request(1.0, ExportBackground::Transparent),
        ),
        OutputProfile::Srgb,
        &target(Assets::default(), Glyphs(layouts)),
    )
    .unwrap();
    // glyph 1: pen (2 + 0, 2 + 8) 기준 6×7 → pixel x 2..8, y 3..10. glyph 3은 빈 glyph, 다음 glyph 1은 x 18..24.
    assert_eq!(px(&image, 4, 6), [255, 0, 0, 255]);
    assert_eq!(px(&image, 12, 6), [0, 0, 0, 0]);
    assert_eq!(px(&image, 20, 6), [255, 0, 0, 255]);
    assert_eq!(px(&image, 4, 1), [0, 0, 0, 0]);
    // outline 없는 glyph는 미지원 오류입니다(조용히 빼지 않음).
    let mut layouts = HashMap::new();
    layouts.insert(LAYOUT, Arc::new(text_layout(&[2], None)));
    let error = render_with(
        &capture_of(
            text_scene(false),
            request(1.0, ExportBackground::Transparent),
        ),
        OutputProfile::Srgb,
        &target(Assets::default(), Glyphs(layouts)),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        OffscreenError::Unsupported {
            feature: UnsupportedFeature::GlyphWithoutOutline,
            ..
        }
    ));
    // layout 없음은 확인하지 않으면 오류입니다.
    let error = render_with(
        &capture_of(
            text_scene(false),
            request(1.0, ExportBackground::Transparent),
        ),
        OutputProfile::Srgb,
        &target(Assets::default(), Glyphs::default()),
    )
    .unwrap_err();
    assert!(matches!(error, OffscreenError::MissingTextLayout { .. }));
}

#[test]
fn unrendered_effects_are_errors_not_approximations() {
    type Apply = Box<dyn Fn(&mut NodeInput)>;
    let cases: [(&str, Apply, UnsupportedFeature); 3] = [
        (
            "shadow",
            Box::new(|node: &mut NodeInput| {
                node.paint.shadows.push(Shadow::new(
                    fx::p(1.0, 1.0),
                    BlurRadius::new(2.0).unwrap(),
                    srgb(0.0, 0.0, 0.0, 0.5),
                ));
            }),
            UnsupportedFeature::Shadow,
        ),
        (
            "blur",
            Box::new(|node: &mut NodeInput| node.paint.layer_blur = BlurRadius::new(1.0).unwrap()),
            UnsupportedFeature::LayerBlur,
        ),
        (
            "blend",
            Box::new(|node: &mut NodeInput| node.paint.blend = SceneBlendMode::Multiply),
            UnsupportedFeature::BlendMode,
        ),
    ];
    for (name, apply, feature) in cases {
        let mut child = frame(2, 0.0, 0.0, 2.0, 2.0, srgb(1.0, 0.0, 0.0, 1.0));
        apply(&mut child.node);
        let roots = scene(&frame(1, 0.0, 0.0, 4.0, 4.0, transparent()), |b, s| {
            leaf_frame(b, s, &child)
        });
        let capture = capture_of(roots, request(1.0, ExportBackground::Transparent));
        assert_eq!(
            unsupported_features(&capture),
            vec![(fx::node_id(2), feature)],
            "{name}"
        );
        let error = render_with(
            &capture,
            OutputProfile::Srgb,
            &target(Assets::default(), Glyphs::default()),
        )
        .unwrap_err();
        assert_eq!(
            error,
            OffscreenError::Unsupported {
                node: fx::node_id(2),
                feature
            },
            "{name}"
        );
    }
}

#[test]
fn selection_ignores_ancestor_clip_and_opacity() {
    let mut parent = frame(1, 0.0, 0.0, 4.0, 4.0, transparent());
    parent.clip = FrameClipInput::Bounds;
    parent.node.paint.opacity = Opacity::new(0.25).unwrap();
    let roots = scene(&parent, |b, s| {
        leaf_frame(
            b,
            s,
            &frame(2, 2.0, 2.0, 4.0, 4.0, srgb(0.0, 1.0, 0.0, 1.0)),
        );
    });
    let source = ExportSource::selection(fx::view_id(), [fx::node_id(2)]).unwrap();
    let image = render(&capture_of(
        roots,
        request_for(source, 1.0, ExportBackground::Transparent),
    ));
    // 선택 node 전체(4×4)가 조상 clip·opacity 없이 나옵니다.
    assert_eq!((image.width(), image.height()), (4, 4));
    assert!(image.pixels().chunks(4).all(|p| p == [0, 255, 0, 255]));
}

#[test]
fn display_p3_output_converts_colors() {
    let roots = scene(
        &frame(1, 0.0, 0.0, 2.0, 2.0, srgb(1.0, 0.0, 0.0, 1.0)),
        |b, s| {
            let p3 = Color::new(ColorSpace::DisplayP3, 1.0, 0.0, 0.0, 1.0).unwrap();
            leaf_frame(b, s, &frame(2, 1.0, 0.0, 1.0, 2.0, p3));
        },
    );
    let capture = capture_of(roots, request(1.0, ExportBackground::Transparent));
    let target = target(Assets::default(), Glyphs::default());
    let p3 = render_with(&capture, OutputProfile::DisplayP3, &target).unwrap();
    assert_eq!(p3.profile(), OutputProfile::DisplayP3);
    // sRGB 빨강은 P3 안: encode(P3 선형) ≈ (0.9175, 0.2003, 0.1386) → (234, 51, 35).
    let red = px(&p3, 0, 0);
    for (got, want) in red.iter().zip([234u8, 51, 35, 255]) {
        assert!(got.abs_diff(want) <= 1, "{red:?}");
    }
    assert_eq!(px(&p3, 1, 0), [255, 0, 0, 255]);
    // sRGB 출력에서 P3 빨강은 gamut 밖이라 잘립니다(255, 0, 0).
    let srgb_out = render_with(&capture, OutputProfile::Srgb, &target).unwrap();
    assert_eq!(px(&srgb_out, 0, 0), [255, 0, 0, 255]);
    assert_eq!(px(&srgb_out, 1, 0), [255, 0, 0, 255]);
}

#[test]
fn rendering_is_deterministic_and_cancellable() {
    let roots = fx::basic_roots(&fx::Basic::default());
    let capture = capture_of(
        roots,
        request(2.0, ExportBackground::Solid(srgb(0.9, 0.9, 0.9, 1.0))),
    );
    let mut map = HashMap::new();
    map.insert(fx::asset(1), quad_png());
    let target = SoftwareTarget::new(Arc::new(Assets(map)), Arc::new(Glyphs::default()));
    let a = render_with(&capture, OutputProfile::Srgb, &target).unwrap();
    let b = render_with(&capture, OutputProfile::Srgb, &target).unwrap();
    assert_eq!(a, b);
    let export = ConfirmedExport::assume_confirmed(capture);
    let request = OffscreenRequest::new(&export, OutputProfile::Srgb);
    let cancel = CancelToken::new();
    cancel.cancel();
    assert_eq!(
        target.render(&request, &cancel),
        Err(OffscreenError::Cancelled)
    );
}

#[test]
fn oversized_output_is_rejected_before_rendering() {
    let roots = scene(
        &frame(1, 0.0, 0.0, 2000.0, 2000.0, srgb(1.0, 0.0, 0.0, 1.0)),
        |_, _| {},
    );
    let capture = capture_of(roots, request(4.0, ExportBackground::Transparent));
    let export = ConfirmedExport::assume_confirmed(capture);
    let request = OffscreenRequest::new(&export, OutputProfile::Srgb);
    assert_eq!(
        request.size(),
        Err(OffscreenError::TooLarge {
            width: 8000,
            height: 8000
        })
    );
    let error = target(Assets::default(), Glyphs::default()).render(&request, &CancelToken::new());
    assert!(matches!(error, Err(OffscreenError::TooLarge { .. })));
}

#[test]
fn profile_names_round_trip() {
    for profile in OutputProfile::ALL {
        assert_eq!(OutputProfile::from_name(profile.name()), Ok(profile));
    }
    assert!(OutputProfile::from_name("rec2020").is_err());
    assert!(OutputProfile::from_name("SRGB").is_err());
}

#[test]
fn check_image_rejects_contract_violations() {
    let roots = scene(
        &frame(1, 0.0, 0.0, 2.0, 2.0, srgb(1.0, 0.0, 0.0, 1.0)),
        |_, _| {},
    );
    let capture = capture_of(roots, request(1.0, ExportBackground::Transparent));
    let export = ConfirmedExport::assume_confirmed(capture);
    let request = OffscreenRequest::new(&export, OutputProfile::Srgb);
    let wrong_size = RgbaImage::new(3, 2, OutputProfile::Srgb, vec![0; 24]).unwrap();
    assert!(matches!(
        check_image(&wrong_size, &request),
        Err(OffscreenError::Contract { .. })
    ));
    let wrong_profile = RgbaImage::new(2, 2, OutputProfile::DisplayP3, vec![0; 16]).unwrap();
    assert!(matches!(
        check_image(&wrong_profile, &request),
        Err(OffscreenError::Contract { .. })
    ));
    assert!(RgbaImage::new(2, 2, OutputProfile::Srgb, vec![0; 15]).is_none());
}

#[test]
fn stroke_alignment_inside_and_outside() {
    use stellar_scene::node::{HitTestMeta, RectPrimitive, SceneNodeSpec, ZOrder};
    use stellar_scene::stack::{
        ClipStackRef, PaintLayer, PaintStack, PaintStackRef, StrokeAlign, StrokeCap, StrokeLayer,
    };
    use stellar_scene::SceneContent;
    // 4×4 사각형(world 2..6)에 폭 1 stroke. 내보내기 영역은 그리기 영역(Outside면 1..7)입니다.
    let node = |align: StrokeAlign| {
        Arc::new(SceneNode::leaf(SceneNodeSpec {
            source: fx::node_id(1),
            content: SceneContent::Rect(RectPrimitive {
                rect: fx::rect(0.0, 0.0, 4.0, 4.0),
                radii: CornerRadii::ZERO,
            }),
            world_transform: fx::translate(2.0, 2.0),
            layout_rect: fx::rect(0.0, 0.0, 4.0, 4.0),
            clip: ClipStackRef::none(),
            paint: PaintStackRef::new(PaintStack {
                layers: vec![
                    PaintLayer::Fill(Paint::Solid(srgb(1.0, 1.0, 1.0, 1.0))),
                    PaintLayer::Stroke(
                        StrokeLayer::new(
                            Paint::Solid(srgb(0.0, 0.0, 0.0, 1.0)),
                            1.0,
                            align,
                            StrokeCap::Butt,
                        )
                        .unwrap(),
                    ),
                ],
                ..PaintStack::empty()
            }),
            z: ZOrder(1),
            hit: HitTestMeta::INTERACTIVE,
        }))
    };
    let inside = render(&capture_of(
        vec![node(StrokeAlign::Inside)],
        request(1.0, ExportBackground::Transparent),
    ));
    assert_eq!((inside.width(), inside.height()), (4, 4));
    assert_eq!(px(&inside, 0, 2), [0, 0, 0, 255]);
    assert_eq!(px(&inside, 1, 2), [255, 255, 255, 255]);
    let outside = render(&capture_of(
        vec![node(StrokeAlign::Outside)],
        request(1.0, ExportBackground::Transparent),
    ));
    // 영역 1..7 → 6×6. stroke는 사각형 바깥 1px 띠이고 안쪽은 흰 fill 그대로입니다.
    assert_eq!((outside.width(), outside.height()), (6, 6));
    assert_eq!(px(&outside, 0, 3), [0, 0, 0, 255]);
    assert_eq!(px(&outside, 1, 3), [255, 255, 255, 255]);
    assert_eq!(px(&outside, 5, 3), [0, 0, 0, 255]);
    assert_eq!(px(&outside, 4, 3), [255, 255, 255, 255]);
}

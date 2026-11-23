//! Metal offscreen target (XX.2.1, ADR-0116 결정 4).
//!
//! - 결정론(같은 요청 같은 pixel)과 tile 경계. CPU target과의 golden 대조는 [`super::png_golden`]입니다.
//! - CPU target이 아직 그리지 않는 효과(shadow, layer blur, Normal이 아닌 blend)는 같은 그리기 목록을 renderer-core CPU 기준
//!   rasterizer로 그린 결과와 비교합니다(같은 수식·순서, ADR-0102).
//! - selection 미지원·취소를 확인합니다.
//!
//! 결과 PNG는 `target/render-conformance/png-export/` 아래에 씁니다(검토용).

use std::collections::HashMap;
use std::sync::Arc;

use stellar_core_types::color::Color;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::shadow::{BlurRadius, Shadow};
use stellar_core_types::value::Size2;
use stellar_export::capture::{
    AssetRevision, EncodedImage, ExportAssetSource, ExportCapture, FontRevision, ImageMedia,
    ResourcePins, SceneSource,
};
use stellar_export::png::{
    decode_png, encode_png, render_png, DecodeLimits, DecodedPng, PngBitDepth, PngOptions,
};
use stellar_export::preflight::{
    ConfirmedExport, ExportPreflight, PreflightInput, TargetCapabilityReport,
};
use stellar_export::raster::software::SoftwareTarget;
use stellar_export::raster::{
    CancelToken, OffscreenError, OffscreenRequest, OffscreenTarget, OutputProfile, RgbaImage,
};
use stellar_export::request::{
    ExportBackground, ExportFormat, ExportRequest, ExportScale, ExportSource, OutputPath,
    TargetProfileKey,
};
use stellar_export::svg::NoRasterizer;
use stellar_export::text::{
    ExportFontFace, ExportTextSource, FontFileHash, GlyphOutline, GlyphOutlineError,
    GlyphOutlineKey, GlyphRun, GlyphVariations, PlacedGlyph, RunSize, TextGlyphLayout,
};
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
use stellar_scene::{ColorSpace, StableId, Transform2D, Vec2};

use crate::offscreen_target::{ExportMemoryReport, MetalOffscreenTarget, SUPERSAMPLE};

pub(super) const PINS: ResourcePins = ResourcePins {
    fonts: FontRevision(1),
    assets: AssetRevision(1),
};

const FONT: ExportFontFace = ExportFontFace::new(FontFileHash::from_sha256_digest([0xB2; 32]), 0);

pub(super) const LAYOUT: TextLayoutKey = TextLayoutKey::from_bytes([0x61; 16]);

pub(super) fn id(n: u64) -> StableId {
    StableId::new_v7(0x0195_c530_3acd, 0x7bc, 0x4000 + n)
}

pub(super) fn view_id() -> StableId {
    id(0xFFFF)
}

pub(super) fn profile() -> TargetProfileKey {
    TargetProfileKey::new("base").unwrap()
}

pub(super) fn srgb(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color::new(ColorSpace::Srgb, r, g, b, a).unwrap()
}

pub(super) fn translate(x: f32, y: f32) -> Transform2D {
    Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y).unwrap()
}

pub(super) fn p(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y).unwrap()
}

pub(super) fn asset() -> AssetRef {
    AssetRef::new(id(0xA55E))
}

pub(super) struct Assets(HashMap<AssetRef, EncodedImage>);

impl ExportAssetSource for Assets {
    fn encoded_image(&self, asset: AssetRef, revision: AssetRevision) -> Option<EncodedImage> {
        (revision == PINS.assets)
            .then(|| self.0.get(&asset).cloned())
            .flatten()
    }
}

/// glyph 1 = 0.6em × 0.7em 사각형(baseline 위), glyph 2 = 삼각형.
pub(super) struct Glyphs(HashMap<TextLayoutKey, Arc<TextGlyphLayout>>);

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
        let commands = match key.glyph {
            1 => vec![
                PathCommand::MoveTo(p(0.0, -0.7 * size)),
                PathCommand::LineTo(p(0.6 * size, -0.7 * size)),
                PathCommand::LineTo(p(0.6 * size, 0.0)),
                PathCommand::LineTo(p(0.0, 0.0)),
                PathCommand::Close,
            ],
            _ => vec![
                PathCommand::MoveTo(p(0.0, 0.0)),
                PathCommand::LineTo(p(0.3 * size, -0.7 * size)),
                PathCommand::LineTo(p(0.6 * size, 0.0)),
                PathCommand::Close,
            ],
        };
        Ok(GlyphOutline::Path(PathGeometry::new(commands).unwrap()))
    }
}

pub(super) fn glyphs() -> Glyphs {
    let layout = TextGlyphLayout {
        runs: vec![GlyphRun {
            font: FONT,
            size: RunSize::from_px(9.0).unwrap(),
            variations: GlyphVariations::none(),
            fill: Some(srgb(0.3, 0.1, 0.5, 1.0)),
            glyphs: [1, 2, 1]
                .iter()
                .enumerate()
                .map(|(i, glyph)| PlacedGlyph {
                    glyph: *glyph,
                    x: (i as i32) * 7 * 64,
                    y: 8 * 64,
                })
                .collect(),
        }],
    };
    Glyphs(HashMap::from([(LAYOUT, Arc::new(layout))]))
}

/// 4×4 색 이미지(PNG).
pub(super) fn assets() -> Assets {
    let mut pixels = Vec::new();
    for y in 0..4u8 {
        for x in 0..4u8 {
            pixels.extend_from_slice(&[40 + x * 60, 200 - y * 40, 90 + x * 20 + y * 20, 255]);
        }
    }
    let image = RgbaImage::new(4, 4, OutputProfile::Srgb, pixels).unwrap();
    let bytes = encode_png(&image, 1.0, &CancelToken::new()).unwrap();
    Assets(HashMap::from([(
        asset(),
        EncodedImage::new(ImageMedia::Png, Arc::from(bytes), 4, 4).unwrap(),
    )]))
}

pub(super) fn frame(n: u64, x: f32, y: f32, w: f32, h: f32, color: Color) -> FrameInput {
    FrameInput {
        node: NodeInput::new(id(n), translate(x, y)),
        size: Size2::new(w, h).unwrap(),
        corner_radii: CornerRadii::ZERO,
        background: Some(color),
        border: None,
        clip: FrameClipInput::None,
    }
}

pub(super) fn leaf(builder: &mut SceneBuilder, scope: &mut SceneScope, input: &FrameInput) {
    let frame = builder.begin_frame(scope, input).unwrap();
    builder.finish_frame(scope, frame).unwrap();
}

pub(super) fn scene(
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

/// 공통 scene: 둥근 clip Frame(테두리) 안 opacity 그룹, path fill·stroke, 이미지, text.
pub(super) fn shared_roots() -> Vec<Arc<SceneNode>> {
    let mut root = frame(1, 0.0, 0.0, 48.0, 32.0, srgb(0.95, 0.95, 0.9, 1.0));
    root.corner_radii = CornerRadii::new(6.0, 6.0, 6.0, 6.0).unwrap();
    root.clip = FrameClipInput::Bounds;
    root.border = Some(BorderInput {
        color: srgb(0.1, 0.1, 0.3, 1.0),
        thickness: 1.5,
    });
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
        node.paint.strokes.push(StrokeInput {
            paint: Paint::Solid(srgb(0.0, 0.0, 0.0, 1.0)),
            width: 1.25,
        });
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

/// CPU target이 그리지 않는 효과 scene: shadow, layer blur, multiply, 겹친 opacity.
fn effect_roots() -> Vec<Arc<SceneNode>> {
    let root = frame(1, 0.0, 0.0, 50.0, 28.0, srgb(0.9, 0.85, 0.7, 1.0));
    scene(&root, |b, s| {
        let mut card = frame(2, 4.0, 4.0, 14.0, 10.0, srgb(1.0, 1.0, 1.0, 1.0));
        card.node.paint.shadows.push(Shadow::new(
            p(1.0, 2.0),
            BlurRadius::new(2.0).unwrap(),
            srgb(0.0, 0.0, 0.2, 0.6),
        ));
        leaf(b, s, &card);
        let mut blurred = frame(3, 22.0, 4.0, 14.0, 10.0, srgb(0.1, 0.4, 0.9, 1.0));
        blurred.node.paint.layer_blur = BlurRadius::new(1.5).unwrap();
        leaf(b, s, &blurred);
        // 배경 없는 Frame의 box-shadow(상자 모양, 상자 안 비움)와 소수 offset(ADR-0130 결정 1·3).
        let mut hollow = frame(7, 39.0, 17.0, 7.0, 7.0, srgb(0.0, 0.0, 0.0, 0.0));
        hollow.background = None;
        hollow.corner_radii = CornerRadii::new(2.0, 2.0, 2.0, 2.0).unwrap();
        hollow.node.paint.shadows.push(Shadow::new(
            p(1.25, 0.75),
            BlurRadius::new(1.5).unwrap(),
            srgb(0.6, 0.0, 0.1, 0.8),
        ));
        leaf(b, s, &hollow);
        let mut multiply = frame(4, 6.0, 16.0, 28.0, 10.0, srgb(0.0, 0.0, 0.0, 0.0));
        multiply.background = None;
        multiply.node.paint.blend = SceneBlendMode::Multiply;
        let mut scope = b.begin_frame(s, &multiply).unwrap();
        leaf(
            b,
            scope.scope_mut(),
            &frame(5, 0.0, 0.0, 16.0, 10.0, srgb(1.0, 0.2, 0.2, 1.0)),
        );
        leaf(
            b,
            scope.scope_mut(),
            &frame(6, 10.0, 0.0, 18.0, 10.0, srgb(0.2, 1.0, 0.2, 1.0)),
        );
        b.finish_frame(s, scope).unwrap();
    })
}

/// 흔한 크기의 box-shadow(blur 40 → σ 20)와 layer blur(반경 12): 배수 8이면 supersample σ가 상한(64)을 넘어 효과만 낮은
/// 해상도로 그립니다(ADR-0130 결정 2, ADR-0136).
fn large_blur_roots() -> Vec<Arc<SceneNode>> {
    let root = frame(1, 0.0, 0.0, 120.0, 90.0, srgb(0.95, 0.95, 0.9, 1.0));
    scene(&root, |b, s| {
        let mut card = frame(2, 20.0, 20.0, 40.0, 30.0, srgb(1.0, 1.0, 1.0, 1.0));
        card.node.paint.shadows.push(Shadow::new(
            p(4.0, 6.0),
            BlurRadius::new(40.0).unwrap(),
            srgb(0.0, 0.0, 0.2, 0.5),
        ));
        leaf(b, s, &card);
        let mut blurred = frame(3, 70.0, 40.0, 30.0, 30.0, srgb(0.1, 0.4, 0.9, 1.0));
        blurred.node.paint.layer_blur = BlurRadius::new(12.0).unwrap();
        leaf(b, s, &blurred);
    })
}

pub(super) fn confirmed(
    roots: Vec<Arc<SceneNode>>,
    scale: f32,
    background: ExportBackground,
) -> ConfirmedExport {
    let snapshot = SceneRevisionMap::new()
        .generate(DocumentRevision(1), roots)
        .unwrap();
    let request = ExportRequest::new(
        ExportSource::view(view_id()).unwrap(),
        profile(),
        ExportScale::new(scale).unwrap(),
        background,
        OutputPath::new("/tmp/stellar-metal-export/out.png").unwrap(),
        ExportFormat::Png,
    )
    .unwrap();
    let capture = ExportCapture::new(
        request,
        SceneSource {
            view: view_id(),
            target_profile: profile(),
            snapshot: SnapshotHandle::new(snapshot),
        },
        PINS,
    )
    .unwrap();
    let report = TargetCapabilityReport::Validated {
        profile: profile(),
        findings: Vec::new(),
    };
    let preflight = ExportPreflight::run(
        capture,
        &PreflightInput {
            capabilities: &report,
            font_fallbacks: &[],
            assets: &assets(),
            text: &glyphs(),
            rasterizer: &NoRasterizer,
        },
    )
    .unwrap();
    preflight.proceed(None).unwrap()
}

pub(super) fn metal() -> MetalOffscreenTarget {
    MetalOffscreenTarget::new(Arc::new(assets()), Arc::new(glyphs()))
}

pub(super) fn cpu() -> SoftwareTarget {
    SoftwareTarget::new(Arc::new(assets()), Arc::new(glyphs()))
}

pub(super) fn png(
    export: &ConfirmedExport,
    target: &dyn OffscreenTarget,
    profile: OutputProfile,
) -> DecodedPng {
    let options = PngOptions::new(profile, PngBitDepth::Eight).unwrap();
    let file = render_png(export, target, options, &CancelToken::new()).unwrap();
    decode_png(file.bytes(), DecodeLimits::default()).unwrap()
}

pub(super) fn write_artifact(name: &str, image: &DecodedPng, profile: OutputProfile) {
    let dir = super::conformance::artifact_dir("png-export");
    let _ = std::fs::create_dir_all(&dir);
    let rgba = RgbaImage::new(image.width, image.height, profile, image.rgba.clone()).unwrap();
    if let Ok(bytes) = encode_png(&rgba, 1.0, &CancelToken::new()) {
        let _ = std::fs::write(dir.join(format!("{name}.png")), bytes);
    }
}

/// effect scene을 Metal target과 같은 경로(frame plan → 그리기 목록)로 CPU 기준 rasterizer에 그립니다(tile 하나).
fn cpu_reference_of(export: &ConfirmedExport, s: u32) -> DecodedPng {
    use stellar_renderer_core::clip_plan::ClipCapabilities;
    use stellar_renderer_core::color::decode_pixel;
    use stellar_renderer_core::effects::fixture::{Category, Fixture};
    use stellar_renderer_core::effects::raster::{draw_ops, encode_canvas, Canvas};
    use stellar_renderer_core::effects::scene::encode_plan;
    use stellar_renderer_core::output_color::OutputColor;
    use stellar_renderer_core::pass_compiler::{
        compile_frame, CompileInput, EffectBoundaries, LAYER_FORMAT,
    };
    use stellar_renderer_core::render_graph::{ClearColor, SampleCount, TextureFormat};
    use stellar_scene::visibility::Viewport;

    let request = OffscreenRequest::new(export, OutputProfile::Srgb);
    let (width, height) = request.size().unwrap();
    let grid = request.grid();
    let origin = grid.world_rect(&grid.canvas());
    let k = (grid.scale() * f64::from(s)) as f32;
    let view = Transform2D::new(
        k,
        0.0,
        0.0,
        k,
        (-origin[0] * f64::from(k)) as f32,
        (-origin[1] * f64::from(k)) as f32,
    )
    .unwrap();
    let (sw, sh) = (width * s, height * s);
    let viewport = Viewport::new(Size2::new(sw as f32, sh as f32).unwrap(), view).unwrap();
    let plan = compile_frame(&CompileInput {
        snapshot: export.capture().snapshot(),
        viewport: &viewport,
        visible: None,
        surface_format: LAYER_FORMAT,
        clear: ClearColor::TRANSPARENT,
        boundaries: EffectBoundaries::Minimal,
        clip_caps: ClipCapabilities {
            stencil_bits: 8,
            samples: SampleCount::X4,
        },
    })
    .unwrap();
    struct Nothing;
    impl stellar_renderer_core::effects::scene::FrameResources for Nothing {
        fn image(
            &self,
            _: &SceneNode,
            _: &stellar_scene::node::ImagePrimitive,
        ) -> Result<
            Option<Arc<stellar_renderer_core::effects::fixture::ImageData>>,
            stellar_renderer_core::effects::scene::EncodeError,
        > {
            Ok(None)
        }
        fn text(
            &self,
            _: &SceneNode,
            _: &stellar_scene::node::TextPrimitive,
        ) -> Result<
            Option<Vec<stellar_renderer_core::effects::scene::GlyphShape>>,
            stellar_renderer_core::effects::scene::EncodeError,
        > {
            Ok(None)
        }
    }
    let ops = encode_plan(&plan, &viewport, &Nothing).unwrap();
    // 크기가 fixture 상한(512)을 넘을 수 있어 canvas에 직접 그립니다(같은 CPU 기준 rasterizer).
    let mut fixture = Fixture::new("effects", Category::Modifier, 1, 1);
    fixture.requested = ColorSpace::Srgb;
    fixture.display_p3 = true;
    fixture.format = TextureFormat::Rgba8UnormSrgb;
    fixture.clear = OutputColor {
        space: ColorSpace::Srgb,
        rgb: [0.0; 3],
        alpha: 0.0,
    };
    let color_plan = fixture.plan();
    let mut canvas = Canvas::stored(
        sw,
        sh,
        fixture.clear_working(),
        fixture.format,
        color_plan.surface(),
    );
    draw_ops(&mut canvas, &ops, &color_plan).unwrap();
    let image = encode_canvas(&canvas, &fixture);
    // 4×4 상자 평균 → 비연관 → encode(Metal target과 같은 마지막 단계).
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.0f32; 4];
            for sy in 0..s {
                for sx in 0..s {
                    let c = decode_pixel(
                        &image.pixel(x * s + sx, y * s + sy),
                        fixture.format,
                        ColorSpace::Srgb,
                    )
                    .unwrap();
                    for (acc, v) in sum.iter_mut().zip(c.rgba) {
                        *acc += v;
                    }
                }
            }
            let n = (s * s) as f32;
            let alpha = (sum[3] / n).clamp(0.0, 1.0);
            let at = ((y * width + x) * 4) as usize;
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            rgba[at + 3] = q(alpha);
            if rgba[at + 3] > 0 {
                for i in 0..3 {
                    rgba[at + i] =
                        q(ColorSpace::Srgb.encode_channel((sum[i] / n / alpha).clamp(0.0, 1.0)));
                }
            }
        }
    }
    let rgba_image = RgbaImage::new(width, height, OutputProfile::Srgb, rgba).unwrap();
    let bytes = encode_png(&rgba_image, 1.0, &CancelToken::new()).unwrap();
    decode_png(&bytes, DecodeLimits::default()).unwrap()
}

#[test]
fn metal_target_draws_effects_like_the_cpu_reference() {
    let export = confirmed(effect_roots(), 1.0, ExportBackground::Transparent);
    // CPU target은 이 효과를 근사하지 않고 거부합니다(ADR-0103 결정 2).
    let request = OffscreenRequest::new(&export, OutputProfile::Srgb);
    assert!(matches!(
        cpu().render(&request, &CancelToken::new()),
        Err(OffscreenError::Unsupported { .. })
    ));
    let actual = png(&export, &metal(), OutputProfile::Srgb);
    let expected = cpu_reference_of(&export, SUPERSAMPLE);
    write_artifact("effects_srgb_1x_metal", &actual, OutputProfile::Srgb);
    write_artifact("effects_srgb_1x_reference", &expected, OutputProfile::Srgb);
    let mut over = 0;
    let mut worst = 0;
    for (pa, pb) in expected.rgba.chunks(4).zip(actual.rgba.chunks(4)) {
        let diff = pa
            .iter()
            .zip(pb)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap();
        worst = worst.max(diff);
        // 8-bit sRGB layer 저장(CPU 기준)과 half float(GPU)의 정밀도 차이.
        if diff > 3 {
            over += 1;
        }
    }
    assert_eq!(over, 0, "최대 차이 {worst}");
}

/// 연관 알파 값(RGB × 알파). 그림자 끝의 알파가 몇 단계뿐인 pixel은 비연관 RGB가 크게 흔들리므로 이 값으로 비교합니다.
fn associated(p: &[u8]) -> [i32; 4] {
    let a = i32::from(p[3]);
    [
        (i32::from(p[0]) * a + 127) / 255,
        (i32::from(p[1]) * a + 127) / 255,
        (i32::from(p[2]) * a + 127) / 255,
        a,
    ]
}

/// 두 image의 연관 값 차이가 `tolerance`를 넘는 pixel 수와 최대 차이.
fn associated_diff(a: &DecodedPng, b: &DecodedPng, tolerance: u32) -> (usize, u32) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let mut over = 0;
    let mut worst = 0;
    for (pa, pb) in a.rgba.chunks(4).zip(b.rgba.chunks(4)) {
        let (ea, eb) = (associated(pa), associated(pb));
        let diff = ea.iter().zip(eb).map(|(x, y)| x.abs_diff(y)).max().unwrap();
        worst = worst.max(diff);
        if diff > tolerance {
            over += 1;
        }
    }
    (over, worst)
}

/// Metal target으로 그려 PNG decode 결과와 메모리 측정을 돌려줍니다.
fn measured(
    export: &ConfirmedExport,
    target: &MetalOffscreenTarget,
) -> Result<(DecodedPng, ExportMemoryReport), OffscreenError> {
    let request = OffscreenRequest::new(export, OutputProfile::Srgb);
    let (image, report) = target.render_measured(&request, &CancelToken::new())?;
    let bytes = encode_png(&image, 1.0, &CancelToken::new()).unwrap();
    Ok((decode_png(&bytes, DecodeLimits::default()).unwrap(), report))
}

/// 불투명 pixel이 처음 나오는 자리(root 왼쪽 위). 내보내기 영역은 그림자 번짐만큼 root보다 넓습니다.
fn root_origin(image: &DecodedPng) -> (u32, u32) {
    image
        .rgba
        .chunks(4)
        .position(|p| p[3] == 255)
        .map(|i| (i as u32 % image.width, i as u32 / image.width))
        .unwrap()
}

/// 큰 blur도 supersampling 배수 8로 그립니다. 효과만 낮은 해상도로 그리고(ADR-0130 결정 2) 효과 여백은 target에 넣지 않고
/// 효과 window에 그리므로(ADR-0136) 배율 4(출력 σ 80, 여백 약 250 pixel)도 배수 8입니다.
#[test]
fn large_blur_keeps_supersampling_and_draws_effects_at_low_resolution() {
    let export = confirmed(large_blur_roots(), 1.0, ExportBackground::Transparent);
    let (actual, report) = measured(&export, &metal()).unwrap();
    assert_eq!(
        (report.supersample_min, report.supersample_max),
        (SUPERSAMPLE, SUPERSAMPLE)
    );
    eprintln!("large blur 1x: {report:?}");
    let expected = cpu_reference_of(&export, SUPERSAMPLE);
    write_artifact("large_blur_srgb_1x_metal", &actual, OutputProfile::Srgb);
    write_artifact(
        "large_blur_srgb_1x_reference",
        &expected,
        OutputProfile::Srgb,
    );
    // CPU 기준은 출력 전체를 window 하나로 그리고 Metal은 tile마다 효과 window를 따로 잡습니다. 낮은 해상도 격자는 같고
    // (ADR-0136 "정렬"), 8-bit layer 저장과 half float 차이가 blur 단계와 bilinear 키우기에서 쌓입니다.
    let (over, worst) = associated_diff(&expected, &actual, 4);
    assert_eq!(over, 0, "최대 차이 {worst}");
    // 그림자가 카드 밖으로 번졌는지: 카드 오른쪽 아래 바깥 pixel은 배경(sRGB 약 242)보다 어둡고, 멀리 떨어진 왼쪽 위
    // 모서리는 배경 그대로입니다.
    let origin = root_origin(&actual);
    let red =
        |x: u32, y: u32| actual.rgba[(((origin.1 + y) * actual.width + origin.0 + x) * 4) as usize];
    assert!(red(64, 56) < 236, "{}", red(64, 56));
    assert!(red(0, 89).abs_diff(red(119, 0)) <= 1);

    // 배율 4: 출력 pixel σ 80(device σ 640)도 배수 8입니다(ADR-0130까지는 여백 때문에 배수 2~7).
    let zoomed = confirmed(large_blur_roots(), 4.0, ExportBackground::Transparent);
    let (image, report) = measured(&zoomed, &metal()).unwrap();
    eprintln!("large blur 4x: {report:?}");
    assert_eq!(report.supersample_min, SUPERSAMPLE);
    assert!(report.largest_texture_side <= crate::offscreen_target::MAX_TILE_TEXTURE);
    write_artifact("large_blur_srgb_4x_metal", &image, OutputProfile::Srgb);
    // 카드 오른쪽 아래 바깥(그림자)은 같은 줄 먼 곳보다 어둡습니다.
    let origin = root_origin(&image);
    let at =
        |x: u32, y: u32| image.rgba[(((origin.1 + y) * image.width + origin.0 + x) * 4) as usize];
    assert!(
        at(64 * 4, 56 * 4) + 8 < at(4, 56 * 4),
        "{} {}",
        at(256, 224),
        at(4, 224)
    );
    // 같은 장면 1배와 4배는 같은 자리(출력 pixel 기준)에서 가깝습니다(그림자가 tile 경계에서 끊기지 않음).
    let one = |x: u32, y: u32| {
        actual.rgba[(((root_origin(&actual).1 + y) * actual.width + root_origin(&actual).0 + x) * 4)
            as usize]
    };
    for (x, y) in [(64u32, 56u32), (30, 60), (100, 20)] {
        assert!(one(x, y).abs_diff(at(x * 4 + 2, y * 4 + 2)) <= 6, "{x} {y}");
    }
}

/// 여백이 출력 1984 pixel을 넘어 ADR-0130 규칙으로는 실패하던 blur 1300(σ 650)도 그립니다. 배수 8이면 device σ가 상한
/// 4096을 넘으므로 tile 배수만 6으로 낮춥니다. 배수 1로도 σ 상한을 넘으면 실패합니다(근사하지 않음).
///
/// 출력을 작게 두려고 root가 자기 상자로 자르고, 그 안에 배경 없는 큰 Frame(box-shadow, 상자 안 비움)의 왼쪽 가장자리를
/// 둡니다. 그림자는 상자 밖(root 왼쪽)에만 보입니다.
#[test]
fn huge_blur_lowers_supersampling_only_for_the_sigma_limit() {
    let huge = |blur: f32| {
        let mut root = frame(1, 0.0, 0.0, 40.0, 30.0, srgb(0.0, 0.0, 0.0, 0.0));
        root.background = None;
        root.clip = FrameClipInput::Bounds;
        scene(&root, |b, s| {
            let mut card = frame(2, 20.0, -2000.0, 4000.0, 4000.0, srgb(0.0, 0.0, 0.0, 0.0));
            card.background = None;
            card.node.paint.shadows.push(Shadow::new(
                p(0.0, 0.0),
                BlurRadius::new(blur).unwrap(),
                srgb(0.0, 0.0, 0.0, 0.5),
            ));
            leaf(b, s, &card);
        })
    };
    let export = confirmed(huge(1300.0), 1.0, ExportBackground::Transparent);
    let (image, report) = measured(&export, &metal()).unwrap();
    eprintln!(
        "huge blur 1300: {}x{} {report:?}",
        image.width, image.height
    );
    assert_eq!(report.supersample_max, 6, "{report:?}");
    assert!(report.largest_texture_side <= crate::offscreen_target::MAX_TILE_TEXTURE);
    write_artifact("huge_blur_srgb_1x_metal", &image, OutputProfile::Srgb);
    // 상자 밖(왼쪽)은 상자 가장자리 그림자(알파 약 0.5 × 0.5), 상자 안(오른쪽)은 비어 있습니다.
    let alpha = |x: u32, y: u32| image.rgba[((y * image.width + x) * 4 + 3) as usize];
    let y = image.height / 2;
    assert!((50..=80).contains(&alpha(10, y)), "{}", alpha(10, y));
    assert!(alpha(0, y).abs_diff(alpha(19, y)) <= 2);
    assert_eq!(alpha(30, y), 0);

    // σ 8193(blur 16386): 배수 1에서도 device σ 상한(4096)을 넘습니다.
    let export = confirmed(huge(16_386.0), 1.0, ExportBackground::Transparent);
    let request = OffscreenRequest::new(&export, OutputProfile::Srgb);
    let result = metal().render(&request, &CancelToken::new());
    assert!(
        matches!(&result, Err(OffscreenError::Failed { message }) if message.contains("blur_too_large")),
        "{result:?}"
    );
}

/// 큰 offset 그림자: ADR-0130 규칙은 여백에 |offset|을 넣어 tile texture가 커졌고 offset 300이면 배수가 5로
/// 내려갔습니다(퇴행). 이제 원본 window를 offset만큼 옮겨 그리므로 배수 8이고 CPU 기준과 같습니다(ADR-0136).
#[test]
fn large_offset_shadow_keeps_supersampling() {
    let root = frame(1, 0.0, 0.0, 30.0, 20.0, srgb(0.95, 0.95, 0.9, 1.0));
    let roots = scene(&root, |b, s| {
        let mut card = frame(2, 4.0, 4.0, 12.0, 10.0, srgb(0.2, 0.5, 0.9, 1.0));
        card.node.paint.shadows.push(Shadow::new(
            p(300.0, 3.0),
            BlurRadius::new(3.0).unwrap(),
            srgb(0.3, 0.0, 0.0, 0.8),
        ));
        leaf(b, s, &card);
    });
    let export = confirmed(roots, 1.0, ExportBackground::Transparent);
    let request = OffscreenRequest::new(&export, OutputProfile::Srgb);
    let margin = crate::offscreen_target::tile_margin(&export, request.grid().scale());
    // ADR-0130 규칙(여백을 더한 tile texture 한 변 ≤ 4096)이면 배수 5였습니다.
    let old = (1..=SUPERSAMPLE)
        .rev()
        .find(|s| (128 + 2 * margin) * s <= crate::offscreen_target::MAX_TILE_TEXTURE)
        .unwrap();
    assert!(old < SUPERSAMPLE, "{old}");
    let (actual, report) = measured(&export, &metal()).unwrap();
    eprintln!("large offset: margin {margin}, {report:?}");
    assert_eq!(report.supersample_min, SUPERSAMPLE);
    let expected = cpu_reference_of(&export, SUPERSAMPLE);
    write_artifact("large_offset_srgb_1x_metal", &actual, OutputProfile::Srgb);
    write_artifact(
        "large_offset_srgb_1x_reference",
        &expected,
        OutputProfile::Srgb,
    );
    let (over, worst) = associated_diff(&expected, &actual, 3);
    assert_eq!(over, 0, "최대 차이 {worst}");
    // 그림자는 카드에서 300 pixel 오른쪽에 있고, 카드와 그림자 사이는 비어 있습니다(root 밖은 투명).
    let origin = root_origin(&actual);
    let pixel = |x: u32, y: u32| {
        let at = (((origin.1 + y) * actual.width + origin.0 + x) * 4) as usize;
        [
            actual.rgba[at],
            actual.rgba[at + 1],
            actual.rgba[at + 2],
            actual.rgba[at + 3],
        ]
    };
    let shadow = pixel(4 + 300 + 6, 4 + 3 + 5);
    assert!(shadow[3] > 150 && shadow[0] > shadow[2], "{shadow:?}");
    assert_eq!(pixel(160, 10)[3], 0);
}

#[test]
fn selection_and_cancel_are_reported() {
    let roots = shared_roots();
    let child = roots[0].children().next().unwrap().source();
    let snapshot = SceneRevisionMap::new()
        .generate(DocumentRevision(1), roots)
        .unwrap();
    let request = ExportRequest::new(
        ExportSource::selection(view_id(), [child]).unwrap(),
        profile(),
        ExportScale::ONE,
        ExportBackground::Transparent,
        OutputPath::new("/tmp/stellar-metal-export/sel.png").unwrap(),
        ExportFormat::Png,
    )
    .unwrap();
    let capture = ExportCapture::new(
        request,
        SceneSource {
            view: view_id(),
            target_profile: profile(),
            snapshot: SnapshotHandle::new(snapshot),
        },
        PINS,
    )
    .unwrap();
    let report = TargetCapabilityReport::Validated {
        profile: profile(),
        findings: Vec::new(),
    };
    let export = ExportPreflight::run(
        capture,
        &PreflightInput {
            capabilities: &report,
            font_fallbacks: &[],
            assets: &assets(),
            text: &glyphs(),
            rasterizer: &NoRasterizer,
        },
    )
    .unwrap()
    .proceed(None)
    .unwrap();
    let request = OffscreenRequest::new(&export, OutputProfile::Srgb);
    assert!(matches!(
        metal().render(&request, &CancelToken::new()),
        Err(OffscreenError::Failed { .. })
    ));
    let export = confirmed(shared_roots(), 1.0, ExportBackground::Transparent);
    let request = OffscreenRequest::new(&export, OutputProfile::Srgb);
    let cancel = CancelToken::new();
    cancel.cancel();
    assert_eq!(
        metal().render(&request, &cancel),
        Err(OffscreenError::Cancelled)
    );
}

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

use crate::offscreen_target::MetalOffscreenTarget;

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
    let root = frame(1, 0.0, 0.0, 40.0, 28.0, srgb(0.9, 0.85, 0.7, 1.0));
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
fn cpu_reference_of(export: &ConfirmedExport) -> DecodedPng {
    use stellar_renderer_core::clip_plan::ClipCapabilities;
    use stellar_renderer_core::color::{decode_pixel, to_working};
    use stellar_renderer_core::effects::fixture::{Category, Fixture};
    use stellar_renderer_core::effects::raster::render_reference;
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
    let s = crate::offscreen_target::SUPERSAMPLE;
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
    let mut fixture = Fixture::new("effects", Category::Modifier, sw, sh);
    fixture.requested = ColorSpace::Srgb;
    fixture.display_p3 = true;
    fixture.format = TextureFormat::Rgba8UnormSrgb;
    fixture.clear = OutputColor {
        space: ColorSpace::Srgb,
        rgb: [0.0; 3],
        alpha: 0.0,
    };
    fixture.ops = ops;
    let image = render_reference(&fixture).unwrap();
    let _ = to_working;
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
    let expected = cpu_reference_of(&export);
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

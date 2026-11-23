//! PNG 내보내기의 Metal offscreen target (XX.2.1, ADR-0103 결정 2, ADR-0116 결정 4).
//!
//! [`MetalOffscreenTarget`]은 `stellar_export::raster::OffscreenTarget`을 GPU로 실행합니다. 경로는 제품 렌더러와
//! 같습니다: capture snapshot → pass compiler frame plan(효과 pass 포함) → scene frame encoder(그리기 목록) → 공통
//! lowering → headless Metal → readback.
//!
//! # 규칙
//!
//! - **좌표**: device pixel `p`는 world `내보내기 영역 원점 + p / scale`입니다(CPU target과 같은 grid). 가장자리 부분
//!   coverage는 축마다 [`SUPERSAMPLE`]배 supersampling으로 만들고, readback한 선형광 연관 알파 값을 output pixel마다
//!   상자 평균합니다. 그 뒤 비연관으로 되돌려 출력 색공간 transfer로 encode하고 8-bit로 반올림합니다(CPU target과 같은
//!   마지막 단계). 알파 0 pixel의 RGB는 0입니다.
//! - **tile**: 출력을 [`TILE`] pixel 정사각 tile로 나눠 그립니다. tile마다 효과가 번지는 거리(shadow offset + 3σ, layer
//!   blur 3σ의 경로 합) 이상 여백을 두고 그린 뒤 가운데만 씁니다. 그래서 tile 경계에서 blur·shadow가 끊기지 않습니다.
//! - **색**: working space는 출력 profile 색공간 선형광이고 target은 `Rgba16Float`입니다. 배경이 있으면 clear 색입니다.
//! - **효과 근사 거부**: frame plan에 효과 근사 진단(σ 상한, 기울임 blur)이 있으면 그리지 않고 실패합니다. 내보내기는
//!   근사하지 않습니다(ADR-0103 결정 2 원칙).
//! - **범위**: View 내보내기(capture root가 snapshot root 전체)를 그립니다. selection은 조상 clip·합성 효과를 빼야 하는데
//!   pass compiler가 아직 subtree 시작을 받지 않아 실패합니다(근사하지 않음).
//! - **resource**: 이미지는 PNG만(`decode_png`, 색 정보는 PNG 색 판정 또는 primitive override), text는 고정 font revision
//!   glyph outline(SVG 내보내기와 같은 `text_outlines`)입니다. 확인한 missing image·layout·font만 생략합니다.

use std::cell::RefCell;
use std::sync::Arc;

use stellar_export::capture::{ExportAssetSource, ImageMedia};
use stellar_export::png::{decode_png, DecodeLimits};
use stellar_export::preflight::ConfirmedExport;
use stellar_export::raster::{
    CancelToken, OffscreenError, OffscreenRequest, OffscreenTarget, RgbaImage, UnsupportedFeature,
};
use stellar_export::svg::text::{text_outlines, TextOutlineError, MAX_TEXT_OUTLINE_COMMANDS};
use stellar_export::text::{ExportTextSource, GlyphOutline, GlyphOutlineError, GlyphOutlineKey};
use stellar_renderer_core::backend::{
    BackendKind, CheckedBackend, DeviceConfig, EncodeOutcome, FrameId, RendererBackend,
    SurfaceConfig, SurfaceSize,
};
use stellar_renderer_core::clip_plan::ClipCapabilities;
use stellar_renderer_core::color::{decode_pixel, to_working, WorkingColor};
use stellar_renderer_core::effects::fixture::ImageData;
use stellar_renderer_core::effects::lower::{lower_ops, OpsTarget};
use stellar_renderer_core::effects::scene::{encode_plan, EncodeError, FrameResources, GlyphShape};
use stellar_renderer_core::output_color::OutputColor;
use stellar_renderer_core::pass_compiler::{
    compile_frame, CompileInput, EffectBoundaries, LAYER_FORMAT,
};
use stellar_renderer_core::render_graph::{ClearColor, SampleCount, TextureFormat};
use stellar_scene::node::{ImagePrimitive, SceneNode, TextPrimitive, Visibility};
use stellar_scene::stack::Paint;
use stellar_scene::visibility::Viewport;
use stellar_scene::{ColorSpace, Size2, Transform2D};

use crate::MetalBackend;

/// 축마다 supersampling 배수.
pub const SUPERSAMPLE: u32 = 8;

/// tile 한 변(출력 pixel).
pub const TILE: u32 = 128;

/// target format(선형광 연관 알파 half float).
pub const TARGET_FORMAT: TextureFormat = LAYER_FORMAT;

/// Metal offscreen target. asset·text source는 capture가 고정한 revision으로만 조회합니다.
#[derive(Clone)]
pub struct MetalOffscreenTarget {
    assets: Arc<dyn ExportAssetSource + Send + Sync>,
    text: Arc<dyn ExportTextSource + Send + Sync>,
}

impl std::fmt::Debug for MetalOffscreenTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MetalOffscreenTarget")
    }
}

impl MetalOffscreenTarget {
    /// source로 target을 만듭니다. Metal device는 렌더마다 headless로 엽니다.
    pub fn new(
        assets: Arc<dyn ExportAssetSource + Send + Sync>,
        text: Arc<dyn ExportTextSource + Send + Sync>,
    ) -> MetalOffscreenTarget {
        MetalOffscreenTarget { assets, text }
    }
}

fn failed(message: impl Into<String>) -> OffscreenError {
    OffscreenError::Failed {
        message: message.into(),
    }
}

/// encoder resource: capture 고정 revision의 이미지·glyph. 첫 실패를 typed 오류로 보관합니다.
struct Resources<'a> {
    export: &'a ConfirmedExport,
    assets: &'a dyn ExportAssetSource,
    text: &'a dyn ExportTextSource,
    error: RefCell<Option<OffscreenError>>,
}

impl Resources<'_> {
    fn fail(&self, node: &SceneNode, error: OffscreenError) -> EncodeError {
        let message = error.to_string();
        self.error.borrow_mut().get_or_insert(error);
        EncodeError::Resource {
            node: node.source(),
            message,
        }
    }
}

impl FrameResources for Resources<'_> {
    fn image(
        &self,
        node: &SceneNode,
        image: &ImagePrimitive,
    ) -> Result<Option<Arc<ImageData>>, EncodeError> {
        let capture = self.export.capture();
        let Some(encoded) = capture.encoded_image(self.assets, image.asset) else {
            if self.export.omits_image(image.asset) {
                return Ok(None);
            }
            return Err(self.fail(
                node,
                OffscreenError::MissingImage {
                    node: node.source(),
                    asset: image.asset,
                },
            ));
        };
        if encoded.media() != ImageMedia::Png {
            return Err(self.fail(
                node,
                OffscreenError::Unsupported {
                    node: node.source(),
                    feature: UnsupportedFeature::ImageFormat,
                },
            ));
        }
        let decoded = decode_png(encoded.bytes(), DecodeLimits::default()).map_err(|error| {
            self.fail(
                node,
                OffscreenError::BadImage {
                    node: node.source(),
                    asset: image.asset,
                    reason: error.to_string(),
                },
            )
        })?;
        let space = match image.color_space {
            Some(space) => space,
            None => decoded.color.color_space().map_err(|_| {
                self.fail(
                    node,
                    OffscreenError::Unsupported {
                        node: node.source(),
                        feature: UnsupportedFeature::ImageColorProfile,
                    },
                )
            })?,
        };
        let texels = decoded
            .rgba
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect();
        Ok(Some(Arc::new(ImageData {
            width: decoded.width,
            height: decoded.height,
            space,
            texels,
        })))
    }

    fn text(
        &self,
        node: &SceneNode,
        text: &TextPrimitive,
    ) -> Result<Option<Vec<GlyphShape>>, EncodeError> {
        let capture = self.export.capture();
        let Some(glyphs) = capture.glyph_layout(self.text, text.layout) else {
            if self.export.omits_text_layout(text.layout) {
                return Ok(None);
            }
            return Err(self.fail(
                node,
                OffscreenError::MissingTextLayout {
                    node: node.source(),
                    layout: text.layout,
                },
            ));
        };
        let mut lookup = |key: &GlyphOutlineKey| match capture.glyph_outline(self.text, key) {
            Err(GlyphOutlineError::FontUnavailable) if self.export.omits_font(key.font) => {
                Ok(GlyphOutline::Empty)
            }
            other => other,
        };
        let outlines = text_outlines(
            &glyphs,
            text.bounds.origin(),
            &mut lookup,
            MAX_TEXT_OUTLINE_COMMANDS,
        )
        .map_err(|error| {
            let error = match error {
                TextOutlineError::NoOutline { .. } => OffscreenError::Unsupported {
                    node: node.source(),
                    feature: UnsupportedFeature::GlyphWithoutOutline,
                },
                error => OffscreenError::Text {
                    node: node.source(),
                    error,
                },
            };
            self.fail(node, error)
        })?;
        let mut shapes = Vec::new();
        for run in &outlines.runs {
            let Some(color) = run.fill else {
                continue;
            };
            let geometry = outlines.run_geometry(run).map_err(|error| {
                self.fail(
                    node,
                    OffscreenError::Text {
                        node: node.source(),
                        error,
                    },
                )
            })?;
            if geometry.is_empty() {
                continue;
            }
            shapes.push(GlyphShape {
                geometry,
                color: OutputColor::from_paint(Paint::Solid(color)),
            });
        }
        Ok(Some(shapes))
    }
}

/// node world 변환의 큰 축 배율.
fn axis_scale(transform: &Transform2D) -> f32 {
    let (a, b, c, d, _, _) = transform.components();
    (a * a + c * c).max(b * b + d * d).sqrt()
}

/// 효과가 번지는 world 거리의 경로 합 최댓값(모듈 문서 "tile").
fn effect_reach(export: &ConfirmedExport) -> f32 {
    let mut reach = 0.0f32;
    for root in export.capture().roots() {
        let mut stack: Vec<(&SceneNode, f32)> = vec![(root.node(), 0.0)];
        while let Some((node, above)) = stack.pop() {
            if node.hit().visibility == Visibility::Hidden {
                continue;
            }
            let paint = node.paint().get();
            let scale = axis_scale(&node.world_transform());
            let shadow = paint
                .shadows
                .iter()
                .map(|s| {
                    let (x, y) = (s.offset().x(), s.offset().y());
                    (x * x + y * y).sqrt() + 1.5 * s.blur().get()
                })
                .fold(0.0f32, f32::max);
            let own = (shadow + 3.0 * paint.layer_blur.get()) * scale;
            let total = above + if own.is_finite() { own } else { 0.0 };
            reach = reach.max(total);
            stack.extend(node.children().map(|child| (&**child, total)));
        }
    }
    reach
}

impl OffscreenTarget for MetalOffscreenTarget {
    fn render(
        &self,
        request: &OffscreenRequest<'_>,
        cancel: &CancelToken,
    ) -> Result<RgbaImage, OffscreenError> {
        let (width, height) = request.size()?;
        let export = request.export();
        let capture = export.capture();
        let snapshot = capture.snapshot();
        let full_view = capture.roots().len() == snapshot.roots().count()
            && capture
                .roots()
                .iter()
                .zip(snapshot.roots())
                .all(|(root, node)| Arc::ptr_eq(root.node(), node) && root.base_clip_depth() == 0);
        if !full_view {
            return Err(failed(
                "Metal target은 selection 내보내기(조상 clip·효과 제외)를 아직 그리지 않습니다",
            ));
        }
        let plan = request.profile().color_plan();
        let clear = match request.background() {
            Some(color) => to_working(&plan, OutputColor::from_paint(Paint::Solid(color))),
            None => WorkingColor::premultiplied([0.0; 4]),
        };
        let grid = request.grid();
        let origin = grid.world_rect(&grid.canvas());
        let scale = grid.scale();
        let margin = (f64::from(effect_reach(export)) * scale).ceil() as u32 + 1;
        let resources = Resources {
            export,
            assets: self.assets.as_ref(),
            text: self.text.as_ref(),
            error: RefCell::new(None),
        };
        let mut backend = CheckedBackend::new(MetalBackend::headless());
        let caps = backend
            .init(&DeviceConfig {
                surface: SurfaceConfig {
                    size: SurfaceSize {
                        width: 64,
                        height: 64,
                    },
                    format: TextureFormat::Bgra8Unorm,
                    color_space: ColorSpace::Srgb,
                },
                frames_in_flight: 1,
                timestamps: false,
            })
            .map_err(|error| failed(format!("Metal device를 열지 못했습니다: {error}")))?;
        if caps.backend != BackendKind::Metal {
            return Err(failed("Metal backend가 아닙니다"));
        }
        let space = request.profile().color_space();
        let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];
        let mut frame = 1u64;
        let s = SUPERSAMPLE;
        for tile_y in (0..height).step_by(TILE as usize) {
            for tile_x in (0..width).step_by(TILE as usize) {
                if cancel.is_cancelled() {
                    return Err(OffscreenError::Cancelled);
                }
                let tw = TILE.min(width - tile_x);
                let th = TILE.min(height - tile_y);
                let (sw, sh) = ((tw + 2 * margin) * s, (th + 2 * margin) * s);
                if sw > caps.max_texture_dimension || sh > caps.max_texture_dimension {
                    return Err(failed("효과 여백을 더한 tile이 texture 상한을 넘습니다"));
                }
                // world → tile supersample device.
                let k = scale * f64::from(s);
                let ox =
                    (-origin[0] * scale - f64::from(tile_x) + f64::from(margin)) * f64::from(s);
                let oy =
                    (-origin[1] * scale - f64::from(tile_y) + f64::from(margin)) * f64::from(s);
                let view = Transform2D::new(k as f32, 0.0, 0.0, k as f32, ox as f32, oy as f32)
                    .map_err(|_| failed("변환 값이 유한하지 않습니다"))?;
                let viewport = Viewport::new(
                    Size2::new(sw as f32, sh as f32).map_err(|_| failed("tile 크기"))?,
                    view,
                )
                .map_err(|_| failed("viewport를 만들지 못했습니다"))?;
                let frame_plan = compile_frame(&CompileInput {
                    snapshot,
                    viewport: &viewport,
                    visible: None,
                    surface_format: TARGET_FORMAT,
                    clear: ClearColor::TRANSPARENT,
                    boundaries: EffectBoundaries::Minimal,
                    clip_caps: ClipCapabilities {
                        stencil_bits: caps.stencil_bits,
                        samples: SampleCount::X4,
                    },
                })
                .map_err(|error| failed(format!("frame compile 실패: {error}")))?;
                if let Some(diagnostic) = frame_plan.effect_diagnostics.first() {
                    return Err(failed(format!(
                        "효과를 정확히 그릴 수 없습니다({})",
                        diagnostic.code()
                    )));
                }
                let ops = encode_plan(&frame_plan, &viewport, &resources).map_err(|error| {
                    resources
                        .error
                        .borrow_mut()
                        .take()
                        .unwrap_or_else(|| failed(error.to_string()))
                })?;
                let lowered = lower_ops(
                    &mut backend,
                    &OpsTarget {
                        plan,
                        width: sw,
                        height: sh,
                        format: TARGET_FORMAT,
                        clear,
                    },
                    &ops,
                )
                .map_err(|error| failed(format!("lowering 실패: {error}")))?;
                let result = (|| {
                    let EncodeOutcome::Encoded(encoded) =
                        backend.encode(&lowered.request(FrameId(frame)))?
                    else {
                        return Err(stellar_renderer_core::backend::BackendError::new(
                            stellar_renderer_core::backend::BackendErrorKind::Internal,
                            "offscreen frame을 건너뛰었습니다",
                        ));
                    };
                    backend.submit(encoded)?;
                    backend.inner_mut().wait_idle()?;
                    backend.inner_mut().read_texture(lowered.target)
                })();
                frame += 1;
                let release = lowered.release(&mut backend);
                let bytes = result.map_err(|error| failed(format!("GPU 렌더 실패: {error}")))?;
                release.map_err(|error| failed(format!("GPU resource 해제 실패: {error}")))?;
                cancel.advance(1);
                resolve_tile(
                    &bytes,
                    (sw, s, margin),
                    (tile_x, tile_y, tw, th),
                    (width, space),
                    &mut pixels,
                )?;
            }
        }
        RgbaImage::new(width, height, request.profile(), pixels)
            .ok_or_else(|| failed("출력 크기가 계약과 다릅니다"))
    }
}

/// supersample tile을 output pixel로 평균해 `pixels`에 씁니다(모듈 문서 "좌표").
fn resolve_tile(
    bytes: &[u8],
    (stride, s, margin): (u32, u32, u32),
    (tile_x, tile_y, tw, th): (u32, u32, u32, u32),
    (width, space): (u32, ColorSpace),
    pixels: &mut [u8],
) -> Result<(), OffscreenError> {
    let texel = |x: u32, y: u32| -> Result<WorkingColor, OffscreenError> {
        let at = ((y as usize) * (stride as usize) + x as usize) * 8;
        bytes
            .get(at..at + 8)
            .and_then(|b| decode_pixel(b, TARGET_FORMAT, space))
            .ok_or_else(|| failed("readback 크기가 다릅니다"))
    };
    let samples = (s * s) as f32;
    for y in 0..th {
        for x in 0..tw {
            let mut sum = [0.0f32; 4];
            for sy in 0..s {
                for sx in 0..s {
                    let c = texel((x + margin) * s + sx, (y + margin) * s + sy)?;
                    for (acc, v) in sum.iter_mut().zip(c.rgba) {
                        *acc += v;
                    }
                }
            }
            let alpha = (sum[3] / samples).clamp(0.0, 1.0);
            let at = (((tile_y + y) as usize) * (width as usize) + (tile_x + x) as usize) * 4;
            let quantize = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            let out = &mut pixels[at..at + 4];
            out[3] = quantize(alpha);
            if out[3] == 0 {
                out[..3].fill(0);
                continue;
            }
            for i in 0..3 {
                let straight = (sum[i] / samples) / alpha;
                out[i] = quantize(space.encode_channel(straight.clamp(0.0, 1.0)));
            }
        }
    }
    Ok(())
}

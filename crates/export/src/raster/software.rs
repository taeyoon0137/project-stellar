//! CPU offscreen target (XX.2.1, ADR-0103 결정 2).
//!
//! [`SoftwareTarget`]은 [`crate::raster`]의 렌더 계약을 CPU에서 실행합니다.
//!
//! - **geometry**: Rect는 모서리 둥근 사각형 path, Path는 자기 채우기 규칙, Image는 배치 상자, Text는 glyph outline
//!   (SVG 내보내기와 같은 [`text_outlines`], non-zero)입니다. 칠하기는 [`super::coverage`] 부분 coverage입니다.
//! - **stroke**: renderer stroke tessellation([`tessellate_stroke`], XI.6.2)의 삼각형 mesh를 non-zero 합집합으로 칠합니다.
//!   `Inside`는 geometry 채우기 coverage를 곱하고 `Outside`는 그 여집합을 곱합니다(renderer `StrokeStyle::from_layer`
//!   규약: 두 배 폭 중심 stroke를 geometry 안·밖으로 자름). 평탄화 허용 오차는 device [`FLATTEN_TOLERANCE`] pixel입니다.
//! - **paint 순서**: node마다 image pixel·text run 고유 색 → paint layer(아래 → 위) → 자식입니다.
//! - **합성**: 출력 색공간 선형광 연관 알파 f32(`dst = src × c + dst × (1 − src.a × c)`, `c`는 coverage × clip).
//!   합성 불투명도 < 1 node는 행 단위로 필요할 때만 만드는 layer에 subtree를 그린 뒤 불투명도와 바깥 clip으로
//!   합성합니다. 마지막에 비연관 알파로 되돌려 transfer encode하고 8-bit로 반올림합니다.
//! - **미지원**: shadow·layer blur·Normal이 아닌 blend가 있는 보이는 node가 있으면 그리기 전에
//!   [`OffscreenError::Unsupported`]로 멈춥니다([`unsupported_features`]).
//! - tree 순회는 명시적 stack이라 깊이와 무관하게 호출 stack을 쓰지 않습니다.

use std::collections::HashMap;
use std::sync::Arc;

use stellar_core_types::ids::StableId;
use stellar_core_types::reference::AssetRef;
use stellar_renderer_core::output_color::{OutputColor, OutputColorPlan};
use stellar_renderer_core::stroke::{tessellate_stroke, StrokeStyle};
use stellar_renderer_core::tessellation::FlattenTolerance;
use stellar_scene::geometry::{FillRule, PathGeometry};
use stellar_scene::node::{ImagePrimitive, SceneContent, SceneNode, TextPrimitive, Visibility};
use stellar_scene::stack::{
    ClipShape, Opacity, Paint, PaintLayer, SceneBlendMode, StrokeAlign, StrokeLayer,
};

use super::coverage::{rasterize, Mask};
use super::image::{decode_texture, ImageIssue, Texture};
use super::path::{
    flatten, rect_geometry, rounded_rect_geometry, Affine, Contour, FLATTEN_TOLERANCE,
};
use super::{
    CancelToken, OffscreenError, OffscreenRequest, OffscreenTarget, RgbaImage, UnsupportedFeature,
};
use crate::capture::{ExportAssetSource, ExportCapture};
use crate::preflight::ConfirmedExport;
use crate::svg::raster::PixelRect;
use crate::svg::text::{text_outlines, TextOutlineError, MAX_TEXT_OUTLINE_COMMANDS};
use crate::text::{ExportTextSource, GlyphOutline, GlyphOutlineError, GlyphOutlineKey};

/// CPU offscreen target. asset·text source는 capture가 고정한 revision으로만 조회합니다.
#[derive(Clone)]
pub struct SoftwareTarget {
    assets: Arc<dyn ExportAssetSource + Send + Sync>,
    text: Arc<dyn ExportTextSource + Send + Sync>,
}

impl std::fmt::Debug for SoftwareTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SoftwareTarget")
    }
}

impl SoftwareTarget {
    /// source로 target을 만듭니다.
    pub fn new(
        assets: Arc<dyn ExportAssetSource + Send + Sync>,
        text: Arc<dyn ExportTextSource + Send + Sync>,
    ) -> SoftwareTarget {
        SoftwareTarget { assets, text }
    }
}

/// capture 범위의 보이는 node 중 CPU target이 그릴 수 없는 합성 기능(paint 순서, node마다 첫 기능).
pub fn unsupported_features(capture: &ExportCapture) -> Vec<(StableId, UnsupportedFeature)> {
    let mut out = Vec::new();
    for root in capture.roots() {
        let mut stack: Vec<&SceneNode> = vec![root.node()];
        while let Some(node) = stack.pop() {
            if node.hit().visibility == Visibility::Hidden {
                continue;
            }
            let paint = node.paint().get();
            let feature = if !paint.shadows.is_empty() {
                Some(UnsupportedFeature::Shadow)
            } else if paint.layer_blur.get() > 0.0 {
                Some(UnsupportedFeature::LayerBlur)
            } else if paint.blend != SceneBlendMode::Normal {
                Some(UnsupportedFeature::BlendMode)
            } else {
                None
            };
            if let Some(feature) = feature {
                out.push((node.source(), feature));
            }
            let start = stack.len();
            stack.extend(node.children().map(|child| &**child));
            stack[start..].reverse();
        }
    }
    out
}

/// 행 단위로 필요할 때 만드는 선형광 연관 알파 buffer.
struct Surface {
    rect: PixelRect,
    rows: Vec<Option<Box<[[f32; 4]]>>>,
}

impl Surface {
    fn new(rect: PixelRect) -> Surface {
        Surface {
            rect,
            rows: (0..rect.height()).map(|_| None).collect(),
        }
    }

    fn row_mut(&mut self, y: i64) -> &mut [[f32; 4]] {
        let width = self.rect.width() as usize;
        self.rows[(y - self.rect.y0()) as usize]
            .get_or_insert_with(|| vec![[0.0; 4]; width].into_boxed_slice())
    }

    /// `color`(연관 알파)를 coverage `mask`로 합성합니다.
    fn fill(&mut self, mask: &Mask, color: [f32; 4]) {
        let rect = mask.rect();
        let x0 = (rect.x0() - self.rect.x0()) as usize;
        for y in rect.y0()..rect.y1() {
            let coverage = mask.row(y);
            if coverage.iter().all(|c| *c <= 0.0) {
                continue;
            }
            let row = self.row_mut(y);
            for (i, c) in coverage.iter().enumerate() {
                if *c <= 0.0 {
                    continue;
                }
                let dst = &mut row[x0 + i];
                let keep = 1.0 - color[3] * c;
                for k in 0..4 {
                    dst[k] = color[k] * c + dst[k] * keep;
                }
            }
        }
    }

    /// pixel마다 다른 색(`shade(x, y)`, 연관 알파)을 coverage로 합성합니다.
    fn shade(&mut self, mask: &Mask, mut shade: impl FnMut(i64, i64) -> [f32; 4]) {
        let rect = mask.rect();
        let x0 = (rect.x0() - self.rect.x0()) as usize;
        for y in rect.y0()..rect.y1() {
            let coverage = mask.row(y);
            if coverage.iter().all(|c| *c <= 0.0) {
                continue;
            }
            let row = self.row_mut(y);
            for (i, c) in coverage.iter().enumerate() {
                if *c <= 0.0 {
                    continue;
                }
                let src = shade(rect.x0() + i as i64, y);
                let dst = &mut row[x0 + i];
                let keep = 1.0 - src[3] * c;
                for k in 0..4 {
                    dst[k] = src[k] * c + dst[k] * keep;
                }
            }
        }
    }

    /// layer를 `opacity`와 clip coverage로 합성합니다.
    fn composite(&mut self, layer: &Surface, opacity: f32, clip: &Clip) {
        for (index, row) in layer.rows.iter().enumerate() {
            let Some(row) = row else {
                continue;
            };
            let y = layer.rect.y0() + index as i64;
            let target = self.row_mut(y);
            for (i, src) in row.iter().enumerate() {
                if src[3] <= 0.0 {
                    continue;
                }
                let x = layer.rect.x0() + i as i64;
                let c = opacity * clip.at(x, y);
                if c <= 0.0 {
                    continue;
                }
                let dst = &mut target[(x - layer.rect.x0()) as usize];
                let keep = 1.0 - src[3] * c;
                for k in 0..4 {
                    dst[k] = src[k] * c + dst[k] * keep;
                }
            }
        }
    }
}

/// draw 하나에 적용할 clip.
#[derive(Debug, Clone)]
enum Clip {
    /// 고려할 clip 없음.
    All,
    /// 보이는 곳이 없음.
    Empty,
    /// coverage.
    Mask(Arc<Mask>),
}

impl Clip {
    fn at(&self, x: i64, y: i64) -> f32 {
        match self {
            Clip::All => 1.0,
            Clip::Empty => 0.0,
            Clip::Mask(mask) => mask.at(x, y),
        }
    }

    /// draw coverage에 clip을 곱합니다.
    fn apply(&self, mask: Mask) -> Option<Mask> {
        match self {
            Clip::All => Some(mask),
            Clip::Empty => None,
            Clip::Mask(clip) => mask.intersect(clip),
        }
    }
}

/// 순회 단계.
enum Visit<'a> {
    Enter(&'a SceneNode, usize),
    /// layer를 닫고 부모 target에 합성합니다(경계 node, 부모 scope clip depth).
    Close(&'a SceneNode, usize),
}

/// 렌더 한 번의 상태.
struct Frame<'r> {
    export: &'r ConfirmedExport,
    target: &'r SoftwareTarget,
    plan: OutputColorPlan,
    device: Affine,
    canvas: PixelRect,
    clips: HashMap<(usize, usize), Clip>,
    textures: HashMap<AssetRef, Result<Arc<Texture>, ImageIssue>>,
    outlines: HashMap<GlyphOutlineKey, Result<GlyphOutline, GlyphOutlineError>>,
}

fn device_contours(geometry: &PathGeometry, transform: &Affine) -> Vec<Contour> {
    flatten(geometry, transform)
}

impl<'r> Frame<'r> {
    fn capture(&self) -> &'r ExportCapture {
        self.export.capture()
    }

    /// 연관 알파 선형광 색.
    fn color(&self, paint: Paint) -> [f32; 4] {
        let converted = self.plan.convert(OutputColor::from_paint(paint));
        let a = converted.alpha;
        let [r, g, b] = converted.linear;
        [r * a, g * a, b * a, a]
    }

    /// clip shape 하나의 coverage.
    fn clip_shape(&self, shape: &ClipShape) -> Option<Mask> {
        let (geometry, rule, transform) = match shape {
            ClipShape::Rect {
                rect,
                radii,
                transform,
            } => (
                rounded_rect_geometry(*rect, *radii)?,
                FillRule::NonZero,
                transform,
            ),
            ClipShape::Path {
                geometry,
                fill_rule,
                transform,
            } => (geometry.clone(), *fill_rule, transform),
        };
        let affine = Affine::from_transform(transform).then(&self.device);
        rasterize(&device_contours(&geometry, &affine), rule, self.canvas)
    }

    /// `node` clip stack의 `from` 이후(바깥 → 안) coverage. 같은 stack·시작 depth는 한 번만 계산합니다.
    fn clip(&mut self, node: &SceneNode, from: usize) -> Clip {
        let stack = node.clip();
        if stack.depth() <= from {
            return Clip::All;
        }
        let key = (stack.storage_addr(), from);
        if let Some(hit) = self.clips.get(&key) {
            return hit.clone();
        }
        let mut acc: Option<Mask> = None;
        let mut empty = false;
        for shape in stack.to_outer_first().into_iter().skip(from) {
            let next = match (self.clip_shape(shape), acc.take()) {
                (Some(mask), None) => Some(mask),
                (Some(mask), Some(prev)) => prev.intersect(&mask),
                (None, _) => None,
            };
            if next.is_none() {
                empty = true;
                break;
            }
            acc = next;
        }
        let clip = match acc {
            Some(mask) if !empty => Clip::Mask(Arc::new(mask)),
            _ => Clip::Empty,
        };
        self.clips.insert(key, clip.clone());
        clip
    }

    fn texture(
        &mut self,
        image: &ImagePrimitive,
        node: StableId,
    ) -> Result<Option<Arc<Texture>>, OffscreenError> {
        let asset = image.asset;
        if !self.textures.contains_key(&asset) {
            let Some(encoded) = self
                .capture()
                .encoded_image(self.target.assets.as_ref(), asset)
            else {
                if self.export.omits_image(asset) {
                    return Ok(None);
                }
                return Err(OffscreenError::MissingImage { node, asset });
            };
            let decoded = decode_texture(&encoded, image.color_space, &self.plan).map(Arc::new);
            self.textures.insert(asset, decoded);
        }
        match &self.textures[&asset] {
            Ok(texture) => Ok(Some(Arc::clone(texture))),
            Err(ImageIssue::Unsupported(feature)) => Err(OffscreenError::Unsupported {
                node,
                feature: *feature,
            }),
            Err(ImageIssue::Bad(reason)) => Err(OffscreenError::BadImage {
                node,
                asset,
                reason: reason.clone(),
            }),
        }
    }

    /// image pixel을 그립니다.
    fn draw_image(
        &mut self,
        surface: &mut Surface,
        node: &SceneNode,
        image: &ImagePrimitive,
        affine: &Affine,
        clip: &Clip,
    ) -> Result<(), OffscreenError> {
        let Some(texture) = self.texture(image, node.source())? else {
            return Ok(());
        };
        let (cx, cy, cw, ch) = image.crop.components();
        let (tw, th) = (texture.width() as f32, texture.height() as f32);
        let (px, py, pw, ph) = (cx * tw, cy * th, cw * tw, ch * th);
        let bounds = image.bounds;
        let (bx, by) = (bounds.origin().x(), bounds.origin().y());
        let (bw, bh) = (bounds.size().width(), bounds.size().height());
        // 배치는 scene `ImageFit::place`(CSS object-fit, 캔버스·SVG와 같은 함수)이고 원본 크기는 crop 영역입니다.
        let Some(placement) = image.fit.place([bx, by, bw, bh], [pw, ph]) else {
            return Ok(());
        };
        let [dx, dy, dw, dh] = placement.dest;
        // 보이는 상자: 배치 상자와 image 상자의 교집합(Contain·Fill은 배치 상자가 상자 안).
        let (vx0, vy0) = (dx.max(bx), dy.max(by));
        let (vx1, vy1) = ((dx + dw).min(bx + bw), (dy + dh).min(by + bh));
        if vx1 <= vx0 || vy1 <= vy0 {
            return Ok(());
        }
        let contour: Contour = [(vx0, vy0), (vx1, vy0), (vx1, vy1), (vx0, vy1)]
            .iter()
            .map(|(x, y)| affine.apply((f64::from(*x), f64::from(*y))))
            .collect();
        let Some(mask) = rasterize(&[contour], FillRule::NonZero, self.canvas) else {
            return Ok(());
        };
        let Some(mask) = clip.apply(mask) else {
            return Ok(());
        };
        let Some(inverse) = affine.inverse() else {
            return Ok(());
        };
        // crop texel 범위(바깥으로 정수화).
        let crop = (
            (f64::from(px)).floor() as i64,
            (f64::from(py)).floor() as i64,
            (f64::from(px + pw)).ceil().min(f64::from(tw)) as i64,
            (f64::from(py + ph)).ceil().min(f64::from(th)) as i64,
        );
        let crop = (
            crop.0,
            crop.1,
            crop.2.max(crop.0 + 1),
            crop.3.max(crop.1 + 1),
        );
        let (sx, sy) = (f64::from(pw) / f64::from(dw), f64::from(ph) / f64::from(dh));
        let sampling = image.sampling;
        surface.shade(&mask, |x, y| {
            let (lx, ly) = inverse.apply((x as f64 + 0.5, y as f64 + 0.5));
            let u = f64::from(px) + (lx - f64::from(dx)) * sx;
            let v = f64::from(py) + (ly - f64::from(dy)) * sy;
            texture.sample(u, v, crop, sampling)
        });
        Ok(())
    }

    /// text glyph outline을 만들고 run 고유 색을 칠합니다. paint layer 기하(모든 glyph)를 돌려줍니다.
    fn draw_text(
        &mut self,
        surface: &mut Surface,
        node: &SceneNode,
        text: &TextPrimitive,
        affine: &Affine,
        clip: &Clip,
    ) -> Result<PathGeometry, OffscreenError> {
        let source = node.source();
        let capture = self.capture();
        let Some(glyphs) = capture.glyph_layout(self.target.text.as_ref(), text.layout) else {
            if self.export.omits_text_layout(text.layout) {
                return Ok(PathGeometry::empty());
            }
            return Err(OffscreenError::MissingTextLayout {
                node: source,
                layout: text.layout,
            });
        };
        let (export, source_text, cache) =
            (self.export, self.target.text.as_ref(), &mut self.outlines);
        let mut lookup = |key: &GlyphOutlineKey| {
            let outline = cache
                .entry(key.clone())
                .or_insert_with(|| capture.glyph_outline(source_text, key))
                .clone();
            match outline {
                Err(GlyphOutlineError::FontUnavailable) if export.omits_font(key.font) => {
                    Ok(GlyphOutline::Empty)
                }
                other => other,
            }
        };
        let outlines = text_outlines(
            &glyphs,
            text.bounds.origin(),
            &mut lookup,
            MAX_TEXT_OUTLINE_COMMANDS,
        )
        .map_err(|error| match error {
            TextOutlineError::NoOutline { .. } => OffscreenError::Unsupported {
                node: source,
                feature: UnsupportedFeature::GlyphWithoutOutline,
            },
            error => OffscreenError::Text {
                node: source,
                error,
            },
        })?;
        for run in &outlines.runs {
            let Some(color) = run.fill else {
                continue;
            };
            let geometry = outlines
                .run_geometry(run)
                .map_err(|error| OffscreenError::Text {
                    node: source,
                    error,
                })?;
            if let Some(mask) = rasterize(
                &device_contours(&geometry, affine),
                FillRule::NonZero,
                self.canvas,
            )
            .and_then(|mask| clip.apply(mask))
            {
                surface.fill(&mask, self.color(Paint::Solid(color)));
            }
        }
        Ok(outlines.combined)
    }

    /// stroke coverage(정렬 규칙 포함). geometry 채우기 coverage는 필요할 때 한 번 계산해 둡니다.
    fn stroke_mask(
        &self,
        node: StableId,
        stroke: &StrokeLayer,
        geometry: &PathGeometry,
        fill: &mut Option<Option<Mask>>,
        rule: FillRule,
        affine: &Affine,
    ) -> Result<Option<Mask>, OffscreenError> {
        let scale = affine.max_scale();
        let tolerance = FlattenTolerance::new((FLATTEN_TOLERANCE / scale.max(1e-6)) as f32)
            .map_err(|error| OffscreenError::Stroke { node, error })?;
        let style = StrokeStyle::from_layer(stroke)
            .map_err(|error| OffscreenError::Stroke { node, error })?
            .with_tolerance(tolerance);
        let mesh = tessellate_stroke(geometry, &style)
            .map_err(|error| OffscreenError::Stroke { node, error })?;
        let vertices: Vec<(f64, f64)> = mesh
            .vertices()
            .iter()
            .map(|v| affine.apply((f64::from(v.x()), f64::from(v.y()))))
            .collect();
        let mut triangles: Vec<Contour> = Vec::with_capacity(mesh.triangle_count());
        for tri in mesh.indices().chunks_exact(3) {
            let mut points: Contour = tri.iter().map(|i| vertices[*i as usize]).collect();
            let (a, b, c) = (points[0], points[1], points[2]);
            let area = (b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1);
            if area == 0.0 || !area.is_finite() {
                continue;
            }
            if area < 0.0 {
                points.reverse();
            }
            triangles.push(points);
        }
        let Some(mask) = rasterize(&triangles, FillRule::NonZero, self.canvas) else {
            return Ok(None);
        };
        let fill_mask = |fill: &mut Option<Option<Mask>>| {
            fill.get_or_insert_with(|| {
                rasterize(&device_contours(geometry, affine), rule, self.canvas)
            })
            .clone()
        };
        Ok(match stroke.align() {
            StrokeAlign::Center => Some(mask),
            StrokeAlign::Inside => fill_mask(fill).and_then(|inside| mask.intersect(&inside)),
            StrokeAlign::Outside => Some(match fill_mask(fill) {
                Some(inside) => mask.subtract(&inside),
                None => mask,
            }),
        })
    }

    /// node 자기 내용을 그립니다.
    fn draw_content(
        &mut self,
        surface: &mut Surface,
        node: &SceneNode,
        clip_from: usize,
    ) -> Result<(), OffscreenError> {
        let clip = self.clip(node, clip_from);
        if matches!(clip, Clip::Empty) {
            return Ok(());
        }
        let affine = Affine::from_transform(&node.world_transform()).then(&self.device);
        let source = node.source();
        let (geometry, rule) = match node.content() {
            SceneContent::Rect(rect) => match rounded_rect_geometry(rect.rect, rect.radii) {
                Some(geometry) => (geometry, FillRule::NonZero),
                None => return Ok(()),
            },
            SceneContent::Path(path) => (path.geometry.clone(), path.fill_rule),
            SceneContent::Image(image) => {
                self.draw_image(surface, node, image, &affine, &clip)?;
                match rect_geometry(image.bounds) {
                    Some(geometry) => (geometry, FillRule::NonZero),
                    None => return Ok(()),
                }
            }
            SceneContent::Text(text) => (
                self.draw_text(surface, node, text, &affine, &clip)?,
                FillRule::NonZero,
            ),
        };
        if geometry.is_empty() {
            return Ok(());
        }
        let mut fill: Option<Option<Mask>> = None;
        for layer in &node.paint().get().layers {
            let mask = match layer {
                PaintLayer::Fill(_) => fill
                    .get_or_insert_with(|| {
                        rasterize(&device_contours(&geometry, &affine), rule, self.canvas)
                    })
                    .clone(),
                PaintLayer::Stroke(stroke) => {
                    if stroke.width() <= 0.0 {
                        continue;
                    }
                    self.stroke_mask(source, stroke, &geometry, &mut fill, rule, &affine)?
                }
            };
            let paint = match layer {
                PaintLayer::Fill(paint) => *paint,
                PaintLayer::Stroke(stroke) => stroke.paint(),
            };
            if let Some(mask) = mask.and_then(|mask| clip.apply(mask)) {
                surface.fill(&mask, self.color(paint));
            }
        }
        Ok(())
    }
}

impl OffscreenTarget for SoftwareTarget {
    fn render(
        &self,
        request: &OffscreenRequest<'_>,
        cancel: &CancelToken,
    ) -> Result<RgbaImage, OffscreenError> {
        let (width, height) = request.size()?;
        let export = request.export();
        let capture = export.capture();
        if let Some((node, feature)) = unsupported_features(capture).into_iter().next() {
            return Err(OffscreenError::Unsupported { node, feature });
        }
        let canvas = request.pixels();
        let mut frame = Frame {
            export,
            target: self,
            plan: request.profile().color_plan(),
            device: Affine::device(request.grid()),
            canvas,
            clips: HashMap::new(),
            textures: HashMap::new(),
            outlines: HashMap::new(),
        };
        let mut main = Surface::new(canvas);
        if let Some(background) = request.background() {
            let color = frame.color(Paint::Solid(background));
            main.fill(&Mask::filled(canvas, 1.0), color);
        }
        // layer stack: (buffer). main은 맨 아래입니다.
        let mut layers: Vec<Surface> = Vec::new();
        let mut stack: Vec<Visit<'_>> = capture
            .roots()
            .iter()
            .rev()
            .map(|root| Visit::Enter(root.node().as_ref(), root.base_clip_depth()))
            .collect();
        while let Some(visit) = stack.pop() {
            if cancel.is_cancelled() {
                return Err(OffscreenError::Cancelled);
            }
            match visit {
                Visit::Enter(node, depth) => {
                    if node.hit().visibility == Visibility::Hidden {
                        continue;
                    }
                    cancel.advance(1);
                    let opacity = node.paint().get().opacity;
                    let child_depth = if opacity < Opacity::OPAQUE {
                        // 경계 node: 자기 내용과 자손을 layer에, 안쪽 clip만 적용합니다.
                        layers.push(Surface::new(canvas));
                        stack.push(Visit::Close(node, depth));
                        node.clip().depth()
                    } else {
                        depth
                    };
                    let surface = layers.last_mut().unwrap_or(&mut main);
                    frame.draw_content(surface, node, child_depth)?;
                    let start = stack.len();
                    stack.extend(
                        node.children()
                            .map(|child| Visit::Enter(child.as_ref(), child_depth)),
                    );
                    stack[start..].reverse();
                }
                Visit::Close(node, depth) => {
                    let Some(layer) = layers.pop() else {
                        continue;
                    };
                    let clip = frame.clip(node, depth);
                    let opacity = node.paint().get().opacity.get();
                    let surface = layers.last_mut().unwrap_or(&mut main);
                    surface.composite(&layer, opacity, &clip);
                }
            }
        }
        let space = request.profile().color_space();
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        for (index, row) in main.rows.iter().enumerate() {
            let Some(row) = row else {
                continue;
            };
            let out = &mut pixels[index * width as usize * 4..(index + 1) * width as usize * 4];
            for (i, px) in row.iter().enumerate() {
                let quantize = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                let a = px[3].clamp(0.0, 1.0);
                let alpha = quantize(a);
                if alpha == 0 {
                    // 투명 pixel의 RGB는 0입니다(렌더 계약).
                    continue;
                }
                for k in 0..3 {
                    out[i * 4 + k] = quantize(space.encode_channel((px[k] / a).clamp(0.0, 1.0)));
                }
                out[i * 4 + 3] = alpha;
            }
        }
        RgbaImage::new(width, height, request.profile(), pixels).ok_or_else(|| {
            OffscreenError::Contract {
                message: "pixel buffer 크기".to_string(),
            }
        })
    }
}

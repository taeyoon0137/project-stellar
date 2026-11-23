//! CPU 기준 rasterizer (XI.11.1, ADR-0102 결정 6).
//!
//! fixture를 GPU와 같은 규칙으로 `f32` working 값 canvas에 그린 뒤 target format으로 encode합니다.
//!
//! - **표본**: pixel 중심 `(x + 0.5, y + 0.5)` 하나. 삼각형 변 위의 중심은 top-left 규칙으로 판정합니다(위쪽 수평 변과
//!   왼쪽 변이면 안쪽). GPU rasterizer와 같은 규칙이며, 정점 subpixel 반올림 차이는 structural 허용 오차가 흡수합니다.
//! - **합성**: premultiplied source-over와 blend mode([`crate::blend`], ADR-0102 결정 3).
//! - **저장 정밀도**: canvas는 쓸 때마다 target format 저장 값으로 맞춥니다(8-bit target은 양자화, layer는 half float).
//!   GPU blend가 draw마다 저장된 값을 다시 읽는 것과 같습니다.
//! - **layer**: 내용을 투명 layer canvas(`pass_compiler::LAYER_FORMAT` 정밀도)에 그린 뒤 효과 순서(내용 → shadow → layer
//!   blur → clip → mask → opacity → blend, ADR-0102 결정 5)대로 target 전체에 합성합니다. blur는 GPU와 같은 두 단계 분리
//!   적용이고 단계마다 layer 정밀도로 저장합니다. clip·mask coverage는 scissor·stencil 0/1, mask는 8-bit 저장 값입니다.
//! - **겹침**: [`Coverage::Disjoint`] mesh는 삼각형마다 합성하고(GPU draw 순서와 같음), [`Coverage::Union`]은 pixel마다
//!   한 번만 합성합니다.
//! - **이미지**: texture에 올리는 byte([`ImageData::texture_bytes`])를 decode한 값으로 sampling합니다. Nearest는
//!   `floor(u × w)` texel, Linear는 texel 중심 기준 bilinear이고 가장자리는 wrap(clamp·repeat)을 따릅니다.
//! - **glyph**: mask byte / 255를 coverage로 색에 곱합니다.
//! - **gradient**: pixel 중심의 `t`를 [`GradientRamp::evaluate`]로 정확히 계산합니다(XI.10.3).

use crate::blend::{composite, source_over};
use crate::color::{decode_pixel, encode_pixel, to_working, ColorSpace, WorkingColor};
use crate::output_color::OutputColorPlan;
use crate::pass_compiler::LAYER_FORMAT;
use crate::placeholder::{TextureFilter, TextureWrap};
use crate::render_graph::TextureFormat;

use super::artifact::RgbaImage;
use super::blur::{kernel, layer_blur_sigma, shadow_sigma};
use super::fixture::{
    ClipSpec, Coverage, CoverageShape, FillOp, Fixture, FixtureError, GlyphOp, ImageData, ImageOp,
    LayerOp, Op, Paint, IMAGE_TEXTURE_FORMAT,
};
use super::gradient::GradientRamp;

/// working 값 canvas. 쓸 때마다 target format 저장 정밀도로 맞춥니다(GPU attachment가 draw마다 저장 값을 남기고 다음
/// blend가 그 값을 읽는 것과 같게).
#[derive(Debug, Clone, PartialEq)]
pub struct Canvas {
    /// 너비.
    pub width: u32,
    /// 높이.
    pub height: u32,
    /// pixel(행 우선).
    pub pixels: Vec<WorkingColor>,
    /// 저장 format과 transfer 곡선 surface(`None`이면 `f32` 그대로).
    pub storage: Option<(TextureFormat, ColorSpace)>,
}

impl Canvas {
    /// `clear`로 채운 `f32` canvas.
    pub fn new(width: u32, height: u32, clear: WorkingColor) -> Canvas {
        Canvas {
            width,
            height,
            pixels: vec![clear; (width * height) as usize],
            storage: None,
        }
    }

    /// `format` 저장 정밀도 canvas.
    pub fn stored(
        width: u32,
        height: u32,
        clear: WorkingColor,
        format: TextureFormat,
        surface: ColorSpace,
    ) -> Canvas {
        let mut canvas = Canvas::new(width, height, clear);
        canvas.storage = Some((format, surface));
        let clear = canvas.quantize(clear);
        canvas.pixels.fill(clear);
        canvas
    }

    /// 저장 정밀도로 맞춘 값.
    pub fn quantize(&self, color: WorkingColor) -> WorkingColor {
        match self.storage {
            None => color,
            Some((format, surface)) => encode_pixel(color, format, surface)
                .and_then(|bytes| decode_pixel(&bytes, format, surface))
                .unwrap_or(color),
        }
    }

    /// pixel을 씁니다(저장 정밀도 적용).
    pub fn set(&mut self, x: u32, y: u32, color: WorkingColor) {
        let stored = self.quantize(color);
        *self.at_mut(x, y) = stored;
    }

    /// pixel 값.
    pub fn get(&self, x: i64, y: i64) -> WorkingColor {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return WorkingColor::TRANSPARENT;
        }
        self.pixels[(y as usize) * self.width as usize + x as usize]
    }

    /// pixel 참조.
    pub fn at_mut(&mut self, x: u32, y: u32) -> &mut WorkingColor {
        &mut self.pixels[(y * self.width + x) as usize]
    }

    /// premultiplied source-over.
    pub fn over(&mut self, x: u32, y: u32, src: WorkingColor) {
        let dst = *self.at_mut(x, y);
        self.set(x, y, source_over(src, dst));
    }
}

/// 삼각형이 덮는 pixel 중심마다 `f(x, y)`를 부릅니다(top-left 규칙).
pub fn raster_triangle(tri: [[f32; 2]; 3], width: u32, height: u32, mut f: impl FnMut(u32, u32)) {
    let p = tri.map(|[x, y]| (f64::from(x), f64::from(y)));
    let (a, mut b, mut c) = (p[0], p[1], p[2]);
    let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    if area == 0.0 || !area.is_finite() {
        return;
    }
    if area < 0.0 {
        std::mem::swap(&mut b, &mut c);
    }
    let edges = [(a, b), (b, c), (c, a)];
    let min_x = a.0.min(b.0).min(c.0).floor().max(0.0) as i64;
    let max_x = (a.0.max(b.0).max(c.0).ceil() as i64).min(i64::from(width));
    let min_y = a.1.min(b.1).min(c.1).floor().max(0.0) as i64;
    let max_y = (a.1.max(b.1).max(c.1).ceil() as i64).min(i64::from(height));
    for y in min_y..max_y {
        for x in min_x..max_x {
            let center = (x as f64 + 0.5, y as f64 + 0.5);
            let inside = edges.iter().all(|&(s, e)| {
                let (dx, dy) = (e.0 - s.0, e.1 - s.1);
                let w = dx * (center.1 - s.1) - dy * (center.0 - s.0);
                // y 아래 좌표계에서 넓이가 양수(시계 방향)인 삼각형: 왼쪽 변은 위로(dy < 0), 위쪽 변은 오른쪽으로(dy = 0, dx > 0).
                w > 0.0 || (w == 0.0 && (dy < 0.0 || (dy == 0.0 && dx > 0.0)))
            });
            if inside {
                f(x as u32, y as u32);
            }
        }
    }
}

/// mesh가 덮는 pixel을 겹침 처리대로 부릅니다.
pub(crate) fn raster_mesh(
    positions: &[[f32; 2]],
    coverage: Coverage,
    width: u32,
    height: u32,
    mut f: impl FnMut(u32, u32),
) {
    match coverage {
        Coverage::Disjoint => {
            for tri in positions.chunks_exact(3) {
                raster_triangle([tri[0], tri[1], tri[2]], width, height, &mut f);
            }
        }
        Coverage::Union => {
            let mut seen = vec![false; (width * height) as usize];
            for tri in positions.chunks_exact(3) {
                raster_triangle([tri[0], tri[1], tri[2]], width, height, |x, y| {
                    let index = (y * width + x) as usize;
                    if !seen[index] {
                        seen[index] = true;
                        f(x, y);
                    }
                });
            }
        }
    }
}

/// texture byte를 decode한 working texel.
pub(crate) fn decoded_texels(image: &ImageData, plan: &OutputColorPlan) -> Vec<WorkingColor> {
    image
        .texture_bytes(plan)
        .chunks_exact(4)
        .map(|bytes| decode_pixel(bytes, IMAGE_TEXTURE_FORMAT, plan.surface()).expect("rgba8"))
        .collect()
}

/// `(u, v)` sampling(가장자리 clamp).
pub(crate) fn sample(
    texels: &[WorkingColor],
    width: u32,
    height: u32,
    u: f64,
    v: f64,
    filter: TextureFilter,
    wrap: TextureWrap,
) -> WorkingColor {
    let clamp = |i: i64, n: u32| match wrap {
        TextureWrap::ClampToEdge => i.clamp(0, i64::from(n) - 1) as usize,
        TextureWrap::Repeat => i.rem_euclid(i64::from(n)) as usize,
    };
    let at = |x: i64, y: i64| texels[clamp(y, height) * width as usize + clamp(x, width)];
    match filter {
        TextureFilter::Nearest => at(
            (u * f64::from(width)).floor() as i64,
            (v * f64::from(height)).floor() as i64,
        ),
        TextureFilter::Linear => {
            let tx = u * f64::from(width) - 0.5;
            let ty = v * f64::from(height) - 0.5;
            let (x0, y0) = (tx.floor(), ty.floor());
            let (fx, fy) = ((tx - x0) as f32, (ty - y0) as f32);
            let (x0, y0) = (x0 as i64, y0 as i64);
            let lerp = |a: WorkingColor, b: WorkingColor, t: f32| {
                WorkingColor::premultiplied(std::array::from_fn(|i| {
                    a.rgba[i] + (b.rgba[i] - a.rgba[i]) * t
                }))
            };
            let top = lerp(at(x0, y0), at(x0 + 1, y0), fx);
            let bottom = lerp(at(x0, y0 + 1), at(x0 + 1, y0 + 1), fx);
            lerp(top, bottom, fy)
        }
    }
}

/// 그리기 목록을 canvas에 그립니다.
pub fn draw_ops(canvas: &mut Canvas, ops: &[Op], plan: &OutputColorPlan) {
    for op in ops {
        match op {
            Op::Fill(fill) => draw_fill(canvas, fill, plan),
            Op::Image(image) => draw_image(canvas, image, plan),
            Op::Glyphs(glyph) => draw_glyph(canvas, glyph, plan),
            Op::Layer(layer) => draw_layer(canvas, layer, plan),
        }
    }
}

/// layer canvas(GPU layer format 저장 정밀도).
pub(crate) fn layer_canvas(parent: &Canvas, plan: &OutputColorPlan) -> Canvas {
    Canvas::stored(
        parent.width,
        parent.height,
        WorkingColor::TRANSPARENT,
        LAYER_FORMAT,
        plan.surface(),
    )
}

/// layer를 효과 순서(ADR-0102 결정 5)대로 그린 뒤 부모에 합성합니다.
///
/// 내용 → shadow(내용 아래) → layer blur → clip → mask → opacity → blend.
fn draw_layer(canvas: &mut Canvas, layer: &LayerOp, plan: &OutputColorPlan) {
    let mut group = layer_canvas(canvas, plan);
    draw_ops(&mut group, &layer.content, plan);
    if !layer.shadows.is_empty() {
        let mut stacked = layer_canvas(canvas, plan);
        for shadow in &layer.shadows {
            let sigma = shadow_sigma(shadow.blur);
            let alpha = blur_pass(
                &group,
                sigma,
                [1, 0],
                true,
                WorkingColor::premultiplied([1.0; 4]),
            );
            let colored = blur_pass(&alpha, sigma, [0, 1], false, to_working(plan, shadow.color));
            let [ox, oy] = shadow.offset.map(i64::from);
            for y in 0..stacked.height {
                for x in 0..stacked.width {
                    let src = colored.get(i64::from(x) - ox, i64::from(y) - oy);
                    stacked.over(x, y, src);
                }
            }
        }
        for y in 0..stacked.height {
            for x in 0..stacked.width {
                let src = group.get(i64::from(x), i64::from(y));
                stacked.over(x, y, src);
            }
        }
        group = stacked;
    }
    let sigma = layer_blur_sigma(layer.blur);
    if sigma > 0.0 {
        let unit = WorkingColor::premultiplied([1.0; 4]);
        let horizontal = blur_pass(&group, sigma, [1, 0], false, unit);
        group = blur_pass(&horizontal, sigma, [0, 1], false, unit);
    }
    let (w, h) = (canvas.width, canvas.height);
    // clip·mask coverage(scissor·stencil은 0/1, mask는 저장한 8-bit coverage).
    let mut coverage = vec![1.0f32; (w * h) as usize];
    let mut apply = |mask: &Canvas| {
        for (c, m) in coverage.iter_mut().zip(mask.pixels.iter()) {
            *c *= m.alpha();
        }
    };
    match &layer.clip {
        None => {}
        Some(ClipSpec::Scissor {
            x,
            y,
            width,
            height,
        }) => {
            let mut mask = Canvas::new(w, h, WorkingColor::TRANSPARENT);
            for py in *y..(y.saturating_add(*height)).min(h) {
                for px in *x..(x.saturating_add(*width)).min(w) {
                    mask.set(px, py, WorkingColor::premultiplied([0.0, 0.0, 0.0, 1.0]));
                }
            }
            apply(&mask);
        }
        Some(ClipSpec::Stencil(mesh)) => {
            let mut mask = Canvas::new(w, h, WorkingColor::TRANSPARENT);
            raster_mesh(&mesh.positions, Coverage::Union, w, h, |x, y| {
                mask.set(x, y, WorkingColor::premultiplied([0.0, 0.0, 0.0, 1.0]))
            });
            apply(&mask);
        }
        Some(ClipSpec::Mask(shapes)) => apply(&coverage_canvas(shapes, w, h, plan)),
    }
    if !layer.mask.is_empty() {
        apply(&coverage_canvas(&layer.mask, w, h, plan));
    }
    for y in 0..h {
        for x in 0..w {
            let k = coverage[(y * w + x) as usize];
            if k <= 0.0 {
                continue;
            }
            let src = group
                .get(i64::from(x), i64::from(y))
                .scale(layer.opacity * k);
            let dst = canvas.get(i64::from(x), i64::from(y));
            canvas.set(x, y, composite(layer.blend, src, dst));
        }
    }
}

/// coverage 도형을 R8 coverage canvas에 합집합으로 그립니다(GPU `ClipMask` + premultiplied over와 같음).
fn coverage_canvas(shapes: &[CoverageShape], w: u32, h: u32, plan: &OutputColorPlan) -> Canvas {
    let mut mask = Canvas::stored(
        w,
        h,
        WorkingColor::TRANSPARENT,
        TextureFormat::R8Unorm,
        plan.surface(),
    );
    for shape in shapes {
        let value = WorkingColor::premultiplied([0.0, 0.0, 0.0, shape.coverage]);
        raster_mesh(&shape.mesh.positions, Coverage::Disjoint, w, h, |x, y| {
            mask.over(x, y, value)
        });
    }
    mask
}

/// 한 방향 gaussian blur(모듈 [`super::blur`] 이산화). `source_alpha`면 texel alpha를 모든 채널로 퍼뜨려 읽습니다.
/// 결과는 `tint`를 곱해 source와 같은 저장 정밀도 canvas에 씁니다.
pub(crate) fn blur_pass(
    source: &Canvas,
    sigma: f32,
    direction: [i64; 2],
    source_alpha: bool,
    tint: WorkingColor,
) -> Canvas {
    let weights = kernel(sigma);
    let radius = (weights.len() / 2) as i64;
    let mut out = source.clone();
    for y in 0..source.height {
        for x in 0..source.width {
            let mut sum = [0.0f32; 4];
            for (k, w) in weights.iter().enumerate() {
                let i = k as i64 - radius;
                let v = source.get(
                    i64::from(x) + direction[0] * i,
                    i64::from(y) + direction[1] * i,
                );
                let v = if source_alpha { [v.alpha(); 4] } else { v.rgba };
                for c in 0..4 {
                    sum[c] += w * v[c];
                }
            }
            out.set(
                x,
                y,
                WorkingColor::premultiplied(std::array::from_fn(|c| sum[c] * tint.rgba[c])),
            );
        }
    }
    out
}

fn draw_fill(canvas: &mut Canvas, fill: &FillOp, plan: &OutputColorPlan) {
    let (width, height) = (canvas.width, canvas.height);
    match &fill.paint {
        Paint::Solid(color) => {
            let working = to_working(plan, *color);
            raster_mesh(
                &fill.mesh.positions,
                fill.coverage,
                width,
                height,
                |x, y| canvas.over(x, y, working),
            );
        }
        Paint::Gradient { geometry, stops } => {
            let Ok(ramp) = GradientRamp::resolve(plan, stops) else {
                return;
            };
            raster_mesh(
                &fill.mesh.positions,
                fill.coverage,
                width,
                height,
                |x, y| {
                    let t = geometry.parameter([x as f32 + 0.5, y as f32 + 0.5]);
                    canvas.over(x, y, ramp.evaluate(t))
                },
            );
        }
    }
}

fn draw_image(canvas: &mut Canvas, image: &ImageOp, plan: &OutputColorPlan) {
    let texels = decoded_texels(&image.image, plan);
    let [x0, y0, x1, y1] = image.rect;
    let [u0, v0, u1, v1] = image.uv.map(f64::from);
    let (w, h) = (canvas.width, canvas.height);
    let mesh = super::fixture::Mesh::rect(x0, y0, x1, y1);
    let mut covered = Vec::new();
    raster_mesh(&mesh.positions, Coverage::Disjoint, w, h, |x, y| {
        covered.push((x, y))
    });
    for (x, y) in covered {
        let s = (f64::from(x) + 0.5 - f64::from(x0)) / f64::from(x1 - x0);
        let t = (f64::from(y) + 0.5 - f64::from(y0)) / f64::from(y1 - y0);
        let c = sample(
            &texels,
            image.image.width,
            image.image.height,
            u0 + (u1 - u0) * s,
            v0 + (v1 - v0) * t,
            image.filter,
            image.wrap,
        );
        canvas.over(x, y, c.scale(image.opacity));
    }
}

fn draw_glyph(canvas: &mut Canvas, glyph: &GlyphOp, plan: &OutputColorPlan) {
    let color = to_working(plan, glyph.color);
    let mask = &glyph.mask;
    for my in 0..mask.height {
        for mx in 0..mask.width {
            let x = i64::from(glyph.origin[0]) + i64::from(mx);
            let y = i64::from(glyph.origin[1]) + i64::from(my);
            if x < 0 || y < 0 || x >= i64::from(canvas.width) || y >= i64::from(canvas.height) {
                continue;
            }
            let coverage = f32::from(mask.data[(my * mask.width + mx) as usize]) / 255.0;
            canvas.over(x as u32, y as u32, color.scale(coverage));
        }
    }
}

/// canvas를 target format byte(RGBA 순서)로 encode합니다.
pub fn encode_canvas(canvas: &Canvas, fixture: &Fixture) -> RgbaImage {
    let surface = fixture.plan().surface();
    let pixels = canvas
        .pixels
        .iter()
        .flat_map(|c| encode_pixel(*c, fixture.format, surface).expect("rgba8 format"))
        .collect();
    RgbaImage {
        width: canvas.width,
        height: canvas.height,
        pixels,
    }
}

/// fixture의 CPU 기준 image.
pub fn render_reference(fixture: &Fixture) -> Result<RgbaImage, FixtureError> {
    fixture.validate()?;
    let plan = fixture.plan();
    let mut canvas = Canvas::stored(
        fixture.width,
        fixture.height,
        fixture.clear_working(),
        fixture.format,
        plan.surface(),
    );
    draw_ops(&mut canvas, &fixture.ops, &plan);
    Ok(encode_canvas(&canvas, fixture))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::effects::fixture::{srgb, Category, Fixture, LayerOp, SceneBlendMode};
    use crate::effects::inventory::{effect, rect};

    fn plan() -> OutputColorPlan {
        Fixture::new("plan", Category::Modifier, 1, 1).plan()
    }

    fn working(fixture: &Fixture, x: u32, y: u32) -> WorkingColor {
        let image = render_reference(fixture).unwrap();
        decode_pixel(&image.pixel(x, y), fixture.format, fixture.plan().surface()).unwrap()
    }

    #[test]
    fn group_opacity_composites_once_over_overlap() {
        // 겹친 영역은 위 사각형(파랑)만 보이는 그룹을 한 번 0.5로 합성합니다(개별 불투명도와 다름).
        let content = vec![
            rect(0.0, 0.0, 6.0, 6.0, srgb(1.0, 0.0, 0.0, 1.0)),
            rect(2.0, 2.0, 8.0, 8.0, srgb(0.0, 0.0, 1.0, 1.0)),
        ];
        let mut fixture = Fixture::new("group", Category::Modifier, 8, 8);
        fixture.clear = srgb(1.0, 1.0, 1.0, 1.0);
        fixture.ops = vec![effect(
            LayerOp::new(content, SceneBlendMode::Normal).with_opacity(0.5),
        )];
        let plan = plan();
        let blue = to_working(&plan, srgb(0.0, 0.0, 1.0, 1.0)).scale(0.5);
        let white = to_working(&plan, srgb(1.0, 1.0, 1.0, 1.0));
        let expected = source_over(blue, white);
        let actual = working(&fixture, 4, 4);
        for c in 0..4 {
            assert!(
                (actual.rgba[c] - expected.rgba[c]).abs() < 0.01,
                "{actual:?} {expected:?}"
            );
        }
        // 빨강 성분이 남지 않습니다(개별 불투명도면 빨강이 비쳐 보임).
        assert!(actual.rgba[0] < 0.51);
    }

    #[test]
    fn blur_preserves_energy_and_runs_before_clip() {
        let content = || vec![rect(12.0, 12.0, 20.0, 20.0, srgb(0.0, 0.0, 0.0, 1.0))];
        let mut blurred = Fixture::new("energy", Category::Modifier, 32, 32);
        blurred.clear = srgb(0.0, 0.0, 0.0, 0.0);
        blurred.format = crate::render_graph::TextureFormat::Rgba8Unorm;
        blurred.ops = vec![effect(
            LayerOp::new(content(), SceneBlendMode::Normal).with_blur(2.0),
        )];
        let image = render_reference(&blurred).unwrap();
        let total: f32 = image
            .pixels
            .chunks_exact(4)
            .map(|p| f32::from(p[3]) / 255.0)
            .sum();
        assert!((total - 64.0).abs() < 1.0, "alpha 합 {total}");
        // blur 뒤 clip: clip 밖은 비고, clip 안 가장자리에는 clip 밖 내용이 번진 값이 있습니다.
        let mut clipped = blurred.clone();
        clipped.ops = vec![effect(
            LayerOp::new(content(), SceneBlendMode::Normal)
                .with_blur(2.0)
                .with_clip(crate::effects::fixture::ClipSpec::Scissor {
                    x: 20,
                    y: 0,
                    width: 12,
                    height: 32,
                }),
        )];
        assert_eq!(working(&clipped, 19, 16).alpha(), 0.0);
        assert!(working(&clipped, 20, 16).alpha() > 0.1);
    }

    #[test]
    fn shadow_follows_content_alpha_offset_and_color() {
        let mut fixture = Fixture::new("shadow", Category::Modifier, 16, 16);
        fixture.clear = srgb(0.0, 0.0, 0.0, 0.0);
        fixture.format = crate::render_graph::TextureFormat::Rgba8Unorm;
        fixture.ops = vec![effect(
            LayerOp::new(
                vec![rect(2.0, 2.0, 8.0, 8.0, srgb(1.0, 1.0, 1.0, 0.5))],
                SceneBlendMode::Normal,
            )
            .with_shadow([4, 4], 0.0, srgb(0.0, 0.0, 1.0, 1.0)),
        )];
        // blur 0 shadow는 내용 alpha(0.5) × 색, offset (4, 4): (10, 10)은 shadow만, (3, 3)은 내용만.
        let shadow_only = working(&fixture, 10, 10);
        assert!((shadow_only.alpha() - 0.5).abs() < 0.01);
        assert!((shadow_only.rgba[2] - 0.5).abs() < 0.01 && shadow_only.rgba[0] < 0.01);
        let content_only = working(&fixture, 3, 3);
        assert!((content_only.rgba[0] - 0.5).abs() < 0.01);
        // 겹친 곳은 내용이 shadow 위입니다.
        let both = working(&fixture, 7, 7);
        assert!((both.alpha() - 0.75).abs() < 0.01);
    }

    fn count(tri: [[f32; 2]; 3], w: u32, h: u32) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        raster_triangle(tri, w, h, |x, y| out.push((x, y)));
        out
    }

    #[test]
    fn shared_edges_cover_each_pixel_once() {
        // 사각형 두 삼각형(대각선 공유)과 반대 방향 삼각형에서 pixel마다 정확히 한 번.
        let mesh = super::super::fixture::Mesh::rect(0.0, 0.0, 4.0, 4.0);
        let mut hits = vec![0; 16];
        for tri in mesh.positions.chunks_exact(3) {
            raster_triangle([tri[0], tri[1], tri[2]], 4, 4, |x, y| {
                hits[(y * 4 + x) as usize] += 1
            });
        }
        assert!(hits.iter().all(|&h| h == 1), "{hits:?}");
        // 중심을 지나는 수직 변: 왼쪽 삼각형은 안 덮고 오른쪽 삼각형이 덮습니다(top-left).
        let left = count([[0.0, 0.0], [0.5, 0.0], [0.5, 2.0]], 2, 2);
        let right = count([[0.5, 0.0], [2.0, 0.0], [2.0, 2.0]], 2, 2);
        assert!(!left.contains(&(0, 0)));
        assert!(right.contains(&(0, 0)) || right.contains(&(1, 0)));
        assert!(count([[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]], 4, 4).is_empty());
    }

    #[test]
    fn union_coverage_paints_overlap_once() {
        let mut once = Vec::new();
        let positions = [
            [0.0, 0.0],
            [4.0, 0.0],
            [0.0, 4.0],
            [0.0, 0.0],
            [4.0, 0.0],
            [0.0, 4.0],
        ];
        raster_mesh(&positions, Coverage::Union, 4, 4, |x, y| once.push((x, y)));
        let mut twice = Vec::new();
        raster_mesh(&positions, Coverage::Disjoint, 4, 4, |x, y| {
            twice.push((x, y))
        });
        assert_eq!(twice.len(), once.len() * 2);
    }

    #[test]
    fn bilinear_sampling_interpolates_between_texel_centers() {
        let a = WorkingColor::premultiplied([0.0, 0.0, 0.0, 1.0]);
        let b = WorkingColor::premultiplied([1.0, 1.0, 1.0, 1.0]);
        let texels = [a, b];
        let mid = sample(
            &texels,
            2,
            1,
            0.5,
            0.5,
            TextureFilter::Linear,
            TextureWrap::ClampToEdge,
        );
        assert!((mid.rgba[0] - 0.5).abs() < 1e-6);
        let edge = sample(
            &texels,
            2,
            1,
            0.0,
            0.5,
            TextureFilter::Linear,
            TextureWrap::ClampToEdge,
        );
        assert_eq!(edge, a);
        assert_eq!(
            sample(
                &texels,
                2,
                1,
                0.74,
                0.5,
                TextureFilter::Nearest,
                TextureWrap::ClampToEdge
            ),
            b
        );
    }
}

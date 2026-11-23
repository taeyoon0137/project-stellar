//! CPU 기준 rasterizer (XI.11.1, ADR-0102 결정 6).
//!
//! fixture를 GPU와 같은 규칙으로 `f32` working 값 canvas에 그린 뒤 target format으로 encode합니다.
//!
//! - **표본**: pixel 중심 `(x + 0.5, y + 0.5)` 하나. 삼각형 변 위의 중심은 top-left 규칙으로 판정합니다(위쪽 수평 변과
//!   왼쪽 변이면 안쪽). GPU rasterizer와 같은 규칙이며, 정점 subpixel 반올림 차이는 structural 허용 오차가 흡수합니다.
//! - **합성**: premultiplied source-over(ADR-0102 결정 3).
//! - **겹침**: [`Coverage::Disjoint`] mesh는 삼각형마다 합성하고(GPU draw 순서와 같음), [`Coverage::Union`]은 pixel마다
//!   한 번만 합성합니다.
//! - **이미지**: texture에 올리는 byte([`ImageData::texture_bytes`])를 decode한 값으로 sampling합니다. Nearest는
//!   `floor(u × w)` texel, Linear는 texel 중심 기준 bilinear(가장자리 clamp)입니다.
//! - **glyph**: mask byte / 255를 coverage로 색에 곱합니다.

use crate::color::{decode_pixel, encode_pixel, to_working, WorkingColor};
use crate::output_color::OutputColorPlan;
use crate::placeholder::TextureFilter;

use super::artifact::RgbaImage;
use super::fixture::{
    Coverage, FillOp, Fixture, FixtureError, GlyphOp, ImageData, ImageOp, Op, Paint,
    IMAGE_TEXTURE_FORMAT,
};

/// working 값 canvas.
#[derive(Debug, Clone, PartialEq)]
pub struct Canvas {
    /// 너비.
    pub width: u32,
    /// 높이.
    pub height: u32,
    /// pixel(행 우선).
    pub pixels: Vec<WorkingColor>,
}

impl Canvas {
    /// `clear`로 채운 canvas.
    pub fn new(width: u32, height: u32, clear: WorkingColor) -> Canvas {
        Canvas {
            width,
            height,
            pixels: vec![clear; (width * height) as usize],
        }
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
        let dst = self.at_mut(x, y);
        *dst = source_over(src, *dst);
    }
}

/// premultiplied source-over `src + dst × (1 − src.a)`.
pub fn source_over(src: WorkingColor, dst: WorkingColor) -> WorkingColor {
    let k = 1.0 - src.alpha();
    WorkingColor::premultiplied(std::array::from_fn(|i| src.rgba[i] + dst.rgba[i] * k))
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
) -> WorkingColor {
    let clamp = |i: i64, n: u32| i.clamp(0, i64::from(n) - 1) as usize;
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
        }
    }
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
    let mut canvas = Canvas::new(fixture.width, fixture.height, fixture.clear_working());
    draw_ops(&mut canvas, &fixture.ops, &plan);
    Ok(encode_canvas(&canvas, fixture))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mid = sample(&texels, 2, 1, 0.5, 0.5, TextureFilter::Linear);
        assert!((mid.rgba[0] - 0.5).abs() < 1e-6);
        let edge = sample(&texels, 2, 1, 0.0, 0.5, TextureFilter::Linear);
        assert_eq!(edge, a);
        assert_eq!(sample(&texels, 2, 1, 0.74, 0.5, TextureFilter::Nearest), b);
    }
}

//! coverage 래스터화: 닫힌 polygon 목록 → pixel별 부분 coverage `[0, 1]`.
//!
//! pixel 한 행을 [`SUBSAMPLES`]개 sub-scanline(세로 위치 `(k + 0.5) / SUBSAMPLES`)으로 나누고, sub-scanline마다
//! 변의 교차점을 x 순으로 훑어 채우기 규칙(non-zero·even-odd)의 안쪽 구간을 구합니다. 구간은 가로 방향으로 정확한
//! 넓이(소수 pixel 포함)를 더하므로 가로는 연속, 세로는 1/[`SUBSAMPLES`] 해상도의 coverage입니다. 계산은 f64이고 순서가
//! 고정이라 결과가 결정론적입니다.

use stellar_scene::geometry::FillRule;

use super::path::Contour;
use crate::svg::raster::PixelRect;

/// pixel 행 하나의 sub-scanline 수.
pub(crate) const SUBSAMPLES: usize = 16;

/// pixel 상자 위 coverage.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Mask {
    rect: PixelRect,
    data: Vec<f32>,
}

impl Mask {
    /// 상자 전체가 `value`인 mask.
    pub(crate) fn filled(rect: PixelRect, value: f32) -> Mask {
        Mask {
            rect,
            data: vec![value; (rect.width() * rect.height()) as usize],
        }
    }

    /// 상자.
    pub(crate) const fn rect(&self) -> PixelRect {
        self.rect
    }

    /// device pixel `(x, y)`의 coverage(상자 밖은 0).
    pub(crate) fn at(&self, x: i64, y: i64) -> f32 {
        let r = &self.rect;
        if x < r.x0() || x >= r.x1() || y < r.y0() || y >= r.y1() {
            return 0.0;
        }
        self.data[((y - r.y0()) * r.width() + (x - r.x0())) as usize]
    }

    /// 상자 행 `y`의 coverage(상자 x0부터).
    pub(crate) fn row(&self, y: i64) -> &[f32] {
        let w = self.rect.width() as usize;
        let start = ((y - self.rect.y0()) as usize) * w;
        &self.data[start..start + w]
    }

    /// `other`를 곱합니다(교집합). 결과 상자는 두 상자의 교집합이며 비면 `None`입니다.
    pub(crate) fn intersect(self, other: &Mask) -> Option<Mask> {
        let rect = self.rect.intersection(&other.rect)?;
        let mut out = Mask::filled(rect, 0.0);
        let w = rect.width() as usize;
        for y in rect.y0()..rect.y1() {
            let row = ((y - rect.y0()) as usize) * w;
            for x in rect.x0()..rect.x1() {
                out.data[row + (x - rect.x0()) as usize] = self.at(x, y) * other.at(x, y);
            }
        }
        Some(out)
    }

    /// `other`의 여집합을 곱합니다(`self × (1 − other)`). 상자는 그대로입니다.
    pub(crate) fn subtract(mut self, other: &Mask) -> Mask {
        let rect = self.rect;
        let w = rect.width() as usize;
        for y in rect.y0()..rect.y1() {
            let row = ((y - rect.y0()) as usize) * w;
            for x in rect.x0()..rect.x1() {
                let slot = &mut self.data[row + (x - rect.x0()) as usize];
                *slot *= 1.0 - other.at(x, y);
            }
        }
        self
    }
}

/// 방향이 있는 변(`y0 < y1`, 원래 아래로 가면 `dir = 1`).
#[derive(Debug, Clone, Copy)]
struct Edge {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    dir: i32,
}

impl Edge {
    fn x_at(&self, y: f64) -> f64 {
        self.x0 + (y - self.y0) * (self.x1 - self.x0) / (self.y1 - self.y0)
    }
}

fn edges(contours: &[Contour]) -> Vec<Edge> {
    let mut out = Vec::new();
    for contour in contours {
        let n = contour.len();
        for i in 0..n {
            let (ax, ay) = contour[i];
            let (bx, by) = contour[(i + 1) % n];
            if ay == by || !(ax.is_finite() && ay.is_finite() && bx.is_finite() && by.is_finite()) {
                continue;
            }
            out.push(if ay < by {
                Edge {
                    x0: ax,
                    y0: ay,
                    x1: bx,
                    y1: by,
                    dir: 1,
                }
            } else {
                Edge {
                    x0: bx,
                    y0: by,
                    x1: ax,
                    y1: ay,
                    dir: -1,
                }
            });
        }
    }
    out
}

/// span `[xa, xb)`(device x)을 행 누적에 더합니다. `partial`은 끝 pixel 부분 넓이, `full`은 완전 pixel 차분 배열입니다.
fn add_span(partial: &mut [f64], full: &mut [i32], x0: i64, xa: f64, xb: f64) {
    let width = partial.len() as f64;
    let (xa, xb) = (
        (xa - x0 as f64).clamp(0.0, width),
        (xb - x0 as f64).clamp(0.0, width),
    );
    if xb <= xa {
        return;
    }
    let (ia, ib) = (xa.floor() as usize, xb.floor() as usize);
    if ia == ib {
        partial[ia] += xb - xa;
        return;
    }
    partial[ia] += (ia + 1) as f64 - xa;
    if ia + 1 < ib {
        full[ia + 1] += 1;
        full[ib] -= 1;
    }
    if ib < partial.len() {
        partial[ib] += xb - ib as f64;
    }
}

/// polygon 목록을 `clip` 상자 안에서 래스터화합니다. 덮는 곳이 없으면 `None`입니다.
pub(crate) fn rasterize(contours: &[Contour], rule: FillRule, clip: PixelRect) -> Option<Mask> {
    let mut edges = edges(contours);
    if edges.is_empty() {
        return None;
    }
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for e in &edges {
        min_x = min_x.min(e.x0.min(e.x1));
        max_x = max_x.max(e.x0.max(e.x1));
        min_y = min_y.min(e.y0);
        max_y = max_y.max(e.y1);
    }
    let limit = (1i64 << 40) as f64;
    let clamp = |v: f64| v.clamp(-limit, limit) as i64;
    let bounds = PixelRect::new(
        clamp(min_x.floor()),
        clamp(min_y.floor()),
        clamp(max_x.floor()) + 1,
        clamp(max_y.ceil()),
    )?;
    let rect = bounds.intersection(&clip)?;
    edges.sort_by(|a, b| a.y0.total_cmp(&b.y0));
    let width = rect.width() as usize;
    let mut mask = Mask::filled(rect, 0.0);
    let mut active: Vec<Edge> = Vec::new();
    let mut next = 0usize;
    let mut crossings: Vec<(f64, i32)> = Vec::new();
    let mut partial = vec![0.0f64; width];
    let mut full = vec![0i32; width + 1];
    let weight = 1.0 / SUBSAMPLES as f64;
    for y in rect.y0()..rect.y1() {
        partial.fill(0.0);
        full.fill(0);
        for k in 0..SUBSAMPLES {
            let sy = y as f64 + (k as f64 + 0.5) * weight;
            while next < edges.len() && edges[next].y0 <= sy {
                active.push(edges[next]);
                next += 1;
            }
            active.retain(|e| e.y1 > sy);
            crossings.clear();
            crossings.extend(
                active
                    .iter()
                    .filter(|e| e.y0 <= sy)
                    .map(|e| (e.x_at(sy), e.dir)),
            );
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let inside = |w: i32| match rule {
                FillRule::NonZero => w != 0,
                FillRule::EvenOdd => w % 2 != 0,
            };
            let mut winding = 0i32;
            let mut span_start = 0.0f64;
            for &(x, dir) in &crossings {
                let was = inside(winding);
                winding += dir;
                match (was, inside(winding)) {
                    (false, true) => span_start = x,
                    (true, false) => add_span(&mut partial, &mut full, rect.x0(), span_start, x),
                    _ => {}
                }
            }
        }
        let row = ((y - rect.y0()) as usize) * width;
        let mut run = 0i32;
        for i in 0..width {
            run += full[i];
            let value = (partial[i] + f64::from(run)) * weight;
            mask.data[row + i] = value.clamp(0.0, 1.0) as f32;
        }
    }
    Some(mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Contour {
        vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    }

    fn total(mask: &Mask) -> f64 {
        mask.data.iter().map(|v| f64::from(*v)).sum()
    }

    fn clip() -> PixelRect {
        PixelRect::new(-100, -100, 100, 100).unwrap()
    }

    #[test]
    fn pixel_aligned_square_has_exact_coverage() {
        let mask = rasterize(&[square(1.0, 2.0, 4.0, 5.0)], FillRule::NonZero, clip()).unwrap();
        assert_eq!(mask.at(1, 2), 1.0);
        assert_eq!(mask.at(3, 4), 1.0);
        assert_eq!(mask.at(4, 4), 0.0);
        assert_eq!(mask.at(0, 2), 0.0);
        assert!((total(&mask) - 9.0).abs() < 1e-6);
    }

    #[test]
    fn fractional_edges_have_partial_coverage() {
        let mask = rasterize(&[square(0.25, 0.0, 1.75, 1.0)], FillRule::NonZero, clip()).unwrap();
        assert!((mask.at(0, 0) - 0.75).abs() < 1e-6);
        assert!((mask.at(1, 0) - 0.75).abs() < 1e-6);
        let half_row = rasterize(&[square(0.0, 0.0, 1.0, 0.5)], FillRule::NonZero, clip()).unwrap();
        assert!((half_row.at(0, 0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn fill_rules_differ_on_nested_contours() {
        let outer = square(0.0, 0.0, 10.0, 10.0);
        let inner = square(3.0, 3.0, 7.0, 7.0); // 같은 방향
        let contours = [outer, inner];
        let nonzero = rasterize(&contours, FillRule::NonZero, clip()).unwrap();
        let evenodd = rasterize(&contours, FillRule::EvenOdd, clip()).unwrap();
        assert_eq!(nonzero.at(5, 5), 1.0);
        assert_eq!(evenodd.at(5, 5), 0.0);
        assert!((total(&evenodd) - 84.0).abs() < 1e-6);
    }

    #[test]
    fn clip_box_limits_mask_and_masks_combine() {
        let mask = rasterize(
            &[square(-50.0, -50.0, 50.0, 50.0)],
            FillRule::NonZero,
            PixelRect::new(0, 0, 4, 4).unwrap(),
        )
        .unwrap();
        assert_eq!(mask.rect(), PixelRect::new(0, 0, 4, 4).unwrap());
        assert!((total(&mask) - 16.0).abs() < 1e-6);
        let hole = rasterize(&[square(1.0, 1.0, 3.0, 3.0)], FillRule::NonZero, clip()).unwrap();
        let ring = mask.clone().subtract(&hole);
        assert!((total(&ring) - 12.0).abs() < 1e-6);
        let center = mask.intersect(&hole).unwrap();
        assert!((total(&center) - 4.0).abs() < 1e-6);
        assert!(rasterize(
            &[square(200.0, 200.0, 201.0, 201.0)],
            FillRule::NonZero,
            clip()
        )
        .is_none());
    }

    #[test]
    fn triangle_area_is_preserved() {
        let tri = vec![(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)];
        let mask = rasterize(&[tri], FillRule::NonZero, clip()).unwrap();
        assert!((total(&mask) - 50.0).abs() < 0.05, "{}", total(&mask));
    }
}

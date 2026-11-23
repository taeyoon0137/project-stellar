//! 적합성 fixture용 fill tessellator: 가로 band 사다리꼴 분해 (ADR-0102 결정 6).
//!
//! 제품 fill tessellator가 아직 없어(XI.6.1은 interface만) harness가 쓰는 [`FillTessellator`] 구현입니다.
//!
//! 1. path를 subpath마다 닫힌 다각형으로 평탄화합니다(cubic은 균등 분할, 분할 수는 control polygon 길이와 허용
//!    오차로 정함). 열린 subpath는 fill 규칙대로 시작점으로 닫습니다.
//! 2. 수평이 아닌 변을 모으고, 정점 y와 변끼리 교차하는 y를 모두 band 경계로 둡니다. band 안에서는 변이 교차하지
//!    않으므로 x 순서가 고정됩니다.
//! 3. band마다 band 중간 높이의 x 순서로 변을 훑어 winding을 누적하고, fill 규칙(nonzero·evenodd)으로 안쪽인 구간을
//!    사다리꼴(삼각형 2개)로 냅니다.
//!
//! 결과 삼각형은 서로 겹치지 않으므로(경계만 공유) premultiplied over로 바로 그려도 pixel마다 한 번 칠해집니다.
//! 변 수가 [`MAX_EDGES`]를 넘으면 교차 계산이 제곱 비용이라 [`TessellationError::TooComplex`]입니다.
//!
//! stroke tessellator(XI.6.2)의 내부 평탄화와는 별도 구현입니다. 두 평탄화를 합치는 정리는 ADR-0102 Consequences의
//! 후속입니다.

use stellar_scene::geometry::{FillRule, PathCommand, PathGeometry};
use stellar_scene::Vec2;

use crate::tessellation::{FillMesh, FillStyle, FillTessellator, TessellationError};

/// 평탄화 뒤 변 수 상한.
pub const MAX_EDGES: usize = 4096;

/// cubic 하나의 최대 분할 수.
const MAX_CUBIC_STEPS: u32 = 256;

/// 가로 band 사다리꼴 분해 fill tessellator(모듈 문서).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepFillTessellator;

impl FillTessellator for SweepFillTessellator {
    fn tessellate_fill(
        &self,
        path: &PathGeometry,
        style: &FillStyle,
    ) -> Result<FillMesh, TessellationError> {
        let polygons = flatten_closed(path, f64::from(style.tolerance.get()));
        sweep(&polygons, style.fill_rule)
    }
}

#[derive(Debug, Clone, Copy)]
struct Edge {
    /// 위쪽 끝(y 작은 쪽).
    top: (f64, f64),
    /// 아래쪽 끝.
    bottom: (f64, f64),
    /// 원래 방향이 y 증가면 +1, 감소면 −1.
    winding: i32,
}

impl Edge {
    fn x_at(&self, y: f64) -> f64 {
        let (x0, y0) = self.top;
        let (x1, y1) = self.bottom;
        if y1 == y0 {
            return x0;
        }
        x0 + (x1 - x0) * (y - y0) / (y1 - y0)
    }
}

fn point(v: Vec2) -> (f64, f64) {
    (f64::from(v.x()), f64::from(v.y()))
}

/// subpath마다 닫힌 다각형 점 목록(마지막 점에서 첫 점으로 닫힘).
pub(crate) fn flatten_closed(path: &PathGeometry, tolerance: f64) -> Vec<Vec<(f64, f64)>> {
    let mut polygons = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    let tolerance = if tolerance.is_finite() && tolerance > 0.0 {
        tolerance
    } else {
        0.25
    };
    for command in path.commands() {
        match *command {
            PathCommand::MoveTo(to) => {
                if current.len() >= 3 {
                    polygons.push(std::mem::take(&mut current));
                }
                current.clear();
                current.push(point(to));
            }
            PathCommand::LineTo(to) => current.push(point(to)),
            PathCommand::CubicTo { c1, c2, to } => {
                let p0 = *current.last().unwrap_or(&(0.0, 0.0));
                let (p1, p2, p3) = (point(c1), point(c2), point(to));
                let dist = |a: (f64, f64), b: (f64, f64)| {
                    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
                };
                let length = dist(p0, p1) + dist(p1, p2) + dist(p2, p3);
                let steps = ((length / tolerance).sqrt().ceil() as u32).clamp(1, MAX_CUBIC_STEPS);
                for i in 1..=steps {
                    let t = f64::from(i) / f64::from(steps);
                    let u = 1.0 - t;
                    let b0 = u * u * u;
                    let b1 = 3.0 * u * u * t;
                    let b2 = 3.0 * u * t * t;
                    let b3 = t * t * t;
                    current.push((
                        b0 * p0.0 + b1 * p1.0 + b2 * p2.0 + b3 * p3.0,
                        b0 * p0.1 + b1 * p1.1 + b2 * p2.1 + b3 * p3.1,
                    ));
                }
            }
            PathCommand::Close => {
                if current.len() >= 3 {
                    polygons.push(current.clone());
                }
                // 닫은 뒤 이어지는 그리기는 같은 시작점에서 새 subpath로 시작합니다.
                let start = current.first().copied();
                current.clear();
                if let Some(start) = start {
                    current.push(start);
                }
            }
        }
    }
    if current.len() >= 3 {
        polygons.push(current);
    }
    polygons
}

fn edges_of(polygons: &[Vec<(f64, f64)>]) -> Vec<Edge> {
    let mut edges = Vec::new();
    for polygon in polygons {
        for i in 0..polygon.len() {
            let a = polygon[i];
            let b = polygon[(i + 1) % polygon.len()];
            if a.1 == b.1 {
                continue;
            }
            if a.1 < b.1 {
                edges.push(Edge {
                    top: a,
                    bottom: b,
                    winding: 1,
                });
            } else {
                edges.push(Edge {
                    top: b,
                    bottom: a,
                    winding: -1,
                });
            }
        }
    }
    edges
}

/// 두 변이 y 범위 안에서 교차하는 y.
fn crossing_y(a: &Edge, b: &Edge) -> Option<f64> {
    let y0 = a.top.1.max(b.top.1);
    let y1 = a.bottom.1.min(b.bottom.1);
    if y1 <= y0 {
        return None;
    }
    let d0 = a.x_at(y0) - b.x_at(y0);
    let d1 = a.x_at(y1) - b.x_at(y1);
    if (d0 > 0.0 && d1 < 0.0) || (d0 < 0.0 && d1 > 0.0) {
        let t = d0 / (d0 - d1);
        Some(y0 + (y1 - y0) * t)
    } else {
        None
    }
}

fn sweep(polygons: &[Vec<(f64, f64)>], rule: FillRule) -> Result<FillMesh, TessellationError> {
    let edges = edges_of(polygons);
    if edges.len() > MAX_EDGES {
        return Err(TessellationError::TooComplex { limit: MAX_EDGES });
    }
    let mut ys: Vec<f64> = edges.iter().flat_map(|e| [e.top.1, e.bottom.1]).collect();
    for i in 0..edges.len() {
        for j in (i + 1)..edges.len() {
            if let Some(y) = crossing_y(&edges[i], &edges[j]) {
                ys.push(y);
            }
        }
    }
    ys.retain(|y| y.is_finite());
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    let mut vertices: Vec<Vec2> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut active: Vec<(f64, usize)> = Vec::new();
    for band in ys.windows(2) {
        let (y0, y1) = (band[0], band[1]);
        if y1 <= y0 {
            continue;
        }
        let mid = (y0 + y1) * 0.5;
        active.clear();
        active.extend(
            edges
                .iter()
                .enumerate()
                .filter(|(_, e)| e.top.1 <= y0 && e.bottom.1 >= y1)
                .map(|(i, e)| (e.x_at(mid), i)),
        );
        active.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut winding = 0i32;
        for pair in 0..active.len() {
            let edge = &edges[active[pair].1];
            winding += edge.winding;
            let inside = match rule {
                FillRule::NonZero => winding != 0,
                FillRule::EvenOdd => winding % 2 != 0,
            };
            if !inside || pair + 1 >= active.len() {
                continue;
            }
            let right = &edges[active[pair + 1].1];
            let quad = [
                (edge.x_at(y0), y0),
                (right.x_at(y0), y0),
                (right.x_at(y1), y1),
                (edge.x_at(y1), y1),
            ];
            push_quad(&mut vertices, &mut indices, quad)?;
        }
    }
    FillMesh::new(vertices, indices)
}

fn push_quad(
    vertices: &mut Vec<Vec2>,
    indices: &mut Vec<u32>,
    quad: [(f64, f64); 4],
) -> Result<(), TessellationError> {
    let base = vertices.len() as u32;
    for (x, y) in quad {
        vertices.push(
            Vec2::new(x as f32, y as f32).map_err(|_| TessellationError::Failed {
                reason: "평탄화 좌표가 유한하지 않습니다",
            })?,
        );
    }
    let top_width = quad[1].0 - quad[0].0;
    let bottom_width = quad[2].0 - quad[3].0;
    if top_width > 0.0 {
        indices.extend_from_slice(&[base, base + 1, base + 2]);
    }
    if bottom_width > 0.0 {
        indices.extend_from_slice(&[base, base + 2, base + 3]);
    }
    if top_width <= 0.0 && bottom_width <= 0.0 {
        vertices.truncate(base as usize);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y).unwrap()
    }

    fn polygon(points: &[(f32, f32)]) -> Vec<PathCommand> {
        let mut commands = vec![PathCommand::MoveTo(v(points[0].0, points[0].1))];
        commands.extend(points[1..].iter().map(|p| PathCommand::LineTo(v(p.0, p.1))));
        commands.push(PathCommand::Close);
        commands
    }

    fn area(mesh: &FillMesh) -> f64 {
        mesh.indices()
            .chunks_exact(3)
            .map(|t| {
                let p = |i: u32| point(mesh.vertices()[i as usize]);
                let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
                ((b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1)).abs() / 2.0
            })
            .sum()
    }

    fn fill(commands: Vec<PathCommand>, rule: FillRule) -> FillMesh {
        SweepFillTessellator
            .tessellate_fill(
                &PathGeometry::new(commands).unwrap(),
                &FillStyle {
                    fill_rule: rule,
                    ..FillStyle::default()
                },
            )
            .unwrap()
    }

    #[test]
    fn rectangle_and_concave_polygon_areas_are_exact() {
        let rect = fill(
            polygon(&[(1.0, 1.0), (9.0, 1.0), (9.0, 5.0), (1.0, 5.0)]),
            FillRule::NonZero,
        );
        assert!((area(&rect) - 32.0).abs() < 1e-6);
        // L 모양(오목) 면적 = 4×4 − 2×2.
        let l = fill(
            polygon(&[
                (0.0, 0.0),
                (4.0, 0.0),
                (4.0, 2.0),
                (2.0, 2.0),
                (2.0, 4.0),
                (0.0, 4.0),
            ]),
            FillRule::EvenOdd,
        );
        assert!((area(&l) - 12.0).abs() < 1e-6);
    }

    #[test]
    fn fill_rules_differ_on_self_intersecting_star() {
        // 오각별: nonzero는 가운데 오각형까지 채우고 evenodd는 비웁니다.
        let star: Vec<(f32, f32)> = (0..5)
            .map(|i| {
                let angle = std::f32::consts::PI * 2.0 * (i as f32 * 2.0) / 5.0
                    - std::f32::consts::FRAC_PI_2;
                (16.0 + 14.0 * angle.cos(), 16.0 + 14.0 * angle.sin())
            })
            .collect();
        let nonzero = area(&fill(polygon(&star), FillRule::NonZero));
        let evenodd = area(&fill(polygon(&star), FillRule::EvenOdd));
        assert!(nonzero > evenodd + 50.0, "{nonzero} {evenodd}");
        // 두 사각형 겹침: nonzero는 합집합, evenodd는 대칭차.
        let mut two = polygon(&[(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)]);
        two.extend(polygon(&[(2.0, 2.0), (6.0, 2.0), (6.0, 6.0), (2.0, 6.0)]));
        assert!((area(&fill(two.clone(), FillRule::NonZero)) - 28.0).abs() < 1e-6);
        assert!((area(&fill(two, FillRule::EvenOdd)) - 24.0).abs() < 1e-6);
    }

    #[test]
    fn opposite_winding_hole_is_empty_under_nonzero() {
        let mut ring = polygon(&[(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)]);
        ring.extend(polygon(&[(2.0, 2.0), (2.0, 6.0), (6.0, 6.0), (6.0, 2.0)]));
        assert!((area(&fill(ring, FillRule::NonZero)) - 48.0).abs() < 1e-6);
    }

    #[test]
    fn cubic_circle_area_converges() {
        // 4개 cubic 원(반지름 10): 면적 ≈ π r².
        let k = 0.552_284_8 * 10.0;
        let (cx, cy, r) = (16.0, 16.0, 10.0);
        let commands = vec![
            PathCommand::MoveTo(v(cx + r, cy)),
            PathCommand::CubicTo {
                c1: v(cx + r, cy + k),
                c2: v(cx + k, cy + r),
                to: v(cx, cy + r),
            },
            PathCommand::CubicTo {
                c1: v(cx - k, cy + r),
                c2: v(cx - r, cy + k),
                to: v(cx - r, cy),
            },
            PathCommand::CubicTo {
                c1: v(cx - r, cy - k),
                c2: v(cx - k, cy - r),
                to: v(cx, cy - r),
            },
            PathCommand::CubicTo {
                c1: v(cx + k, cy - r),
                c2: v(cx + r, cy - k),
                to: v(cx + r, cy),
            },
            PathCommand::Close,
        ];
        let a = area(&fill(commands, FillRule::NonZero));
        assert!((a - std::f64::consts::PI * 100.0).abs() < 2.0, "{a}");
        assert_eq!(fill(Vec::new(), FillRule::NonZero).triangle_count(), 0);
    }
}

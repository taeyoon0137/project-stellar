//! path 평탄화와 device 변환.
//!
//! scene geometry(node local)를 world transform → 내보내기 device 변환으로 옮겨 닫힌 polygon 목록으로 바꿉니다.
//! 계산은 f64입니다. cubic은 device 좌표에서 Wang 공식으로 나눌 수를 정해 허용 오차 [`FLATTEN_TOLERANCE`] pixel
//! 안으로 평탄화합니다.

use stellar_core_types::value::{Rect, Transform2D, Vec2};
use stellar_scene::geometry::{PathCommand, PathGeometry};
use stellar_scene::node::CornerRadii;

use crate::svg::raster::{PixelGrid, PixelRect};

/// 곡선 평탄화 허용 오차(device pixel).
pub(crate) const FLATTEN_TOLERANCE: f64 = 0.05;

/// cubic 하나의 최대 분할 수.
const MAX_SEGMENTS: usize = 4096;

/// 원호를 cubic으로 근사하는 control 거리 비율(4/3·(√2 − 1)).
const KAPPA: f32 = 0.552_284_8;

/// f64 2D affine 변환 `(a, b, c, d, tx, ty)`: `x' = a·x + c·y + tx`, `y' = b·x + d·y + ty`(core-types
/// `Transform2D`와 같은 배치).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Affine {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
}

impl Affine {
    /// scene 변환.
    pub(crate) fn from_transform(transform: &Transform2D) -> Affine {
        let (a, b, c, d, tx, ty) = transform.components();
        Affine {
            a: f64::from(a),
            b: f64::from(b),
            c: f64::from(c),
            d: f64::from(d),
            tx: f64::from(tx),
            ty: f64::from(ty),
        }
    }

    /// world → device 변환(`(p − 영역 원점) × scale`). grid 원점·배율은 grid 자신의 pixel → world 정의에서 읽습니다.
    pub(crate) fn device(grid: &PixelGrid) -> Affine {
        let s = grid.scale();
        let unit = PixelRect::new(0, 0, 1, 1).map(|r| grid.world_rect(&r));
        let [ox, oy, _, _] = unit.unwrap_or([0.0; 4]);
        Affine {
            a: s,
            b: 0.0,
            c: 0.0,
            d: s,
            tx: -ox * s,
            ty: -oy * s,
        }
    }

    /// `self`를 먼저, 그다음 `next`.
    pub(crate) fn then(&self, next: &Affine) -> Affine {
        Affine {
            a: next.a * self.a + next.c * self.b,
            b: next.b * self.a + next.d * self.b,
            c: next.a * self.c + next.c * self.d,
            d: next.b * self.c + next.d * self.d,
            tx: next.a * self.tx + next.c * self.ty + next.tx,
            ty: next.b * self.tx + next.d * self.ty + next.ty,
        }
    }

    /// 점 변환.
    pub(crate) fn apply(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.tx,
            self.b * x + self.d * y + self.ty,
        )
    }

    /// 역변환. 행렬식이 0이거나 비유한이면 `None`.
    pub(crate) fn inverse(&self) -> Option<Affine> {
        let det = self.a * self.d - self.b * self.c;
        if det == 0.0 || !det.is_finite() {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        let inverse = Affine {
            a,
            b,
            c,
            d,
            tx: -(a * self.tx + c * self.ty),
            ty: -(b * self.tx + d * self.ty),
        };
        [a, b, c, d, inverse.tx, inverse.ty]
            .iter()
            .all(|v| v.is_finite())
            .then_some(inverse)
    }

    /// local 1 단위가 device에서 늘어나는 최대 길이(선형부 최대 특이값).
    pub(crate) fn max_scale(&self) -> f64 {
        let p = self.a * self.a + self.b * self.b;
        let q = self.c * self.c + self.d * self.d;
        let r = self.a * self.c + self.b * self.d;
        let mean = (p + q) / 2.0;
        let spread = (((p - q) / 2.0).powi(2) + r * r).sqrt();
        (mean + spread).sqrt()
    }
}

/// 닫힌 polygon(마지막 점 → 첫 점 변이 암묵적으로 있음).
pub(crate) type Contour = Vec<(f64, f64)>;

fn point(v: Vec2) -> (f64, f64) {
    (f64::from(v.x()), f64::from(v.y()))
}

/// cubic 하나를 device 좌표에서 평탄화해 `out`에 끝점들을 더합니다(시작점 제외).
fn flatten_cubic(out: &mut Contour, p: [(f64, f64); 4], tolerance: f64) {
    let second = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| {
        let (x, y) = (a.0 - 2.0 * b.0 + c.0, a.1 - 2.0 * b.1 + c.1);
        (x * x + y * y).sqrt()
    };
    let m = second(p[0], p[1], p[2]).max(second(p[1], p[2], p[3]));
    let n = (0.75 * m / tolerance).sqrt().ceil();
    let n = if n.is_finite() {
        (n as usize).clamp(1, MAX_SEGMENTS)
    } else {
        1
    };
    for i in 1..=n {
        let t = i as f64 / n as f64;
        let u = 1.0 - t;
        let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        out.push((
            w0 * p[0].0 + w1 * p[1].0 + w2 * p[2].0 + w3 * p[3].0,
            w0 * p[0].1 + w1 * p[1].1 + w2 * p[2].1 + w3 * p[3].1,
        ));
    }
}

/// geometry를 `transform`(local → device)으로 옮겨 subpath마다 닫힌 polygon으로 평탄화합니다. 열린 subpath도 채우기
/// 규칙대로 닫힌 것으로 봅니다(SVG·renderer fill 규칙).
pub(crate) fn flatten(geometry: &PathGeometry, transform: &Affine) -> Vec<Contour> {
    let mut contours = Vec::new();
    let mut current: Contour = Vec::new();
    let mut last = (0.0, 0.0);
    let finish = |current: &mut Contour, contours: &mut Vec<Contour>| {
        if current.len() >= 3 {
            contours.push(std::mem::take(current));
        } else {
            current.clear();
        }
    };
    for command in geometry.commands() {
        match command {
            PathCommand::MoveTo(to) => {
                finish(&mut current, &mut contours);
                last = transform.apply(point(*to));
                current.push(last);
            }
            PathCommand::LineTo(to) => {
                last = transform.apply(point(*to));
                current.push(last);
            }
            PathCommand::CubicTo { c1, c2, to } => {
                let p = [
                    last,
                    transform.apply(point(*c1)),
                    transform.apply(point(*c2)),
                    transform.apply(point(*to)),
                ];
                flatten_cubic(&mut current, p, FLATTEN_TOLERANCE);
                last = p[3];
            }
            PathCommand::Close => {
                let start = current.first().copied();
                finish(&mut current, &mut contours);
                if let Some(start) = start {
                    // Close 뒤 다음 command는 subpath 시작점에서 이어집니다.
                    last = start;
                    current.push(start);
                }
            }
        }
    }
    finish(&mut current, &mut contours);
    contours
}

/// 사각형 path(시계 방향, 화면 좌표).
pub(crate) fn rect_geometry(rect: Rect) -> Option<PathGeometry> {
    rounded_rect_geometry(rect, CornerRadii::ZERO)
}

/// 모서리 둥근 사각형 path(시계 방향). radius는 scene 실효 값이며 원호는 cubic 하나로 근사합니다.
pub(crate) fn rounded_rect_geometry(rect: Rect, radii: CornerRadii) -> Option<PathGeometry> {
    let (x, y) = (rect.origin().x(), rect.origin().y());
    let (w, h) = (rect.size().width(), rect.size().height());
    let (tl, tr, br, bl) = radii.components();
    let v = |px: f32, py: f32| Vec2::new(px, py).ok();
    let mut commands = vec![PathCommand::MoveTo(v(x + tl, y)?)];
    let corner = |commands: &mut Vec<PathCommand>,
                  line_to: (f32, f32),
                  c1: (f32, f32),
                  c2: (f32, f32),
                  to: (f32, f32),
                  r: f32|
     -> Option<()> {
        commands.push(PathCommand::LineTo(v(line_to.0, line_to.1)?));
        if r > 0.0 {
            commands.push(PathCommand::CubicTo {
                c1: v(c1.0, c1.1)?,
                c2: v(c2.0, c2.1)?,
                to: v(to.0, to.1)?,
            });
        }
        Some(())
    };
    let k = KAPPA;
    corner(
        &mut commands,
        (x + w - tr, y),
        (x + w - tr + tr * k, y),
        (x + w, y + tr - tr * k),
        (x + w, y + tr),
        tr,
    )?;
    corner(
        &mut commands,
        (x + w, y + h - br),
        (x + w, y + h - br + br * k),
        (x + w - br + br * k, y + h),
        (x + w - br, y + h),
        br,
    )?;
    corner(
        &mut commands,
        (x + bl, y + h),
        (x + bl - bl * k, y + h),
        (x, y + h - bl + bl * k),
        (x, y + h - bl),
        bl,
    )?;
    corner(
        &mut commands,
        (x, y + tl),
        (x, y + tl - tl * k),
        (x + tl - tl * k, y),
        (x + tl, y),
        tl,
    )?;
    commands.push(PathCommand::Close);
    PathGeometry::new(commands).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::value::Size2;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(Vec2::new(x, y).unwrap(), Size2::new(w, h).unwrap())
    }

    fn area(contour: &Contour) -> f64 {
        let mut sum = 0.0;
        for i in 0..contour.len() {
            let (x0, y0) = contour[i];
            let (x1, y1) = contour[(i + 1) % contour.len()];
            sum += x0 * y1 - x1 * y0;
        }
        sum / 2.0
    }

    #[test]
    fn rounded_rect_area_approaches_exact_value() {
        let geometry = rounded_rect_geometry(
            rect(0.0, 0.0, 100.0, 60.0),
            CornerRadii::new(10.0, 10.0, 10.0, 10.0).unwrap(),
        )
        .unwrap();
        let identity = Affine::from_transform(&Transform2D::identity());
        let contours = flatten(&geometry, &identity);
        assert_eq!(contours.len(), 1);
        let exact = 100.0 * 60.0 - (4.0 - std::f64::consts::PI) * 100.0;
        // 평탄화 chord는 곡선 안쪽이라 넓이가 조금 줄어듭니다(모서리마다 허용 오차 × 호 길이 정도).
        assert!(
            (area(&contours[0]) - exact).abs() < 2.5,
            "{}",
            area(&contours[0])
        );
        // 직각 사각형은 꼭짓점 4개(닫는 점 포함 5개)입니다.
        let plain = flatten(&rect_geometry(rect(1.0, 2.0, 3.0, 4.0)).unwrap(), &identity);
        assert_eq!(plain[0].len(), 5);
        assert!((area(&plain[0]) - 12.0).abs() < 1e-9);
    }

    #[test]
    fn affine_composition_and_inverse() {
        let t = Affine::from_transform(&Transform2D::new(0.0, 1.0, -1.0, 0.0, 5.0, 7.0).unwrap());
        let s = Affine::from_transform(&Transform2D::new(2.0, 0.0, 0.0, 2.0, 0.0, 0.0).unwrap());
        let both = t.then(&s);
        assert_eq!(both.apply((1.0, 0.0)), (10.0, 16.0));
        let back = both.inverse().unwrap().apply((10.0, 16.0));
        assert!((back.0 - 1.0).abs() < 1e-12 && back.1.abs() < 1e-12);
        assert!((both.max_scale() - 2.0).abs() < 1e-12);
    }
}

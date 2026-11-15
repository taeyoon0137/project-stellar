//! Shape payload (VI.5).
//!
//! 파라메트릭 도형은 **하나의 node kind**([`NodeKind::Shape`](crate::node_kind::NodeKind))
//! 안에서 [`ShapeKind`]로 구분합니다 — 도형 종류마다 node kind를 늘리지 않습니다.

use std::fmt;

use stellar_core_types::color::Color;
use stellar_core_types::value::{Rect, Size2, Transform2D, Vec2};

use crate::frame::{Border, CornerRadius};
use crate::modifier::{ModifierId, ModifierPayload, ModifierStack};
use crate::path::PathCommand;

/// Shape payload 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShapeError {
    /// 알 수 없는 shape kind tag.
    UnknownKind {
        /// 발견된 tag.
        tag: u8,
    },
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
    /// 이 버전이 모르는 mandatory field입니다(unknown field 정책 R6).
    UnknownMandatoryField {
        /// field tag.
        tag: u32,
    },
    /// 필수 field가 없습니다.
    MissingField {
        /// field tag.
        tag: u32,
    },
    /// polygon 변 수가 허용 범위(3..=MAX_POLYGON_SIDES)를 벗어납니다.
    InvalidSideCount {
        /// 요청한 변 수.
        sides: u32,
    },
    /// corner radius가 음수이거나 비유한입니다.
    InvalidCornerRadius,
    /// bounds 끝 좌표(origin + size)나 path 평가 결과가 f32 유한 범위를 벗어납니다.
    NonFiniteGeometry,
}

impl fmt::Display for ShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShapeError::UnknownKind { tag } => write!(f, "알 수 없는 shape kind tag: {tag}"),
            ShapeError::Malformed { reason } => write!(f, "Shape binary 형식 위반: {reason}"),
            ShapeError::UnknownMandatoryField { tag } => {
                write!(f, "알 수 없는 mandatory field입니다: {tag}")
            }
            ShapeError::MissingField { tag } => write!(f, "필수 field가 없습니다: {tag}"),
            ShapeError::InvalidSideCount { sides } => write!(
                f,
                "polygon 변 수는 3..={MAX_POLYGON_SIDES}이어야 합니다: {sides}"
            ),
            ShapeError::InvalidCornerRadius => {
                f.write_str("corner radius는 비음수 유한 값이어야 합니다")
            }
            ShapeError::NonFiniteGeometry => {
                f.write_str("도형 좌표가 f32 유한 범위를 벗어납니다(bounds 끝 좌표 또는 path 평가)")
            }
        }
    }
}

impl std::error::Error for ShapeError {}

/// 도형 종류. 숫자 tag는 저장 포맷의 일부입니다(golden test로 고정).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ShapeKind {
    /// 사각형(VI.5.2).
    Rectangle = 1,
    /// 타원(VI.5.3).
    Ellipse = 2,
    /// 직선(VI.5.4).
    Line = 3,
    /// 정다각형(VI.5.5).
    Polygon = 4,
}

impl ShapeKind {
    /// 정의된 모든 종류.
    pub const ALL: [ShapeKind; 4] = [
        ShapeKind::Rectangle,
        ShapeKind::Ellipse,
        ShapeKind::Line,
        ShapeKind::Polygon,
    ];

    /// 안정 numeric tag.
    pub fn tag(self) -> u8 {
        self as u8
    }

    /// tag에서 복원합니다.
    pub fn from_tag(tag: u8) -> Result<ShapeKind, ShapeError> {
        ShapeKind::ALL
            .into_iter()
            .find(|k| k.tag() == tag)
            .ok_or(ShapeError::UnknownKind { tag })
    }

    /// 진단용 이름.
    pub fn name(self) -> &'static str {
        match self {
            ShapeKind::Rectangle => "rectangle",
            ShapeKind::Ellipse => "ellipse",
            ShapeKind::Line => "line",
            ShapeKind::Polygon => "polygon",
        }
    }
}

/// Rectangle 파라미터 (VI.5.2).
///
/// 저장값은 사용자가 지정한 **원본 radius**를 그대로 유지합니다. bounds를 넘는
/// radius는 평가 단계([`RectangleParams::effective_corner_radius`])에서만
/// clamp합니다 — 크기를 다시 키우면 원래 둥글기가 돌아옵니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RectangleParams {
    /// 크기(비음수).
    pub size: Size2,
    /// 개별 corner radius(원본, 비음수).
    pub corner_radius: CornerRadius,
}

impl RectangleParams {
    /// 렌더·path 평가에 쓰는 실제 radius `(top_left, top_right, bottom_right,
    /// bottom_left)`.
    ///
    /// 각 변에서 인접한 두 radius의 합이 변 길이를 넘으면, 모든 radius를 같은
    /// 비율 `f = min(변 길이 / 인접 radius 합)`으로 줄입니다(형태 비율 보존).
    pub fn effective_corner_radius(&self) -> (f32, f32, f32, f32) {
        let (tl, tr, br, bl) = self.corner_radius.components();
        let w = self.size.width();
        let h = self.size.height();
        let mut factor: f32 = 1.0;
        for (side, sum) in [(w, tl + tr), (h, tr + br), (w, br + bl), (h, bl + tl)] {
            if sum > side {
                factor = factor.min(if sum > 0.0 { side / sum } else { 1.0 });
            }
        }
        (tl * factor, tr * factor, br * factor, bl * factor)
    }

    /// 사각형 path(shape local 원점 기준). 실효 radius가 0인 모서리는 직각,
    /// 양수면 1/4 원호를 cubic(κ)으로 근사합니다. 위 변 왼쪽에서 시작해 y-down
    /// 기준 시계 방향으로 그리고 닫습니다.
    pub fn to_path(&self) -> Vec<PathCommand> {
        let (tl, tr, br, bl) = self.effective_corner_radius();
        let w = self.size.width();
        let h = self.size.height();
        // size·실효 radius가 유한 비음수이고 각 좌표가 [0, w]·[0, h] 안이므로 항상 유한합니다
        // (origin 없음, 합이 넘칠 수 없음).
        let p = |x: f32, y: f32| Vec2::new(x, y).expect("finite rectangle");
        let k = CIRCLE_KAPPA;
        let mut path = vec![PathCommand::MoveTo(p(tl, 0.0))];
        path.push(PathCommand::LineTo(p(w - tr, 0.0)));
        if tr > 0.0 {
            path.push(PathCommand::CubicTo {
                c1: p(w - tr + k * tr, 0.0),
                c2: p(w, tr - k * tr),
                to: p(w, tr),
            });
        }
        path.push(PathCommand::LineTo(p(w, h - br)));
        if br > 0.0 {
            path.push(PathCommand::CubicTo {
                c1: p(w, h - br + k * br),
                c2: p(w - br + k * br, h),
                to: p(w - br, h),
            });
        }
        path.push(PathCommand::LineTo(p(bl, h)));
        if bl > 0.0 {
            path.push(PathCommand::CubicTo {
                c1: p(bl - k * bl, h),
                c2: p(0.0, h - bl + k * bl),
                to: p(0.0, h - bl),
            });
        }
        path.push(PathCommand::LineTo(p(0.0, tl)));
        if tl > 0.0 {
            path.push(PathCommand::CubicTo {
                c1: p(0.0, tl - k * tl),
                c2: p(tl - k * tl, 0.0),
                to: p(tl, 0.0),
            });
        }
        path.push(PathCommand::Close);
        path
    }
}

/// 저장 계층이 파싱한 tagged field 하나(canonical R6 형식의 의미 단위).
///
/// document crate는 chunk codec(storage)에 의존하지 않으므로, 저장 계층이 읽은
/// field를 이 형태로 넘겨받아 의미를 해석합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawField {
    /// field tag.
    pub tag: u32,
    /// 모르는 reader가 거부해야 하면 true.
    pub mandatory: bool,
    /// payload bytes.
    pub payload: Vec<u8>,
}

/// Ellipse 파라미터 (VI.5.3).
///
/// 타원은 **bounds로만** 정의합니다. 초기 범위에서는 start/end angle(호·부채꼴)을
/// 지원하지 않으며, 파일에 그런 field가 있으면 unknown field 정책을 따릅니다:
/// optional이면 무시하고 mandatory이면 거부합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EllipseParams {
    /// 외접 사각형(shape local 좌표).
    pub bounds: Rect,
}

/// Ellipse field tag: bounds.
pub const ELLIPSE_FIELD_BOUNDS: u32 = 1;

/// 원을 4개 cubic으로 근사하는 control 거리 비율 `4(√2 − 1)/3`.
pub const CIRCLE_KAPPA: f32 = 0.552_284_8;

fn rect_from_bytes(bytes: &[u8]) -> Result<Rect, ShapeError> {
    if bytes.len() != 16 {
        return Err(ShapeError::Malformed {
            reason: "bounds는 16바이트",
        });
    }
    let f = |i: usize| f32::from_le_bytes(bytes[i..i + 4].try_into().expect("4바이트"));
    let origin = Vec2::new(f(0), f(4)).map_err(|_| ShapeError::Malformed {
        reason: "bounds 비유한",
    })?;
    let size = Size2::new(f(8), f(12)).map_err(|_| ShapeError::Malformed {
        reason: "bounds 음수·비유한",
    })?;
    let rect = Rect::new(origin, size);
    check_far_edge(&rect)?;
    Ok(rect)
}

/// bounds의 먼 쪽 끝 좌표(origin + size)도 유한해야 합니다. origin과 size가 각각 유한해도 합이
/// f32 범위를 넘으면 path 평가 좌표가 무한대가 됩니다(posthoc DOC-SHAPE-PANIC).
fn check_far_edge(rect: &Rect) -> Result<(), ShapeError> {
    let far_x = rect.origin().x() + rect.size().width();
    let far_y = rect.origin().y() + rect.size().height();
    if far_x.is_finite() && far_y.is_finite() {
        Ok(())
    } else {
        Err(ShapeError::NonFiniteGeometry)
    }
}

/// path 평가 좌표. 비유한이면 panic 대신 [`ShapeError::NonFiniteGeometry`]입니다.
fn finite_point(x: f32, y: f32) -> Result<Vec2, ShapeError> {
    Vec2::new(x, y).map_err(|_| ShapeError::NonFiniteGeometry)
}

fn rect_to_bytes(rect: &Rect) -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    for v in [
        rect.origin().x(),
        rect.origin().y(),
        rect.size().width(),
        rect.size().height(),
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

impl EllipseParams {
    /// 검증된 ellipse를 만듭니다. bounds 끝 좌표(origin + size)가 비유한이면 거부합니다.
    pub fn new(bounds: Rect) -> Result<EllipseParams, ShapeError> {
        check_far_edge(&bounds)?;
        Ok(EllipseParams { bounds })
    }

    /// tagged field로 기록합니다.
    pub fn to_fields(&self) -> Vec<RawField> {
        vec![RawField {
            tag: ELLIPSE_FIELD_BOUNDS,
            mandatory: true,
            payload: rect_to_bytes(&self.bounds),
        }]
    }

    /// tagged field에서 해석합니다. 모르는 optional field(예: 미래 버전의 start
    /// angle)는 무시하고, 모르는 mandatory field는 거부합니다.
    pub fn from_fields(fields: &[RawField]) -> Result<EllipseParams, ShapeError> {
        let mut bounds = None;
        for field in fields {
            match field.tag {
                ELLIPSE_FIELD_BOUNDS => bounds = Some(rect_from_bytes(&field.payload)?),
                tag if field.mandatory => return Err(ShapeError::UnknownMandatoryField { tag }),
                _ => {} // unknown optional: skip.
            }
        }
        Ok(EllipseParams {
            bounds: bounds.ok_or(ShapeError::MissingField {
                tag: ELLIPSE_FIELD_BOUNDS,
            })?,
        })
    }

    /// 전체 타원 path(4개 cubic). 오른쪽 끝에서 시작해 y-down 좌표 기준
    /// 시계 방향(오른쪽 → 아래 → 왼쪽 → 위)으로 그리고 닫습니다.
    ///
    /// bounds 필드를 직접 채워 끝 좌표가 f32 범위를 넘으면(디코딩·[`EllipseParams::new`]는 거부)
    /// [`ShapeError::NonFiniteGeometry`]를 반환합니다.
    pub fn to_path(&self) -> Result<Vec<PathCommand>, ShapeError> {
        let o = self.bounds.origin();
        let s = self.bounds.size();
        let rx = s.width() / 2.0;
        let ry = s.height() / 2.0;
        let cx = o.x() + rx;
        let cy = o.y() + ry;
        let kx = rx * CIRCLE_KAPPA;
        let ky = ry * CIRCLE_KAPPA;
        let p = finite_point;
        Ok(vec![
            PathCommand::MoveTo(p(cx + rx, cy)?),
            PathCommand::CubicTo {
                c1: p(cx + rx, cy + ky)?,
                c2: p(cx + kx, cy + ry)?,
                to: p(cx, cy + ry)?,
            },
            PathCommand::CubicTo {
                c1: p(cx - kx, cy + ry)?,
                c2: p(cx - rx, cy + ky)?,
                to: p(cx - rx, cy)?,
            },
            PathCommand::CubicTo {
                c1: p(cx - rx, cy - ky)?,
                c2: p(cx - kx, cy - ry)?,
                to: p(cx, cy - ry)?,
            },
            PathCommand::CubicTo {
                c1: p(cx + kx, cy - ry)?,
                c2: p(cx + rx, cy - ky)?,
                to: p(cx + rx, cy)?,
            },
            PathCommand::Close,
        ])
    }
}

/// 선 끝 모양 (VI.5.4). tag는 저장 포맷의 일부입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum LineCap {
    /// 끝점에서 자름(기본).
    #[default]
    Butt = 0,
    /// 반원 끝.
    Round = 1,
    /// 반폭만큼 연장한 사각 끝.
    Square = 2,
}

/// Line 파라미터 (VI.5.4).
///
/// **zero-length 정책**: 시작점과 끝점이 같으면 선 방향이 없으므로 "점 cap"으로
/// 그립니다 — Round는 지름 = 선 두께인 원, Square는 한 변 = 선 두께인 축 정렬
/// 정사각형, Butt는 면적이 없어 아무것도 그리지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineParams {
    /// 시작점.
    pub start: Vec2,
    /// 끝점.
    pub end: Vec2,
    /// 끝 모양.
    pub cap: LineCap,
}

fn bounds_of(points: &[(f32, f32)]) -> Option<Rect> {
    let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
    let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for (x, y) in points {
        min_x = min_x.min(*x);
        min_y = min_y.min(*y);
        max_x = max_x.max(*x);
        max_y = max_y.max(*y);
    }
    let origin = Vec2::new(min_x, min_y).ok()?;
    let size = Size2::new(max_x - min_x, max_y - min_y).ok()?;
    Some(Rect::new(origin, size))
}

impl LineParams {
    /// 선 길이가 0인지.
    pub fn is_zero_length(&self) -> bool {
        self.start == self.end
    }

    /// 두께 `width`(비음수)로 stroke했을 때의 bounds. 그릴 것이 없으면 `None`.
    pub fn stroked_bounds(&self, width: f32) -> Option<Rect> {
        let h = width.max(0.0) / 2.0;
        let (sx, sy) = (self.start.x(), self.start.y());
        let (ex, ey) = (self.end.x(), self.end.y());
        if self.is_zero_length() {
            // 점 cap 정책.
            return match self.cap {
                LineCap::Butt => None,
                LineCap::Round | LineCap::Square => {
                    bounds_of(&[(sx - h, sy - h), (sx + h, sy + h)])
                }
            };
        }
        let len = ((ex - sx).powi(2) + (ey - sy).powi(2)).sqrt();
        let (dx, dy) = ((ex - sx) / len, (ey - sy) / len);
        let (nx, ny) = (-dy * h, dx * h);
        match self.cap {
            LineCap::Butt => bounds_of(&[
                (sx + nx, sy + ny),
                (sx - nx, sy - ny),
                (ex + nx, ey + ny),
                (ex - nx, ey - ny),
            ]),
            LineCap::Square => {
                let (ox, oy) = (dx * h, dy * h);
                bounds_of(&[
                    (sx - ox + nx, sy - oy + ny),
                    (sx - ox - nx, sy - oy - ny),
                    (ex + ox + nx, ey + oy + ny),
                    (ex + ox - nx, ey + oy - ny),
                ])
            }
            LineCap::Round => bounds_of(&[
                (sx.min(ex) - h, sy.min(ey) - h),
                (sx.max(ex) + h, sy.max(ey) + h),
            ]),
        }
    }

    /// 중심선 path(Stroke 확장은 modifier 단계, VI.9).
    pub fn to_path(&self) -> Vec<PathCommand> {
        vec![
            PathCommand::MoveTo(self.start),
            PathCommand::LineTo(self.end),
        ]
    }
}

/// polygon 변 수 상한(파일 입력 방어용).
pub const MAX_POLYGON_SIDES: u32 = 1000;

/// 정다각형 파라미터 (VI.5.5).
///
/// 꼭짓점은 bounds에 내접하는 타원 위에 놓입니다. 회전 0°에서 첫 꼭짓점은 위쪽
/// 가운데이며, y-down 좌표 기준 시계 방향으로 이어집니다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolygonParams {
    bounds: Rect,
    sides: u32,
    rotation_degrees: f32,
    corner_radius: f32,
}

impl PolygonParams {
    /// 검증된 polygon을 만듭니다. 변 수 < 3(또는 상한 초과), 비유한 회전, 음수
    /// radius, 끝 좌표(origin + size)가 비유한인 bounds는 거부합니다.
    pub fn new(
        bounds: Rect,
        sides: u32,
        rotation: stellar_core_types::value::Angle,
        corner_radius: f32,
    ) -> Result<PolygonParams, ShapeError> {
        if !(3..=MAX_POLYGON_SIDES).contains(&sides) {
            return Err(ShapeError::InvalidSideCount { sides });
        }
        if !corner_radius.is_finite() || corner_radius < 0.0 {
            return Err(ShapeError::InvalidCornerRadius);
        }
        check_far_edge(&bounds)?;
        Ok(PolygonParams {
            bounds,
            sides,
            rotation_degrees: rotation.get(),
            corner_radius,
        })
    }

    /// 변 수.
    pub fn sides(&self) -> u32 {
        self.sides
    }

    /// 외접 bounds.
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// 회전(도).
    pub fn rotation_degrees(&self) -> f32 {
        self.rotation_degrees
    }

    /// 저장된 원본 corner radius(평가 clamp 전).
    pub fn corner_radius(&self) -> f32 {
        self.corner_radius
    }

    /// 꼭짓점 좌표(첫 꼭짓점부터 시계 방향).
    pub fn vertices(&self) -> Vec<(f32, f32)> {
        let o = self.bounds.origin();
        let s = self.bounds.size();
        let (rx, ry) = (s.width() as f64 / 2.0, s.height() as f64 / 2.0);
        let (cx, cy) = (o.x() as f64 + rx, o.y() as f64 + ry);
        let n = self.sides as f64;
        (0..self.sides)
            .map(|i| {
                let theta =
                    (-90.0 + self.rotation_degrees as f64 + 360.0 * i as f64 / n).to_radians();
                (
                    (cx + rx * theta.cos()) as f32,
                    (cy + ry * theta.sin()) as f32,
                )
            })
            .collect()
    }

    /// polygon path. corner radius가 0이면 직선 polygon, 양수면 각 꼭짓점을
    /// 원호(cubic 근사)로 둥글립니다. 인접 변 길이의 절반을 넘는 radius는
    /// 평가에서 clamp합니다(저장값은 원본 유지).
    ///
    /// 인접 꼭짓점과 겹쳐 변 길이가 0인 꼭짓점(예: 크기 0 bounds)은 둥글릴 방향이 없으므로 직선
    /// 꼭짓점으로 둡니다. 좌표가 f32 범위를 넘으면 [`ShapeError::NonFiniteGeometry`]입니다.
    pub fn to_path(&self) -> Result<Vec<PathCommand>, ShapeError> {
        let vertices = self.vertices();
        let p = finite_point;
        let n = vertices.len();
        if self.corner_radius == 0.0 {
            let mut path = vec![PathCommand::MoveTo(p(vertices[0].0, vertices[0].1)?)];
            for v in &vertices[1..] {
                path.push(PathCommand::LineTo(p(v.0, v.1)?));
            }
            path.push(PathCommand::Close);
            return Ok(path);
        }
        let mut path = Vec::with_capacity(2 * n + 1);
        for i in 0..n {
            let (vx, vy) = vertices[i];
            let (px, py) = vertices[(i + n - 1) % n];
            let (nx, ny) = vertices[(i + 1) % n];
            let (ax, ay) = (px - vx, py - vy);
            let (bx, by) = (nx - vx, ny - vy);
            let la = (ax * ax + ay * ay).sqrt();
            let lb = (bx * bx + by * by).sqrt();
            let cos_alpha = ((ax * bx + ay * by) / (la * lb)).clamp(-1.0, 1.0);
            if la == 0.0 || lb == 0.0 || cos_alpha.is_nan() {
                // 길이 0인 변(또는 길이 곱이 0으로 underflow한 극소 변): 방향이 없어 rounding을
                // 건너뛰고 직선 꼭짓점으로 둡니다(0으로 나누기 NaN 방지).
                let vertex = p(vx, vy)?;
                path.push(if i == 0 {
                    PathCommand::MoveTo(vertex)
                } else {
                    PathCommand::LineTo(vertex)
                });
                continue;
            }
            let alpha = cos_alpha.acos(); // 내각.
            let tan_half = (alpha / 2.0).tan();
            let mut t = if tan_half > 0.0 {
                self.corner_radius / tan_half
            } else {
                0.0
            };
            t = t.min(la / 2.0).min(lb / 2.0);
            let r = t * tan_half; // clamp 후 실효 radius.
            let phi = std::f32::consts::PI - alpha; // 회전각.
            let handle = (4.0 / 3.0) * (phi / 4.0).tan() * r;
            let start = (vx + ax / la * t, vy + ay / la * t);
            let end = (vx + bx / lb * t, vy + by / lb * t);
            let c1 = (start.0 - ax / la * handle, start.1 - ay / la * handle);
            let c2 = (end.0 - bx / lb * handle, end.1 - by / lb * handle);
            if i == 0 {
                path.push(PathCommand::MoveTo(p(start.0, start.1)?));
            } else {
                path.push(PathCommand::LineTo(p(start.0, start.1)?));
            }
            path.push(PathCommand::CubicTo {
                c1: p(c1.0, c1.1)?,
                c2: p(c2.0, c2.1)?,
                to: p(end.0, end.1)?,
            });
        }
        path.push(PathCommand::Close);
        Ok(path)
    }
}

/// 도형 geometry: 종류별 파라미터 (VI.5.6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShapeGeometry {
    /// 사각형.
    Rectangle(RectangleParams),
    /// 타원.
    Ellipse(EllipseParams),
    /// 직선.
    Line(LineParams),
    /// 정다각형.
    Polygon(PolygonParams),
}

impl ShapeGeometry {
    /// 도형 종류.
    pub fn kind(&self) -> ShapeKind {
        match self {
            ShapeGeometry::Rectangle(_) => ShapeKind::Rectangle,
            ShapeGeometry::Ellipse(_) => ShapeKind::Ellipse,
            ShapeGeometry::Line(_) => ShapeKind::Line,
            ShapeGeometry::Polygon(_) => ShapeKind::Polygon,
        }
    }
}

/// Shape를 Path geometry로 평가하는 **순수 함수**입니다(VI.5.6).
///
/// node를 Path로 바꾸지 않습니다 — 렌더·hit test·boolean이 쓰는 geometry만
/// 계산합니다. 같은 입력은 항상 bit 단위로 같은 command를 냅니다. 파괴 변환
/// (Shape node → Path node)은 사용자 명시 command만 수행합니다(VI.6.5).
///
/// 평가 좌표가 f32 범위를 넘으면 panic 대신 [`ShapeError::NonFiniteGeometry`]를 반환합니다. 디코딩한
/// geometry는 bounds 끝 좌표를 검사했으므로 정상 크기에서는 실패하지 않습니다.
pub fn evaluate_shape_path(geometry: &ShapeGeometry) -> Result<Vec<PathCommand>, ShapeError> {
    match geometry {
        ShapeGeometry::Rectangle(params) => Ok(params.to_path()),
        ShapeGeometry::Ellipse(params) => params.to_path(),
        ShapeGeometry::Line(params) => Ok(params.to_path()),
        ShapeGeometry::Polygon(params) => params.to_path(),
    }
}

/// 도형 modifier stack을 평가한 transform·fill·stroke입니다([`evaluate_shape_modifiers`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeModifierStyle {
    /// 활성 Transform을 OrderKey 순서로 `identity.then(first).then(next)` 합성한 값(없으면 identity).
    pub transform: Transform2D,
    /// 마지막 활성 Transform의 id와 그 local 값. 부모 공간 이동 편집은 이 항목에 적용합니다.
    pub last_transform: Option<(ModifierId, Transform2D)>,
    /// 활성 단색 Fill.
    pub fill: Option<Color>,
    /// 활성 단색 Stroke(path 가운데 정렬 선).
    pub stroke: Option<Border>,
}

/// [`evaluate_shape_modifiers`]가 평가하지 않는 modifier stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeModifierError {
    /// Transform 합성 결과가 f32 유한 범위를 벗어납니다.
    NonFiniteTransform,
    /// 활성 modifier가 지원 순서(Transform 0개 이상 → Fill 최대 1개 → Stroke 최대 1개) 밖이거나 다른 종류입니다.
    UnsupportedOrder,
    /// Stroke 두께를 선으로 쓸 수 없습니다(음수·비유한).
    InvalidStroke,
}

impl fmt::Display for ShapeModifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ShapeModifierError::NonFiniteTransform => "도형 Transform 합성 결과가 비유한입니다",
            ShapeModifierError::UnsupportedOrder => {
                "도형 modifier는 활성 Transform 0개 이상 → Fill 최대 1개 → Stroke 최대 1개 순서만 평가합니다"
            }
            ShapeModifierError::InvalidStroke => "도형 Stroke 두께가 음수이거나 비유한입니다",
        })
    }
}

impl std::error::Error for ShapeModifierError {}

/// 도형(Rectangle·Ellipse·Line·Polygon)의 modifier stack을 평가하는 **순수 함수**입니다.
///
/// 활성 항목만 OrderKey 순서로 읽고 비활성 항목은 건너뜁니다. 지원 순서는 Transform 0개 이상 → 단색 Fill 최대
/// 1개 → 단색 Stroke 최대 1개입니다. Fill·Stroke 뒤 Transform, 두 번째 Fill·Stroke, Stroke 뒤 Fill, 그 밖의
/// 종류(Opacity·Blur 등)는 [`ShapeModifierError::UnsupportedOrder`]입니다. 캔버스와 코드 내보내기가 같은 결과를
/// 내도록 이 규칙은 여기 한 곳에만 둡니다. 출력 쪽에서 표현하지 못하는 항목(예: 웹 출력의 Stroke)은 호출자가
/// 진단합니다.
pub fn evaluate_shape_modifiers(
    stack: Option<&ModifierStack>,
) -> Result<ShapeModifierStyle, ShapeModifierError> {
    let mut style = ShapeModifierStyle {
        transform: Transform2D::identity(),
        last_transform: None,
        fill: None,
        stroke: None,
    };
    for entry in stack.into_iter().flat_map(ModifierStack::active_entries) {
        match entry.payload {
            ModifierPayload::Transform(next) if style.fill.is_none() && style.stroke.is_none() => {
                style.transform = style
                    .transform
                    .then(&next)
                    .map_err(|_| ShapeModifierError::NonFiniteTransform)?;
                style.last_transform = Some((entry.id, next));
            }
            ModifierPayload::Fill { color } if style.fill.is_none() && style.stroke.is_none() => {
                style.fill = Some(color);
            }
            ModifierPayload::Stroke { color, width } if style.stroke.is_none() => {
                style.stroke = Some(
                    Border::new(color, width.get())
                        .map_err(|_| ShapeModifierError::InvalidStroke)?,
                );
            }
            _ => return Err(ShapeModifierError::UnsupportedOrder),
        }
    }
    Ok(style)
}

// --- canonical binary 인코딩 (VI.10.2) ---
//
// node 레코드 직렬화(VI.10.2)가 쓰는 storage 독립 형식입니다: shape kind tag(u8) +
// 종류별 고정 필드(f32 LE, `-0.0`은 `+0.0`으로 정규화). 저장값은 평가 clamp 전 원본입니다.

fn put_f32(buf: &mut Vec<u8>, value: f32) {
    let normalized = if value == 0.0 { 0.0f32 } else { value };
    buf.extend_from_slice(&normalized.to_le_bytes());
}

fn put_rect(buf: &mut Vec<u8>, rect: &Rect) {
    for v in [
        rect.origin().x(),
        rect.origin().y(),
        rect.size().width(),
        rect.size().height(),
    ] {
        put_f32(buf, v);
    }
}

impl ShapeGeometry {
    /// canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = vec![self.kind().tag()];
        match self {
            ShapeGeometry::Rectangle(params) => {
                put_f32(&mut buf, params.size.width());
                put_f32(&mut buf, params.size.height());
                let (tl, tr, br, bl) = params.corner_radius.components();
                for v in [tl, tr, br, bl] {
                    put_f32(&mut buf, v);
                }
            }
            ShapeGeometry::Ellipse(params) => put_rect(&mut buf, &params.bounds),
            ShapeGeometry::Line(params) => {
                for v in [
                    params.start.x(),
                    params.start.y(),
                    params.end.x(),
                    params.end.y(),
                ] {
                    put_f32(&mut buf, v);
                }
                buf.push(params.cap as u8);
            }
            ShapeGeometry::Polygon(params) => {
                put_rect(&mut buf, &params.bounds);
                buf.extend_from_slice(&params.sides.to_le_bytes());
                put_f32(&mut buf, params.rotation_degrees);
                put_f32(&mut buf, params.corner_radius);
            }
        }
        buf
    }

    /// bytes에서 복원하고 검증합니다(음수 size·잘못된 변 수 등 거부).
    pub fn decode(bytes: &[u8]) -> Result<ShapeGeometry, ShapeError> {
        let bad = |reason| ShapeError::Malformed { reason };
        let (&tag, body) = bytes.split_first().ok_or(bad("kind tag 없음"))?;
        let kind = ShapeKind::from_tag(tag)?;
        let expected_len = match kind {
            ShapeKind::Rectangle => 24,
            ShapeKind::Ellipse => 16,
            ShapeKind::Line => 17,
            ShapeKind::Polygon => 28,
        };
        if body.len() != expected_len {
            return Err(bad("payload 길이 불일치"));
        }
        let f = |i: usize| -> Result<f32, ShapeError> {
            let raw: [u8; 4] = body
                .get(i..i + 4)
                .and_then(|s| s.try_into().ok())
                .ok_or(bad("입력 부족"))?;
            let value = f32::from_le_bytes(raw);
            if value.is_finite() {
                Ok(value)
            } else {
                Err(bad("비유한 값"))
            }
        };
        let geometry = match kind {
            ShapeKind::Rectangle => ShapeGeometry::Rectangle(RectangleParams {
                size: Size2::new(f(0)?, f(4)?).map_err(|_| bad("size 음수"))?,
                corner_radius: CornerRadius::new(f(8)?, f(12)?, f(16)?, f(20)?)
                    .map_err(|_| ShapeError::InvalidCornerRadius)?,
            }),
            ShapeKind::Ellipse => ShapeGeometry::Ellipse(EllipseParams {
                bounds: rect_from_bytes(body)?,
            }),
            ShapeKind::Line => {
                let point = |x: f32, y: f32| Vec2::new(x, y).map_err(|_| bad("비유한 값"));
                let cap = match body[16] {
                    0 => LineCap::Butt,
                    1 => LineCap::Round,
                    2 => LineCap::Square,
                    _ => return Err(bad("알 수 없는 cap tag")),
                };
                ShapeGeometry::Line(LineParams {
                    start: point(f(0)?, f(4)?)?,
                    end: point(f(8)?, f(12)?)?,
                    cap,
                })
            }
            ShapeKind::Polygon => {
                let bounds = rect_from_bytes(&body[..16])?;
                let raw_sides: [u8; 4] = body[16..20].try_into().map_err(|_| bad("입력 부족"))?;
                let sides = u32::from_le_bytes(raw_sides);
                let rotation =
                    stellar_core_types::value::Angle::new(f(20)?).map_err(|_| bad("비유한 값"))?;
                ShapeGeometry::Polygon(PolygonParams::new(bounds, sides, rotation, f(24)?)?)
            }
        };
        Ok(geometry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_variants_round_trip() {
        // 검증 기준(VI.5.1): kind variant round-trip.
        for kind in ShapeKind::ALL {
            assert_eq!(ShapeKind::from_tag(kind.tag()).unwrap(), kind);
        }
    }

    #[test]
    fn golden_tags_are_stable() {
        let golden = [
            (ShapeKind::Rectangle, 1, "rectangle"),
            (ShapeKind::Ellipse, 2, "ellipse"),
            (ShapeKind::Line, 3, "line"),
            (ShapeKind::Polygon, 4, "polygon"),
        ];
        for (kind, tag, name) in golden {
            assert_eq!(kind.tag(), tag);
            assert_eq!(kind.name(), name);
        }
    }

    #[test]
    fn unknown_tags_are_rejected() {
        for tag in [0u8, 5, 255] {
            assert_eq!(
                ShapeKind::from_tag(tag).unwrap_err(),
                ShapeError::UnknownKind { tag }
            );
        }
    }

    #[test]
    fn shapes_share_one_node_kind() {
        // 도형 종류를 node kind로 늘리지 않습니다.
        use crate::node_kind::NodeKind;
        for name in ["rectangle", "ellipse", "line", "polygon"] {
            assert!(NodeKind::ALL.iter().all(|k| k.name() != name));
        }
    }

    #[test]
    fn oversize_radius_reference_fixture() {
        // 검증 기준(VI.5.2): oversize radius reference fixture 통과.
        // 100×50, 모든 radius 40 → 세로 변 합 80 > 50 → f = 0.625 → 25.
        let params = RectangleParams {
            size: Size2::new(100.0, 50.0).unwrap(),
            corner_radius: CornerRadius::new(40.0, 40.0, 40.0, 40.0).unwrap(),
        };
        assert_eq!(params.effective_corner_radius(), (25.0, 25.0, 25.0, 25.0));
        // 저장값은 원본 유지.
        assert_eq!(params.corner_radius.components(), (40.0, 40.0, 40.0, 40.0));

        // 비대칭: 200×100, tl=150, 나머지 0 → 위 변 150 ≤ 200, 왼 변 150 > 100 → f=2/3.
        let params = RectangleParams {
            size: Size2::new(200.0, 100.0).unwrap(),
            corner_radius: CornerRadius::new(150.0, 0.0, 0.0, 0.0).unwrap(),
        };
        let (tl, tr, br, bl) = params.effective_corner_radius();
        assert!((tl - 100.0).abs() < 1e-4, "tl={tl}");
        assert_eq!((tr, br, bl), (0.0, 0.0, 0.0));
    }

    #[test]
    fn fitting_radius_is_unchanged_and_zero_size_collapses() {
        let params = RectangleParams {
            size: Size2::new(100.0, 100.0).unwrap(),
            corner_radius: CornerRadius::new(10.0, 20.0, 30.0, 40.0).unwrap(),
        };
        assert_eq!(params.effective_corner_radius(), (10.0, 20.0, 30.0, 40.0));
        // 크기 0이면 실효 radius 0(원본은 유지).
        let params = RectangleParams {
            size: Size2::ZERO,
            corner_radius: CornerRadius::new(8.0, 8.0, 8.0, 8.0).unwrap(),
        };
        assert_eq!(params.effective_corner_radius(), (0.0, 0.0, 0.0, 0.0));
        assert_eq!(params.corner_radius.components(), (8.0, 8.0, 8.0, 8.0));
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::new(Vec2::new(x, y).unwrap(), Size2::new(w, h).unwrap())
    }

    #[test]
    fn full_ellipse_path_matches_reference() {
        // 검증 기준(VI.5.3): full ellipse path reference와 일치.
        let ellipse = EllipseParams {
            bounds: rect(0.0, 0.0, 200.0, 100.0),
        };
        let path = ellipse.to_path().unwrap();
        // 손 계산 reference: cx=100, cy=50, rx=100, ry=50, k=0.5522848.
        let reference: [(f32, f32); 13] = [
            (200.0, 50.0),
            (200.0, 77.61424),
            (155.22848, 100.0),
            (100.0, 100.0),
            (44.77152, 100.0),
            (0.0, 77.61424),
            (0.0, 50.0),
            (0.0, 22.38576),
            (44.77152, 0.0),
            (100.0, 0.0),
            (155.22848, 0.0),
            (200.0, 22.38576),
            (200.0, 50.0),
        ];
        let mut points = Vec::new();
        for command in &path {
            match *command {
                PathCommand::MoveTo(p) => points.push(p),
                PathCommand::CubicTo { c1, c2, to } => points.extend([c1, c2, to]),
                PathCommand::Close => {}
                PathCommand::LineTo(_) => panic!("타원에 LineTo 없음"),
            }
        }
        assert_eq!(points.len(), reference.len());
        for (p, (x, y)) in points.iter().zip(reference) {
            assert!(
                (p.x() - x).abs() < 1e-3 && (p.y() - y).abs() < 1e-3,
                "{p:?} vs ({x},{y})"
            );
        }
        assert_eq!(path.last(), Some(&PathCommand::Close));
        // 결정론: 같은 입력 → 같은 command(bit 동일).
        assert_eq!(ellipse.to_path().unwrap(), path);
    }

    #[test]
    fn ellipse_unknown_field_policy() {
        let ellipse = EllipseParams {
            bounds: rect(1.0, 2.0, 30.0, 40.0),
        };
        let mut fields = ellipse.to_fields();
        assert_eq!(EllipseParams::from_fields(&fields).unwrap(), ellipse);
        // 미래 버전의 optional start angle → 무시.
        fields.push(RawField {
            tag: 2,
            mandatory: false,
            payload: 1.0f32.to_le_bytes().to_vec(),
        });
        assert_eq!(EllipseParams::from_fields(&fields).unwrap(), ellipse);
        // mandatory로 표시된 end angle → 거부.
        fields.push(RawField {
            tag: 3,
            mandatory: true,
            payload: 2.0f32.to_le_bytes().to_vec(),
        });
        assert_eq!(
            EllipseParams::from_fields(&fields).unwrap_err(),
            ShapeError::UnknownMandatoryField { tag: 3 }
        );
        // bounds 누락.
        assert_eq!(
            EllipseParams::from_fields(&[]).unwrap_err(),
            ShapeError::MissingField {
                tag: ELLIPSE_FIELD_BOUNDS
            }
        );
    }

    fn assert_rect(actual: Option<Rect>, expected: (f32, f32, f32, f32)) {
        let r = actual.expect("bounds가 있어야 함");
        let got = (
            r.origin().x(),
            r.origin().y(),
            r.size().width(),
            r.size().height(),
        );
        for (a, e) in [got.0, got.1, got.2, got.3]
            .iter()
            .zip([expected.0, expected.1, expected.2, expected.3])
        {
            assert!((a - e).abs() < 1e-4, "{got:?} vs {expected:?}");
        }
    }

    fn line(sx: f32, sy: f32, ex: f32, ey: f32, cap: LineCap) -> LineParams {
        LineParams {
            start: Vec2::new(sx, sy).unwrap(),
            end: Vec2::new(ex, ey).unwrap(),
            cap,
        }
    }

    #[test]
    fn cap_bounds_fixtures() {
        // 검증 기준(VI.5.4): cap별 bounds fixture.
        // 수평 (0,0)-(10,0), 두께 2.
        assert_rect(
            line(0.0, 0.0, 10.0, 0.0, LineCap::Butt).stroked_bounds(2.0),
            (0.0, -1.0, 10.0, 2.0),
        );
        assert_rect(
            line(0.0, 0.0, 10.0, 0.0, LineCap::Square).stroked_bounds(2.0),
            (-1.0, -1.0, 12.0, 2.0),
        );
        assert_rect(
            line(0.0, 0.0, 10.0, 0.0, LineCap::Round).stroked_bounds(2.0),
            (-1.0, -1.0, 12.0, 2.0),
        );
        // 대각 (0,0)-(3,4), 길이 5, 두께 2: butt 모서리 x[-0.8,3.8], y[-0.6,4.6].
        assert_rect(
            line(0.0, 0.0, 3.0, 4.0, LineCap::Butt).stroked_bounds(2.0),
            (-0.8, -0.6, 4.6, 5.2),
        );
        // square는 방향으로 반폭 연장: x[-1.4,4.4], y[-1.4,5.4].
        assert_rect(
            line(0.0, 0.0, 3.0, 4.0, LineCap::Square).stroked_bounds(2.0),
            (-1.4, -1.4, 5.8, 6.8),
        );
        // round는 끝점 bbox를 반폭만큼 확장.
        assert_rect(
            line(0.0, 0.0, 3.0, 4.0, LineCap::Round).stroked_bounds(2.0),
            (-1.0, -1.0, 5.0, 6.0),
        );
    }

    #[test]
    fn zero_length_line_uses_point_cap_policy() {
        let at = |cap| line(5.0, 5.0, 5.0, 5.0, cap);
        assert!(at(LineCap::Round).is_zero_length());
        assert_rect(at(LineCap::Round).stroked_bounds(4.0), (3.0, 3.0, 4.0, 4.0));
        assert_rect(
            at(LineCap::Square).stroked_bounds(4.0),
            (3.0, 3.0, 4.0, 4.0),
        );
        assert_eq!(
            at(LineCap::Butt).stroked_bounds(4.0),
            None,
            "butt 점은 그리지 않음"
        );
    }

    #[test]
    fn line_path_is_center_line() {
        let l = line(1.0, 2.0, 3.0, 4.0, LineCap::Butt);
        assert_eq!(
            l.to_path(),
            vec![PathCommand::MoveTo(l.start), PathCommand::LineTo(l.end)]
        );
    }

    fn polygon(sides: u32, rotation: f32, radius: f32) -> Result<PolygonParams, ShapeError> {
        PolygonParams::new(
            rect(0.0, 0.0, 100.0, 100.0),
            sides,
            stellar_core_types::value::Angle::new(rotation).unwrap(),
            radius,
        )
    }

    #[test]
    fn two_sides_are_rejected() {
        // 검증 기준(VI.5.5): 2 sides 거부.
        assert_eq!(
            polygon(2, 0.0, 0.0).unwrap_err(),
            ShapeError::InvalidSideCount { sides: 2 }
        );
        assert!(polygon(MAX_POLYGON_SIDES + 1, 0.0, 0.0).is_err());
        assert_eq!(
            polygon(3, 0.0, -1.0).unwrap_err(),
            ShapeError::InvalidCornerRadius
        );
    }

    #[test]
    fn three_four_and_hundred_side_path_fixtures() {
        // 검증 기준(VI.5.5): 3·4·100 sides path fixture 통과.
        for sides in [3u32, 4, 100] {
            let poly = polygon(sides, 0.0, 0.0).unwrap();
            let path = poly.to_path().unwrap();
            // MoveTo + (n−1) LineTo + Close.
            assert_eq!(path.len(), sides as usize + 1, "{sides} sides");
            assert!(matches!(path[0], PathCommand::MoveTo(_)));
            assert_eq!(path.last(), Some(&PathCommand::Close));
            // 모든 꼭짓점이 반지름 50 원 위.
            for (x, y) in poly.vertices() {
                let d = ((x - 50.0).powi(2) + (y - 50.0).powi(2)).sqrt();
                assert!((d - 50.0).abs() < 1e-3, "{sides} sides: d={d}");
            }
            // 첫 꼭짓점은 위쪽 가운데.
            let (x0, y0) = poly.vertices()[0];
            assert!((x0 - 50.0).abs() < 1e-4 && y0.abs() < 1e-4);
        }
        // 4 sides(회전 0)는 다이아몬드: (50,0),(100,50),(50,100),(0,50).
        let diamond = polygon(4, 0.0, 0.0).unwrap().vertices();
        let expected = [(50.0, 0.0), (100.0, 50.0), (50.0, 100.0), (0.0, 50.0)];
        for ((x, y), (ex, ey)) in diamond.iter().zip(expected) {
            assert!((x - ex).abs() < 1e-4 && (y - ey).abs() < 1e-4);
        }
        // 결정론.
        assert_eq!(
            polygon(100, 0.0, 0.0).unwrap().to_path().unwrap(),
            polygon(100, 0.0, 0.0).unwrap().to_path().unwrap()
        );
    }

    #[test]
    fn rotation_and_corner_radius_shape_the_path() {
        // 45° 회전한 4각형은 축 정렬 정사각형.
        let square = polygon(4, 45.0, 0.0).unwrap().vertices();
        let half = 50.0 * std::f32::consts::FRAC_1_SQRT_2;
        assert!((square[0].0 - (50.0 + half)).abs() < 1e-3);
        assert!((square[0].1 - (50.0 - half)).abs() < 1e-3);

        // 둥근 모서리: MoveTo + n·Cubic + (n−1)·LineTo + Close.
        let rounded = polygon(4, 45.0, 10.0).unwrap().to_path().unwrap();
        assert_eq!(rounded.len(), 1 + 4 + 3 + 1);
        let cubics = rounded
            .iter()
            .filter(|c| matches!(c, PathCommand::CubicTo { .. }))
            .count();
        assert_eq!(cubics, 4);
        // 과도한 radius는 변 절반으로 clamp되어도 유한 path.
        let clamped = polygon(3, 0.0, 10_000.0).unwrap().to_path().unwrap();
        assert_eq!(clamped.len(), 1 + 3 + 2 + 1);
    }

    fn sample_geometries() -> Vec<ShapeGeometry> {
        vec![
            ShapeGeometry::Rectangle(RectangleParams {
                size: Size2::new(120.0, 80.0).unwrap(),
                corner_radius: CornerRadius::new(0.0, 12.0, 30.0, 6.5).unwrap(),
            }),
            ShapeGeometry::Ellipse(EllipseParams {
                bounds: rect(-10.0, 5.0, 33.3, 77.7),
            }),
            ShapeGeometry::Line(line(0.5, 0.25, 99.0, -42.0, LineCap::Round)),
            ShapeGeometry::Polygon(polygon(7, 13.0, 4.0).unwrap()),
        ]
    }

    #[test]
    fn evaluation_is_bit_stable() {
        // 검증 기준(VI.5.6): 동일 입력 bit-stable path commands.
        use crate::path::encode_commands;
        for geometry in sample_geometries() {
            let first = encode_commands(&evaluate_shape_path(&geometry).unwrap());
            for _ in 0..100 {
                assert_eq!(
                    encode_commands(&evaluate_shape_path(&geometry).unwrap()),
                    first,
                    "{}",
                    geometry.kind().name()
                );
            }
        }
    }

    #[test]
    fn rectangle_path_golden() {
        // 사칙연산만 쓰는 사각형 path는 플랫폼 간에도 bit 동일해야 합니다.
        let square = RectangleParams {
            size: Size2::new(10.0, 20.0).unwrap(),
            corner_radius: CornerRadius::ZERO,
        };
        let p = |x: f32, y: f32| Vec2::new(x, y).unwrap();
        assert_eq!(
            square.to_path(),
            vec![
                PathCommand::MoveTo(p(0.0, 0.0)),
                PathCommand::LineTo(p(10.0, 0.0)),
                PathCommand::LineTo(p(10.0, 20.0)),
                PathCommand::LineTo(p(0.0, 20.0)),
                PathCommand::LineTo(p(0.0, 0.0)),
                PathCommand::Close,
            ]
        );
        // 둥근 모서리 4개: MoveTo + 4 LineTo + 4 Cubic + Close.
        let rounded = RectangleParams {
            size: Size2::new(100.0, 50.0).unwrap(),
            corner_radius: CornerRadius::new(40.0, 40.0, 40.0, 40.0).unwrap(),
        };
        let path = rounded.to_path();
        assert_eq!(path.len(), 10);
        // 실효 radius 25로 clamp된 시작점.
        assert_eq!(path[0], PathCommand::MoveTo(p(25.0, 0.0)));
    }

    #[test]
    fn evaluation_does_not_convert_the_node() {
        // 순수 평가: geometry 값 자체는 그대로(kind 유지).
        for geometry in sample_geometries() {
            let before = geometry;
            let _ = evaluate_shape_path(&geometry);
            assert_eq!(geometry, before);
            assert_ne!(geometry.kind().name(), "path");
        }
    }

    #[test]
    fn geometry_codec_round_trips_every_kind() {
        // VI.10.2: node 레코드 직렬화용 shape 인코딩.
        let rect = |x: f32, y: f32, w: f32, h: f32| {
            Rect::new(Vec2::new(x, y).unwrap(), Size2::new(w, h).unwrap())
        };
        let geometries = [
            ShapeGeometry::Rectangle(RectangleParams {
                size: Size2::new(30.0, 12.5).unwrap(),
                corner_radius: CornerRadius::new(1.0, 2.0, 3.0, 400.0).unwrap(),
            }),
            ShapeGeometry::Ellipse(EllipseParams {
                bounds: rect(-4.0, 2.0, 8.0, 6.0),
            }),
            ShapeGeometry::Line(LineParams {
                start: Vec2::new(1.0, -1.0).unwrap(),
                end: Vec2::new(1.0, -1.0).unwrap(),
                cap: LineCap::Square,
            }),
            ShapeGeometry::Polygon(
                PolygonParams::new(
                    rect(0.0, 0.0, 10.0, 10.0),
                    7,
                    stellar_core_types::value::Angle::new(-30.5).unwrap(),
                    2.25,
                )
                .unwrap(),
            ),
        ];
        for geometry in geometries {
            let bytes = geometry.encode();
            assert_eq!(bytes[0], geometry.kind().tag());
            let decoded = ShapeGeometry::decode(&bytes).unwrap();
            assert_eq!(decoded, geometry);
            assert_eq!(decoded.encode(), bytes, "재인코딩 byte-identical");
            // 절단·trailing은 거부.
            assert!(ShapeGeometry::decode(&bytes[..bytes.len() - 1]).is_err());
            let mut extended = bytes.clone();
            extended.push(0);
            assert!(ShapeGeometry::decode(&extended).is_err());
        }
    }

    #[test]
    fn geometry_codec_rejects_invalid_values() {
        assert!(matches!(
            ShapeGeometry::decode(&[9]),
            Err(ShapeError::UnknownKind { tag: 9 })
        ));
        assert!(ShapeGeometry::decode(&[]).is_err());
        // polygon 변 수 2.
        let mut bytes = ShapeGeometry::Polygon(
            PolygonParams::new(
                Rect::new(Vec2::ZERO, Size2::new(1.0, 1.0).unwrap()),
                3,
                stellar_core_types::value::Angle::new(0.0).unwrap(),
                0.0,
            )
            .unwrap(),
        )
        .encode();
        bytes[17..21].copy_from_slice(&2u32.to_le_bytes());
        assert!(matches!(
            ShapeGeometry::decode(&bytes),
            Err(ShapeError::InvalidSideCount { sides: 2 })
        ));
        // 음수 width.
        let mut bytes = ShapeGeometry::Rectangle(RectangleParams {
            size: Size2::new(1.0, 1.0).unwrap(),
            corner_radius: CornerRadius::ZERO,
        })
        .encode();
        bytes[1..5].copy_from_slice(&(-1.0f32).to_le_bytes());
        assert!(ShapeGeometry::decode(&bytes).is_err());
        // NaN 좌표.
        let mut bytes = ShapeGeometry::Line(LineParams {
            start: Vec2::ZERO,
            end: Vec2::ZERO,
            cap: LineCap::Butt,
        })
        .encode();
        bytes[1..5].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(ShapeGeometry::decode(&bytes).is_err());
    }

    /// shape payload bytes: kind tag + f32 LE 값들 + 꼬리 bytes.
    fn payload(tag: u8, values: &[f32], tail: &[u8]) -> Vec<u8> {
        let mut bytes = vec![tag];
        for v in values {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes.extend_from_slice(tail);
        bytes
    }

    fn polygon_tail(sides: u32, rotation: f32, radius: f32) -> Vec<u8> {
        let mut tail = sides.to_le_bytes().to_vec();
        tail.extend_from_slice(&rotation.to_le_bytes());
        tail.extend_from_slice(&radius.to_le_bytes());
        tail
    }

    #[test]
    fn zero_size_polygon_with_radius_evaluates_without_panic() {
        // posthoc DOC-SHAPE-PANIC 회귀: 크기 0 bounds + radius 1인 삼각형은 디코딩·validator를
        // 통과하지만, 변 길이 0으로 나누어 NaN 좌표가 생기고 `expect("finite polygon")`이 panic했습니다
        // (release는 abort). 이제 길이 0인 변은 rounding을 건너뛴 직선 꼭짓점입니다.
        let bytes = payload(4, &[0.0; 4], &polygon_tail(3, 0.0, 1.0));
        let geometry = ShapeGeometry::decode(&bytes).expect("디코딩은 수용");
        let zero = PathCommand::LineTo(Vec2::ZERO);
        assert_eq!(
            evaluate_shape_path(&geometry).unwrap(),
            vec![
                PathCommand::MoveTo(Vec2::ZERO),
                zero,
                zero,
                PathCommand::Close
            ]
        );
        // 일부 꼭짓점만 겹치는 경우(높이 0, 45° 회전 사각형: v0=v1, v2=v3)도 유한 path입니다.
        let flat = PolygonParams::new(
            rect(0.0, 0.0, 100.0, 0.0),
            4,
            stellar_core_types::value::Angle::new(45.0).unwrap(),
            5.0,
        )
        .unwrap();
        let path = flat.to_path().unwrap();
        assert!(matches!(path[0], PathCommand::MoveTo(_)));
        assert_eq!(path.last(), Some(&PathCommand::Close));
        assert!(crate::path::path_bounds(&path).is_some());
    }

    #[test]
    fn bounds_whose_far_edge_overflows_are_rejected_without_panic() {
        // posthoc DOC-SHAPE-PANIC 회귀: origin·size가 각각 유한해도 합(origin + size)이 f32 범위를
        // 넘으면 평가 좌표가 무한대가 되어 panic했습니다. 디코딩·생성자는 거부하고, 필드를 직접
        // 채운 geometry의 평가·변환 계획은 panic 대신 오류입니다.
        let huge = [f32::MAX, 0.0, f32::MAX, 1.0];
        assert_eq!(
            ShapeGeometry::decode(&payload(2, &huge, &[])),
            Err(ShapeError::NonFiniteGeometry)
        );
        assert_eq!(
            ShapeGeometry::decode(&payload(4, &huge, &polygon_tail(5, 0.0, 0.0))),
            Err(ShapeError::NonFiniteGeometry)
        );
        let bounds = rect(huge[0], huge[1], huge[2], huge[3]);
        assert_eq!(
            EllipseParams::new(bounds),
            Err(ShapeError::NonFiniteGeometry)
        );
        assert_eq!(
            PolygonParams::new(
                bounds,
                5,
                stellar_core_types::value::Angle::new(0.0).unwrap(),
                0.0
            ),
            Err(ShapeError::NonFiniteGeometry)
        );
        let field = ShapeGeometry::Ellipse(EllipseParams { bounds });
        assert_eq!(
            evaluate_shape_path(&field),
            Err(ShapeError::NonFiniteGeometry)
        );
        let mut next = || stellar_core_types::ids::StableId::new_v7(1, 0, 1);
        assert_eq!(
            crate::path::plan_shape_to_path(
                stellar_core_types::ids::StableId::new_v7(2, 0, 2),
                &field,
                &mut next
            ),
            Err(crate::path::PathError::NonFinite)
        );
        // 경계 안(끝 좌표가 정확히 f32::MAX)은 수용하고 평가합니다.
        let edge = ShapeGeometry::decode(&payload(2, &[0.0, 0.0, f32::MAX, 2.0], &[])).unwrap();
        assert!(evaluate_shape_path(&edge).is_ok());
    }

    #[test]
    fn extreme_value_sweep_never_panics() {
        // posthoc DOC-SHAPE-PANIC 회귀: 극단값 조합 sweep(감사 시 ellipse 1,070건·polygon 10,773건
        // panic). 디코딩을 통과한 모든 geometry의 평가는 panic하지 않고, 성공하면 변환 계획도
        // 성공합니다. 디코딩을 통과한 ellipse·rectangle·line은 평가가 항상 성공합니다.
        let values = [
            0.0,
            1.0,
            -1.0,
            1.0e-45,
            1.0e-20,
            1.0e20,
            f32::MAX / 2.0,
            f32::MAX,
            -f32::MAX,
        ];
        let mut decoded = 0usize;
        let mut check = |bytes: Vec<u8>| {
            let Ok(geometry) = ShapeGeometry::decode(&bytes) else {
                return;
            };
            decoded += 1;
            match evaluate_shape_path(&geometry) {
                Ok(commands) => {
                    let data =
                        crate::path::PathData::new(crate::path::WindingRule::NonZero, commands);
                    assert!(data.is_ok(), "평가 결과는 유효 path: {bytes:?}");
                }
                Err(error) => {
                    assert_eq!(error, ShapeError::NonFiniteGeometry);
                    assert_eq!(
                        geometry.kind(),
                        ShapeKind::Polygon,
                        "디코딩한 {}의 평가 실패: {bytes:?}",
                        geometry.kind().name()
                    );
                }
            }
        };
        for &x in &values {
            for &y in &values {
                for &w in &values {
                    for &h in &values {
                        check(payload(2, &[x, y, w, h], &[]));
                        check(payload(3, &[x, y, w, h], &[0]));
                        for sides in [3, 4, 5] {
                            for radius in [0.0, 0.5, f32::MAX] {
                                for rotation in [0.0, 45.0] {
                                    check(payload(
                                        4,
                                        &[x, y, w, h],
                                        &polygon_tail(sides, rotation, radius),
                                    ));
                                }
                            }
                        }
                    }
                    check(payload(1, &[x, y, w, w, w, w], &[]));
                    check(payload(1, &[x, y, 0.0, w, 0.0, w], &[]));
                }
            }
        }
        assert!(decoded > 10_000, "디코딩된 조합 {decoded}");
    }

    fn modifier_stack(payloads: &[(ModifierPayload, bool)]) -> ModifierStack {
        use stellar_core_types::ids::StableId;
        let mut stack = ModifierStack::new(StableId::new_v7(1, 0, 1));
        for (index, (payload, enabled)) in payloads.iter().enumerate() {
            let id = ModifierId::new(StableId::new_v7(10 + index as u64, 0, 10 + index as u64));
            stack.push_back(id, payload.kind()).unwrap();
            stack.set_payload(id, payload.clone()).unwrap();
            stack.set_enabled(id, *enabled).unwrap();
        }
        stack
    }

    #[test]
    fn shape_modifiers_follow_transform_fill_stroke_order() {
        use stellar_core_types::color::ColorSpace;
        use stellar_core_types::value::FiniteF32;
        let color = Color::new(ColorSpace::Srgb, 1.0, 0.5, 0.0, 1.0).unwrap();
        let move_by = |x: f32| {
            ModifierPayload::Transform(Transform2D::new(1.0, 0.0, 0.0, 1.0, x, 0.0).unwrap())
        };
        let fill = ModifierPayload::Fill { color };
        let stroke = ModifierPayload::Stroke {
            color,
            width: FiniteF32::new(2.0).unwrap(),
        };
        let blur = ModifierPayload::default_for(crate::modifier::ModifierKind::Blur);

        // stack 없음은 identity·무채움입니다.
        let empty = evaluate_shape_modifiers(None).unwrap();
        assert_eq!(empty.transform, Transform2D::identity());
        assert_eq!(
            (empty.fill, empty.stroke, empty.last_transform),
            (None, None, None)
        );

        // Transform 둘은 순서대로 합성하고, 비활성 항목은 건너뜁니다.
        let stack = modifier_stack(&[
            (move_by(10.0), true),
            (move_by(5.0), true),
            (blur.clone(), false),
            (fill.clone(), true),
            (stroke.clone(), true),
        ]);
        let style = evaluate_shape_modifiers(Some(&stack)).unwrap();
        assert_eq!(
            style.transform,
            Transform2D::new(1.0, 0.0, 0.0, 1.0, 15.0, 0.0).unwrap()
        );
        assert_eq!(
            style.last_transform.map(|(_, t)| t),
            Some(Transform2D::new(1.0, 0.0, 0.0, 1.0, 5.0, 0.0).unwrap())
        );
        assert_eq!(style.fill, Some(color));
        assert_eq!(style.stroke, Some(Border::new(color, 2.0).unwrap()));

        // 순서 밖과 다른 종류는 거부합니다.
        for (name, payloads) in [
            ("Fill 뒤 Transform", vec![fill.clone(), move_by(1.0)]),
            ("Stroke 뒤 Transform", vec![stroke.clone(), move_by(1.0)]),
            ("Stroke 뒤 Fill", vec![stroke.clone(), fill.clone()]),
            ("두 번째 Fill", vec![fill.clone(), fill.clone()]),
            ("두 번째 Stroke", vec![stroke.clone(), stroke.clone()]),
            ("다른 종류", vec![blur.clone()]),
        ] {
            let stack =
                modifier_stack(&payloads.into_iter().map(|p| (p, true)).collect::<Vec<_>>());
            assert_eq!(
                evaluate_shape_modifiers(Some(&stack)),
                Err(ShapeModifierError::UnsupportedOrder),
                "{name}"
            );
        }

        // 합성이 f32 범위를 넘으면 비유한 오류입니다.
        let scale = ModifierPayload::Transform(
            Transform2D::new(f32::MAX, 0.0, 0.0, f32::MAX, 0.0, 0.0).unwrap(),
        );
        let stack = modifier_stack(&[(scale.clone(), true), (scale, true)]);
        assert_eq!(
            evaluate_shape_modifiers(Some(&stack)),
            Err(ShapeModifierError::NonFiniteTransform)
        );
    }
}

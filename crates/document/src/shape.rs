//! Shape payload (VI.5).
//!
//! 파라메트릭 도형은 **하나의 node kind**([`NodeKind::Shape`](crate::node_kind::NodeKind))
//! 안에서 [`ShapeKind`]로 구분합니다 — 도형 종류마다 node kind를 늘리지 않습니다.

use std::fmt;

use stellar_core_types::value::{Rect, Size2, Vec2};

use crate::frame::CornerRadius;
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
    Ok(Rect::new(origin, size))
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
    pub fn to_path(&self) -> Vec<PathCommand> {
        let o = self.bounds.origin();
        let s = self.bounds.size();
        let rx = s.width() / 2.0;
        let ry = s.height() / 2.0;
        let cx = o.x() + rx;
        let cy = o.y() + ry;
        let kx = rx * CIRCLE_KAPPA;
        let ky = ry * CIRCLE_KAPPA;
        let p = |x: f32, y: f32| Vec2::new(x, y).expect("finite bounds");
        vec![
            PathCommand::MoveTo(p(cx + rx, cy)),
            PathCommand::CubicTo {
                c1: p(cx + rx, cy + ky),
                c2: p(cx + kx, cy + ry),
                to: p(cx, cy + ry),
            },
            PathCommand::CubicTo {
                c1: p(cx - kx, cy + ry),
                c2: p(cx - rx, cy + ky),
                to: p(cx - rx, cy),
            },
            PathCommand::CubicTo {
                c1: p(cx - rx, cy - ky),
                c2: p(cx - kx, cy - ry),
                to: p(cx, cy - ry),
            },
            PathCommand::CubicTo {
                c1: p(cx + kx, cy - ry),
                c2: p(cx + rx, cy - ky),
                to: p(cx + rx, cy),
            },
            PathCommand::Close,
        ]
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
        let path = ellipse.to_path();
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
        assert_eq!(ellipse.to_path(), path);
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
}

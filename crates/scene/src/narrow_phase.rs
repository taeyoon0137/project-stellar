//! node 종류별 narrow-phase hit test (XI.3.4).
//!
//! broad-phase(XI.3.3)가 bounds·clip 껍질로 남긴 후보를 content geometry로 다시 거릅니다.
//! **alpha(픽셀 투명도) 판정은 하지 않습니다.** 종류별 정책은 다음과 같습니다.
//!
//! | content | 원본 node | 정책 | 맞는 영역 |
//! |---|---|---|---|
//! | `Rect` | Frame | geometry | 실효 모서리 radius 둥근 사각형 전체(칠 유무 무관) + stroke가 상자 밖으로 나가는 띠 |
//! | `Path` | Shape(VI.5.6 평가 결과)·Path | geometry | fill layer가 있으면 fill rule 내부, stroke layer마다 정렬·끝 모양별 띠. paint layer가 하나도 없으면 내부 |
//! | `Text` | Text | bounds | [`TextHitGeometry`]가 line rect(X.10.3 selection rect)를 주면 그 합집합, 없거나 비면 layout 상자. glyph 윤곽은 보지 않음 |
//! | `Image` | Image | bounds | 배치 상자 전체(fit·crop·투명 픽셀과 무관) |
//!
//! ## geometry 규칙
//!
//! - **fill**: fill은 열린 subpath를 암묵적으로 닫습니다. [`FillRule::NonZero`]는 winding number가
//!   0이 아니면, [`FillRule::EvenOdd`]는 홀수면 안입니다. 경계 위 판정은 부동소수 오차 범위에서
//!   어느 쪽이든 될 수 있습니다.
//! - **stroke 띠**(Path): [`StrokeAlign::Center`]는 윤곽에서 폭/2 이내, [`StrokeAlign::Inside`]는
//!   fill 영역 안이면서 폭 이내, [`StrokeAlign::Outside`]는 fill 영역 밖이면서 폭 이내입니다.
//!   폭 0 stroke는 띠가 없습니다.
//! - **join**: stroke join은 아직 정의되지 않아(XI.6.2) round join으로 봅니다. miter join의 뾰족한
//!   끝은 맞지 않을 수 있고, bevel보다는 넓게 맞습니다.
//! - **cap**: 열린 subpath 끝만 [`StrokeCap`]을 따릅니다. Butt은 끝점에서 자르고, Round는 반원,
//!   Square는 반폭만큼 연장한 사각형입니다. 닫힌 subpath(Close)는 끝 모양이 없습니다. 점 하나뿐인
//!   subpath(MoveTo만)와 길이 0 선분 하나뿐인 열린 subpath는 Round cap일 때만 원으로 맞습니다.
//! - **상자형 content**(Frame·Text·Image): stroke는 상자 밖으로 나가는 거리(Center 폭/2, Outside 폭,
//!   Inside 0)만큼 상자를 넓힙니다.
//! - **곡선**: cubic은 world 기준 허용 오차 [`FLATTEN_TOLERANCE`]를 node 평균 축척으로 나눈 local 허용
//!   오차로 선분화합니다(Wang 공식). cubic 하나당 [`MAX_CUBIC_SEGMENTS`] 이하로 제한해 신뢰할 수 없는
//!   큰 좌표에서도 비용이 command 수에 선형입니다.
//! - **path clip**: broad-phase의 control 껍질 대신 clip path 내부(clip fill rule)로 정밀 판정합니다.
//!   rect clip은 broad-phase가 이미 정확합니다.
//! - **허용 거리**: [`NarrowPhaseOptions::tolerance`](world 단위)만큼 모든 content 영역을 넓힙니다
//!   (가는 선·작은 node 선택용). local 거리는 node 평균 축척(`√|det|`)으로 나눈 근사입니다. clip은
//!   넓히지 않습니다.

use stellar_core_types::value::{FiniteF32, Rect, Transform2D, Vec2};

use crate::bounds::Aabb;
use crate::geometry::{FillRule, PathCommand, PathGeometry};
use crate::hit_test::{broad_phase, linear_scale, rounded_rect_contains, to_local, HitResult};
use crate::node::{
    non_negative, CornerRadii, SceneContent, SceneNode, SceneValueError, TextLayoutKey,
    TextPrimitive,
};
use crate::spatial::SceneSpatialIndex;
use crate::stack::{ClipShape, ClipStackRef, PaintLayer, StrokeAlign, StrokeCap, StrokeLayer};
use crate::visibility::Viewport;

/// 곡선 선분화 허용 오차(world 단위).
pub const FLATTEN_TOLERANCE: f32 = 0.05;

/// cubic 하나를 나누는 최대 선분 수.
pub const MAX_CUBIC_SEGMENTS: usize = 256;

/// narrow-phase 설정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NarrowPhaseOptions {
    tolerance: FiniteF32,
}

impl Default for NarrowPhaseOptions {
    /// [`NarrowPhaseOptions::EXACT`].
    fn default() -> NarrowPhaseOptions {
        NarrowPhaseOptions::EXACT
    }
}

impl NarrowPhaseOptions {
    /// 허용 거리 0(정확한 geometry).
    pub const EXACT: NarrowPhaseOptions = NarrowPhaseOptions {
        tolerance: FiniteF32::ZERO,
    };

    /// content 영역을 `tolerance`(finite·비음수, world 단위 — view 함수에서는 view 단위)만큼 넓힙니다.
    pub fn with_tolerance(tolerance: f32) -> Result<NarrowPhaseOptions, SceneValueError> {
        Ok(NarrowPhaseOptions {
            tolerance: non_negative(tolerance, "hit tolerance")?,
        })
    }

    /// 허용 거리.
    pub const fn tolerance(&self) -> f32 {
        self.tolerance.get()
    }
}

/// text layout 결과의 hit 영역 제공자.
///
/// text layout cache(X.10)는 scene 밖에 있으므로, 호출자가 layout key별 line rect(X.10.3 selection
/// rect, text node local 좌표)를 알려 줍니다. 모르면 `None`을 돌려 layout 상자 정책을 씁니다.
pub trait TextHitGeometry {
    /// `layout`의 line rect들. 모르면 `None`.
    fn line_rects(&self, layout: TextLayoutKey) -> Option<&[Rect]>;
}

/// line rect를 모르는 제공자: Text는 항상 layout 상자로 판정합니다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LayoutBoxesOnly;

impl TextHitGeometry for LayoutBoxesOnly {
    fn line_rects(&self, _layout: TextLayoutKey) -> Option<&[Rect]> {
        None
    }
}

/// 내부 계산용 점(f64).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pt {
    x: f64,
    y: f64,
}

impl Pt {
    fn of(point: Vec2) -> Pt {
        Pt {
            x: f64::from(point.x()),
            y: f64::from(point.y()),
        }
    }

    fn sub(self, other: Pt) -> Pt {
        Pt {
            x: self.x - other.x,
            y: self.y - other.y,
        }
    }

    fn dot(self, other: Pt) -> f64 {
        self.x * other.x + self.y * other.y
    }

    fn cross(self, other: Pt) -> f64 {
        self.x * other.y - self.y * other.x
    }

    fn len2(self) -> f64 {
        self.dot(self)
    }
}

/// 선분화한 subpath.
#[derive(Debug, Clone)]
struct Polyline {
    points: Vec<Pt>,
    closed: bool,
}

/// cubic을 나눌 선분 수(Wang 공식, `[1, MAX_CUBIC_SEGMENTS]`).
fn cubic_segments(p0: Pt, p1: Pt, p2: Pt, p3: Pt, tolerance: f64) -> usize {
    let d1 = Pt {
        x: p0.x - 2.0 * p1.x + p2.x,
        y: p0.y - 2.0 * p1.y + p2.y,
    };
    let d2 = Pt {
        x: p1.x - 2.0 * p2.x + p3.x,
        y: p1.y - 2.0 * p2.y + p3.y,
    };
    let m = d1.len2().max(d2.len2()).sqrt();
    let n = (0.75 * m / tolerance).sqrt().ceil();
    if n.is_finite() && n >= 1.0 {
        (n as usize).min(MAX_CUBIC_SEGMENTS)
    } else if n.is_finite() {
        1
    } else {
        MAX_CUBIC_SEGMENTS
    }
}

fn cubic_at(p0: Pt, p1: Pt, p2: Pt, p3: Pt, t: f64) -> Pt {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    Pt {
        x: a * p0.x + b * p1.x + c * p2.x + d * p3.x,
        y: a * p0.y + b * p1.y + c * p2.y + d * p3.y,
    }
}

/// geometry를 local 허용 오차 `tolerance`로 선분화합니다.
fn flatten(geometry: &PathGeometry, tolerance: f64) -> Vec<Polyline> {
    let tolerance = if tolerance.is_finite() && tolerance > 0.0 {
        tolerance
    } else {
        f64::MIN_POSITIVE
    };
    let mut out: Vec<Polyline> = Vec::new();
    let mut current: Option<Polyline> = None;
    for command in geometry.commands() {
        match *command {
            PathCommand::MoveTo(point) => {
                out.extend(current.take());
                current = Some(Polyline {
                    points: vec![Pt::of(point)],
                    closed: false,
                });
            }
            PathCommand::LineTo(point) => {
                if let Some(line) = current.as_mut() {
                    line.points.push(Pt::of(point));
                }
            }
            PathCommand::CubicTo { c1, c2, to } => {
                if let Some(line) = current.as_mut() {
                    let p0 = line.points.last().copied().unwrap_or(Pt::of(c1));
                    let (p1, p2, p3) = (Pt::of(c1), Pt::of(c2), Pt::of(to));
                    let n = cubic_segments(p0, p1, p2, p3, tolerance);
                    for i in 1..n {
                        line.points
                            .push(cubic_at(p0, p1, p2, p3, i as f64 / n as f64));
                    }
                    line.points.push(p3);
                }
            }
            PathCommand::Close => {
                if let Some(mut line) = current.take() {
                    line.closed = true;
                    out.push(line);
                }
            }
        }
    }
    out.extend(current);
    out
}

/// polyline 목록(각각 암묵적으로 닫음)의 winding number.
fn winding_number(lines: &[Polyline], point: Pt) -> i64 {
    let mut winding = 0i64;
    for line in lines {
        let count = line.points.len();
        for i in 0..count {
            let a = line.points[i];
            let b = line.points[(i + 1) % count];
            let side = b.sub(a).cross(point.sub(a));
            if a.y <= point.y {
                if b.y > point.y && side > 0.0 {
                    winding += 1;
                }
            } else if b.y <= point.y && side < 0.0 {
                winding -= 1;
            }
        }
    }
    winding
}

fn fill_contains(lines: &[Polyline], fill_rule: FillRule, point: Pt) -> bool {
    let winding = winding_number(lines, point);
    match fill_rule {
        FillRule::NonZero => winding != 0,
        FillRule::EvenOdd => winding % 2 != 0,
    }
}

/// 선분 `a→b`에서 `point`까지 거리가 `reach` 이내인지. 열린 끝(`cap_start`, `cap_end`)은 `cap`을
/// 따르고, 나머지 끝은 round(join)입니다.
fn segment_within(
    a: Pt,
    b: Pt,
    point: Pt,
    reach: f64,
    cap: StrokeCap,
    cap_start: bool,
    cap_end: bool,
) -> bool {
    let ab = b.sub(a);
    let ap = point.sub(a);
    let len2 = ab.len2();
    if len2 == 0.0 {
        // 길이 0 선분: 열린 끝이 있으면 Round만 점(원)으로 그립니다.
        let round = !(cap_start || cap_end) || cap == StrokeCap::Round;
        return round && ap.len2() <= reach * reach;
    }
    let t = ap.dot(ab) / len2;
    let len = len2.sqrt();
    let (start_limit, end_limit) = {
        let extend = |open: bool| match (open, cap) {
            (false, _) | (true, StrokeCap::Round) => f64::INFINITY,
            (true, StrokeCap::Butt) => 0.0,
            (true, StrokeCap::Square) => reach / len,
        };
        (extend(cap_start), extend(cap_end))
    };
    if t < -start_limit || t > 1.0 + end_limit {
        return false;
    }
    if (t < 0.0 && cap_start && cap == StrokeCap::Square)
        || (t > 1.0 && cap_end && cap == StrokeCap::Square)
    {
        // Square cap 연장부: 선분 방향 사각형 안.
        return (ab.cross(ap) / len).abs() <= reach;
    }
    let clamped = t.clamp(0.0, 1.0);
    let nearest = Pt {
        x: a.x + ab.x * clamped,
        y: a.y + ab.y * clamped,
    };
    point.sub(nearest).len2() <= reach * reach
}

/// 선분화한 윤곽에서 `point`까지 거리가 `reach` 이내인지(열린 끝은 `cap`).
fn outline_within(lines: &[Polyline], point: Pt, reach: f64, cap: StrokeCap) -> bool {
    for line in lines {
        let points = &line.points;
        match points.len() {
            0 => continue,
            1 => {
                // MoveTo만 있는 subpath: Round cap만 점으로 그립니다.
                if !line.closed
                    && cap == StrokeCap::Round
                    && point.sub(points[0]).len2() <= reach * reach
                {
                    return true;
                }
                continue;
            }
            _ => {}
        }
        let segments = points.len() - 1;
        for i in 0..segments {
            let open = !line.closed;
            if segment_within(
                points[i],
                points[i + 1],
                point,
                reach,
                cap,
                open && i == 0,
                open && i + 1 == segments,
            ) {
                return true;
            }
        }
        if line.closed {
            let (last, first) = (points[segments], points[0]);
            if segment_within(last, first, point, reach, cap, false, false) {
                return true;
            }
        }
    }
    false
}

/// stroke가 상자 밖으로 나가는 거리(상자형 content용, 끝 모양 없음).
fn box_outset(stroke: &StrokeLayer) -> f32 {
    match stroke.align() {
        StrokeAlign::Center => stroke.width() / 2.0,
        StrokeAlign::Inside => 0.0,
        StrokeAlign::Outside => stroke.width(),
    }
}

/// 둥근 사각형에서 `point`까지의 거리(안이면 0). radius는 실효 값입니다.
pub fn rounded_rect_distance(bounds: &Aabb, radii: &CornerRadii, point: Vec2) -> f32 {
    if rounded_rect_contains(bounds, radii, point) {
        return 0.0;
    }
    let (x0, y0, x1, y1) = (
        f64::from(bounds.min_x()),
        f64::from(bounds.min_y()),
        f64::from(bounds.max_x()),
        f64::from(bounds.max_y()),
    );
    let p = Pt::of(point);
    let q = Pt {
        x: p.x.clamp(x0, x1),
        y: p.y.clamp(y0, y1),
    };
    let (tl, tr, br, bl) = radii.components();
    let (tl, tr, br, bl) = (f64::from(tl), f64::from(tr), f64::from(br), f64::from(bl));
    // 가장 가까운 상자 점이 모서리 영역이면 호까지, 아니면 변까지의 거리입니다.
    let corner = if tl > 0.0 && q.x < x0 + tl && q.y < y0 + tl {
        Some((x0 + tl, y0 + tl, tl))
    } else if tr > 0.0 && q.x > x1 - tr && q.y < y0 + tr {
        Some((x1 - tr, y0 + tr, tr))
    } else if br > 0.0 && q.x > x1 - br && q.y > y1 - br {
        Some((x1 - br, y1 - br, br))
    } else if bl > 0.0 && q.x < x0 + bl && q.y > y1 - bl {
        Some((x0 + bl, y1 - bl, bl))
    } else {
        None
    };
    let distance = match corner {
        Some((cx, cy, r)) => (p.sub(Pt { x: cx, y: cy }).len2().sqrt() - r).max(0.0),
        None => p.sub(q).len2().sqrt(),
    };
    distance as f32
}

/// 축척에 맞춘 local 허용 거리와 선분화 허용 오차.
#[derive(Debug, Clone, Copy)]
struct LocalScale {
    tolerance: f64,
    flatten: f64,
}

impl LocalScale {
    fn of(transform: &Transform2D, tolerance: f32) -> Option<LocalScale> {
        let scale = f64::from(linear_scale(transform)?);
        Some(LocalScale {
            tolerance: f64::from(tolerance) / scale,
            flatten: f64::from(FLATTEN_TOLERANCE) / scale,
        })
    }
}

fn path_contains(
    node: &SceneNode,
    geometry: &PathGeometry,
    fill_rule: FillRule,
    point: Pt,
    scale: LocalScale,
) -> bool {
    let lines = flatten(geometry, scale.flatten);
    let layers = &node.paint().get().layers;
    let has_fill = layers
        .iter()
        .any(|layer| matches!(layer, PaintLayer::Fill(_)));
    let strokes = layers.iter().filter_map(|layer| match layer {
        PaintLayer::Stroke(stroke) if stroke.width() > 0.0 => Some(stroke),
        _ => None,
    });
    let mut has_stroke = false;
    let mut inside: Option<bool> = None;
    let mut inside_fill = || *inside.get_or_insert_with(|| fill_contains(&lines, fill_rule, point));
    for stroke in strokes {
        has_stroke = true;
        let width = f64::from(stroke.width());
        let hit = match stroke.align() {
            StrokeAlign::Center => {
                outline_within(&lines, point, width / 2.0 + scale.tolerance, stroke.cap())
            }
            StrokeAlign::Inside => {
                inside_fill()
                    && outline_within(&lines, point, width + scale.tolerance, stroke.cap())
            }
            StrokeAlign::Outside => {
                !inside_fill()
                    && outline_within(&lines, point, width + scale.tolerance, stroke.cap())
            }
        };
        if hit {
            return true;
        }
    }
    if has_fill || !has_stroke {
        // fill 영역(paint가 없으면 내부 정책). 허용 거리는 윤곽 근처까지 넓힙니다.
        return inside_fill()
            || (scale.tolerance > 0.0
                && outline_within(&lines, point, scale.tolerance, StrokeCap::Round));
    }
    false
}

/// 상자형 content의 stroke 바깥 거리(stroke layer 최대값).
fn node_box_outset(node: &SceneNode) -> f32 {
    node.paint()
        .get()
        .layers
        .iter()
        .filter_map(|layer| match layer {
            PaintLayer::Stroke(stroke) => Some(box_outset(stroke)),
            PaintLayer::Fill(_) => None,
        })
        .fold(0.0f32, f32::max)
}

fn rect_within(rect: Rect, radii: &CornerRadii, point: Vec2, reach: f64) -> bool {
    f64::from(rounded_rect_distance(&Aabb::from_rect(rect), radii, point)) <= reach
}

fn text_contains(
    text: &TextPrimitive,
    point: Vec2,
    reach: f64,
    geometry: &dyn TextHitGeometry,
) -> bool {
    match geometry.line_rects(text.layout) {
        Some(rects) if !rects.is_empty() => rects
            .iter()
            .any(|rect| rect_within(*rect, &CornerRadii::ZERO, point, reach)),
        _ => rect_within(text.bounds, &CornerRadii::ZERO, point, reach),
    }
}

/// node local 점 `local`이 node content의 hit 영역 안인지(모듈 문서의 종류별 정책).
///
/// 표시·pointer-events·clip은 보지 않습니다(broad-phase와 [`clip_stack_contains_precise`] 몫).
/// world transform이 역변환 불가(축척 0)면 면적이 없어 거짓입니다.
pub fn content_contains(
    node: &SceneNode,
    local: Vec2,
    options: &NarrowPhaseOptions,
    text: &dyn TextHitGeometry,
) -> bool {
    let Some(scale) = LocalScale::of(&node.world_transform(), options.tolerance()) else {
        return false;
    };
    match node.content() {
        SceneContent::Rect(rect) => rect_within(
            rect.rect,
            &rect.radii,
            local,
            f64::from(node_box_outset(node)) + scale.tolerance,
        ),
        SceneContent::Path(path) => {
            path_contains(node, &path.geometry, path.fill_rule, Pt::of(local), scale)
        }
        SceneContent::Text(primitive) => text_contains(
            primitive,
            local,
            f64::from(node_box_outset(node)) + scale.tolerance,
            text,
        ),
        SceneContent::Image(image) => rect_within(
            image.bounds,
            &CornerRadii::ZERO,
            local,
            f64::from(node_box_outset(node)) + scale.tolerance,
        ),
    }
}

/// world 점이 clip 하나 안인지(정밀). path clip은 clip path 내부를 fill rule로 판정합니다.
pub fn clip_contains_precise(clip: &ClipShape, point: Vec2) -> bool {
    match clip {
        ClipShape::Rect { .. } => crate::hit_test::clip_contains(clip, point),
        ClipShape::Path {
            geometry,
            fill_rule,
            transform,
        } => {
            let (Some(local), Some(scale)) =
                (to_local(transform, point), LocalScale::of(transform, 0.0))
            else {
                return false;
            };
            fill_contains(&flatten(geometry, scale.flatten), *fill_rule, Pt::of(local))
        }
    }
}

/// world 점이 clip stack 전체 안인지(정밀).
pub fn clip_stack_contains_precise(clip: &ClipStackRef, point: Vec2) -> bool {
    clip.iter_inner_first()
        .all(|shape| clip_contains_precise(shape, point))
}

/// world 점 `point`의 hit test: broad-phase(XI.3.3) 후 path clip 정밀 판정과 content geometry
/// narrow-phase를 적용합니다. 결과는 topmost 우선이며 통계에 narrow-phase 제외 수가 더해집니다.
pub fn hit_test(
    index: &SceneSpatialIndex,
    point: Vec2,
    options: &NarrowPhaseOptions,
    text: &dyn TextHitGeometry,
) -> HitResult {
    let mut result = broad_phase(index, point, options.tolerance());
    let mut stats = result.stats;
    result.hits.retain(|hit| {
        let has_path_clip = hit
            .node
            .clip()
            .iter_inner_first()
            .any(|clip| matches!(clip, ClipShape::Path { .. }));
        if has_path_clip && !clip_stack_contains_precise(hit.node.clip(), point) {
            stats.path_clipped += 1;
            return false;
        }
        if !content_contains(&hit.node, hit.local_point, options, text) {
            stats.content_missed += 1;
            return false;
        }
        true
    });
    result.stats = stats;
    result
}

/// view(화면) 좌표 점의 [`hit_test`]. `options.tolerance`는 **view 단위**로 보고 viewport 평균
/// 축척으로 world 단위로 바꿉니다. viewport 역변환이 비유한이면 빈 결과입니다.
pub fn hit_test_view_narrow(
    index: &SceneSpatialIndex,
    viewport: &Viewport,
    view_point: Vec2,
    options: &NarrowPhaseOptions,
    text: &dyn TextHitGeometry,
) -> HitResult {
    let view = viewport.view();
    let (Some(world), Some(scale)) = (to_local(&view, view_point), linear_scale(&view)) else {
        return HitResult::default();
    };
    let world_options = NarrowPhaseOptions::with_tolerance(options.tolerance() / scale)
        .unwrap_or(NarrowPhaseOptions::EXACT);
    hit_test(index, world, &world_options, text)
}

#[cfg(test)]
mod tests;

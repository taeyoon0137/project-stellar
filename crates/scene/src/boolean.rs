//! Boolean modifier geometry 평가 (XII.10.30, ADR-0084 결정 7).
//!
//! Union·Subtract·Intersect·Exclude 결과 path를 **결정론적 순수 함수**([`evaluate_boolean`])로 계산합니다.
//!
//! # operand 규칙
//!
//! - operand 목록의 첫 항목은 Boolean modifier를 가진 host node의 geometry이고, 그 뒤는 참조한 operand node를
//!   **문서 순서(OrderKey 순 pre-order)**로 정렬한 것입니다. 참조 목록의 저장 순서는 쓰지 않습니다. 정렬은 호출자
//!   (document → scene 변환)가 하고 이 함수는 받은 순서를 그대로 씁니다.
//! - Subtract는 첫 operand(host)에서 나머지 operand의 합집합을 뺍니다. Union·Intersect는 순서와 무관하고, Exclude는
//!   홀수 개 operand 안인 영역입니다.
//! - operand node는 결과에 흡수되어 단독으로 표시하지 않습니다. 결과는 host의 paint로 칠합니다.
//!
//! # 계산
//!
//! 1. 각 operand를 local 허용 오차 [`BOOLEAN_FLATTEN_TOLERANCE_PX`]로 선분화하고(narrow phase hit test와 같은
//!    선분화), operand transform으로 결과 좌표계에 옮긴 뒤 [`BOOLEAN_GRID_PER_PX`] 격자 정수 좌표로 반올림합니다.
//!    모든 subpath는 채우기처럼 닫힌 것으로 봅니다.
//! 2. 모든 선분 쌍의 교차점에서 선분을 나눕니다(정수 orientation은 `i128`로 정확히 계산하고 교차점은 격자로
//!    반올림). 반올림으로 생긴 새 교차가 없어질 때까지 반복합니다([`MAX_SPLIT_ROUNDS`]).
//! 3. 같은 끝점을 갖는 선분을 하나로 묶고, operand마다 방향별 개수를 셉니다. 선분 오른쪽 아주 가까운 점의 winding을
//!    operand마다 계산하고, 왼쪽 winding은 오른쪽 + 방향 개수 차입니다(선분을 오른쪽에서 왼쪽으로 건너면 그 선분의
//!    순 기여만큼 바뀜). operand 채우기 규칙과 연산으로 양쪽 안·밖을 정해, 경계인 선분만 결과 안쪽이 왼쪽이 되게
//!    남깁니다.
//! 4. 남은 방향 선분을 이어 닫힌 loop로 만들고, 같은 방향으로 이어지는 점을 지웁니다. loop는 가장 작은 꼭짓점에서
//!    시작하고 그 꼭짓점 순서로 정렬합니다. 결과 채우기 규칙은 [`FillRule::NonZero`]입니다(winding은 선분 합이라
//!    loop 묶는 방식과 무관).
//!
//! 정수 격자와 `f64` 기본 연산만 쓰므로 같은 입력은 bit 단위로 같은 결과입니다. 좌표가
//! [`MAX_BOOLEAN_COORD_PX`]를 넘거나 선분이 [`MAX_BOOLEAN_EDGES`]를 넘으면 거부합니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::value::{Transform2D, Vec2};

use crate::geometry::{FillRule, PathCommand, PathGeometry};
use crate::narrow_phase::flatten;

/// 정수 격자 해상도(px당 칸 수). layout 단위(1/64 px)와 같습니다.
pub const BOOLEAN_GRID_PER_PX: f64 = 64.0;

/// 곡선 선분화 허용 오차(결과 좌표계 px).
pub const BOOLEAN_FLATTEN_TOLERANCE_PX: f64 = 0.05;

/// 결과 좌표계 좌표 절댓값 상한(px).
pub const MAX_BOOLEAN_COORD_PX: f64 = 32_768.0;

/// 선분화 후 선분 수 상한.
pub const MAX_BOOLEAN_EDGES: usize = 20_000;

/// 교차 분할 반복 상한.
pub const MAX_SPLIT_ROUNDS: usize = 16;

/// 오른쪽 winding 표본점까지의 거리(격자 단위). 격자 좌표 선분의 중점과 다른 선분 직선 사이 거리 하한
/// (`1 / (2 · 최대 선분 길이)`)보다 충분히 작습니다.
const PROBE_OFFSET: f64 = 1.0e-8;

/// Boolean 연산. document `BooleanOp`(VI.9.3)와 같은 의미입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BooleanOp {
    /// 합집합.
    Union,
    /// 첫 operand − 나머지 합집합.
    Subtract,
    /// 교집합.
    Intersect,
    /// 홀수 개 operand 안.
    Exclude,
}

impl BooleanOp {
    /// operand별 안쪽 여부를 결과 안쪽 여부로 합칩니다.
    fn combine(self, inside: &[bool]) -> bool {
        match self {
            BooleanOp::Union => inside.iter().any(|&v| v),
            BooleanOp::Intersect => !inside.is_empty() && inside.iter().all(|&v| v),
            BooleanOp::Subtract => match inside.split_first() {
                Some((&first, rest)) => first && !rest.iter().any(|&v| v),
                None => false,
            },
            BooleanOp::Exclude => inside.iter().filter(|&&v| v).count() % 2 == 1,
        }
    }
}

/// operand 하나.
#[derive(Debug, Clone, PartialEq)]
pub struct BooleanOperand {
    /// operand local geometry.
    pub geometry: PathGeometry,
    /// operand 채우기 규칙.
    pub fill_rule: FillRule,
    /// operand local → 결과(host local) 좌표 변환.
    pub transform: Transform2D,
}

/// Boolean 평가 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BooleanError {
    /// operand가 없습니다.
    NoOperands,
    /// 결과 좌표가 [`MAX_BOOLEAN_COORD_PX`]를 넘거나 유한하지 않습니다.
    OutOfRange {
        /// operand index.
        operand: usize,
    },
    /// 선분 수가 [`MAX_BOOLEAN_EDGES`]를 넘습니다.
    TooManyEdges {
        /// 선분 수.
        edges: usize,
    },
    /// 교차 분할이 [`MAX_SPLIT_ROUNDS`] 안에 끝나지 않았습니다.
    Unstable,
}

impl fmt::Display for BooleanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BooleanError::NoOperands => f.write_str("Boolean operand가 없습니다"),
            BooleanError::OutOfRange { operand } => {
                write!(
                    f,
                    "Boolean operand {operand}의 좌표가 허용 범위를 벗어났습니다"
                )
            }
            BooleanError::TooManyEdges { edges } => {
                write!(f, "Boolean 선분 {edges}개가 상한을 넘습니다")
            }
            BooleanError::Unstable => f.write_str("Boolean 교차 분할이 수렴하지 않았습니다"),
        }
    }
}

impl std::error::Error for BooleanError {}

/// 격자 좌표 점.
type P = (i64, i64);

#[derive(Debug, Clone, Copy)]
struct Edge {
    a: P,
    b: P,
    operand: usize,
}

fn orient(p: P, q: P, r: P) -> i128 {
    let (px, py) = (i128::from(p.0), i128::from(p.1));
    (i128::from(q.0) - px) * (i128::from(r.1) - py)
        - (i128::from(q.1) - py) * (i128::from(r.0) - px)
}

fn on_segment(p: P, q: P, r: P) -> bool {
    // r이 p–q 위(끝점 제외)에 있는지. 세 점이 한 직선 위라고 가정합니다.
    r != p
        && r != q
        && r.0 >= p.0.min(q.0)
        && r.0 <= p.0.max(q.0)
        && r.1 >= p.1.min(q.1)
        && r.1 <= p.1.max(q.1)
}

/// 정수 나눗셈 반올림(0에서 먼 쪽으로 절반 반올림).
fn div_round(num: i128, den: i128) -> i128 {
    let (num, den) = if den < 0 { (-num, -den) } else { (num, den) };
    if num >= 0 {
        (2 * num + den) / (2 * den)
    } else {
        -((-2 * num + den) / (2 * den))
    }
}

/// 두 선분의 분할점(각 선분 위, 끝점 제외).
fn split_points(e: &Edge, f: &Edge, out_e: &mut Vec<P>, out_f: &mut Vec<P>) {
    let (a, b, c, d) = (e.a, e.b, f.a, f.b);
    if a.0.max(b.0) < c.0.min(d.0)
        || c.0.max(d.0) < a.0.min(b.0)
        || a.1.max(b.1) < c.1.min(d.1)
        || c.1.max(d.1) < a.1.min(b.1)
    {
        return;
    }
    let o1 = orient(a, b, c);
    let o2 = orient(a, b, d);
    let o3 = orient(c, d, a);
    let o4 = orient(c, d, b);
    if o1.signum() * o2.signum() < 0 && o3.signum() * o4.signum() < 0 {
        // 진교차: a + (b - a) · o3 / (o3 - o4).
        let den = o3 - o4;
        let x = i128::from(a.0) + div_round((i128::from(b.0) - i128::from(a.0)) * o3, den);
        let y = i128::from(a.1) + div_round((i128::from(b.1) - i128::from(a.1)) * o3, den);
        let p = (x as i64, y as i64);
        if p != a && p != b {
            out_e.push(p);
        }
        if p != c && p != d {
            out_f.push(p);
        }
        return;
    }
    // 끝점이 다른 선분 위에 놓인 경우(한 직선 위 겹침 포함).
    if o1 == 0 && on_segment(a, b, c) {
        out_e.push(c);
    }
    if o2 == 0 && on_segment(a, b, d) {
        out_e.push(d);
    }
    if o3 == 0 && on_segment(c, d, a) {
        out_f.push(a);
    }
    if o4 == 0 && on_segment(c, d, b) {
        out_f.push(b);
    }
}

/// 교차점에서 선분을 나눕니다. 새 분할이 없으면 `None`.
fn split_round(edges: &[Edge]) -> Option<Vec<Edge>> {
    let mut points: Vec<Vec<P>> = vec![Vec::new(); edges.len()];
    // x 시작점 순으로 정렬해 x 범위가 겹치는 쌍만 봅니다.
    let mut order: Vec<usize> = (0..edges.len()).collect();
    order.sort_by_key(|&i| (edges[i].a.0.min(edges[i].b.0), i));
    for (n, &i) in order.iter().enumerate() {
        let max_x = edges[i].a.0.max(edges[i].b.0);
        for &j in &order[n + 1..] {
            if edges[j].a.0.min(edges[j].b.0) > max_x {
                break;
            }
            let (mut pi, mut pj) = (Vec::new(), Vec::new());
            split_points(&edges[i], &edges[j], &mut pi, &mut pj);
            points[i].extend(pi);
            points[j].extend(pj);
        }
    }
    if points.iter().all(Vec::is_empty) {
        return None;
    }
    let mut out = Vec::with_capacity(edges.len() * 2);
    for (edge, mut cuts) in edges.iter().zip(points) {
        if cuts.is_empty() {
            out.push(*edge);
            continue;
        }
        let (dx, dy) = (
            i128::from(edge.b.0 - edge.a.0),
            i128::from(edge.b.1 - edge.a.1),
        );
        cuts.sort_by_key(|p| {
            (
                i128::from(p.0 - edge.a.0) * dx + i128::from(p.1 - edge.a.1) * dy,
                *p,
            )
        });
        cuts.dedup();
        let mut start = edge.a;
        for cut in cuts {
            if cut != start {
                out.push(Edge {
                    a: start,
                    b: cut,
                    operand: edge.operand,
                });
                start = cut;
            }
        }
        if start != edge.b {
            out.push(Edge {
                a: start,
                b: edge.b,
                operand: edge.operand,
            });
        }
    }
    Some(out)
}

/// operand별 y 구간 bucket(winding 표본 가속).
struct WindingIndex {
    min_y: i64,
    bucket: i64,
    buckets: Vec<Vec<usize>>,
}

const WINDING_BUCKETS: i64 = 256;

impl WindingIndex {
    fn new(edges: &[Edge]) -> WindingIndex {
        let min_y = edges.iter().map(|e| e.a.1.min(e.b.1)).min().unwrap_or(0);
        let max_y = edges.iter().map(|e| e.a.1.max(e.b.1)).max().unwrap_or(0);
        let bucket = ((max_y - min_y) / WINDING_BUCKETS + 1).max(1);
        let mut buckets = vec![Vec::new(); WINDING_BUCKETS as usize + 1];
        for (i, e) in edges.iter().enumerate() {
            let lo = (e.a.1.min(e.b.1) - min_y) / bucket;
            let hi = (e.a.1.max(e.b.1) - min_y) / bucket;
            for slot in lo..=hi {
                buckets[slot as usize].push(i);
            }
        }
        WindingIndex {
            min_y,
            bucket,
            buckets,
        }
    }

    /// 점 `(x, y)`(격자 단위 f64)의 operand별 winding.
    fn winding(&self, edges: &[Edge], operands: usize, x: f64, y: f64) -> Vec<i64> {
        let mut winding = vec![0i64; operands];
        let slot = ((y.floor() as i64 - self.min_y) / self.bucket).clamp(0, WINDING_BUCKETS);
        for &i in &self.buckets[slot as usize] {
            let e = &edges[i];
            let (ax, ay) = (e.a.0 as f64, e.a.1 as f64);
            let (bx, by) = (e.b.0 as f64, e.b.1 as f64);
            let side = (bx - ax) * (y - ay) - (by - ay) * (x - ax);
            if ay <= y {
                if by > y && side > 0.0 {
                    winding[e.operand] += 1;
                }
            } else if by <= y && side < 0.0 {
                winding[e.operand] -= 1;
            }
        }
        winding
    }
}

fn inside(rule: FillRule, winding: i64) -> bool {
    match rule {
        FillRule::NonZero => winding != 0,
        FillRule::EvenOdd => winding % 2 != 0,
    }
}

/// operand 하나를 격자 선분으로 바꿉니다.
fn operand_edges(
    index: usize,
    operand: &BooleanOperand,
    out: &mut Vec<Edge>,
) -> Result<(), BooleanError> {
    let (a, b, c, d, tx, ty) = operand.transform.components();
    let (a, b, c, d, tx, ty) = (
        f64::from(a),
        f64::from(b),
        f64::from(c),
        f64::from(d),
        f64::from(tx),
        f64::from(ty),
    );
    // local 허용 오차 = 결과 허용 오차 / 선형부 배율.
    let scale = (a * d - b * c).abs().sqrt();
    let tolerance = if scale.is_finite() && scale > 0.0 {
        BOOLEAN_FLATTEN_TOLERANCE_PX / scale
    } else {
        BOOLEAN_FLATTEN_TOLERANCE_PX
    };
    let limit = MAX_BOOLEAN_COORD_PX * BOOLEAN_GRID_PER_PX;
    for line in flatten(&operand.geometry, tolerance) {
        let mut points: Vec<P> = Vec::with_capacity(line.points.len());
        for p in &line.points {
            let x = (a * p.x + c * p.y + tx) * BOOLEAN_GRID_PER_PX;
            let y = (b * p.x + d * p.y + ty) * BOOLEAN_GRID_PER_PX;
            if !x.is_finite() || !y.is_finite() || x.abs() > limit || y.abs() > limit {
                return Err(BooleanError::OutOfRange { operand: index });
            }
            let q = (x.round() as i64, y.round() as i64);
            if points.last() != Some(&q) {
                points.push(q);
            }
        }
        while points.len() > 1 && points.first() == points.last() {
            points.pop();
        }
        if points.len() < 3 {
            continue;
        }
        for i in 0..points.len() {
            out.push(Edge {
                a: points[i],
                b: points[(i + 1) % points.len()],
                operand: index,
            });
        }
    }
    Ok(())
}

/// Boolean 결과 path(채우기 규칙 [`FillRule::NonZero`])를 계산합니다(모듈 문서 규칙).
pub fn evaluate_boolean(
    op: BooleanOp,
    operands: &[BooleanOperand],
) -> Result<PathGeometry, BooleanError> {
    if operands.is_empty() {
        return Err(BooleanError::NoOperands);
    }
    let mut edges = Vec::new();
    for (index, operand) in operands.iter().enumerate() {
        operand_edges(index, operand, &mut edges)?;
        if edges.len() > MAX_BOOLEAN_EDGES {
            return Err(BooleanError::TooManyEdges { edges: edges.len() });
        }
    }
    let mut rounds = 0;
    while let Some(next) = split_round(&edges) {
        rounds += 1;
        if rounds > MAX_SPLIT_ROUNDS {
            return Err(BooleanError::Unstable);
        }
        if next.len() > MAX_BOOLEAN_EDGES * 4 {
            return Err(BooleanError::TooManyEdges { edges: next.len() });
        }
        edges = next;
    }
    // 같은 끝점 선분 묶기: (작은 점, 큰 점) → operand별 방향 개수(작은 점 → 큰 점 방향 +1).
    let mut segments: BTreeMap<(P, P), Vec<i64>> = BTreeMap::new();
    for e in &edges {
        let (key, sign) = if e.a < e.b {
            ((e.a, e.b), 1)
        } else {
            ((e.b, e.a), -1)
        };
        segments
            .entry(key)
            .or_insert_with(|| vec![0; operands.len()])[e.operand] += sign;
    }
    let index = WindingIndex::new(&edges);
    let rules: Vec<FillRule> = operands.iter().map(|o| o.fill_rule).collect();
    let mut kept: Vec<(P, P)> = Vec::new();
    for (&(u, v), net) in &segments {
        let (dx, dy) = ((v.0 - u.0) as f64, (v.1 - u.1) as f64);
        let length = (dx * dx + dy * dy).sqrt();
        // 왼쪽 법선(-dy, dx)의 반대쪽 = 오른쪽 표본점.
        let mx = (u.0 as f64 + v.0 as f64) * 0.5 + dy / length * PROBE_OFFSET;
        let my = (u.1 as f64 + v.1 as f64) * 0.5 - dx / length * PROBE_OFFSET;
        let right = index.winding(&edges, operands.len(), mx, my);
        let inside_right: Vec<bool> = right
            .iter()
            .zip(&rules)
            .map(|(&w, &rule)| inside(rule, w))
            .collect();
        let inside_left: Vec<bool> = right
            .iter()
            .zip(net)
            .zip(&rules)
            .map(|((&w, &n), &rule)| inside(rule, w + n))
            .collect();
        let (left, right) = (op.combine(&inside_left), op.combine(&inside_right));
        if left != right {
            kept.push(if left { (u, v) } else { (v, u) });
        }
    }
    Ok(link_loops(kept))
}

/// 방향 선분을 닫힌 loop path로 잇습니다.
fn link_loops(edges: Vec<(P, P)>) -> PathGeometry {
    let mut outgoing: BTreeMap<P, Vec<usize>> = BTreeMap::new();
    for (i, (a, _)) in edges.iter().enumerate() {
        outgoing.entry(*a).or_default().push(i);
    }
    let mut used = vec![false; edges.len()];
    let mut loops: Vec<Vec<P>> = Vec::new();
    let order: BTreeSet<(P, P, usize)> = edges
        .iter()
        .enumerate()
        .map(|(i, (a, b))| (*a, *b, i))
        .collect();
    for &(_, _, first) in &order {
        if used[first] {
            continue;
        }
        used[first] = true;
        let start = edges[first].0;
        let mut points = vec![start];
        let mut current = first;
        loop {
            let (a, b) = edges[current];
            if b == start {
                break;
            }
            points.push(b);
            let din = ((b.0 - a.0) as f64, (b.1 - a.1) as f64);
            let next = outgoing.get(&b).and_then(|candidates| {
                candidates
                    .iter()
                    .copied()
                    .filter(|&c| !used[c])
                    .min_by(|&x, &y| {
                        let angle = |c: usize| {
                            let (p, q) = edges[c];
                            let d = ((q.0 - p.0) as f64, (q.1 - p.1) as f64);
                            (din.0 * d.1 - din.1 * d.0).atan2(din.0 * d.0 + din.1 * d.1)
                        };
                        angle(x).total_cmp(&angle(y)).then(x.cmp(&y))
                    })
            });
            match next {
                Some(next) => {
                    used[next] = true;
                    current = next;
                }
                // 열린 사슬은 경계 불변식상 생기지 않습니다. 생기면 여기서 닫습니다.
                None => break,
            }
        }
        let points = simplify(points);
        if points.len() >= 3 {
            loops.push(points);
        }
    }
    for points in &mut loops {
        let min = points
            .iter()
            .enumerate()
            .min_by_key(|(_, p)| **p)
            .map_or(0, |(i, _)| i);
        points.rotate_left(min);
    }
    loops.sort();
    let mut commands = Vec::new();
    for points in loops {
        let mut iter = points.into_iter().map(to_vec2);
        if let Some(first) = iter.next() {
            commands.push(PathCommand::MoveTo(first));
            commands.extend(iter.map(PathCommand::LineTo));
            commands.push(PathCommand::Close);
        }
    }
    PathGeometry::new(commands).unwrap_or_else(|_| PathGeometry::empty())
}

/// 한 직선 위 같은 방향으로 이어지는 꼭짓점을 지웁니다(닫힌 loop).
fn simplify(mut points: Vec<P>) -> Vec<P> {
    loop {
        let n = points.len();
        if n < 3 {
            return points;
        }
        let mut removed = false;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let prev = points[(i + n - 1) % n];
            let here = points[i];
            let next = points[(i + 1) % n];
            let straight = orient(prev, here, next) == 0
                && (i128::from(here.0 - prev.0) * i128::from(next.0 - here.0)
                    + i128::from(here.1 - prev.1) * i128::from(next.1 - here.1))
                    > 0;
            if straight && !removed {
                removed = true;
                continue;
            }
            out.push(here);
        }
        if !removed {
            return out;
        }
        points = out;
    }
}

fn to_vec2(p: P) -> Vec2 {
    let x = p.0 as f64 / BOOLEAN_GRID_PER_PX;
    let y = p.1 as f64 / BOOLEAN_GRID_PER_PX;
    // 좌표는 MAX_BOOLEAN_COORD_PX 안이라 f32로 유한합니다.
    Vec2::new(x as f32, y as f32).unwrap_or(Vec2::ZERO)
}

/// 닫힌 loop 목록(NonZero)의 넓이(px², 방향 무관 절댓값 합이 아닌 부호 합의 절댓값). 테스트·진단용입니다.
pub fn signed_area(geometry: &PathGeometry) -> f64 {
    let mut total = 0.0;
    let mut start: Option<Vec2> = None;
    let mut last: Option<Vec2> = None;
    let cross = |a: Vec2, b: Vec2| {
        f64::from(a.x()) * f64::from(b.y()) - f64::from(b.x()) * f64::from(a.y())
    };
    for command in geometry.commands() {
        match *command {
            PathCommand::MoveTo(p) => {
                start = Some(p);
                last = Some(p);
            }
            PathCommand::LineTo(p) | PathCommand::CubicTo { to: p, .. } => {
                if let Some(l) = last {
                    total += cross(l, p);
                }
                last = Some(p);
            }
            PathCommand::Close => {
                if let (Some(l), Some(s)) = (last, start) {
                    total += cross(l, s);
                }
                last = start;
            }
        }
    }
    total * 0.5
}

#[cfg(test)]
mod tests;

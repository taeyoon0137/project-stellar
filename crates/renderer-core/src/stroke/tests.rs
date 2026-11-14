//! stroke tessellation 테스트 (XI.6.2).
//!
//! 기대 bounds는 구현과 독립적인 해석 기하(반폭·법선·miter 비율 공식)로 계산한 reference 값입니다.

use super::*;
use crate::tessellation::{CacheOutcome, FillStyle, TessellationCache, TessellationKey};
use std::sync::Arc;
use stellar_scene::bounds::{path_control_bounds, Aabb};

const EPS: f32 = 1e-4;

fn v(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y).unwrap()
}

fn polyline(points: &[[f32; 2]], closed: bool) -> PathGeometry {
    let mut commands = Vec::new();
    for (index, [x, y]) in points.iter().enumerate() {
        let point = v(*x, *y);
        commands.push(if index == 0 {
            PathCommand::MoveTo(point)
        } else {
            PathCommand::LineTo(point)
        });
    }
    if closed {
        commands.push(PathCommand::Close);
    }
    PathGeometry::new(commands).unwrap()
}

fn style(width: f32) -> StrokeStyle {
    StrokeStyle::new(width).unwrap()
}

fn mesh(path: &PathGeometry, style: &StrokeStyle) -> FillMesh {
    tessellate_stroke(path, style).unwrap()
}

fn bounds(path: &PathGeometry, style: &StrokeStyle) -> Aabb {
    mesh(path, style).bounds().expect("비어 있지 않은 mesh")
}

#[track_caller]
fn assert_bounds(actual: Aabb, expected: [f32; 4], eps: f32) {
    let got = [
        actual.min_x(),
        actual.min_y(),
        actual.max_x(),
        actual.max_y(),
    ];
    for (index, (a, e)) in got.iter().zip(expected.iter()).enumerate() {
        assert!(
            (a - e).abs() <= eps,
            "bounds[{index}] 기대 {e}, 실제 {a} (전체 {got:?})"
        );
    }
}

fn has_vertex(mesh: &FillMesh, x: f32, y: f32) -> bool {
    mesh.vertices()
        .iter()
        .any(|p| (p.x() - x).abs() <= EPS && (p.y() - y).abs() <= EPS)
}

#[test]
fn caps_reference_bounds() {
    // 검증 기준(XI.6.2): reference geometry bounds fixture. 폭 2(반폭 1)의 수평선 (0,0)-(10,0).
    let line = polyline(&[[0.0, 0.0], [10.0, 0.0]], false);
    let cases = [
        (StrokeCap::Butt, [0.0, -1.0, 10.0, 1.0]),
        (StrokeCap::Square, [-1.0, -1.0, 11.0, 1.0]),
        (StrokeCap::Round, [-1.0, -1.0, 11.0, 1.0]),
    ];
    for (cap, expected) in cases {
        assert_bounds(bounds(&line, &style(2.0).with_cap(cap)), expected, EPS);
    }
    // Butt 선분은 사각형 하나(삼각형 2개).
    assert_eq!(mesh(&line, &style(2.0)).triangle_count(), 2);
}

#[test]
fn joins_reference_bounds() {
    // 꼭짓점 B=(10,0)에서 내각 60°로 꺾이는 V. 반폭 1.
    // miter 비율 = 1/sin(30°) = 2 → 끝 x = 10 + 2. round = 10 + 1. bevel = 10 + sin(30°) = 10.5.
    let (s, c) = (0.5f32, 3.0f32.sqrt() / 2.0);
    let a = [10.0 - 10.0 * c, -10.0 * s];
    let b = [10.0, 0.0];
    let cc = [10.0 - 10.0 * c, 10.0 * s];
    let path = polyline(&[a, b, cc], false);
    let base = style(2.0);
    let limit = |value: f32| MiterLimit::new(value).unwrap();
    let cases = [
        (base.with_join(StrokeJoin::Miter), 12.0),
        (base.with_join(StrokeJoin::Round), 11.0),
        (base.with_join(StrokeJoin::Bevel), 10.5),
        // limit 1.5 < 비율 2 → bevel로 대체.
        (
            base.with_join(StrokeJoin::Miter)
                .with_miter_limit(limit(1.5)),
            10.5,
        ),
        // limit 정확히 2 → miter 유지.
        (
            base.with_join(StrokeJoin::Miter)
                .with_miter_limit(limit(2.0)),
            12.0,
        ),
    ];
    // 양 끝 Butt cap 모서리: 끝점 ± 법선(−sin30, cos30) 방향 반폭.
    let min_x = a[0] - s;
    let min_y = a[1] - c;
    for (index, (stroke, max_x)) in cases.iter().enumerate() {
        let got = bounds(&path, stroke);
        assert!(
            (got.max_x() - max_x).abs() <= 2e-3,
            "case {index}: max_x 기대 {max_x}, 실제 {}",
            got.max_x()
        );
        assert_bounds(got, [min_x, min_y, got.max_x(), -min_y], EPS);
    }
}

#[test]
fn closed_square_has_joins_but_no_caps() {
    let square = polyline(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], true);
    for join in [StrokeJoin::Miter, StrokeJoin::Round, StrokeJoin::Bevel] {
        for cap in [StrokeCap::Butt, StrokeCap::Square, StrokeCap::Round] {
            let stroke = style(2.0).with_join(join).with_cap(cap);
            assert_bounds(bounds(&square, &stroke), [-1.0, -1.0, 11.0, 11.0], EPS);
            // cap은 닫힌 subpath에 영향이 없습니다.
            assert_eq!(
                mesh(&square, &stroke),
                mesh(&square, &style(2.0).with_join(join)),
                "{join:?} {cap:?}"
            );
        }
    }
    // miter만 바깥 모서리 꼭짓점 (−1,−1)을 만듭니다(시작점 join 포함).
    assert!(has_vertex(&mesh(&square, &style(2.0)), -1.0, -1.0));
    assert!(!has_vertex(
        &mesh(&square, &style(2.0).with_join(StrokeJoin::Bevel)),
        -1.0,
        -1.0
    ));
    // 4 segment(삼각형 8) + 4 miter join(삼각형 8).
    assert_eq!(mesh(&square, &style(2.0)).triangle_count(), 16);
}

#[test]
fn cubic_quarter_circle_bounds_within_tolerance() {
    // 반지름 10 사분원(표준 cubic 근사 k = 0.5523) + 폭 2 Butt.
    let k = 0.552_284_8f32 * 10.0;
    let path = PathGeometry::new(vec![
        PathCommand::MoveTo(v(10.0, 0.0)),
        PathCommand::CubicTo {
            c1: v(10.0, k),
            c2: v(k, 10.0),
            to: v(0.0, 10.0),
        },
    ])
    .unwrap();
    for tolerance in [0.25f32, 0.05, 0.01] {
        let stroke = style(2.0).with_tolerance(FlattenTolerance::new(tolerance).unwrap());
        let got = bounds(&path, &stroke);
        // 끝 cap 모서리 (9,0),(11,0),(0,9),(0,11)이 bounds를 정합니다. 곡선 접선은 평탄화 chord로
        // 근사하므로 계약은 "허용 오차 이내"입니다.
        assert_bounds(got, [0.0, 0.0, 11.0, 11.0], tolerance + 1e-3);
        // 모든 정점은 바깥 offset 원(반지름 11) 밖으로 허용 오차 넘게 나가지 않습니다.
        let m = mesh(&path, &stroke);
        let farthest = m
            .vertices()
            .iter()
            .map(|p| (p.x() * p.x() + p.y() * p.y()).sqrt())
            .fold(0.0f32, f32::max);
        assert!(
            farthest <= 11.0 + tolerance + 0.01,
            "tolerance {tolerance}: 최대 반지름 {farthest}"
        );
        let diagonal_outer = m
            .vertices()
            .iter()
            .filter(|p| (p.x() - p.y()).abs() < 1.0)
            .map(|p| (p.x() * p.x() + p.y() * p.y()).sqrt())
            .fold(0.0f32, f32::max);
        assert!(
            (diagonal_outer - 11.0).abs() <= tolerance + 0.05,
            "tolerance {tolerance}: 대각 바깥 반지름 {diagonal_outer}"
        );
    }
    // 넓은 stroke는 offset 오차를 지키려고 더 잘게 나눕니다.
    let wide = style(20.0);
    let wide_bounds = bounds(&path, &wide);
    assert_bounds(wide_bounds, [0.0, 0.0, 20.0, 20.0], 0.25 + 1e-3);
    assert!(mesh(&path, &wide).triangle_count() > mesh(&path, &style(2.0)).triangle_count());
    // 허용 오차가 작을수록 segment(삼각형)가 늘어납니다.
    let coarse = mesh(&path, &style(2.0)).triangle_count();
    let fine = mesh(
        &path,
        &style(2.0).with_tolerance(FlattenTolerance::new(0.01).unwrap()),
    )
    .triangle_count();
    assert!(fine > coarse);
}

#[test]
fn dash_reference_bounds() {
    let line = polyline(&[[0.0, 0.0], [10.0, 0.0]], false);
    let dash = |intervals: &[f32], offset: f32| DashPattern::new(intervals, offset).unwrap();
    // (pattern, cap, 기대 bounds, dash 수)
    let cases = [
        // [0,3], [5,8].
        (
            dash(&[3.0, 2.0], 0.0),
            StrokeCap::Butt,
            [0.0, -1.0, 8.0, 1.0],
            2,
        ),
        // offset 1: [0,2], [4,7], [9,10].
        (
            dash(&[3.0, 2.0], 1.0),
            StrokeCap::Butt,
            [0.0, -1.0, 10.0, 1.0],
            3,
        ),
        // [0,3]만.
        (
            dash(&[3.0, 7.0], 0.0),
            StrokeCap::Butt,
            [0.0, -1.0, 3.0, 1.0],
            1,
        ),
        // 음수 offset −2 = phase 8: off 남은 2 → [2,5]? 아님: phase 8은 off 구간(3..10)의 5 지점.
        // 남은 off 2 뒤 [2,5], 다시 off.
        (
            dash(&[3.0, 7.0], -2.0),
            StrokeCap::Butt,
            [2.0, -1.0, 5.0, 1.0],
            1,
        ),
        // Square cap은 각 dash를 반폭만큼 연장: [−1,4], [4,9].
        (
            dash(&[3.0, 2.0], 0.0),
            StrokeCap::Square,
            [-1.0, -1.0, 9.0, 1.0],
            2,
        ),
        // 홀수 [2] → [2,2]: [0,2],[4,6],[8,10].
        (
            dash(&[2.0], 0.0),
            StrokeCap::Butt,
            [0.0, -1.0, 10.0, 1.0],
            3,
        ),
    ];
    for (index, (pattern, cap, expected, dashes)) in cases.iter().enumerate() {
        let stroke = style(2.0).with_cap(*cap).with_dash(Some(*pattern));
        assert_bounds(bounds(&line, &stroke), *expected, EPS);
        // 직선 dash 하나는 사각형 1개(+ Square cap 사각형 2개).
        let per_dash = if *cap == StrokeCap::Square { 6 } else { 2 };
        assert_eq!(
            mesh(&line, &stroke).triangle_count(),
            dashes * per_dash,
            "case {index}"
        );
    }
    // 홀수 간격은 두 번 반복됩니다.
    let odd = dash(&[2.0, 1.0, 3.0], 0.0);
    assert_eq!(odd.len(), 6);
    assert_eq!(odd.interval(3), Some(2.0));
    // 간격 합 0은 실선과 같은 mesh·같은 key.
    let zero = dash(&[0.0, 0.0], 0.0);
    assert!(zero.is_solid());
    assert_eq!(
        mesh(&line, &style(2.0).with_dash(Some(zero))),
        mesh(&line, &style(2.0))
    );
}

#[test]
fn zero_length_dash_draws_dots_along_direction() {
    // 간격 [0, 5]: 0, 5 위치에 길이 0 dash → Round cap 점 2개(끝점 10에서 시작하는 dash는 없음).
    let line = polyline(&[[0.0, 0.0], [10.0, 0.0]], false);
    let pattern = DashPattern::new(&[0.0, 5.0], 0.0).unwrap();
    let round = style(2.0)
        .with_cap(StrokeCap::Round)
        .with_dash(Some(pattern));
    assert_bounds(bounds(&line, &round), [-1.0, -1.0, 6.0, 1.0], EPS);
    // Butt는 아무것도 그리지 않습니다.
    assert_eq!(
        mesh(&line, &style(2.0).with_dash(Some(pattern))).triangle_count(),
        0
    );
    // 대각선 위 Square 점은 진행 방향으로 회전합니다(정사각형 꼭짓점이 축 위에 옴).
    let diagonal = polyline(&[[0.0, 0.0], [10.0, 10.0]], false);
    let square = style(2.0)
        .with_cap(StrokeCap::Square)
        .with_dash(Some(DashPattern::new(&[0.0, 100.0], 0.0).unwrap()));
    let half_diag = 2f32.sqrt();
    assert_bounds(
        bounds(&diagonal, &square),
        [-half_diag, -half_diag, half_diag, half_diag],
        EPS,
    );
}

#[test]
fn closed_dash_merges_seam_into_join() {
    // 둘레 40인 정사각형, [6,4] offset 3: on 구간 [0,3],[7,13],[17,23],[27,33],[37,40].
    // 끝 [37,40]과 시작 [0,3]이 이어져 dash 4개, 각 dash가 모서리 하나를 지납니다.
    let square = polyline(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], true);
    let pattern = DashPattern::new(&[6.0, 4.0], 3.0).unwrap();
    let flattened = flatten(&square, 0.25, Some(1.0)).unwrap();
    let mut budget = DashBudget::default();
    let runs = dash_polyline(&flattened[0], &ResolvedDash::new(pattern), &mut budget).unwrap();
    assert_eq!(runs.len(), 4);
    assert!(runs.iter().all(|run| run.points.len() == 3 && !run.closed));
    // 이어진 dash의 가운데 점이 시작 모서리 (0,0)입니다.
    assert_eq!(runs[0].points[1], P::new(0.0, 0.0));

    let stroke = style(2.0).with_dash(Some(pattern));
    let m = mesh(&square, &stroke);
    // 시작 모서리에도 miter join 꼭짓점이 생깁니다.
    for (x, y) in [(-1.0, -1.0), (11.0, -1.0), (11.0, 11.0), (-1.0, 11.0)] {
        assert!(has_vertex(&m, x, y), "miter 꼭짓점 ({x}, {y})");
    }
    // dash 4개 × (segment 2개 + miter join 1개) = 삼각형 4 × (4 + 2).
    assert_eq!(m.triangle_count(), 24);

    // 처음부터 끝까지 on이면 닫힌 subpath 그대로(cap 없음, 실선과 같은 mesh).
    let long = DashPattern::new(&[100.0, 1.0], 0.0).unwrap();
    assert_eq!(
        mesh(
            &square,
            &style(2.0).with_cap(StrokeCap::Square).with_dash(Some(long))
        ),
        mesh(&square, &style(2.0))
    );
}

#[test]
fn zero_length_subpaths_follow_line_dot_policy() {
    // document Line 규칙(VI.5.4): Round 원, Square 축 정렬 정사각형, Butt 없음.
    let paths = [
        polyline(&[[5.0, 5.0], [5.0, 5.0]], false),
        // "M p Z"도 길이 0 subpath입니다.
        polyline(&[[5.0, 5.0]], true),
    ];
    for path in &paths {
        assert_bounds(
            bounds(path, &style(4.0).with_cap(StrokeCap::Round)),
            [3.0, 3.0, 7.0, 7.0],
            EPS,
        );
        assert_bounds(
            bounds(path, &style(4.0).with_cap(StrokeCap::Square)),
            [3.0, 3.0, 7.0, 7.0],
            EPS,
        );
        assert_eq!(mesh(path, &style(4.0)).triangle_count(), 0);
    }
    // MoveTo만 있는 subpath는 그리지 않습니다.
    let lone = PathGeometry::new(vec![PathCommand::MoveTo(v(1.0, 1.0))]).unwrap();
    assert_eq!(
        mesh(&lone, &style(4.0).with_cap(StrokeCap::Round)).triangle_count(),
        0
    );
    // 빈 path, 폭 0.
    assert_eq!(
        mesh(&PathGeometry::empty(), &style(2.0)).triangle_count(),
        0
    );
    let line = polyline(&[[0.0, 0.0], [10.0, 0.0]], false);
    assert_eq!(mesh(&line, &style(0.0)), FillMesh::empty());
}

#[test]
fn reversal_joins_are_bounded() {
    // (0,0)→(10,0)→(0,0): 180° 되돌아감. Miter는 bevel(면적 0)로, Round는 반원.
    let back = polyline(&[[0.0, 0.0], [10.0, 0.0], [0.0, 0.0]], false);
    assert_bounds(bounds(&back, &style(2.0)), [0.0, -1.0, 10.0, 1.0], EPS);
    assert_bounds(
        bounds(&back, &style(2.0).with_join(StrokeJoin::Round)),
        [0.0, -1.0, 11.0, 1.0],
        EPS,
    );
}

#[test]
fn style_validation_and_limits() {
    for bad in [-1.0, f32::NAN, f32::INFINITY] {
        assert!(matches!(
            StrokeStyle::new(bad),
            Err(TessellationError::InvalidStroke { .. })
        ));
    }
    for bad in [0.5, f32::NAN, f32::INFINITY] {
        assert!(MiterLimit::new(bad).is_err());
    }
    assert_eq!(MiterLimit::new(1.0).unwrap().get(), 1.0);
    for (intervals, offset) in [
        (&[][..], 0.0),
        (&[1.0, -1.0][..], 0.0),
        (&[1.0, f32::NAN][..], 0.0),
        (&[1.0, 1.0][..], f32::INFINITY),
        // 홀수 9개 → 반복 18개 > 16.
        (&[1.0; 9][..], 0.0),
        (&[1.0; 18][..], 0.0),
    ] {
        assert!(
            DashPattern::new(intervals, offset).is_err(),
            "{intervals:?} {offset}"
        );
    }
    assert!(DashPattern::new(&[1.0; 16], 0.0).is_ok());
    assert!(DashPattern::new(&[1.0; 8], 0.0).is_ok());
    for join in [StrokeJoin::Miter, StrokeJoin::Round, StrokeJoin::Bevel] {
        assert_eq!(StrokeJoin::from_tag(join.tag()), Some(join));
    }
    assert_eq!(StrokeJoin::from_tag(3), None);

    // 아주 짧은 dash로 긴 선: dash 수 상한에서 멈춥니다(무한 반복·거대 할당 없음).
    let long = polyline(&[[0.0, 0.0], [1.0e6, 0.0]], false);
    let tiny = DashPattern::new(&[0.001, 0.001], 0.0).unwrap();
    assert_eq!(
        tessellate_stroke(&long, &style(1.0).with_dash(Some(tiny))),
        Err(TessellationError::TooComplex { limit: MAX_DASHES })
    );
    // f32 범위를 넘는 정점은 오류입니다.
    let huge = polyline(&[[f32::MAX, 0.0], [f32::MAX, 10.0]], false);
    assert!(matches!(
        tessellate_stroke(&huge, &style(f32::MAX)),
        Err(TessellationError::Failed { .. })
    ));
}

#[test]
fn from_layer_maps_alignment_and_cap() {
    use crate::test_scene::{nodes, opaque, rect, snapshot};
    use stellar_scene::stack::PaintLayer;
    // renderer-core는 core-types `Color`를 이름으로 만들 수 없어 scene fixture에서 paint를 얻습니다.
    let scene = snapshot(&[rect(1, [0.0, 0.0, 4.0, 4.0], opaque(1.0, 0.0, 0.0))
        .with_stroke(opaque(0.0, 0.0, 1.0), 3.0)]);
    let node = nodes(&scene).into_iter().next().unwrap();
    let paint = node
        .paint()
        .get()
        .layers
        .iter()
        .find_map(|layer| match layer {
            PaintLayer::Stroke(stroke) => Some(stroke.paint()),
            PaintLayer::Fill(_) => None,
        })
        .unwrap();
    let style_of = |align, cap| {
        StrokeStyle::from_layer(&StrokeLayer::new(paint, 3.0, align, cap).unwrap()).unwrap()
    };
    let center = style_of(StrokeAlign::Center, StrokeCap::Round);
    assert_eq!(center.width(), 3.0);
    assert_eq!(center.cap(), StrokeCap::Round);
    assert_eq!(center.join(), StrokeJoin::Miter);
    assert_eq!(center.dash(), None);
    // Inside/Outside는 폭을 두 배로 한 중심 stroke(clip은 호출자).
    assert_eq!(style_of(StrokeAlign::Inside, StrokeCap::Butt).width(), 6.0);
    assert_eq!(style_of(StrokeAlign::Outside, StrokeCap::Butt).width(), 6.0);
}

#[test]
fn stroke_cache_hits_and_key_separation() {
    let tessellator = ReferenceStrokeTessellator;
    let mut cache = TessellationCache::new();
    let line = polyline(&[[0.0, 0.0], [10.0, 0.0], [10.0, 5.0]], false);
    let base = style(2.0);
    let first = cache.stroke(&line, &base, &tessellator).unwrap();
    assert_eq!(first.outcome, CacheOutcome::Miss);
    let again = cache
        .stroke(
            &polyline(&[[0.0, 0.0], [10.0, 0.0], [10.0, 5.0]], false),
            &base,
            &tessellator,
        )
        .unwrap();
    assert_eq!(again.outcome, CacheOutcome::Hit);
    assert!(Arc::ptr_eq(&first.mesh, &again.mesh));

    // fill과 stroke는 같은 path라도 다른 key.
    assert_ne!(
        TessellationKey::fill(&line, FillStyle::default()),
        TessellationKey::stroke(&line, &base)
    );
    // mesh를 바꾸는 값은 각각 miss.
    let dashed = DashPattern::new(&[2.0, 1.0], 0.0).unwrap();
    let variants = [
        style(3.0),
        base.with_join(StrokeJoin::Round),
        base.with_miter_limit(MiterLimit::new(1.2).unwrap()),
        base.with_cap(StrokeCap::Square),
        base.with_dash(Some(dashed)),
        base.with_dash(Some(DashPattern::new(&[2.0, 1.0], 0.5).unwrap())),
        base.with_tolerance(FlattenTolerance::new(0.1).unwrap()),
    ];
    for (index, variant) in variants.iter().enumerate() {
        assert_eq!(
            cache.stroke(&line, variant, &tessellator).unwrap().outcome,
            CacheOutcome::Miss,
            "variant {index}"
        );
    }
    // mesh가 같은 style은 정규화되어 같은 key: Round join의 miter limit, 합 0 dash, 폭 0의 나머지.
    let round = base.with_join(StrokeJoin::Round);
    assert_eq!(
        TessellationKey::stroke(&line, &round),
        TessellationKey::stroke(
            &line,
            &round.with_miter_limit(MiterLimit::new(9.0).unwrap())
        )
    );
    assert_eq!(
        TessellationKey::stroke(&line, &base),
        TessellationKey::stroke(
            &line,
            &base.with_dash(Some(DashPattern::new(&[0.0, 0.0], 3.0).unwrap()))
        )
    );
    assert_eq!(
        TessellationKey::stroke(&line, &style(0.0)),
        TessellationKey::stroke(
            &line,
            &style(0.0)
                .with_join(StrokeJoin::Bevel)
                .with_cap(StrokeCap::Round)
        )
    );
    assert_eq!(
        TessellationKey::stroke(&line, &style(-0.0)),
        TessellationKey::stroke(&line, &style(0.0))
    );
}

/// 결정적 xorshift64*.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn coord(&mut self) -> f32 {
        // 1/8 격자에 맞춰 중복·일직선 점이 자주 나오게 합니다.
        ((self.unit() * 160.0).floor() / 8.0) - 10.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn random_paths_stay_within_outset_bounds_and_are_deterministic() {
    let mut rng = Rng(0x5eed_6202);
    let caps = [StrokeCap::Butt, StrokeCap::Square, StrokeCap::Round];
    let joins = [StrokeJoin::Miter, StrokeJoin::Round, StrokeJoin::Bevel];
    for case in 0..300 {
        let mut commands = vec![PathCommand::MoveTo(v(rng.coord(), rng.coord()))];
        for _ in 0..rng.below(8) {
            commands.push(match rng.below(4) {
                0 => PathCommand::CubicTo {
                    c1: v(rng.coord(), rng.coord()),
                    c2: v(rng.coord(), rng.coord()),
                    to: v(rng.coord(), rng.coord()),
                },
                1 => {
                    let last = match commands.last() {
                        Some(PathCommand::MoveTo(p)) | Some(PathCommand::LineTo(p)) => *p,
                        Some(PathCommand::CubicTo { to, .. }) => *to,
                        _ => v(0.0, 0.0),
                    };
                    PathCommand::LineTo(last)
                }
                _ => PathCommand::LineTo(v(rng.coord(), rng.coord())),
            });
        }
        if rng.below(2) == 0 {
            commands.push(PathCommand::Close);
            commands.push(PathCommand::MoveTo(v(rng.coord(), rng.coord())));
            commands.push(PathCommand::LineTo(v(rng.coord(), rng.coord())));
        }
        let path = PathGeometry::new(commands).unwrap();
        let mut stroke = style(0.25 + rng.unit() * 6.0)
            .with_cap(caps[rng.below(3) as usize])
            .with_join(joins[rng.below(3) as usize])
            .with_miter_limit(MiterLimit::new(1.0 + rng.unit() * 8.0).unwrap());
        if rng.below(3) == 0 {
            let intervals = [rng.unit() * 4.0, rng.unit() * 4.0, rng.unit() * 2.0];
            stroke = stroke.with_dash(Some(
                DashPattern::new(&intervals[..1 + rng.below(3) as usize], rng.coord()).unwrap(),
            ));
        }
        let first = tessellate_stroke(&path, &stroke).unwrap();
        // 결정론: 같은 입력 → 같은 mesh.
        assert_eq!(
            first,
            tessellate_stroke(&path, &stroke).unwrap(),
            "case {case}"
        );
        // 모든 정점은 control bounds를 outset만큼 넓힌 영역 안입니다.
        if let (Some(control), Some(mesh_bounds)) = (path_control_bounds(&path), first.bounds()) {
            let allowed = control.expanded(stroke.outset() + 1e-3);
            assert!(
                allowed.contains_point(v(mesh_bounds.min_x(), mesh_bounds.min_y()))
                    && allowed.contains_point(v(mesh_bounds.max_x(), mesh_bounds.max_y())),
                "case {case}: mesh {mesh_bounds:?} ⊄ {allowed:?}"
            );
        }
    }
}

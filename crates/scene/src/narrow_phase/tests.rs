//! narrow-phase hit test 테스트 (XI.3.4).

use std::collections::HashMap;
use std::sync::Arc;

use super::*;
use crate::builder::{FrameClipInput, FrameInput, NodeInput, SceneBuilder};
use crate::fixtures::{self, id, p, rect, solid, translate, FixtureConfig, SplitMix64};
use crate::hit_test::hit_test_broad;
use crate::node::{
    HitTestMeta, ImageCrop, ImageFit, ImagePrimitive, ImageSampling, PathPrimitive, RectPrimitive,
    SceneNodeSpec, ZOrder,
};
use crate::snapshot::SceneSnapshot;
use crate::stack::{PaintStack, PaintStackRef};
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::revision::{DocumentRevision, SceneRevision};
use stellar_core_types::value::Size2;

fn fill() -> PaintLayer {
    PaintLayer::Fill(solid(0.2, 0.4, 0.6, 1.0))
}

fn stroke(width: f32, align: StrokeAlign, cap: StrokeCap) -> PaintLayer {
    PaintLayer::Stroke(StrokeLayer::new(solid(0.0, 0.0, 0.0, 1.0), width, align, cap).unwrap())
}

fn node(
    source: u64,
    z: u64,
    content: SceneContent,
    transform: Transform2D,
    layers: Vec<PaintLayer>,
) -> Arc<SceneNode> {
    let mut spec = fixtures::spec(source, content, z);
    spec.world_transform = transform;
    spec.paint = PaintStackRef::new(PaintStack {
        layers,
        ..PaintStack::empty()
    });
    Arc::new(SceneNode::leaf(spec))
}

fn index_of(nodes: Vec<Arc<SceneNode>>) -> SceneSpatialIndex {
    SceneSpatialIndex::build(
        &SceneSnapshot::new(DocumentRevision(1), SceneRevision(1), nodes).unwrap(),
    )
}

fn narrow(index: &SceneSpatialIndex, x: f32, y: f32) -> HitResult {
    hit_test(index, p(x, y), &NarrowPhaseOptions::EXACT, &LayoutBoxesOnly)
}

fn sources(result: &HitResult) -> Vec<StableId> {
    result.hits.iter().map(|hit| hit.source).collect()
}

fn path(commands: Vec<PathCommand>, fill_rule: FillRule) -> SceneContent {
    SceneContent::Path(PathPrimitive {
        geometry: PathGeometry::new(commands).unwrap(),
        fill_rule,
    })
}

/// document Ellipse 평가와 같은 형태(4개 cubic, κ)의 `w×h` 타원.
fn ellipse(w: f32, h: f32) -> Vec<PathCommand> {
    let k = 0.552_284_8f32;
    let (rx, ry) = (w / 2.0, h / 2.0);
    vec![
        PathCommand::MoveTo(p(w, ry)),
        PathCommand::CubicTo {
            c1: p(w, ry + k * ry),
            c2: p(rx + k * rx, h),
            to: p(rx, h),
        },
        PathCommand::CubicTo {
            c1: p(rx - k * rx, h),
            c2: p(0.0, ry + k * ry),
            to: p(0.0, ry),
        },
        PathCommand::CubicTo {
            c1: p(0.0, ry - k * ry),
            c2: p(rx - k * rx, 0.0),
            to: p(rx, 0.0),
        },
        PathCommand::CubicTo {
            c1: p(rx + k * rx, 0.0),
            c2: p(w, ry - k * ry),
            to: p(w, ry),
        },
        PathCommand::Close,
    ]
}

/// `(x0, y0)`~`(x1, y1)` 사각형 subpath(`clockwise`면 화면 기준 시계 방향).
fn square(x0: f32, y0: f32, x1: f32, y1: f32, clockwise: bool) -> Vec<PathCommand> {
    let corners = if clockwise {
        [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    } else {
        [(x0, y0), (x0, y1), (x1, y1), (x1, y0)]
    };
    let mut commands = vec![PathCommand::MoveTo(p(corners[0].0, corners[0].1))];
    commands.extend(
        corners[1..]
            .iter()
            .map(|&(x, y)| PathCommand::LineTo(p(x, y))),
    );
    commands.push(PathCommand::Close);
    commands
}

fn rect_frame(w: f32, h: f32, radius: f32) -> SceneContent {
    SceneContent::Rect(RectPrimitive {
        rect: rect(0.0, 0.0, w, h),
        radii: CornerRadii::new(radius, radius, radius, radius).unwrap(),
    })
}

#[test]
fn shape_ellipse_hits_curve_interior_not_bounding_box_corner() {
    // 검증 기준(XI.3.4): Shape — Ellipse 평가 path는 곡선 안만 맞고 bounds 모서리는 통과합니다.
    let index = index_of(vec![node(
        1,
        1,
        path(ellipse(100.0, 60.0), FillRule::NonZero),
        translate(10.0, 20.0),
        vec![fill()],
    )]);
    assert_eq!(sources(&narrow(&index, 60.0, 50.0)), vec![id(1)]);
    // bounds 안이지만 곡선 밖: broad-phase는 후보, narrow-phase는 제외.
    assert_eq!(hit_test_broad(&index, p(13.0, 23.0)).hits.len(), 1);
    let corner = narrow(&index, 13.0, 23.0);
    assert!(corner.hits.is_empty());
    assert_eq!(corner.stats.content_missed, 1);

    // 독립 oracle: 해석적 타원 식과 격자 비교(경계 0.1 이내는 선분화 오차로 제외).
    let mut compared = 0usize;
    for ix in 0..=120 {
        for iy in 0..=80 {
            let (x, y) = (ix as f32, iy as f32 + 10.0);
            let (u, v) = ((x - 60.0) / 50.0, (y - 50.0) / 30.0);
            let radial = (u * u + v * v).sqrt();
            // 반지름 방향 거리의 보수적 근사(짧은 축 30 기준).
            if ((radial - 1.0) * 30.0).abs() < 0.1 {
                continue;
            }
            assert_eq!(
                !narrow(&index, x, y).hits.is_empty(),
                radial < 1.0,
                "({x}, {y})"
            );
            compared += 1;
        }
    }
    assert!(compared > 9_000, "비교한 점 {compared}");
}

#[test]
fn stroke_only_shape_hits_band_and_passes_interior_to_lower_node() {
    // 아래 Frame(z1) 위에 stroke만 있는 원(z2, 폭 4 Center).
    let index = index_of(vec![
        node(
            1,
            1,
            rect_frame(300.0, 300.0, 0.0),
            Transform2D::identity(),
            Vec::new(),
        ),
        node(
            2,
            2,
            path(ellipse(100.0, 100.0), FillRule::NonZero),
            translate(100.0, 100.0),
            vec![stroke(4.0, StrokeAlign::Center, StrokeCap::Butt)],
        ),
    ]);
    // 원 안쪽은 stroke 띠가 아니라 아래 Frame이 topmost입니다.
    let interior = narrow(&index, 150.0, 150.0);
    assert_eq!(sources(&interior), vec![id(1)]);
    assert_eq!(interior.stats.content_missed, 1);
    // 윤곽 위·반폭 이내는 원, 반폭 밖은 Frame.
    for x in [200.0, 201.9, 198.1] {
        assert_eq!(narrow(&index, x, 150.0).topmost().unwrap().source, id(2));
    }
    for x in [202.3, 197.7] {
        assert_eq!(narrow(&index, x, 150.0).topmost().unwrap().source, id(1));
    }
}

#[test]
fn open_path_caps_follow_stroke_cap() {
    // 가로선 (10,50)→(90,50), 폭 10(반폭 5).
    let line = || {
        path(
            vec![
                PathCommand::MoveTo(p(10.0, 50.0)),
                PathCommand::LineTo(p(90.0, 50.0)),
            ],
            FillRule::NonZero,
        )
    };
    let cases = [
        // (x, y, butt, round, square)
        (5.0, 50.0, false, true, true),
        (6.0, 46.0, false, false, true),
        (11.0, 46.0, true, true, true),
        (50.0, 54.5, true, true, true),
        (50.0, 55.5, false, false, false),
        (94.0, 50.0, false, true, true),
        (96.0, 50.0, false, false, false),
    ];
    for (cap, column) in [
        (StrokeCap::Butt, 0usize),
        (StrokeCap::Round, 1),
        (StrokeCap::Square, 2),
    ] {
        let index = index_of(vec![node(
            1,
            1,
            line(),
            Transform2D::identity(),
            vec![stroke(10.0, StrokeAlign::Center, cap)],
        )]);
        for (x, y, butt, round, square) in cases {
            let expected = [butt, round, square][column];
            assert_eq!(
                !narrow(&index, x, y).hits.is_empty(),
                expected,
                "{cap:?} ({x}, {y})"
            );
        }
    }
    // 닫힌 subpath는 끝 모양이 없고 join은 round입니다: 사각형 꼭짓점 바깥 대각 4.9는 맞고 5.1은 아님.
    let index = index_of(vec![node(
        1,
        1,
        path(square(20.0, 20.0, 80.0, 80.0, true), FillRule::NonZero),
        Transform2D::identity(),
        vec![stroke(10.0, StrokeAlign::Center, StrokeCap::Butt)],
    )]);
    let d = |r: f32| r / std::f32::consts::SQRT_2;
    assert!(!narrow(&index, 80.0 + d(4.9), 80.0 + d(4.9)).hits.is_empty());
    assert!(narrow(&index, 80.0 + d(5.1), 80.0 + d(5.1)).hits.is_empty());
}

#[test]
fn path_fill_rules_and_stroke_alignment() {
    // 검증 기준(XI.3.4): Path — fill rule과 stroke 정렬별 띠.
    let mut nested = square(0.0, 0.0, 100.0, 100.0, true);
    nested.extend(square(25.0, 25.0, 75.0, 75.0, true));
    let mut reversed = square(0.0, 0.0, 100.0, 100.0, true);
    reversed.extend(square(25.0, 25.0, 75.0, 75.0, false));
    for (commands, fill_rule, hole_hit) in [
        (nested.clone(), FillRule::EvenOdd, false),
        (nested, FillRule::NonZero, true),
        (reversed, FillRule::NonZero, false),
    ] {
        let index = index_of(vec![node(
            1,
            1,
            path(commands, fill_rule),
            Transform2D::identity(),
            vec![fill()],
        )]);
        assert_eq!(
            !narrow(&index, 50.0, 50.0).hits.is_empty(),
            hole_hit,
            "{fill_rule:?}"
        );
        assert!(!narrow(&index, 10.0, 10.0).hits.is_empty());
    }

    // 사각형 20~80, stroke 폭 6, fill 없음.
    let cases = [
        // (x, center, inside, outside)
        (21.0, true, true, false),
        (24.0, false, true, false),
        (27.0, false, false, false),
        (19.0, true, false, true),
        (17.5, true, false, true),
        (16.5, false, false, true),
        (15.0, false, false, true),
        (13.0, false, false, false),
    ];
    for (align, column) in [
        (StrokeAlign::Center, 0usize),
        (StrokeAlign::Inside, 1),
        (StrokeAlign::Outside, 2),
    ] {
        let index = index_of(vec![node(
            1,
            1,
            path(square(20.0, 20.0, 80.0, 80.0, true), FillRule::NonZero),
            Transform2D::identity(),
            vec![stroke(6.0, align, StrokeCap::Butt)],
        )]);
        for (x, center, inside, outside) in cases {
            let expected = [center, inside, outside][column];
            assert_eq!(
                !narrow(&index, x, 50.0).hits.is_empty(),
                expected,
                "{align:?} x={x}"
            );
        }
        // fill이 없으므로 중앙은 통과합니다.
        assert!(narrow(&index, 50.0, 50.0).hits.is_empty());
    }

    // paint layer가 없는 path는 fill 영역으로 판정합니다(선택 불가 node 방지).
    let index = index_of(vec![node(
        1,
        1,
        path(square(20.0, 20.0, 80.0, 80.0, true), FillRule::NonZero),
        Transform2D::identity(),
        Vec::new(),
    )]);
    assert!(!narrow(&index, 50.0, 50.0).hits.is_empty());
    assert!(narrow(&index, 10.0, 50.0).hits.is_empty());
}

#[test]
fn frame_uses_rounded_box_and_outset_strokes() {
    // 검증 기준(XI.3.4): Frame(Rect) — 칠 없는 상자 전체, 둥근 모서리 밖 제외.
    let index = index_of(vec![node(
        1,
        1,
        rect_frame(100.0, 100.0, 20.0),
        Transform2D::identity(),
        Vec::new(),
    )]);
    assert!(!narrow(&index, 50.0, 50.0).hits.is_empty());
    let corner = narrow(&index, 2.0, 2.0);
    assert!(corner.hits.is_empty());
    assert_eq!(corner.stats.content_missed, 1);
    // Inside stroke(Frame 테두리)는 상자를 넓히지 않고, Outside는 폭만큼 넓힙니다.
    for (align, x, expected) in [
        (StrokeAlign::Inside, -1.0, false),
        (StrokeAlign::Outside, -3.5, true),
        (StrokeAlign::Outside, -4.5, false),
        (StrokeAlign::Center, -1.5, true),
        (StrokeAlign::Center, -2.5, false),
    ] {
        let index = index_of(vec![node(
            1,
            1,
            rect_frame(100.0, 100.0, 20.0),
            Transform2D::identity(),
            vec![stroke(4.0, align, StrokeCap::Butt)],
        )]);
        assert_eq!(
            !narrow(&index, x, 50.0).hits.is_empty(),
            expected,
            "{align:?} x={x}"
        );
    }
    // 둥근 모서리 거리: 중심 (20,20) 반지름 20 호에서 대각 바깥 2.
    let bounds = Aabb::new(0.0, 0.0, 100.0, 100.0).unwrap();
    let radii = CornerRadii::new(20.0, 20.0, 20.0, 20.0).unwrap();
    let off = 20.0 - 22.0 / std::f32::consts::SQRT_2;
    assert!((rounded_rect_distance(&bounds, &radii, p(off, off)) - 2.0).abs() < 1e-4);
    assert_eq!(rounded_rect_distance(&bounds, &radii, p(50.0, 50.0)), 0.0);
    assert!((rounded_rect_distance(&bounds, &radii, p(50.0, -3.0)) - 3.0).abs() < 1e-6);
}

/// layout key별 line rect 표.
struct LineRects(HashMap<TextLayoutKey, Vec<Rect>>);

impl TextHitGeometry for LineRects {
    fn line_rects(&self, layout: TextLayoutKey) -> Option<&[Rect]> {
        self.0.get(&layout).map(Vec::as_slice)
    }
}

#[test]
fn text_uses_line_rects_or_layout_box() {
    // 검증 기준(XI.3.4): Text — bounds 정책(line rect가 있으면 그 합집합).
    let key = TextLayoutKey::from_bytes([3; 16]);
    let empty_key = TextLayoutKey::from_bytes([4; 16]);
    let text = |layout| {
        SceneContent::Text(TextPrimitive {
            layout,
            bounds: rect(0.0, 0.0, 200.0, 40.0),
        })
    };
    let index = index_of(vec![
        node(1, 1, text(key), Transform2D::identity(), Vec::new()),
        node(2, 2, text(empty_key), translate(0.0, 100.0), Vec::new()),
    ]);
    // 둘째 줄이 짧은 text: line rect (0,0,200,20), (0,20,80,20).
    let lines = LineRects(HashMap::from([
        (
            key,
            vec![rect(0.0, 0.0, 200.0, 20.0), rect(0.0, 20.0, 80.0, 20.0)],
        ),
        (empty_key, Vec::new()),
    ]));
    let hit = |x: f32, y: f32, geometry: &dyn TextHitGeometry| {
        !hit_test(&index, p(x, y), &NarrowPhaseOptions::EXACT, geometry)
            .hits
            .is_empty()
    };
    // layout 상자 정책: 상자 안이면 줄 밖이어도 맞습니다.
    assert!(hit(150.0, 30.0, &LayoutBoxesOnly));
    // line rect 정책: 짧은 둘째 줄 오른쪽은 통과, 첫 줄·둘째 줄 안은 맞음.
    assert!(!hit(150.0, 30.0, &lines));
    assert!(hit(150.0, 10.0, &lines));
    assert!(hit(50.0, 30.0, &lines));
    // 빈 line rect(빈 text)는 layout 상자로 되돌아가 선택할 수 있습니다.
    assert!(hit(150.0, 130.0, &lines));
    assert!(!hit(150.0, 141.0, &lines));
}

#[test]
fn image_uses_placement_box_without_alpha() {
    // 검증 기준(XI.3.4): Image — 배치 상자 정책(fit·투명 픽셀 무관).
    for fit in [ImageFit::Contain, ImageFit::Cover, ImageFit::None] {
        let content = SceneContent::Image(ImagePrimitive {
            asset: AssetRef::new(id(9)),
            bounds: rect(0.0, 0.0, 100.0, 50.0),
            fit,
            crop: ImageCrop::FULL,
            sampling: ImageSampling::Linear,
            color_space: None,
        });
        let index = index_of(vec![node(
            1,
            1,
            content,
            translate(10.0, 10.0),
            vec![stroke(6.0, StrokeAlign::Inside, StrokeCap::Butt)],
        )]);
        assert!(!narrow(&index, 11.0, 11.0).hits.is_empty(), "{fit:?}");
        assert!(!narrow(&index, 110.0, 60.0).hits.is_empty(), "{fit:?}");
        assert!(narrow(&index, 111.0, 30.0).hits.is_empty(), "{fit:?}");
    }
}

#[test]
fn path_clip_is_tested_precisely() {
    // Frame A(z1, 0~100)가 삼각형 path clip을 자식 scope에 걸고, 자식 B(z2)가 A 전체를 덮습니다.
    let mut builder = SceneBuilder::new();
    let mut root = builder.root_scope();
    let triangle = PathGeometry::new(vec![
        PathCommand::MoveTo(p(0.0, 0.0)),
        PathCommand::LineTo(p(100.0, 0.0)),
        PathCommand::LineTo(p(0.0, 100.0)),
        PathCommand::Close,
    ])
    .unwrap();
    let frame_input = |source: u64, clip: FrameClipInput| FrameInput {
        node: NodeInput::new(id(source), Transform2D::identity()),
        size: Size2::new(100.0, 100.0).unwrap(),
        corner_radii: CornerRadii::ZERO,
        background: None,
        border: None,
        clip,
    };
    let mut a = builder
        .begin_frame(
            &root,
            &frame_input(
                1,
                FrameClipInput::Path {
                    geometry: triangle,
                    fill_rule: FillRule::NonZero,
                },
            ),
        )
        .unwrap();
    let b = builder
        .begin_frame(a.scope(), &frame_input(2, FrameClipInput::None))
        .unwrap();
    builder.finish_frame(a.scope_mut(), b).unwrap();
    builder.finish_frame(&mut root, a).unwrap();
    let index = SceneSpatialIndex::build(
        &SceneSnapshot::new(DocumentRevision(1), SceneRevision(1), builder.finish(root)).unwrap(),
    );
    // (80,80)은 삼각형 밖이지만 control 껍질 안: broad-phase는 B를 남기고 narrow-phase가 뺍니다.
    assert_eq!(
        hit_test_broad(&index, p(80.0, 80.0)).z_orders(),
        vec![ZOrder(2), ZOrder(1)]
    );
    let result = narrow(&index, 80.0, 80.0);
    assert_eq!(sources(&result), vec![id(1)]);
    assert_eq!(result.stats.path_clipped, 1);
    // 삼각형 안 (20,20)은 B가 topmost.
    assert_eq!(narrow(&index, 20.0, 20.0).topmost().unwrap().source, id(2));
    // 정밀 판정 함수 자체.
    let clip = ClipShape::Path {
        geometry: PathGeometry::new(square(0.0, 0.0, 10.0, 10.0, true)).unwrap(),
        fill_rule: FillRule::EvenOdd,
        transform: translate(100.0, 0.0),
    };
    assert!(clip_contains_precise(&clip, p(105.0, 5.0)));
    assert!(!clip_contains_precise(&clip, p(95.0, 5.0)));
}

#[test]
fn tolerance_widens_hairlines_in_local_and_view_units() {
    // 칠 없는 열린 선(면적 0)은 허용 거리 없이는 맞지 않습니다. node는 2배 확대.
    let scaled = Transform2D::new(2.0, 0.0, 0.0, 2.0, 0.0, 0.0).unwrap();
    let index = index_of(vec![node(
        1,
        1,
        path(
            vec![
                PathCommand::MoveTo(p(0.0, 50.0)),
                PathCommand::LineTo(p(100.0, 50.0)),
            ],
            FillRule::NonZero,
        ),
        scaled,
        Vec::new(),
    )]);
    // world 선은 y = 100, x 0~200.
    assert!(narrow(&index, 100.0, 100.0).hits.is_empty());
    let options = NarrowPhaseOptions::with_tolerance(2.0).unwrap();
    let within = |x: f32, y: f32| {
        !hit_test(&index, p(x, y), &options, &LayoutBoxesOnly)
            .hits
            .is_empty()
    };
    assert!(within(100.0, 101.9));
    assert!(within(100.0, 98.1));
    assert!(!within(100.0, 102.1));
    // view 4배 확대: view 허용 8 = world 2.
    let view = Transform2D::new(4.0, 0.0, 0.0, 4.0, 0.0, 0.0).unwrap();
    let viewport = Viewport::new(Size2::new(1000.0, 1000.0).unwrap(), view).unwrap();
    let view_options = NarrowPhaseOptions::with_tolerance(8.0).unwrap();
    let view_hit = |y: f32| {
        !hit_test_view_narrow(
            &index,
            &viewport,
            p(400.0, y),
            &view_options,
            &LayoutBoxesOnly,
        )
        .hits
        .is_empty()
    };
    assert!(view_hit(407.6));
    assert!(!view_hit(408.4));
    assert!(NarrowPhaseOptions::with_tolerance(-1.0).is_err());
    assert!(NarrowPhaseOptions::with_tolerance(f32::NAN).is_err());
    assert_eq!(NarrowPhaseOptions::default(), NarrowPhaseOptions::EXACT);
}

#[test]
fn narrow_hits_are_ordered_subset_of_broad_on_random_fixture() {
    let snapshot = fixtures::generate(FixtureConfig {
        seed: 0x35,
        roots: 5,
        children: 5,
        depth: 3,
    });
    let index = SceneSpatialIndex::build(&snapshot);
    let mut rng = SplitMix64::new(0x35);
    let (mut rejected, mut kept) = (0usize, 0usize);
    for _ in 0..400 {
        let point = p(rng.range(-300.0, 1_500.0), rng.range(-300.0, 1_500.0));
        let broad = hit_test_broad(&index, point);
        let fine = hit_test(&index, point, &NarrowPhaseOptions::EXACT, &LayoutBoxesOnly);
        let broad_z = broad.z_orders();
        let fine_z = fine.z_orders();
        // 순서를 유지한 부분 집합이고, 빠진 수는 통계와 같습니다.
        let mut rest = broad_z.iter();
        assert!(fine_z.iter().all(|z| rest.any(|b| b == z)), "{point:?}");
        assert_eq!(
            fine_z.len() + fine.stats.content_missed + fine.stats.path_clipped,
            broad_z.len()
        );
        // 남은 후보는 모두 content 안입니다.
        for hit in &fine.hits {
            assert!(content_contains(
                &hit.node,
                hit.local_point,
                &NarrowPhaseOptions::EXACT,
                &LayoutBoxesOnly
            ));
        }
        rejected += broad_z.len() - fine_z.len();
        kept += fine_z.len();
    }
    assert!(kept > 0 && rejected > 0, "kept {kept}, rejected {rejected}");
}

#[test]
fn huge_or_degenerate_geometry_stays_bounded() {
    // 신뢰할 수 없는 큰 좌표의 cubic도 선분 수 상한을 지킵니다.
    let geometry = PathGeometry::new(vec![
        PathCommand::MoveTo(p(-1e30, -1e30)),
        PathCommand::CubicTo {
            c1: p(1e30, -1e30),
            c2: p(-1e30, 1e30),
            to: p(1e30, 1e30),
        },
    ])
    .unwrap();
    let lines = flatten(&geometry, f64::from(FLATTEN_TOLERANCE));
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].points.len(), MAX_CUBIC_SEGMENTS + 1);
    // 허용 오차 0·비유한도 상한 안에서 끝납니다.
    for tolerance in [0.0, f64::NAN, f64::INFINITY] {
        let lines = flatten(&geometry, tolerance);
        assert!(lines[0].points.len() <= MAX_CUBIC_SEGMENTS + 1);
    }
    // 축척 0 node는 면적이 없어 맞지 않습니다.
    let flat = Transform2D::new(0.0, 0.0, 0.0, 0.0, 0.0, 0.0).unwrap();
    let degenerate = SceneNode::leaf(SceneNodeSpec {
        world_transform: flat,
        hit: HitTestMeta::INTERACTIVE,
        ..fixtures::spec(1, rect_frame(10.0, 10.0, 0.0), 1)
    });
    assert!(!content_contains(
        &degenerate,
        p(1.0, 1.0),
        &NarrowPhaseOptions::EXACT,
        &LayoutBoxesOnly
    ));
    // MoveTo만 있는 subpath와 길이 0 선분은 Round cap일 때만 점으로 맞습니다.
    for (commands, cap, expected) in [
        (
            vec![PathCommand::MoveTo(p(5.0, 5.0))],
            StrokeCap::Round,
            true,
        ),
        (
            vec![PathCommand::MoveTo(p(5.0, 5.0))],
            StrokeCap::Butt,
            false,
        ),
        (
            vec![
                PathCommand::MoveTo(p(5.0, 5.0)),
                PathCommand::LineTo(p(5.0, 5.0)),
            ],
            StrokeCap::Round,
            true,
        ),
        (
            vec![
                PathCommand::MoveTo(p(5.0, 5.0)),
                PathCommand::LineTo(p(5.0, 5.0)),
            ],
            StrokeCap::Square,
            false,
        ),
    ] {
        let lines = flatten(&PathGeometry::new(commands).unwrap(), 0.05);
        assert_eq!(
            outline_within(&lines, Pt::of(p(6.0, 6.0)), 2.0, 0.0, cap),
            expected,
            "{cap:?}"
        );
    }
}

fn tolerant(index: &SceneSpatialIndex, x: f32, y: f32, tolerance: f32) -> bool {
    let options = NarrowPhaseOptions::with_tolerance(tolerance).unwrap();
    !hit_test(index, p(x, y), &options, &LayoutBoxesOnly)
        .hits
        .is_empty()
}

#[test]
fn zero_width_stroke_only_path_has_no_hit_area() {
    // 리뷰 지적: 폭 0 stroke만 있는 path는 paint layer가 있으므로 내부 정책이 아니고, 폭 0 stroke는 띠가
    // 없어 어디도 맞지 않습니다(허용 거리를 줘도 빈 영역은 넓어지지 않음).
    let zero = || stroke(0.0, StrokeAlign::Center, StrokeCap::Round);
    let index = index_of(vec![node(
        1,
        1,
        path(square(20.0, 20.0, 80.0, 80.0, true), FillRule::NonZero),
        Transform2D::identity(),
        vec![zero()],
    )]);
    for (x, y) in [(50.0, 50.0), (20.0, 50.0), (21.0, 50.0)] {
        let result = narrow(&index, x, y);
        assert!(result.hits.is_empty(), "({x}, {y})");
        assert_eq!(result.stats.content_missed, 1, "({x}, {y})");
        assert!(!tolerant(&index, x, y, 2.0), "허용 거리 ({x}, {y})");
    }
    // fill이 함께 있으면 fill 영역은 그대로 맞습니다.
    let index = index_of(vec![node(
        1,
        1,
        path(square(20.0, 20.0, 80.0, 80.0, true), FillRule::NonZero),
        Transform2D::identity(),
        vec![zero(), fill()],
    )]);
    assert!(!narrow(&index, 50.0, 50.0).hits.is_empty());
    assert!(narrow(&index, 10.0, 50.0).hits.is_empty());
}

#[test]
fn repeated_end_points_do_not_change_caps() {
    // 리뷰 지적: 열린 subpath가 같은 점으로 시작·끝나도(길이 0 선분) cap은 첫·마지막 길이 있는 선분이
    // 받습니다. 가로선 (10,50)→(90,50), 폭 10, open_path_caps_follow_stroke_cap과 같은 기대값.
    let (a, b) = (p(10.0, 50.0), p(90.0, 50.0));
    let variants = [
        vec![a, a, b],
        vec![a, b, b],
        vec![a, a, a, b, b],
        vec![a, p(50.0, 50.0), p(50.0, 50.0), b],
    ];
    let cases = [
        // (x, y, butt, round, square)
        (5.0, 50.0, false, true, true),
        (6.0, 50.0, false, true, true),
        (6.0, 46.0, false, false, true),
        (11.0, 46.0, true, true, true),
        (50.0, 54.5, true, true, true),
        (94.0, 50.0, false, true, true),
        (94.0, 54.0, false, false, true),
        (96.0, 50.0, false, false, false),
    ];
    for points in variants {
        let mut commands = vec![PathCommand::MoveTo(points[0])];
        commands.extend(points[1..].iter().map(|&point| PathCommand::LineTo(point)));
        for (cap, column) in [
            (StrokeCap::Butt, 0usize),
            (StrokeCap::Round, 1),
            (StrokeCap::Square, 2),
        ] {
            let index = index_of(vec![node(
                1,
                1,
                path(commands.clone(), FillRule::NonZero),
                Transform2D::identity(),
                vec![stroke(10.0, StrokeAlign::Center, cap)],
            )]);
            for (x, y, butt, round, square) in cases {
                let expected = [butt, round, square][column];
                assert_eq!(
                    !narrow(&index, x, y).hits.is_empty(),
                    expected,
                    "{points:?} {cap:?} ({x}, {y})"
                );
            }
        }
    }
}

#[test]
fn tolerance_widens_aligned_stroke_bands_on_both_sides() {
    // 리뷰 지적: 사각형 20~80, fill 없는 stroke 폭 6, 허용 거리 2. Inside 띠는 바깥으로, Outside 띠는
    // 안쪽으로도 허용 거리만큼 넓어집니다.
    let cases = [
        // (x, center, inside, outside)
        (28.5, false, false, false),
        (27.5, false, true, false),
        (25.5, false, true, false),
        (24.5, true, true, false),
        (22.5, true, true, false),
        (21.0, true, true, true),
        (19.0, true, true, true),
        (17.5, true, false, true),
        (16.5, true, false, true),
        (14.5, false, false, true),
        (12.5, false, false, true),
        (11.5, false, false, false),
    ];
    for (align, column) in [
        (StrokeAlign::Center, 0usize),
        (StrokeAlign::Inside, 1),
        (StrokeAlign::Outside, 2),
    ] {
        let index = index_of(vec![node(
            1,
            1,
            path(square(20.0, 20.0, 80.0, 80.0, true), FillRule::NonZero),
            Transform2D::identity(),
            vec![stroke(6.0, align, StrokeCap::Butt)],
        )]);
        for (x, center, inside, outside) in cases {
            let expected = [center, inside, outside][column];
            assert_eq!(tolerant(&index, x, 50.0, 2.0), expected, "{align:?} x={x}");
        }
        // fill이 없으므로 중앙은 허용 거리가 있어도 통과합니다.
        assert!(!tolerant(&index, 50.0, 50.0, 2.0), "{align:?}");
    }
}

#[test]
fn tolerance_follows_cap_shape_and_implicit_fill_edge() {
    // 가로선 (10,50)→(90,50), 폭 10(반폭 5), 허용 거리 2.
    let line = |cap| {
        index_of(vec![node(
            1,
            1,
            path(
                vec![
                    PathCommand::MoveTo(p(10.0, 50.0)),
                    PathCommand::LineTo(p(90.0, 50.0)),
                ],
                FillRule::NonZero,
            ),
            Transform2D::identity(),
            vec![stroke(10.0, StrokeAlign::Center, cap)],
        )])
    };
    // Butt: 끝점 너머로도 허용 거리만큼(모서리는 둥글게) 넓어집니다.
    let butt = line(StrokeCap::Butt);
    for (x, y, expected) in [
        (9.0, 50.0, true),
        (8.5, 50.0, true),
        (7.5, 50.0, false),
        (9.0, 55.5, true),
        (8.5, 56.5, false),
        (91.5, 50.0, true),
        (92.5, 50.0, false),
    ] {
        assert_eq!(tolerant(&butt, x, y, 2.0), expected, "Butt ({x}, {y})");
    }
    // Square: 연장 사각형을 둥글게 넓힙니다(모서리 대각 바깥 √2·1.5 > 2는 제외).
    let square_cap = line(StrokeCap::Square);
    for (x, y, expected) in [
        (3.5, 50.0, true),
        (2.5, 50.0, false),
        (4.0, 56.0, true),
        (3.5, 56.5, false),
    ] {
        assert_eq!(
            tolerant(&square_cap, x, y, 2.0),
            expected,
            "Square ({x}, {y})"
        );
    }
    // 열린 path의 fill은 암묵적으로 닫는 변((100,100)→(0,0))도 허용 거리만큼 넓어집니다.
    let index = index_of(vec![node(
        1,
        1,
        path(
            vec![
                PathCommand::MoveTo(p(0.0, 0.0)),
                PathCommand::LineTo(p(100.0, 0.0)),
                PathCommand::LineTo(p(100.0, 100.0)),
            ],
            FillRule::NonZero,
        ),
        Transform2D::identity(),
        vec![fill()],
    )]);
    assert!(narrow(&index, 40.0, 41.0).hits.is_empty());
    assert!(tolerant(&index, 40.0, 41.0, 1.0));
    assert!(!tolerant(&index, 40.0, 42.0, 1.0));
}

#[test]
fn outside_square_cap_corner_is_inside_broad_phase_bounds() {
    // 리뷰 지적: Outside stroke의 Square cap은 끝점 너머로 폭만큼 연장되어 대각 모서리가 폭·√2까지
    // 나갑니다. 대각선 (0,0)→(100,100), Outside 폭 10, fill 없음(면적 0이라 어디나 바깥).
    let index = index_of(vec![node(
        1,
        1,
        path(
            vec![
                PathCommand::MoveTo(p(0.0, 0.0)),
                PathCommand::LineTo(p(100.0, 100.0)),
            ],
            FillRule::NonZero,
        ),
        Transform2D::identity(),
        vec![stroke(10.0, StrokeAlign::Outside, StrokeCap::Square)],
    )]);
    // (0, -12.7)은 시작점에서 선 반대 방향·수직으로 각각 약 8.98: 연장 사각형 안이고 bounds 확장 10
    // 밖입니다.
    let node = index.node(ZOrder(1)).unwrap();
    assert!(content_contains(
        node,
        p(0.0, -12.7),
        &NarrowPhaseOptions::EXACT,
        &LayoutBoxesOnly
    ));
    assert_eq!(sources(&narrow(&index, 0.0, -12.7)), vec![id(1)]);
    // 연장 사각형 모서리 (0, -10√2) 바깥은 맞지 않습니다.
    assert!(narrow(&index, 0.0, -14.5).hits.is_empty());
}

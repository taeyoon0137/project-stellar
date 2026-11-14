//! transient hit-test layer 테스트 (XI.3.5).

use std::sync::Arc;

use super::*;
use crate::codec;
use crate::fixtures::{self, id, p, rect, translate};
use crate::narrow_phase::LayoutBoxesOnly;
use crate::node::{CornerRadii, RectPrimitive, SceneContent, SceneNode};
use crate::snapshot::SceneSnapshot;
use stellar_core_types::revision::{DocumentRevision, SceneRevision};
use stellar_core_types::value::{Size2, Transform2D};

/// A(z1): world (100,100)~(300,200) Frame.
fn scene() -> (SceneSnapshot, SceneSpatialIndex) {
    let mut spec = fixtures::spec(
        1,
        SceneContent::Rect(RectPrimitive {
            rect: rect(0.0, 0.0, 200.0, 100.0),
            radii: CornerRadii::ZERO,
        }),
        1,
    );
    spec.world_transform = translate(100.0, 100.0);
    let snapshot = SceneSnapshot::new(
        DocumentRevision(1),
        SceneRevision(1),
        vec![Arc::new(SceneNode::leaf(spec))],
    )
    .unwrap();
    let index = SceneSpatialIndex::build(&snapshot);
    (snapshot, index)
}

fn quad(x0: f32, y0: f32, x1: f32, y1: f32) -> [Vec2; 4] {
    [p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1)]
}

fn metrics() -> HandleMetrics {
    HandleMetrics::new(4.0, 3.0, 16.0).unwrap()
}

fn identity_viewport() -> Viewport {
    Viewport::new(Size2::new(1000.0, 1000.0).unwrap(), Transform2D::identity()).unwrap()
}

/// 선택 상자 handle + 상자 안 control point 둘((110,110)과 모서리 정사각형에 겹친 (104,104)), 반지름 5.
fn layer() -> TransientLayer {
    let mut layer = TransientLayer::new();
    layer.push_selection_box(id(1), quad(100.0, 100.0, 300.0, 200.0), metrics());
    for (n, (x, y)) in [(110.0, 110.0), (104.0, 104.0)].into_iter().enumerate() {
        layer.push(TransientHandle {
            kind: HandleKind::ModifierControl(ControlPointId(n as u64)),
            target: id(1),
            shape: HandleShape::circle(p(x, y), 5.0).unwrap(),
        });
    }
    layer
}

fn kind_at(layer: &TransientLayer, viewport: &Viewport, x: f32, y: f32) -> Option<HandleKind> {
    layer.hit(viewport, p(x, y)).map(|hit| hit.handle.kind)
}

#[test]
fn handle_priority_fixture() {
    // 검증 기준(XI.3.5): handle 우선순위 fixture.
    let (_, index) = scene();
    let layer = layer();
    let viewport = identity_viewport();
    assert_eq!(layer.len(), 14);
    let canvas = |x: f32, y: f32| {
        hit_test_canvas(
            &layer,
            &index,
            &viewport,
            p(x, y),
            &NarrowPhaseOptions::EXACT,
            &LayoutBoxesOnly,
        )
    };
    let cases = [
        // 꼭짓점 위: 모서리(3)가 변(2)보다 앞섭니다. 상자 경계는 상자 안이라 회전 영역이 아닙니다.
        (
            100.0,
            100.0,
            Some(HandleKind::ResizeCorner(Corner::TopLeft)),
        ),
        (
            303.0,
            203.0,
            Some(HandleKind::ResizeCorner(Corner::BottomRight)),
        ),
        // 변 띠 반폭 3 이내.
        (98.0, 150.0, Some(HandleKind::ResizeEdge(Edge::Left))),
        (200.0, 102.5, Some(HandleKind::ResizeEdge(Edge::Top))),
        // 모서리 바깥 반지름 16 이내는 회전.
        (90.0, 90.0, Some(HandleKind::Rotate(Corner::TopLeft))),
        (310.0, 210.0, Some(HandleKind::Rotate(Corner::BottomRight))),
        // control point(4)는 모서리와 겹쳐도, 더 멀어도 이깁니다((101,101): 모서리 거리 1, 점 거리 4.2).
        (
            110.0,
            110.0,
            Some(HandleKind::ModifierControl(ControlPointId(0))),
        ),
        (
            101.0,
            101.0,
            Some(HandleKind::ModifierControl(ControlPointId(1))),
        ),
        // 상자 안 모서리 근처(모서리 반지름 16 안이지만 모서리·변·control 밖)는 회전이 아닙니다.
        (112.0, 104.0, None),
        (106.0, 125.0, None),
        // 변 띠 밖·회전 영역 밖.
        (95.0, 150.0, None),
    ];
    for (x, y, expected) in cases {
        assert_eq!(kind_at(&layer, &viewport, x, y), expected, "({x}, {y})");
        match (canvas(x, y), expected) {
            (CanvasHit::Handle(hit), Some(kind)) => assert_eq!(hit.handle.kind, kind),
            (CanvasHit::Node(node), None) => assert_eq!(node.source, id(1), "({x}, {y})"),
            (CanvasHit::Empty, None) => assert_eq!((x, y), (95.0, 150.0)),
            (other, _) => panic!("({x}, {y}): {other:?}"),
        }
    }
    // handle은 scene node보다 먼저입니다: 상자 경계 안쪽 변 띠는 node A 위지만 handle.
    assert!(matches!(canvas(200.0, 101.0), CanvasHit::Handle(_)));
    // 상자 안 handle 없는 곳은 node A.
    assert!(matches!(canvas(200.0, 150.0), CanvasHit::Node(ref node) if node.source == id(1)));
}

#[test]
fn overlapping_handles_resolve_by_priority_then_distance_then_order() {
    // 축소해서 6×6 px가 된 선택 상자: 모서리 정사각형 넷이 모두 겹칩니다.
    let mut layer = TransientLayer::new();
    layer.push_selection_box(id(1), quad(0.0, 0.0, 6.0, 6.0), metrics());
    let viewport = identity_viewport();
    // 중심은 네 모서리에서 같은 거리 3: 마지막에 추가한 BottomLeft.
    let center = layer.hit(&viewport, p(3.0, 3.0)).unwrap();
    assert_eq!(
        center.handle.kind,
        HandleKind::ResizeCorner(Corner::BottomLeft)
    );
    assert_eq!(center.index, 11);
    assert_eq!(center.distance, 3.0);
    // 오른쪽 위로 치우치면 거리가 가까운 TopRight.
    assert_eq!(
        kind_at(&layer, &viewport, 4.0, 2.0),
        Some(HandleKind::ResizeCorner(Corner::TopRight))
    );
    // 완전히 같은 handle 두 개면 나중 것이 이깁니다.
    let mut twins = TransientLayer::new();
    for n in [7u64, 8] {
        twins.push(TransientHandle {
            kind: HandleKind::ResizeEdge(Edge::Top),
            target: id(n),
            shape: HandleShape::segment(p(0.0, 0.0), p(10.0, 0.0), 2.0).unwrap(),
        });
    }
    let hit = twins.hit(&viewport, p(5.0, 1.0)).unwrap();
    assert_eq!((hit.index, hit.handle.target), (1, id(8)));
    twins.clear();
    assert!(twins.is_empty() && twins.hit(&viewport, p(5.0, 1.0)).is_none());
}

#[test]
fn handles_keep_view_size_under_zoom_and_rotation() {
    let mut layer = TransientLayer::new();
    layer.push_selection_box(id(1), quad(100.0, 100.0, 300.0, 200.0), metrics());
    // 4배 확대 + pan: world (100,100) = view (100,100).
    let zoom = Transform2D::new(4.0, 0.0, 0.0, 4.0, -300.0, -300.0).unwrap();
    let viewport = Viewport::new(Size2::new(1000.0, 1000.0).unwrap(), zoom).unwrap();
    // view 3.5 px(world 0.875) 떨어진 점은 모서리 반변 4 px 안.
    assert_eq!(
        kind_at(&layer, &viewport, 103.5, 103.5),
        Some(HandleKind::ResizeCorner(Corner::TopLeft))
    );
    // view 거리 4.5 px는 모서리 밖이지만 변(위·왼쪽) 띠 밖이고 상자 안이라 회전도 아님.
    assert_eq!(kind_at(&layer, &viewport, 104.5, 104.5), None);
    // 1/4 축소: world 상자는 view (25,25)~(75,50).
    let out = Transform2D::new(0.25, 0.0, 0.0, 0.25, 0.0, 0.0).unwrap();
    let viewport = Viewport::new(Size2::new(1000.0, 1000.0).unwrap(), out).unwrap();
    assert_eq!(
        kind_at(&layer, &viewport, 22.0, 22.0),
        Some(HandleKind::ResizeCorner(Corner::TopLeft))
    );
    assert_eq!(
        kind_at(&layer, &viewport, 15.0, 15.0),
        Some(HandleKind::Rotate(Corner::TopLeft))
    );

    // 45° 회전한 선택 상자: 회전 영역은 상자 밖에서만 맞습니다.
    let (c, s) = (
        std::f32::consts::FRAC_1_SQRT_2,
        std::f32::consts::FRAC_1_SQRT_2,
    );
    let rotate = |x: f32, y: f32| p(500.0 + c * x - s * y, 500.0 + s * x + c * y);
    let mut rotated = TransientLayer::new();
    rotated.push_selection_box(
        id(2),
        [
            rotate(0.0, 0.0),
            rotate(100.0, 0.0),
            rotate(100.0, 100.0),
            rotate(0.0, 100.0),
        ],
        metrics(),
    );
    let viewport = identity_viewport();
    // 상자 local (-8,-8)은 상자 밖·모서리 반지름 안: 회전. local (8,8)(상자 안)은 아무 handle도 아님.
    let outside = rotate(-8.0, -8.0);
    let inside = rotate(8.0, 8.0);
    assert_eq!(
        kind_at(&rotated, &viewport, outside.x(), outside.y()),
        Some(HandleKind::Rotate(Corner::TopLeft))
    );
    assert_eq!(kind_at(&rotated, &viewport, inside.x(), inside.y()), None);
}

#[test]
fn degenerate_boxes_have_no_handles_for_axes_without_length() {
    // 길이 0인 축만 바꾸는 크기 조절 handle은 만들지 않습니다. 회전 영역은 그대로입니다.
    let kinds = |selection_box: [Vec2; 4]| {
        let mut layer = TransientLayer::new();
        layer.push_selection_box(id(1), selection_box, metrics());
        layer
            .handles()
            .iter()
            .map(|handle| handle.kind)
            .collect::<Vec<_>>()
    };
    let rotate: Vec<HandleKind> = Corner::ALL.into_iter().map(HandleKind::Rotate).collect();
    let corners: Vec<HandleKind> = Corner::ALL
        .into_iter()
        .map(HandleKind::ResizeCorner)
        .collect();
    let with = |edges: &[Edge]| {
        let mut expected = rotate.clone();
        expected.extend(edges.iter().copied().map(HandleKind::ResizeEdge));
        expected.extend(corners.iter().copied());
        expected
    };
    let horizontal = quad(100.0, 150.0, 300.0, 150.0);
    let vertical = quad(80.0, 100.0, 80.0, 200.0);
    let point = [p(5.0, 5.0); 4];
    let parallel = [p(0.0, 0.0), p(10.0, 10.0), p(20.0, 20.0), p(10.0, 10.0)];
    let cases = [
        (
            quad(100.0, 100.0, 300.0, 200.0),
            (true, true),
            with(&Edge::ALL),
        ),
        (horizontal, (true, false), with(&[Edge::Right, Edge::Left])),
        (vertical, (false, true), with(&[Edge::Top, Edge::Bottom])),
        (point, (false, false), rotate.clone()),
        (parallel, (false, false), rotate.clone()),
    ];
    for (selection_box, (h, v), expected) in cases {
        let axes = ResizeAxes::of(&selection_box);
        assert_eq!(
            (axes.horizontal(), axes.vertical()),
            (h, v),
            "{selection_box:?}"
        );
        assert_eq!(kinds(selection_box), expected, "{selection_box:?}");
    }

    // 수평 선 상자: 선 위(끝점 handle 밖)를 누르면 변 handle이 아니라 node가 맞습니다.
    let (_, index) = scene();
    let mut layer = TransientLayer::new();
    layer.push_selection_box(id(1), horizontal, metrics());
    let viewport = identity_viewport();
    for x in [140.0, 200.0, 260.0] {
        assert_eq!(kind_at(&layer, &viewport, x, 150.0), None, "({x}, 150)");
        assert!(matches!(
            hit_test_canvas(
                &layer,
                &index,
                &viewport,
                p(x, 150.0),
                &NarrowPhaseOptions::EXACT,
                &LayoutBoxesOnly,
            ),
            CanvasHit::Node(ref node) if node.source == id(1)
        ));
    }
    // 끝점은 크기 조절 모서리입니다(겹친 두 모서리 중 나중에 추가한 쪽).
    assert_eq!(
        kind_at(&layer, &viewport, 300.0, 150.0),
        Some(HandleKind::ResizeCorner(Corner::BottomRight))
    );
}

#[test]
fn transient_layer_is_not_part_of_scene_or_save() {
    // project 저장에 포함하지 않습니다: layer를 만들고 hit test해도 snapshot bytes·hash·index가 같습니다.
    let (snapshot, index) = scene();
    let bytes = codec::encode(&snapshot);
    let hash = snapshot.semantic_hash();
    let indexed = index.len();
    let layer = layer();
    let viewport = identity_viewport();
    for (x, y) in [(100.0, 100.0), (200.0, 150.0), (90.0, 90.0)] {
        let _ = hit_test_canvas(
            &layer,
            &index,
            &viewport,
            p(x, y),
            &NarrowPhaseOptions::EXACT,
            &LayoutBoxesOnly,
        );
    }
    assert_eq!(codec::encode(&snapshot), bytes);
    assert_eq!(snapshot.semantic_hash(), hash);
    assert_eq!(index.len(), indexed);
    assert_eq!(snapshot.iter_preorder().count(), 1);
    // 빈 layer면 canvas hit은 scene narrow-phase topmost와 같습니다.
    let empty = TransientLayer::default();
    assert!(matches!(
        hit_test_canvas(
            &empty,
            &index,
            &viewport,
            p(100.0, 100.0),
            &NarrowPhaseOptions::EXACT,
            &LayoutBoxesOnly
        ),
        CanvasHit::Node(ref node) if node.source == id(1)
    ));
    assert!(matches!(
        hit_test_canvas(
            &empty,
            &index,
            &viewport,
            p(10.0, 10.0),
            &NarrowPhaseOptions::EXACT,
            &LayoutBoxesOnly
        ),
        CanvasHit::Empty
    ));
}

#[test]
fn handle_sizes_are_validated() {
    assert!(HandleShape::circle(p(0.0, 0.0), -1.0).is_err());
    assert!(HandleShape::square(p(0.0, 0.0), f32::NAN).is_err());
    assert!(HandleShape::segment(p(0.0, 0.0), p(1.0, 0.0), f32::INFINITY).is_err());
    assert!(HandleShape::outside_corner(quad(0.0, 0.0, 1.0, 1.0), Corner::TopLeft, -0.5).is_err());
    assert!(HandleMetrics::new(4.0, -1.0, 16.0).is_err());
    // 공개 생성자로 만든 회전 영역은 push_selection_box의 것과 같습니다.
    let q = quad(0.0, 0.0, 10.0, 10.0);
    let mut layer = TransientLayer::new();
    layer.push_selection_box(id(1), q, metrics());
    assert_eq!(
        layer.handles()[0].shape,
        HandleShape::outside_corner(q, Corner::TopLeft, 16.0).unwrap()
    );
    assert_eq!(
        layer.handles()[8].shape,
        HandleShape::square(q[0], 4.0).unwrap()
    );
    // 면적 0 상자는 회전 영역 판정에서 상자 밖으로 봅니다(점 상자 선택도 회전 가능).
    let point_box = [p(5.0, 5.0); 4];
    let shape = HandleShape::outside_corner(point_box, Corner::TopLeft, 10.0).unwrap();
    assert!(shape
        .view_distance(&identity_viewport(), p(6.0, 6.0))
        .is_some());
}

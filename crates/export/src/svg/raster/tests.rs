//! 부분 래스터화 경계 planner 테스트 (XX.4.1).

use std::cell::Cell;
use std::sync::Arc;
use std::thread;

use stellar_core_types::color::ColorSpace;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::shadow::{BlurRadius, Shadow};
use stellar_core_types::value::{Transform2D, Vec2};
use stellar_scene::handle::SnapshotHandle;
use stellar_scene::node::{
    CornerRadii, HitTestMeta, ImageCrop, ImageFit, ImagePrimitive, ImageSampling, PointerEvents,
    RectPrimitive, SceneNodeSpec,
};
use stellar_scene::revision::SceneRevisionMap;
use stellar_scene::stack::{ClipStackRef, Paint, PaintLayer, PaintStack, PaintStackRef};

use super::*;
use crate::capture::{AssetRevision, FontRevision, ResourcePins, SceneSource};
use crate::request::{ExportFormat, ExportRequest, ExportSource, OutputPath};
use crate::svg::mapping::SvgSupport;
use crate::test_fixtures as fx;

/// 기호 painter 모사 비교.
mod painter;

/// blend 영역 검색과 단계별 순차 탐색 기준 구현 비교.
mod reference;

thread_local! {
    /// 이 thread에서 겹침 판정(`PixelGrid::overlaps`)을 한 횟수.
    static OVERLAP_TESTS: Cell<u64> = const { Cell::new(0) };
}

/// 겹침 판정 횟수를 셉니다(planner 작업량 측정).
pub(super) fn count_overlap_test() {
    OVERLAP_TESTS.with(|count| count.set(count.get() + 1));
}

/// `f`를 실행하는 동안의 겹침 판정 횟수.
fn overlap_tests_during<R>(f: impl FnOnce() -> R) -> (R, u64) {
    let before = OVERLAP_TESTS.with(Cell::get);
    let result = f();
    (result, OVERLAP_TESTS.with(Cell::get) - before)
}

/// 테스트 tree node: 이름 번호, world 사각형(x, y, w, h), paint, 숨김, 자식.
struct T {
    n: u64,
    rect: (f32, f32, f32, f32),
    paint: PaintStack,
    hidden: bool,
    children: Vec<T>,
}

fn t(n: u64, rect: (f32, f32, f32, f32), paint: PaintStack) -> T {
    T {
        n,
        rect,
        paint,
        hidden: false,
        children: Vec::new(),
    }
}

impl T {
    fn kids(mut self, children: Vec<T>) -> T {
        self.children = children;
        self
    }

    fn hidden(mut self) -> T {
        self.hidden = true;
        self
    }
}

fn none() -> PaintStack {
    PaintStack::empty()
}

fn fill() -> PaintStack {
    PaintStack {
        layers: vec![PaintLayer::Fill(Paint::Solid(fx::srgb(1.0, 0.0, 0.0, 1.0)))],
        ..PaintStack::empty()
    }
}

fn blend(mut paint: PaintStack, mode: SceneBlendMode) -> PaintStack {
    paint.blend = mode;
    paint
}

fn multiply(paint: PaintStack) -> PaintStack {
    blend(paint, SceneBlendMode::Multiply)
}

fn blur(mut paint: PaintStack, radius: f32) -> PaintStack {
    paint.layer_blur = BlurRadius::new(radius).unwrap();
    paint
}

fn opacity(mut paint: PaintStack, value: f32) -> PaintStack {
    paint.opacity = Opacity::new(value).unwrap();
    paint
}

fn shadow(mut paint: PaintStack) -> PaintStack {
    paint.shadows.push(Shadow::new(
        Vec2::new(2.0, 2.0).unwrap(),
        BlurRadius::new(3.0).unwrap(),
        fx::srgb(0.0, 0.0, 0.0, 0.5),
    ));
    paint
}

fn hit(hidden: bool) -> HitTestMeta {
    if hidden {
        HitTestMeta {
            visibility: Visibility::Hidden,
            pointer_events: PointerEvents::None,
        }
    } else {
        HitTestMeta::INTERACTIVE
    }
}

/// z는 pre-order 1부터 매깁니다(scene z-order 불변식).
fn build(trees: Vec<T>) -> Vec<Arc<SceneNode>> {
    let mut z = 0;
    trees.into_iter().map(|tree| node(tree, &mut z)).collect()
}

fn node(tree: T, z: &mut u64) -> Arc<SceneNode> {
    *z += 1;
    let (x, y, w, h) = tree.rect;
    let spec = SceneNodeSpec {
        source: fx::node_id(tree.n),
        content: SceneContent::Rect(RectPrimitive {
            rect: fx::rect(0.0, 0.0, w, h),
            radii: CornerRadii::ZERO,
        }),
        world_transform: fx::translate(x, y),
        layout_rect: fx::rect(0.0, 0.0, w, h),
        clip: ClipStackRef::none(),
        paint: PaintStackRef::new(tree.paint),
        z: ZOrder(*z),
        hit: hit(tree.hidden),
    };
    let children = tree
        .children
        .into_iter()
        .map(|child| node(child, z))
        .collect();
    Arc::new(SceneNode::new(spec, children).unwrap())
}

fn grid(width: f32, height: f32, scale: f32) -> PixelGrid {
    PixelGrid::new(
        Aabb::new(0.0, 0.0, width, height).unwrap(),
        ExportScale::new(scale).unwrap(),
    )
}

fn plan(roots: &[Arc<SceneNode>], grid: PixelGrid, background: bool) -> RasterPlan {
    let refs: Vec<&SceneNode> = roots.iter().map(|root| &**root).collect();
    plan_nodes(&refs, grid, background)
}

fn z_of(roots: &[Arc<SceneNode>], n: u64) -> ZOrder {
    let mut stack: Vec<&SceneNode> = roots.iter().map(|root| &**root).collect();
    while let Some(node) = stack.pop() {
        if node.source() == fx::node_id(n) {
            return node.z();
        }
        stack.extend(node.children().map(|child| &**child));
    }
    panic!("node {n} 없음");
}

/// 계획을 사람이 읽는 한 줄씩으로 씁니다.
fn describe(plan: &RasterPlan, names: &[(u64, &'static str)]) -> Vec<String> {
    let name = |node: RasterNode| -> &'static str {
        names
            .iter()
            .find(|(n, _)| fx::node_id(*n) == node.source)
            .map_or("?", |(_, name)| *name)
    };
    plan.boundaries()
        .iter()
        .map(|boundary| {
            let region = match boundary.region() {
                RasterRegion::Content { node } => format!("content({})", name(*node)),
                RasterRegion::Subtrees {
                    parent,
                    first,
                    last,
                } => format!(
                    "subtrees({}: {}..{})",
                    parent.map_or("canvas", name),
                    name(*first),
                    name(*last)
                ),
                RasterRegion::Canvas => "canvas".to_string(),
            };
            let causes: Vec<String> = boundary
                .causes()
                .iter()
                .map(|cause| {
                    let mut text = format!("{} {}", name(cause.node), cause.feature);
                    if let Some(dependency) = cause.blend {
                        let widest = match dependency.widest {
                            None => "none",
                            Some(BackdropItem::Background) => "background",
                            Some(BackdropItem::Node(node)) => name(node),
                        };
                        text.push_str(&format!(
                            " iso={} widest={widest}",
                            dependency.isolation.map_or("canvas", name)
                        ));
                    }
                    text
                })
                .collect();
            format!("{region} [{}]", causes.join(", "))
        })
        .collect()
}

fn pixels(x0: i64, y0: i64, x1: i64, y1: i64) -> Option<PixelRect> {
    PixelRect::new(x0, y0, x1, y1)
}

const NESTED_NAMES: &[(u64, &str)] = &[
    (1, "P"),
    (2, "a"),
    (3, "b"),
    (4, "F"),
    (5, "f1"),
    (6, "c"),
    (7, "G"),
    (8, "g1"),
    (9, "g2"),
    (10, "g2a"),
    (11, "g3"),
    (12, "B"),
    (13, "Bm"),
    (14, "S"),
    (15, "Sc"),
    (16, "O"),
    (17, "Ob"),
    (18, "Obb"),
    (19, "Om"),
    (20, "H"),
];

/// nested blur/blend fixture. 좌표는 world(= pixel, scale 1, canvas 400×300).
fn nested_fixture() -> Vec<Arc<SceneNode>> {
    build(vec![t(1, (0.0, 0.0, 400.0, 300.0), none()).kids(vec![
        // 형제 backdrop: b가 a와 겹침 → a..b.
        t(2, (10.0, 10.0, 50.0, 50.0), fill()),
        t(3, (40.0, 40.0, 50.0, 50.0), multiply(fill())),
        // 부모 자기 내용 backdrop: f1이 F의 배경 위 → F subtree.
        t(4, (150.0, 10.0, 100.0, 70.0), fill()).kids(vec![t(
            5,
            (160.0, 20.0, 40.0, 40.0),
            multiply(fill()),
        )]),
        // blend와 무관한 형제: vector.
        t(6, (300.0, 10.0, 50.0, 50.0), fill()),
        // isolation group(opacity) 안 두 단계 위 형제 backdrop → G 안 g1..g2.
        t(7, (100.0, 100.0, 150.0, 150.0), opacity(none(), 0.5)).kids(vec![
            t(8, (110.0, 110.0, 50.0, 50.0), fill()),
            t(9, (130.0, 130.0, 70.0, 70.0), none()).kids(vec![t(
                10,
                (150.0, 150.0, 40.0, 40.0),
                blend(fill(), SceneBlendMode::Screen),
            )]),
            t(11, (220.0, 220.0, 20.0, 20.0), fill()),
        ]),
        // blur group 안 blend: backdrop은 blur group 내용 → B subtree 하나로 합침.
        t(12, (10.0, 200.0, 50.0, 50.0), blur(fill(), 4.0)).kids(vec![t(
            13,
            (20.0, 210.0, 20.0, 20.0),
            multiply(fill()),
        )]),
        // shadow: 자기 내용만, 자식 vector.
        t(14, (300.0, 200.0, 50.0, 50.0), shadow(fill())).kids(vec![t(
            15,
            (310.0, 210.0, 10.0, 10.0),
            fill(),
        )]),
        // 중첩 blur와 blur로 퍼진 backdrop.
        t(16, (300.0, 100.0, 80.0, 80.0), opacity(none(), 0.8)).kids(vec![
            t(17, (310.0, 110.0, 20.0, 20.0), blur(fill(), 2.0)).kids(vec![t(
                18,
                (312.0, 112.0, 8.0, 8.0),
                blur(fill(), 1.0),
            )]),
            t(19, (325.0, 125.0, 45.0, 45.0), multiply(fill())),
        ]),
        // 숨긴 blend: 계획에 없음.
        t(20, (0.0, 0.0, 400.0, 300.0), multiply(fill())).hidden(),
    ])])
}

#[test]
fn raster_scope_matches_mapping_support() {
    for feature in SvgFeature::ALL {
        let rasterize = matches!(feature.support(), SvgSupport::Rasterize { .. });
        assert_eq!(raster_scope(feature).is_some(), rasterize, "{feature}");
    }
    assert_eq!(
        raster_scope(SvgFeature::DropShadow),
        Some(RasterScope::Content)
    );
    assert_eq!(
        raster_scope(SvgFeature::ImageColorSpaceOverride),
        Some(RasterScope::Content)
    );
    assert_eq!(
        raster_scope(SvgFeature::LayerBlur),
        Some(RasterScope::Subtree)
    );
    assert_eq!(
        raster_scope(SvgFeature::BlendMode),
        Some(RasterScope::Backdrop)
    );
}

#[test]
fn nested_blur_blend_fixture_boundaries_are_exact() {
    // 검증 기준(XX.4.1): nested blur/blend fixture boundary 정확.
    let roots = nested_fixture();
    let plan = plan(&roots, grid(400.0, 300.0, 1.0), false);
    assert_eq!(
        describe(&plan, NESTED_NAMES),
        vec![
            "subtrees(P: a..b) [b BlendMode iso=canvas widest=a]",
            "subtrees(P: F..F) [f1 BlendMode iso=canvas widest=F]",
            "subtrees(G: g1..g2) [g2a BlendMode iso=G widest=g1]",
            "subtrees(P: B..B) [B LayerBlur, Bm BlendMode iso=B widest=B]",
            "content(S) [S DropShadow]",
            "subtrees(O: Ob..Om) [Ob LayerBlur, Obb LayerBlur, Om BlendMode iso=O widest=Ob]",
        ]
    );
    let areas: Vec<Option<PixelRect>> = plan.boundaries().iter().map(|b| b.pixels()).collect();
    assert_eq!(
        areas,
        vec![
            pixels(10, 10, 90, 90),
            pixels(150, 10, 250, 80),
            // g2는 칠하지 않으므로 g1 ∪ g2a.
            pixels(110, 110, 190, 190),
            // B 자기 내용 blur 4(scene render bounds) ∪ 자식 Bm을 B blur만큼 넓힌 영역.
            pixels(6, 196, 64, 254),
            // 내용 ∪ offset (2, 2)·blur 3 shadow.
            pixels(299, 199, 355, 255),
            // Ob(자기 blur 2) ∪ Om.
            pixels(308, 108, 370, 170),
        ]
    );
    let counts: Vec<u64> = plan
        .boundaries()
        .iter()
        .map(|b| b.rasterized_nodes())
        .collect();
    assert_eq!(counts, vec![2, 2, 3, 2, 1, 3]);
    // 숨긴 H를 뺀 19개 중 P·c·G·g3·Sc·O 6개가 vector로 남습니다.
    assert_eq!(plan.visible_nodes(), 19);
    assert_eq!(plan.rasterized_nodes(), 13);
    assert_eq!(plan.vector_nodes(), 6);
    let vector: Vec<&str> = NESTED_NAMES
        .iter()
        .filter(|(n, _)| *n != 20 && !plan.rasterizes(z_of(&roots, *n)))
        .map(|(_, name)| *name)
        .collect();
    assert_eq!(vector, vec!["P", "c", "G", "g3", "Sc", "O"]);
}

#[test]
fn boundary_lookup_by_start_node() {
    let roots = nested_fixture();
    let plan = plan(&roots, grid(400.0, 300.0, 1.0), false);
    let first = plan.boundary_at(z_of(&roots, 2)).unwrap();
    assert_eq!(first, &plan.boundaries()[0]);
    assert!(plan.boundary_at(z_of(&roots, 3)).is_none());
    let content = plan.boundary_at(z_of(&roots, 14)).unwrap();
    assert!(matches!(content.region(), RasterRegion::Content { .. }));
    // content 경계는 자식을 덮지 않습니다.
    assert!(plan.rasterizes(z_of(&roots, 14)));
    assert!(!plan.rasterizes(z_of(&roots, 15)));
    // 숨긴 node와 첫 node 앞 z는 래스터화하지 않습니다.
    assert!(!plan.rasterizes(z_of(&roots, 20)));
    assert!(!plan.rasterizes(ZOrder(0)));
    assert!(!plan.is_empty());
}

#[test]
fn overlapping_sibling_ranges_merge_and_disjoint_ones_stay_apart() {
    let names = &[(1, "P"), (2, "a"), (3, "b"), (4, "c"), (5, "d")];
    let roots = build(vec![t(1, (0.0, 0.0, 300.0, 300.0), none()).kids(vec![
        t(2, (0.0, 0.0, 30.0, 30.0), fill()),
        t(3, (20.0, 20.0, 30.0, 30.0), multiply(fill())),
        // c는 a와 겹치지 않고 b와만 겹침: b..c 범위가 a..b와 합쳐집니다.
        t(
            4,
            (45.0, 45.0, 30.0, 30.0),
            blend(fill(), SceneBlendMode::Screen),
        ),
        t(5, (200.0, 200.0, 10.0, 10.0), multiply(fill())),
    ])]);
    let plan = plan(&roots, grid(300.0, 300.0, 1.0), false);
    assert_eq!(
        describe(&plan, names),
        vec![
            "subtrees(P: a..c) [b BlendMode iso=canvas widest=a, c BlendMode iso=canvas widest=b]",
            "subtrees(P: d..d) [d BlendMode iso=canvas widest=none]",
        ]
    );
    assert_eq!(plan.rasterized_nodes(), 4);
    assert_eq!(plan.vector_nodes(), 1);
}

#[test]
fn isolation_group_content_pulls_the_group_into_the_region() {
    let names = &[(1, "P"), (2, "G"), (3, "g"), (4, "x")];
    let roots = build(vec![t(1, (0.0, 0.0, 200.0, 100.0), none()).kids(vec![
        t(2, (0.0, 0.0, 50.0, 50.0), opacity(fill(), 0.5)).kids(vec![t(
            3,
            (10.0, 10.0, 10.0, 10.0),
            multiply(fill()),
        )]),
        t(4, (100.0, 0.0, 10.0, 10.0), fill()),
    ])]);
    let plan = plan(&roots, grid(200.0, 100.0, 1.0), false);
    assert_eq!(
        describe(&plan, names),
        vec!["subtrees(P: G..G) [g BlendMode iso=G widest=G]"]
    );
}

#[test]
fn top_level_blend_depends_on_earlier_roots_and_background() {
    let names = &[(1, "R1"), (2, "R2"), (3, "R3"), (4, "R4"), (5, "r4")];
    let roots = build(vec![
        t(1, (0.0, 0.0, 50.0, 50.0), fill()),
        t(2, (40.0, 40.0, 50.0, 50.0), multiply(fill())),
        t(3, (200.0, 200.0, 20.0, 20.0), multiply(fill())),
        t(4, (300.0, 0.0, 50.0, 50.0), opacity(none(), 0.5)).kids(vec![t(
            5,
            (310.0, 10.0, 20.0, 20.0),
            multiply(fill()),
        )]),
    ]);
    let transparent = plan(&roots, grid(400.0, 300.0, 1.0), false);
    assert_eq!(
        describe(&transparent, names),
        vec![
            "subtrees(canvas: R1..R2) [R2 BlendMode iso=canvas widest=R1]",
            "subtrees(canvas: R3..R3) [R3 BlendMode iso=canvas widest=none]",
            "subtrees(R4: r4..r4) [r4 BlendMode iso=R4 widest=none]",
        ]
    );
    assert_eq!(transparent.vector_nodes(), 1);
    // 배경을 칠하면 isolation group 밖 blend는 배경을 backdrop으로 가져 canvas 전체가 됩니다.
    // group 안 r4는 배경과 섞이지 않습니다.
    let solid = plan(&roots, grid(400.0, 300.0, 1.0), true);
    assert_eq!(
        describe(&solid, names),
        vec![
            "canvas [R2 BlendMode iso=canvas widest=background, R3 BlendMode iso=canvas widest=background, r4 BlendMode iso=R4 widest=none]"
        ]
    );
    assert_eq!(solid.boundaries()[0].pixels(), pixels(0, 0, 400, 300));
    assert_eq!(solid.vector_nodes(), 0);
    assert!(solid.rasterizes(z_of(&roots, 5)));
    assert_eq!(
        solid.boundary_at(z_of(&roots, 1)).map(|b| b.region()),
        Some(&RasterRegion::Canvas)
    );
    // blend가 없으면 배경만으로는 래스터화하지 않습니다.
    let plain = build(vec![t(1, (0.0, 0.0, 50.0, 50.0), fill())]);
    assert!(plan(&plain, grid(50.0, 50.0, 1.0), true).is_empty());
}

fn pair_widest(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32), grid: PixelGrid) -> String {
    let names = &[(1, "a"), (2, "b")];
    let roots = build(vec![t(1, a, fill()), t(2, b, multiply(fill()))]);
    describe(&plan(&roots, grid, false), names).join("; ")
}

fn pair_with(a: PaintStack, b: (f32, f32, f32, f32)) -> String {
    let names = &[(1, "a"), (2, "b")];
    let roots = build(vec![
        t(1, (0.0, 0.0, 10.0, 10.0), a),
        t(2, b, multiply(fill())),
    ]);
    describe(&plan(&roots, grid(100.0, 100.0, 1.0), false), names).join("; ")
}

#[test]
fn pixel_grid_overlap_rules() {
    let alone = "subtrees(canvas: b..b) [b BlendMode iso=canvas widest=none]";
    let joined = "subtrees(canvas: a..b) [b BlendMode iso=canvas widest=a]";
    let g = grid(100.0, 100.0, 1.0);
    // pixel 경계에 맞닿기만 하면 겹치지 않습니다.
    assert_eq!(
        pair_widest((0.0, 0.0, 10.0, 10.0), (10.0, 0.0, 10.0, 10.0), g),
        alone
    );
    // 한 pixel(10)을 나눠 덮으면 겹칩니다.
    assert_eq!(
        pair_widest((0.0, 0.0, 10.5, 10.0), (10.7, 0.0, 10.0, 10.0), g),
        joined
    );
    // scale 2에서는 같은 두 도형이 pixel 21 경계로 나뉩니다.
    assert_eq!(
        pair_widest(
            (0.0, 0.0, 10.5, 10.0),
            (10.7, 0.0, 10.0, 10.0),
            grid(100.0, 100.0, 2.0)
        ),
        alone
    );
    // 정수 오차(1/256 pixel) 안의 경계는 정수로 붙여 맞닿음으로 봅니다.
    assert_eq!(
        pair_widest((0.0, 0.0, 10.001, 10.0), (10.0, 0.0, 10.0, 10.0), g),
        alone
    );
    // canvas 밖에서만 겹치면 의존이 아닙니다.
    assert_eq!(
        pair_widest((-50.0, 0.0, 20.0, 20.0), (-40.0, 0.0, 50.0, 20.0), g),
        alone
    );
    // 내보내기 영역 원점이 grid 원점입니다.
    let shifted = PixelGrid::new(
        Aabb::new(100.0, 100.0, 200.0, 200.0).unwrap(),
        ExportScale::ONE,
    );
    assert_eq!(
        pair_widest(
            (100.0, 100.0, 10.0, 10.0),
            (110.0, 100.0, 10.0, 10.0),
            shifted
        ),
        alone
    );
    assert_eq!(
        shifted.device_box(&Aabb::new(100.0, 100.0, 110.5, 101.0).unwrap()),
        pixels(0, 0, 11, 1)
    );
}

#[test]
fn canvas_size_follows_bounds_and_scale() {
    assert_eq!(grid(10.2, 5.0, 2.0).canvas(), pixels(0, 0, 21, 10).unwrap());
    assert_eq!(
        grid(10.001, 5.0, 1.0).canvas(),
        pixels(0, 0, 10, 5).unwrap()
    );
    // 아주 작은 영역도 최소 1 pixel입니다.
    assert_eq!(
        grid(0.001, 0.001, 1.0).canvas(),
        pixels(0, 0, 1, 1).unwrap()
    );
    assert_eq!(grid(10.0, 5.0, 1.0).scale(), 1.0);
}

#[test]
fn blur_spreads_the_backdrop_footprint() {
    let alone = "subtrees(canvas: b..b) [b BlendMode iso=canvas widest=none]";
    // blur 없는 a [0, 10)은 b [12, 22)와 겹치지 않습니다.
    assert_eq!(pair_with(fill(), (12.0, 0.0, 10.0, 10.0)), alone);
    // blur 3은 pixel 12까지 퍼지고 13에는 닿지 않습니다.
    assert_eq!(
        pair_with(blur(fill(), 3.0), (12.0, 0.0, 10.0, 10.0)),
        "subtrees(canvas: a..b) [a LayerBlur, b BlendMode iso=canvas widest=a]"
    );
    assert_eq!(
        pair_with(blur(fill(), 3.0), (13.0, 0.0, 10.0, 10.0)),
        "subtrees(canvas: a..a) [a LayerBlur]; subtrees(canvas: b..b) [b BlendMode iso=canvas widest=none]"
    );
}

#[test]
fn blurred_group_spreads_child_content_into_the_backdrop() {
    let names = &[(1, "Q"), (2, "q"), (3, "b")];
    let scene = |radius: f32| {
        let group = if radius > 0.0 {
            blur(none(), radius)
        } else {
            none()
        };
        build(vec![
            t(1, (0.0, 0.0, 10.0, 10.0), group).kids(vec![t(2, (0.0, 0.0, 10.0, 10.0), fill())]),
            t(3, (14.0, 0.0, 10.0, 10.0), multiply(fill())),
        ])
    };
    let blurred = scene(5.0);
    assert_eq!(
        describe(&plan(&blurred, grid(100.0, 100.0, 1.0), false), names),
        vec!["subtrees(canvas: Q..b) [Q LayerBlur, b BlendMode iso=canvas widest=Q]"]
    );
    let sharp = scene(0.0);
    assert_eq!(
        describe(&plan(&sharp, grid(100.0, 100.0, 1.0), false), names),
        vec!["subtrees(canvas: b..b) [b BlendMode iso=canvas widest=none]"]
    );
}

fn blur_node(transform: Transform2D, radius: f32) -> SceneNode {
    SceneNode::leaf(SceneNodeSpec {
        source: fx::node_id(1),
        content: SceneContent::Rect(RectPrimitive {
            rect: fx::rect(0.0, 0.0, 10.0, 10.0),
            radii: CornerRadii::ZERO,
        }),
        world_transform: transform,
        layout_rect: fx::rect(0.0, 0.0, 10.0, 10.0),
        clip: ClipStackRef::none(),
        paint: PaintStackRef::new(blur(fill(), radius)),
        z: ZOrder(1),
        hit: HitTestMeta::INTERACTIVE,
    })
}

#[test]
fn blur_pixels_use_the_transform_scale_bound() {
    let identity = Transform2D::identity();
    let scale2 = Transform2D::new(2.0, 0.0, 0.0, 2.0, 5.0, 5.0).unwrap();
    let (s, c) = (
        std::f32::consts::FRAC_1_SQRT_2,
        std::f32::consts::FRAC_1_SQRT_2,
    );
    let rotate = Transform2D::new(c, s, -s, c, 0.0, 0.0).unwrap();
    let shrink = Transform2D::new(0.25, 0.0, 0.0, 0.25, 0.0, 0.0).unwrap();
    assert_eq!(blur_pixels(&blur_node(identity, 3.0), 1.0), 3);
    assert_eq!(blur_pixels(&blur_node(scale2, 3.0), 1.0), 6);
    assert_eq!(blur_pixels(&blur_node(rotate, 3.0), 1.0), 3);
    // 축소 변환은 배율 1로 봅니다(반경 좌표계를 넓게 잡음).
    assert_eq!(blur_pixels(&blur_node(shrink, 3.0), 1.0), 3);
    assert_eq!(blur_pixels(&blur_node(identity, 3.0), 2.0), 6);
    assert_eq!(blur_pixels(&blur_node(identity, 0.1), 1.0), 1);
    assert_eq!(blur_pixels(&blur_node(identity, 0.0), 1.0), 0);
}

#[test]
fn hidden_nodes_neither_rasterize_nor_form_backdrop() {
    let names = &[(1, "P"), (2, "h"), (3, "x"), (4, "hb")];
    let roots = build(vec![t(1, (0.0, 0.0, 100.0, 100.0), none()).kids(vec![
        t(2, (0.0, 0.0, 100.0, 100.0), fill()).hidden(),
        t(3, (10.0, 10.0, 20.0, 20.0), multiply(fill())),
        t(4, (0.0, 0.0, 100.0, 100.0), blur(multiply(fill()), 2.0)).hidden(),
    ])]);
    let plan = plan(&roots, grid(100.0, 100.0, 1.0), false);
    assert_eq!(
        describe(&plan, names),
        vec!["subtrees(P: x..x) [x BlendMode iso=canvas widest=none]"]
    );
    assert_eq!(plan.visible_nodes(), 2);
}

#[test]
fn content_scope_covers_image_color_space_override() {
    let image = |color_space: Option<ColorSpace>| {
        Arc::new(SceneNode::leaf(SceneNodeSpec {
            source: fx::node_id(1),
            content: SceneContent::Image(ImagePrimitive {
                asset: AssetRef::new(fx::node_id(9)),
                bounds: fx::rect(0.0, 0.0, 10.0, 10.0),
                fit: ImageFit::Fill,
                crop: ImageCrop::FULL,
                sampling: ImageSampling::Linear,
                color_space,
            }),
            world_transform: Transform2D::identity(),
            layout_rect: fx::rect(0.0, 0.0, 10.0, 10.0),
            clip: ClipStackRef::none(),
            paint: PaintStackRef::empty(),
            z: ZOrder(1),
            hit: HitTestMeta::INTERACTIVE,
        }))
    };
    let names = &[(1, "img")];
    let overridden = [image(Some(ColorSpace::DisplayP3))];
    let planned = plan(&overridden, grid(10.0, 10.0, 1.0), false);
    assert_eq!(
        describe(&planned, names),
        vec!["content(img) [img ImageColorSpaceOverride]"]
    );
    assert_eq!(planned.boundaries()[0].pixels(), pixels(0, 0, 10, 10));
    assert!(plan(&[image(None)], grid(10.0, 10.0, 1.0), false).is_empty());
}

#[test]
fn shadow_inside_a_blend_region_is_absorbed() {
    let names = &[(1, "P"), (2, "a"), (3, "b")];
    let roots = build(vec![t(1, (0.0, 0.0, 100.0, 100.0), none()).kids(vec![
        t(2, (0.0, 0.0, 30.0, 30.0), shadow(fill())),
        t(3, (20.0, 20.0, 30.0, 30.0), shadow(multiply(fill()))),
    ])]);
    let plan = plan(&roots, grid(100.0, 100.0, 1.0), false);
    assert_eq!(
        describe(&plan, names),
        vec!["subtrees(P: a..b) [a DropShadow, b DropShadow, b BlendMode iso=canvas widest=a]"]
    );
}

#[test]
fn plans_are_deterministic() {
    let first = plan(&nested_fixture(), grid(400.0, 300.0, 1.0), false);
    let second = plan(&nested_fixture(), grid(400.0, 300.0, 1.0), false);
    assert_eq!(first, second);
    let elsewhere = thread::spawn(|| plan(&nested_fixture(), grid(400.0, 300.0, 1.0), false))
        .join()
        .unwrap();
    assert_eq!(first, elsewhere);
}

#[test]
fn deep_trees_are_planned_without_recursion() {
    const DEPTH: u64 = 2000;
    let planned = thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            // 맨 위 node만 칠하고 맨 아래 leaf가 blend: backdrop은 맨 위 내용입니다.
            let leaf_spec = |n: u64, paint: PaintStack| SceneNodeSpec {
                source: fx::node_id(n),
                content: SceneContent::Rect(RectPrimitive {
                    rect: fx::rect(0.0, 0.0, 100.0, 100.0),
                    radii: CornerRadii::ZERO,
                }),
                world_transform: Transform2D::identity(),
                layout_rect: fx::rect(0.0, 0.0, 100.0, 100.0),
                clip: ClipStackRef::none(),
                paint: PaintStackRef::new(paint),
                z: ZOrder(n),
                hit: HitTestMeta::INTERACTIVE,
            };
            let mut node = Arc::new(SceneNode::leaf(leaf_spec(DEPTH, multiply(fill()))));
            for n in (1..DEPTH).rev() {
                let paint = if n == 1 { fill() } else { none() };
                node = Arc::new(SceneNode::new(leaf_spec(n, paint), vec![node]).unwrap());
            }
            let roots = [node];
            let planned = plan(&roots, grid(100.0, 100.0, 1.0), false);
            (
                describe(&planned, &[(1, "top"), (DEPTH, "leaf")]),
                planned.rasterized_nodes(),
            )
        })
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(
        planned.0,
        vec!["subtrees(canvas: top..top) [leaf BlendMode iso=canvas widest=top]"]
    );
    assert_eq!(planned.1, DEPTH);
}

#[test]
fn wide_blend_sibling_lists_need_near_linear_overlap_work() {
    // 겹치지 않는 1×1 blend 4096개를 한 부모 아래, 또 최상위에 나란히 둡니다. blend마다 앞선 형제를 하나씩
    // 훑으면 겹침 판정이 형제 수의 제곱(약 840만 번)이 됩니다.
    const SIDE: u64 = 64;
    const COUNT: u64 = SIDE * SIDE;
    let cells = |first: u64| -> Vec<T> {
        (0..COUNT)
            .map(|k| {
                let (x, y) = ((k % SIDE) as f32, (k / SIDE) as f32);
                t(first + k, (x, y, 1.0, 1.0), multiply(fill()))
            })
            .collect()
    };
    let side = SIDE as f32;
    let nested = build(vec![t(1, (0.0, 0.0, side, side), none()).kids(cells(2))]);
    let flat = build(cells(1));
    for (roots, nodes) in [(nested, COUNT + 1), (flat, COUNT)] {
        let (planned, work) = overlap_tests_during(|| plan(&roots, grid(side, side, 1.0), false));
        assert_eq!(planned.boundaries().len() as u64, COUNT);
        for boundary in planned.boundaries() {
            let RasterRegion::Subtrees { first, last, .. } = boundary.region() else {
                panic!("{:?}", boundary.region());
            };
            assert_eq!(first, last);
            assert_eq!(boundary.causes().len(), 1);
            assert_eq!(boundary.causes()[0].blend.unwrap().widest, None);
        }
        assert_eq!(planned.visible_nodes(), nodes);
        assert_eq!(planned.rasterized_nodes(), COUNT);
        assert!(work <= COUNT * 64, "겹침 판정 {work}번");
    }
}

const PINS: ResourcePins = ResourcePins {
    fonts: FontRevision(1),
    assets: AssetRevision(1),
};

fn capture_of(
    roots: Vec<Arc<SceneNode>>,
    scale: f32,
    background: ExportBackground,
) -> ExportCapture {
    let snapshot = SceneRevisionMap::new()
        .generate(DocumentRevision(1), roots)
        .unwrap();
    let request = ExportRequest::new(
        ExportSource::view(fx::view_id()).unwrap(),
        fx::profile(),
        ExportScale::new(scale).unwrap(),
        background,
        OutputPath::new("/tmp/stellar-export/out.svg").unwrap(),
        ExportFormat::Svg,
    )
    .unwrap();
    ExportCapture::new(
        request,
        SceneSource {
            view: fx::view_id(),
            target_profile: fx::profile(),
            snapshot: SnapshotHandle::new(snapshot),
        },
        PINS,
    )
    .unwrap()
}

#[test]
fn plan_from_capture_uses_export_grid_and_background() {
    let scene = || {
        build(vec![
            t(1, (20.0, 10.0, 50.0, 50.0), fill()),
            t(2, (100.0, 10.0, 20.0, 20.0), multiply(fill())),
        ])
    };
    let names = &[(1, "R1"), (2, "R2")];
    let transparent = capture_of(scene(), 2.0, ExportBackground::Transparent);
    let planned = plan_raster(&transparent);
    let grid = PixelGrid::of_capture(&transparent);
    let bounds = transparent.bounds();
    assert_eq!(grid, PixelGrid::new(bounds, ExportScale::new(2.0).unwrap()));
    assert_eq!(
        grid.canvas().width(),
        ((bounds.width() * 2.0).ceil()) as i64
    );
    assert_eq!(planned.grid(), &grid);
    assert_eq!(
        describe(&planned, names),
        vec!["subtrees(canvas: R2..R2) [R2 BlendMode iso=canvas widest=none]"]
    );
    // R2 [100, 120) → (100 − 20) × 2 = 160부터.
    assert_eq!(planned.boundaries()[0].pixels(), pixels(160, 0, 200, 40));
    let solid = capture_of(
        scene(),
        2.0,
        ExportBackground::Solid(fx::srgb(1.0, 1.0, 1.0, 1.0)),
    );
    assert_eq!(
        describe(&plan_raster(&solid), names),
        vec!["canvas [R2 BlendMode iso=canvas widest=background]"]
    );
    // 완전 투명 단색 배경은 아무것도 칠하지 않습니다.
    let clear = capture_of(
        scene(),
        2.0,
        ExportBackground::Solid(fx::srgb(1.0, 1.0, 1.0, 0.0)),
    );
    assert_eq!(plan_raster(&clear), planned);
}

#[test]
fn basic_fixture_needs_no_raster() {
    let capture = capture_of(
        fx::basic_roots(&fx::Basic::default()),
        1.0,
        ExportBackground::Transparent,
    );
    let planned = plan_raster(&capture);
    assert!(planned.is_empty());
    assert_eq!(planned.visible_nodes(), 4);
    assert_eq!(planned.vector_nodes(), 4);
}

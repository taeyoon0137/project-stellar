//! 2만 가시 node hit test p99 benchmark (XI.3.6).
//!
//! 1920×1080 viewport를 card 2,000개(Frame 1 + Shape·Path·Text·Image 자식 9 = 10 node)로 채운 scene에서
//! 정확히 20,000 node가 가시(viewport 안·표시)입니다. 숨김 card 100개(1,000 node)와 viewport 밖 card
//! 100개(1,000 node)를 함께 둡니다. 결정론적 pseudo-random view 점마다 scene hit test(broad + narrow,
//! XI.3.3~XI.3.4, view 허용 거리 2 px)를 한 번씩 재고 nearest-rank p50/p95/p99를 냅니다.
//!
//! - 측정 실행(release, 결과 JSON 한 줄 출력):
//!   `cargo test --release -p stellar-scene --test hit_test_benchmark -- --ignored --nocapture`
//! - `STELLAR_HIT_TEST_BENCH_OUT=<path>`를 주면 같은 JSON을 그 파일에도 씁니다.
//! - 기본 `cargo test`는 같은 scene에서 적은 query로 JSON 구성과 집계 일관성만 확인합니다(시간 판정 없음).

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use stellar_core_types::color::Color;
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::AssetRef;
use stellar_core_types::shadow::BlurRadius;
use stellar_core_types::value::{Rect, Transform2D};
use stellar_scene::builder::{
    FrameClipInput, FrameInput, ImageInput, ModifierPaint, NodeInput, PathInput, SceneBuilder,
    SceneScope, ShapeInput, StrokeInput, TextInput,
};
use stellar_scene::geometry::{FillRule, PathCommand, PathGeometry};
use stellar_scene::hit_test::hit_test_broad;
use stellar_scene::narrow_phase::{hit_test_view_narrow, LayoutBoxesOnly, NarrowPhaseOptions};
use stellar_scene::node::{CornerRadii, ImageCrop, ImageFit, ImageSampling, TextLayoutKey};
use stellar_scene::spatial::SceneSpatialIndex;
use stellar_scene::stack::{Opacity, Paint, SceneBlendMode, StrokeCap};
use stellar_scene::visibility::{visible_candidates, Viewport};
use stellar_scene::{ColorSpace, SceneSnapshot, Size2, Vec2};

const VIEW_W: f32 = 1920.0;
const VIEW_H: f32 = 1080.0;
const COLS: usize = 50;
const ROWS: usize = 40;
const NODES_PER_CARD: usize = 10;
const HIDDEN_CARDS: usize = 100;
const OFFSCREEN_CARDS: usize = 100;
/// view 허용 거리(px).
const VIEW_TOLERANCE: f32 = 2.0;
/// 아키텍처 §6.2 "Hit Test 및 인터랙션 해석" 목표·절대 상한(ns).
const BUDGET_TARGET_NS: u128 = 350_000;
const BUDGET_LIMIT_NS: u128 = 750_000;

/// 결정론적 난수(SplitMix64).
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `[0, max)` 실수(1/1024 단위).
    fn unit(&mut self, max: f32) -> f32 {
        let steps = (max * 1024.0) as u64;
        (self.next_u64() % steps) as f32 / 1024.0
    }
}

fn v(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y).expect("finite")
}

fn size(w: f32, h: f32) -> Size2 {
    Size2::new(w, h).expect("size")
}

fn translate(x: f32, y: f32) -> Transform2D {
    Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y).expect("transform")
}

fn color(r: f32) -> Color {
    Color::new(ColorSpace::Srgb, r, 0.5, 0.25, 1.0).expect("color")
}

fn ellipse(w: f32, h: f32) -> PathGeometry {
    let k = 0.552_284_8f32;
    let (rx, ry) = (w / 2.0, h / 2.0);
    PathGeometry::new(vec![
        PathCommand::MoveTo(v(w, ry)),
        PathCommand::CubicTo {
            c1: v(w, ry + k * ry),
            c2: v(rx + k * rx, h),
            to: v(rx, h),
        },
        PathCommand::CubicTo {
            c1: v(rx - k * rx, h),
            c2: v(0.0, ry + k * ry),
            to: v(0.0, ry),
        },
        PathCommand::CubicTo {
            c1: v(0.0, ry - k * ry),
            c2: v(rx - k * rx, 0.0),
            to: v(rx, 0.0),
        },
        PathCommand::CubicTo {
            c1: v(rx + k * rx, 0.0),
            c2: v(w, ry - k * ry),
            to: v(w, ry),
        },
        PathCommand::Close,
    ])
    .expect("ellipse")
}

fn polygon(points: &[(f32, f32)], close: bool) -> PathGeometry {
    let mut commands = vec![PathCommand::MoveTo(v(points[0].0, points[0].1))];
    commands.extend(
        points[1..]
            .iter()
            .map(|&(x, y)| PathCommand::LineTo(v(x, y))),
    );
    if close {
        commands.push(PathCommand::Close);
    }
    PathGeometry::new(commands).expect("polygon")
}

fn paint(fill: bool, stroke: f32) -> ModifierPaint {
    ModifierPaint {
        fills: if fill {
            vec![Paint::Solid(color(0.75))]
        } else {
            Vec::new()
        },
        strokes: if stroke > 0.0 {
            vec![StrokeInput {
                paint: Paint::Solid(color(0.1)),
                width: stroke,
            }]
        } else {
            Vec::new()
        },
        shadows: Vec::new(),
        opacity: Opacity::OPAQUE,
        blend: SceneBlendMode::Normal,
        layer_blur: BlurRadius::ZERO,
    }
}

fn node_input(
    source: &mut u64,
    x: f32,
    y: f32,
    visible: bool,
    fill: bool,
    stroke: f32,
) -> NodeInput {
    *source += 1;
    let mut bytes = [0u8; 16];
    bytes[8..].copy_from_slice(&source.to_be_bytes());
    let mut node = NodeInput::new(StableId::from_bytes(bytes), translate(x, y));
    node.visible = visible;
    node.paint = paint(fill, stroke);
    node
}

/// (x, y) 원점의 36×25 card: Frame + 자식 9개.
fn card(
    builder: &mut SceneBuilder,
    root: &mut SceneScope,
    source: &mut u64,
    x: f32,
    y: f32,
    visible: bool,
) {
    let frame = FrameInput {
        node: node_input(source, x, y, visible, false, 0.0),
        size: size(36.0, 25.0),
        corner_radii: CornerRadii::new(4.0, 4.0, 4.0, 4.0).expect("radii"),
        background: Some(color(0.9)),
        border: None,
        clip: FrameClipInput::Bounds,
    };
    let mut scope = builder.begin_frame(root, &frame).expect("frame");
    let shape = |source: &mut u64, x, y, geometry: PathGeometry, fill, stroke, cap| ShapeInput {
        node: node_input(source, x, y, true, fill, stroke),
        layout_rect: Rect::new(Vec2::ZERO, size(10.0, 10.0)),
        geometry,
        cap,
    };
    let parent = scope.scope_mut();
    let shapes = [
        shape(
            source,
            0.0,
            0.0,
            ellipse(12.0, 12.0),
            true,
            0.0,
            StrokeCap::Butt,
        ),
        shape(
            source,
            13.0,
            0.0,
            polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)], true),
            true,
            1.0,
            StrokeCap::Butt,
        ),
        shape(
            source,
            28.0,
            13.0,
            ellipse(7.0, 7.0),
            false,
            1.5,
            StrokeCap::Butt,
        ),
        shape(
            source,
            30.0,
            20.0,
            polygon(&[(0.0, 0.0), (5.0, 0.0), (5.0, 4.0), (0.0, 4.0)], true),
            true,
            0.0,
            StrokeCap::Butt,
        ),
    ];
    for input in &shapes {
        builder.build_shape(parent, input).expect("shape");
    }
    let paths = [
        PathInput {
            node: node_input(source, 24.0, 0.0, true, true, 0.0),
            layout_rect: Rect::new(Vec2::ZERO, size(11.0, 10.0)),
            geometry: polygon(&[(0.0, 10.0), (5.5, 0.0), (11.0, 10.0)], true),
            fill_rule: FillRule::EvenOdd,
        },
        PathInput {
            node: node_input(source, 1.0, 12.0, true, false, 1.0),
            layout_rect: Rect::new(Vec2::ZERO, size(34.0, 0.0)),
            geometry: polygon(&[(0.0, 0.0), (17.0, 0.0), (34.0, 0.5)], false),
            fill_rule: FillRule::NonZero,
        },
    ];
    for input in &paths {
        builder.build_path(parent, input).expect("path");
    }
    for (y, w) in [(13.0, 18.0), (19.0, 12.0)] {
        let input = TextInput {
            node: node_input(source, 0.0, y, true, true, 0.0),
            layout: TextLayoutKey::from_bytes([y as u8; 16]),
            size: size(w, 5.0),
        };
        builder.build_text(parent, &input).expect("text");
    }
    let image = ImageInput {
        node: node_input(source, 19.0, 13.0, true, false, 0.0),
        asset: AssetRef::new(StableId::from_bytes([7; 16])),
        size: size(8.0, 11.0),
        fit: ImageFit::Cover,
        crop: ImageCrop::FULL,
        sampling: ImageSampling::Linear,
        color_space: None,
    };
    builder.build_image(parent, &image).expect("image");
    builder.finish_frame(root, scope).expect("finish frame");
}

struct BenchScene {
    snapshot: SceneSnapshot,
    viewport: Viewport,
    total_nodes: u64,
}

fn bench_scene() -> BenchScene {
    let mut builder = SceneBuilder::new();
    let mut root = builder.root_scope();
    let mut source = 0u64;
    let (cell_w, cell_h) = (VIEW_W / COLS as f32, VIEW_H / ROWS as f32);
    for row in 0..ROWS {
        for col in 0..COLS {
            let (x, y) = (col as f32 * cell_w + 1.0, row as f32 * cell_h + 1.0);
            card(&mut builder, &mut root, &mut source, x, y, true);
        }
    }
    // 숨김 card는 가시 card 위에 겹쳐 visibility 판정을 거치게 합니다.
    for i in 0..HIDDEN_CARDS {
        let (x, y) = (
            (i % COLS) as f32 * cell_w + 1.0,
            (i / COLS) as f32 * cell_h + 1.0,
        );
        card(&mut builder, &mut root, &mut source, x, y, false);
    }
    for i in 0..OFFSCREEN_CARDS {
        card(
            &mut builder,
            &mut root,
            &mut source,
            3_000.0 + i as f32 * 40.0,
            100.0,
            true,
        );
    }
    let total_nodes = builder.built_count();
    let snapshot = SceneSnapshot::new(
        stellar_core_types::revision::DocumentRevision(1),
        stellar_core_types::revision::SceneRevision(1),
        builder.finish(root),
    )
    .expect("snapshot");
    let viewport = Viewport::new(size(VIEW_W, VIEW_H), Transform2D::identity()).expect("viewport");
    BenchScene {
        snapshot,
        viewport,
        total_nodes,
    }
}

/// nearest-rank 백분위수.
fn percentile(sorted: &[u128], q: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = ((q * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
    sorted[rank - 1]
}

fn stats_json(values: &mut [u128]) -> String {
    values.sort_unstable();
    let total: u128 = values.iter().sum();
    let mean = if values.is_empty() {
        0
    } else {
        total / values.len() as u128
    };
    format!(
        "{{\"p50\":{},\"p95\":{},\"p99\":{},\"max\":{},\"mean\":{},\"total\":{}}}",
        percentile(values, 0.50),
        percentile(values, 0.95),
        percentile(values, 0.99),
        values.last().copied().unwrap_or(0),
        mean,
        total
    )
}

struct Report {
    json: String,
    visible: usize,
    queries: usize,
    candidates: usize,
    broad_hits: usize,
    hits: usize,
    p99_ns: u128,
}

fn run(scene: &BenchScene, queries: usize, warmup: usize, seed: u64) -> Report {
    let build_start = Instant::now();
    let index = SceneSpatialIndex::build(&scene.snapshot);
    let index_build = build_start.elapsed();
    let draw = visible_candidates(&index, &scene.viewport);
    let options = NarrowPhaseOptions::with_tolerance(VIEW_TOLERANCE).expect("tolerance");
    let mut rng = SplitMix64(seed);
    let points: Vec<Vec2> = (0..warmup + queries)
        .map(|_| v(rng.unit(VIEW_W), rng.unit(VIEW_H)))
        .collect();

    let mut narrow_ns = Vec::with_capacity(queries);
    let mut broad_ns = Vec::with_capacity(queries);
    let mut candidate_counts = Vec::with_capacity(queries);
    let (mut candidates, mut broad_hits, mut hits, mut queries_with_hit) = (0usize, 0, 0, 0);
    let (mut hidden, mut pass_through, mut content_missed, mut path_clipped) = (0usize, 0, 0, 0);
    for (i, point) in points.iter().enumerate() {
        let start = Instant::now();
        let result =
            hit_test_view_narrow(&index, &scene.viewport, *point, &options, &LayoutBoxesOnly);
        let elapsed: Duration = start.elapsed();
        let start = Instant::now();
        let broad = hit_test_broad(&index, *point);
        let broad_elapsed = start.elapsed();
        if i < warmup {
            continue;
        }
        narrow_ns.push(elapsed.as_nanos());
        broad_ns.push(broad_elapsed.as_nanos());
        candidates += result.stats.candidates;
        candidate_counts.push(result.stats.candidates as u128);
        broad_hits += broad.hits.len();
        hits += result.hits.len();
        queries_with_hit += usize::from(!result.hits.is_empty());
        hidden += result.stats.hidden;
        pass_through += result.stats.pass_through;
        content_missed += result.stats.content_missed;
        path_clipped += result.stats.path_clipped;
    }
    let mut sorted = narrow_ns.clone();
    sorted.sort_unstable();
    let p99_ns = percentile(&sorted, 0.99);

    let mut json = String::new();
    let _ = write!(
        json,
        "{{\"benchmark\":\"scene_hit_test\",\"wbs\":\"XI.3.6\",\"profile\":\"{}\",\"seed\":{},",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        seed
    );
    let _ = write!(
        json,
        "\"scene\":{{\"nodes\":{},\"indexed_nodes\":{},\"visible_nodes\":{},\"hidden_cards\":{},\"offscreen_cards\":{},\"viewport\":[{},{}],\"view_tolerance_px\":{}}},",
        scene.total_nodes,
        index.len(),
        draw.items.len(),
        HIDDEN_CARDS,
        OFFSCREEN_CARDS,
        VIEW_W,
        VIEW_H,
        VIEW_TOLERANCE
    );
    let _ = write!(
        json,
        "\"query_count\":{},\"warmup_count\":{},\"candidate_count\":{{\"total\":{},\"per_query\":{}}},",
        queries,
        warmup,
        candidates,
        stats_json(&mut candidate_counts)
    );
    let _ = write!(
        json,
        "\"hit_count\":{{\"queries_with_hit\":{},\"narrow_hits\":{},\"broad_hits\":{},\"hidden\":{},\"pass_through\":{},\"content_missed\":{},\"path_clipped\":{}}},",
        queries_with_hit, hits, broad_hits, hidden, pass_through, content_missed, path_clipped
    );
    let _ = write!(
        json,
        "\"duration_ns\":{},\"broad_only_duration_ns\":{},\"index_build_ns\":{},\"budget_ns\":{{\"target\":{},\"limit\":{}}}}}",
        stats_json(&mut narrow_ns),
        stats_json(&mut broad_ns),
        index_build.as_nanos(),
        BUDGET_TARGET_NS,
        BUDGET_LIMIT_NS
    );
    Report {
        json,
        visible: draw.items.len(),
        queries,
        candidates,
        broad_hits,
        hits,
        p99_ns,
    }
}

fn emit(report: &Report) {
    println!("{}", report.json);
    if let Ok(path) = std::env::var("STELLAR_HIT_TEST_BENCH_OUT") {
        std::fs::write(&path, format!("{}\n", report.json)).expect("benchmark JSON 쓰기");
    }
}

#[test]
fn benchmark_json_reports_counts_and_durations_on_20k_visible_scene() {
    // 검증 기준(XI.3.6): 결과 JSON에 query count·candidate count·duration.
    let scene = bench_scene();
    let report = run(&scene, 500, 50, 0x3606);
    assert_eq!(
        scene.total_nodes,
        ((COLS * ROWS + HIDDEN_CARDS + OFFSCREEN_CARDS) * NODES_PER_CARD) as u64
    );
    assert_eq!(report.visible, 20_000, "가시 node 수");
    assert_eq!(report.queries, 500);
    // 모든 점이 card 격자 안이라 query마다 후보가 있습니다. narrow는 허용 거리 2 px로 후보를 넓히므로
    // 허용 거리 0인 broad 기준선보다 적중이 많을 수 있습니다(비교하지 않음).
    assert!(report.candidates >= report.queries);
    assert!(report.hits > 0 && report.broad_hits > 0);
    for key in [
        "\"query_count\":500",
        "\"candidate_count\":{\"total\":",
        "\"per_query\":{\"p50\":",
        "\"duration_ns\":{\"p50\":",
        "\"p99\":",
        "\"visible_nodes\":20000",
        "\"budget_ns\":{\"target\":350000,\"limit\":750000}",
    ] {
        assert!(report.json.contains(key), "{key} 없음: {}", report.json);
    }
    assert!(report.json.starts_with('{') && report.json.ends_with('}'));
    assert_eq!(
        report.json.matches('{').count(),
        report.json.matches('}').count()
    );
    // 같은 seed는 같은 질의·집계(시간 제외).
    let again = run(&scene, 500, 50, 0x3606);
    assert_eq!(
        (again.candidates, again.hits, again.broad_hits),
        (report.candidates, report.hits, report.broad_hits)
    );
    assert!(report.p99_ns > 0);
}

#[test]
#[ignore = "release 측정 전용: cargo test --release -p stellar-scene --test hit_test_benchmark -- --ignored --nocapture"]
fn hit_test_p99_on_20k_visible_nodes() {
    let scene = bench_scene();
    let report = run(&scene, 20_000, 2_000, 0x3606);
    assert_eq!(report.visible, 20_000);
    emit(&report);
}

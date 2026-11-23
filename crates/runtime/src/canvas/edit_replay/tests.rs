//! 판정·JSON 단위 test와 2만 가시 node 측정(release, ignored).

use super::*;
use std::time::Instant;

use stellar_commands::command::Command;
use stellar_commands::selection::{Selection, WindowViewState};
use stellar_commands::style_defaults::SetLayoutProperty;
use stellar_commands::view_tabs::CanvasViewport;
use stellar_core_types::value::{Size2, Vec2};
use stellar_document::layout_record::{LayoutLonghand, LayoutValue};
use stellar_document::model::DocumentModel;
use stellar_document::shape::{RectangleParams, ShapeGeometry};
use stellar_scene::narrow_phase::{LayoutBoxesOnly, NarrowPhaseOptions};

use crate::app::gesture_math::{Modifiers as SizeModifiers, ResizeHandle};
use crate::canvas::handles::{build_handles, HandleTarget};
use crate::canvas::hit::{point_stack, CanvasGeometry};
use crate::canvas::navigate::Navigator;
use crate::canvas::path_edit::{path_world, PathTarget, VectorEditMode};
use crate::canvas::select::{resolve_click, EditingScope, SelectTree};
use crate::canvas::select_area::Marquee;
use crate::canvas::select_fixtures::scene_of;
use crate::canvas::select_hover::hover_target;
use crate::canvas::snap::SnapSettings;
use crate::canvas::tool::Modifiers;
use crate::canvas::transform::drag::MoveDrag;
use crate::canvas::transform::fixtures::{add_shape, at, canvas_document, open, scene, select};
use crate::canvas::transform::resize::ResizeGesture;
use crate::canvas::transform::rotate::RotateGesture;
use crate::input_gesture::{GesturePhase, MomentumPhase, NavigationInput};
use crate::window_session::test_fixtures::{sid, window};

fn result(scenario: CanvasEditScenario, us: u64, n: usize) -> ScenarioResult {
    ScenarioResult::from_samples(scenario, vec![us * 1_000; n])
}

#[test]
fn budgets_and_verdicts_follow_the_plan_row() {
    // 검증 기준(XII.10.43): hover·선택·핸들 표시는 목표 5ms·상한 8ms, 나머지는 10ms.
    use CanvasEditScenario::*;
    for s in [Hover, ClickSelect, HandleDisplay] {
        assert_eq!(
            s.budget(),
            Budget {
                target_us: Some(5_000),
                limit_us: 8_000
            }
        );
    }
    for s in [
        Marquee,
        SnapMove,
        FlexResize,
        Rotate,
        PathPointDrag,
        Pan,
        Zoom,
        ImeComposition,
    ] {
        assert_eq!(s.budget().limit_us, 10_000);
    }
    assert_eq!(
        judge(&result(Hover, 4_000, 1_000)),
        Verdict::Pass {
            within_target: true
        }
    );
    assert_eq!(
        judge(&result(Hover, 6_000, 1_000)),
        Verdict::Pass {
            within_target: false
        }
    );
    assert_eq!(judge(&result(Hover, 9_000, 1_000)), Verdict::OverLimit);
    assert_eq!(
        judge(&result(Rotate, 9_000, 1_000)),
        Verdict::Pass {
            within_target: true
        }
    );
    assert_eq!(judge(&result(Rotate, 1, 999)), Verdict::TooFewSamples);
    assert_eq!(
        judge(&ScenarioResult::not_connected(ImeComposition)),
        Verdict::NotConnected
    );
    assert!(!Verdict::NotConnected.passed());
    // nearest-rank p99: 1,000개 중 990번째.
    let mut samples: Vec<u64> = (1..=1_000).collect();
    samples.reverse();
    let r = ScenarioResult::from_samples(Marquee, samples);
    assert_eq!((r.p50_ns, r.p99_ns, r.max_ns), (500, 990, 1_000));
    let json = to_json(
        "canvas/edit-20k",
        &[r, ScenarioResult::not_connected(ImeComposition)],
    );
    assert!(json.starts_with(
        "{\"schema\":\"stellar.canvas-edit-replay.v1\",\"fixture\":\"canvas/edit-20k\""
    ));
    assert!(json.contains("\"scenario\":\"marquee\",\"samples\":1000,\"p50_us\":0.500"));
    assert!(json.contains("\"scenario\":\"ime_composition\",\"samples\":0"));
    assert!(json.contains("\"verdict\":\"not_connected\""));
    assert!(json.ends_with("]}"));
}

/// 2만 가시 node fixture: root 10(20,000×20,000) 아래 Rectangle 19,900개 격자, flex Frame 11 아래 100개, Path 40.
fn fixture() -> DocumentModel {
    let mut doc = canvas_document();
    doc.set_payload(
        sid(10),
        crate::canvas::transform::fixtures::frame(20_000., 20_000., at(0., 0.)),
    )
    .unwrap();
    let rect = || {
        ShapeGeometry::Rectangle(RectangleParams {
            size: Size2::new(40., 40.).unwrap(),
            corner_radius: Default::default(),
        })
    };
    for i in 0..19_900u64 {
        let (x, y) = (
            (i % 200) as f32 * 100. + 500.,
            (i / 200) as f32 * 100. + 500.,
        );
        add_shape(&mut doc, 100_000 + i, 10, rect(), at(x, y));
    }
    for i in 0..100u64 {
        add_shape(&mut doc, 50_000 + i, 11, rect(), at(i as f32 * 2., 0.));
    }
    doc
}

fn path_into(doc: &mut DocumentModel) {
    use stellar_core_types::clock::ManualClock;
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::ids::StableIdGenerator;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::value::FiniteF32;
    use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierPayload, ModifierStack};
    use stellar_document::path::{IdentifiedPath, PathCommand, PathData, WindingRule};
    use stellar_document::records::NodePayload;
    let v = |x: f32, y: f32| Vec2::new(x, y).unwrap();
    let data = PathData::new(
        WindingRule::NonZero,
        vec![
            PathCommand::MoveTo(v(0., 0.)),
            PathCommand::LineTo(v(40., 0.)),
            PathCommand::LineTo(v(40., 40.)),
            PathCommand::Close,
        ],
    )
    .unwrap();
    let mut generator = StableIdGenerator::new(43);
    let clock = ManualClock::new(5);
    let path = IdentifiedPath::assign(data, &mut || generator.next_id(&clock));
    let mut record = doc.record(sid(12)).unwrap();
    record.id = sid(40);
    record.parent = Some(sid(10));
    record.order_key = doc.next_child_order_key(sid(10));
    record.payload = NodePayload::Path(path);
    doc.insert_node(record).unwrap();
    let mut stack = ModifierStack::new(sid(40));
    let black = Color::new(ColorSpace::Srgb, 0., 0., 0., 1.).unwrap();
    for (n, payload) in [
        ModifierPayload::Transform(at(300., 300.)),
        ModifierPayload::Stroke {
            color: black,
            width: FiniteF32::new(1.).unwrap(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(sid(4100 + n as u64)),
                order: OrderKey::from_bytes(vec![0x40 + n as u8]).unwrap(),
                kind: payload.kind(),
                enabled: true,
                payload,
            })
            .unwrap();
    }
    doc.set_modifier_stack(stack).unwrap();
}

fn measure(mut run: impl FnMut(usize)) -> Vec<u64> {
    (0..MIN_SAMPLES)
        .map(|i| {
            let start = Instant::now();
            run(i);
            start.elapsed().as_nanos() as u64
        })
        .collect()
}

/// 격자 안 점(Rectangle 위와 사이를 번갈아).
fn grid_point(i: usize) -> [f32; 2] {
    let k = (i * 7_919) % 19_900;
    let (x, y) = (
        (k % 200) as f32 * 100. + 520.,
        (k / 200) as f32 * 100. + 520.,
    );
    if i % 2 == 0 {
        [x, y]
    } else {
        [x + 50., y + 50.]
    }
}

#[test]
#[ignore = "release 빌드에서 2만 가시 node 캔버스 편집 replay를 측정"]
fn measure_canvas_edit_replay_20k() {
    // 검증 기준(XII.10.43): scenario별 최소 1,000 sample 결과 JSON, 판정 통과.
    use CanvasEditScenario as S;
    let mut doc = fixture();
    path_into(&mut doc);
    let mut results = Vec::new();

    // 선택 엔진 scenario: 문서 scene·공간 인덱스(항등 camera).
    let built = scene_of(&doc, sid(10));
    let tree = SelectTree::new(&doc, sid(10));
    let geometry = CanvasGeometry::from_snapshot(&built.snapshot);
    let scope = EditingScope::top(&tree);
    let stack_at = |p: [f32; 2]| {
        point_stack(
            &built.index,
            &built.viewport,
            Vec2::new(p[0], p[1]).unwrap(),
            &NarrowPhaseOptions::EXACT,
            &LayoutBoxesOnly,
        )
    };
    // fixture 확인: 격자 점은 Rectangle을 맞추고 마퀴 후보는 2만 개 가까이입니다.
    let leaf = stack_at(grid_point(0)).leaf().unwrap();
    assert!(leaf >= sid(100_000) && leaf < sid(120_000), "{leaf:?}");
    assert!(geometry.boxes(sid(119_899)).len() == 1);
    results.push(ScenarioResult::from_samples(
        S::Hover,
        measure(|i| {
            let stack = stack_at(grid_point(i));
            std::hint::black_box(hover_target(&tree, scope, &stack, Modifiers::NONE));
        }),
    ));
    let current = Selection::empty();
    results.push(ScenarioResult::from_samples(
        S::ClickSelect,
        measure(|i| {
            let stack = stack_at(grid_point(i));
            std::hint::black_box(resolve_click(
                &tree,
                scope,
                &current,
                &stack,
                Modifiers::NONE,
            ));
        }),
    ));
    results.push(ScenarioResult::from_samples(
        S::HandleDisplay,
        measure(|i| {
            let node = sid(100_000 + (i as u64 * 7_919) % 19_900);
            let target = HandleTarget::of(&tree, &geometry, node).unwrap();
            std::hint::black_box(build_handles(&[target], Some(node), &built.viewport));
        }),
    ));
    let mut view = WindowViewState::new(window(9));
    let mut marquee = Marquee::begin(
        &tree,
        scope,
        &geometry,
        &mut view,
        Vec2::new(400., 400.).unwrap(),
        Modifiers::NONE,
    )
    .unwrap();
    assert!(marquee.candidate_count() >= 19_900);
    results.push(ScenarioResult::from_samples(
        S::Marquee,
        measure(|i| {
            let t = 420. + i as f32 * 10.;
            marquee.update(&mut view, Vec2::new(t, t).unwrap()).unwrap();
        }),
    ));
    marquee.cancel(&mut view).unwrap();

    // 창 session scenario: 게시된 캔버스 scene.
    let (_dir, mut w) = open("edit-replay-20k", &doc);
    let flex = vec![Command::SetLayoutProperty(SetLayoutProperty {
        node: sid(11),
        longhand: LayoutLonghand::Display,
        value: LayoutValue::Keyword("flex"),
    })];
    crate::canvas::transform::commit(&mut w, flex, 0).unwrap();
    let s = scene(&w);

    select(&mut w, &[100_000 + 10_050]);
    let start = grid_point(0);
    let drag = MoveDrag::begin(&w, &s, None, start, false, None).unwrap();
    let snapping = Some((
        SnapSettings {
            objects: true,
            pixels: true,
        },
        5.0,
    ));
    results.push(ScenarioResult::from_samples(
        S::SnapMove,
        measure(|i| {
            let p = [
                start[0] + (i % 300) as f32 * 0.7,
                start[1] + (i % 200) as f32 * 0.3,
            ];
            std::hint::black_box(drag.update(p, false, snapping));
        }),
    ));

    select(&mut w, &[50_050]);
    let resize = ResizeGesture::begin(&w, &s, ResizeHandle::BottomRight).unwrap();
    results.push(ScenarioResult::from_samples(
        S::FlexResize,
        measure(|i| {
            let p = [40. + (i % 100) as f32, 40. + (i % 50) as f32];
            std::hint::black_box(resize.update(p, SizeModifiers::default()));
        }),
    ));

    select(&mut w, &[100_000 + 777]);
    let rotate = RotateGesture::begin(&w, &s, [0., 0.]).unwrap();
    results.push(ScenarioResult::from_samples(
        S::Rotate,
        measure(|i| {
            let a = i as f32 * 0.01;
            std::hint::black_box(rotate.update([a.cos() * 300., a.sin() * 300.], i % 2 == 0));
        }),
    ));

    let model = w.document().unwrap().model();
    let mode = VectorEditMode::enter(model, sid(40)).unwrap();
    let world = path_world(model, &s, sid(40)).unwrap();
    let PathTarget::Point(_) = mode.hit(model, world, 1.0, [340., 300.]).unwrap() else {
        panic!("point")
    };
    let mut point = mode
        .begin_drag(
            model,
            world,
            mode.hit(model, world, 1.0, [340., 300.]).unwrap(),
            [340., 300.],
        )
        .unwrap();
    results.push(ScenarioResult::from_samples(
        S::PathPointDrag,
        measure(|i| {
            std::hint::black_box(point.update([340. + (i % 60) as f32, 300. + (i % 40) as f32]));
        }),
    ));

    let size = [1440.0, 900.0];
    let mut nav = Navigator::new();
    let mut viewport = CanvasViewport::DEFAULT;
    results.push(ScenarioResult::from_samples(
        S::Pan,
        measure(|i| {
            let input = NavigationInput::Scroll {
                delta: [((i % 7) as f64) - 3.0, ((i % 5) as f64) - 2.0],
                precise: true,
                phase: Some(GesturePhase::Changed),
                momentum: MomentumPhase::None,
                modifiers: Modifiers::NONE,
                location: [700.0, 400.0],
            };
            if let Some(next) = nav.apply(viewport, size, input, None) {
                viewport = next;
            }
            std::hint::black_box(viewport.world_to_view(1440.0, 900.0));
        }),
    ));
    results.push(ScenarioResult::from_samples(
        S::Zoom,
        measure(|i| {
            let input = NavigationInput::Magnify {
                phase: GesturePhase::Changed,
                magnification: if i % 2 == 0 { 0.05 } else { -0.048 },
                location: [700.0, 400.0],
            };
            if let Some(next) = nav.apply(viewport, size, input, None) {
                viewport = next;
            }
            std::hint::black_box(viewport.world_to_view(1440.0, 900.0));
        }),
    ));
    results.push(ScenarioResult::not_connected(S::ImeComposition));

    let json = to_json("canvas/edit-20k", &results);
    println!("{json}");
    if let Ok(path) = std::env::var("STELLAR_CANVAS_REPLAY_OUT") {
        std::fs::write(path, &json).unwrap();
    }
    for r in &results {
        let verdict = judge(r);
        if r.scenario == S::ImeComposition {
            assert_eq!(verdict, Verdict::NotConnected);
            continue;
        }
        assert_eq!(r.samples, MIN_SAMPLES);
        assert!(
            verdict.passed(),
            "{:?}: {:?} p99 {}ns",
            r.scenario,
            verdict,
            r.p99_ns
        );
    }
}

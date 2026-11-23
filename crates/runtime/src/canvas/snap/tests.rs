use super::*;

const ON: SnapSettings = SnapSettings {
    objects: true,
    pixels: true,
};
const OBJECTS: SnapSettings = SnapSettings {
    objects: true,
    pixels: false,
};

fn parent() -> SnapCandidate {
    SnapCandidate {
        rect: [0.0, 0.0, 400.0, 300.0],
        kind: CandidateKind::Parent,
    }
}

fn sibling(rect: [f32; 4]) -> SnapCandidate {
    SnapCandidate {
        rect,
        kind: CandidateKind::Sibling,
    }
}

fn candidates(siblings: &[[f32; 4]]) -> SnapCandidates {
    let mut boxes = vec![parent()];
    boxes.extend(siblings.iter().map(|r| sibling(*r)));
    SnapCandidates::new(boxes, [0.0; 4])
}

#[test]
fn move_prefers_parent_then_sibling_edges_and_centers() {
    let set = candidates(&[[200.0, 50.0, 50.0, 50.0]]);
    // 부모 왼쪽 가장자리(0)에 3px 안.
    let r = snap([3.0, 120.0, 40.0, 40.0], SnapMode::Move, 6.0, OBJECTS, &set).unwrap();
    assert_eq!(r.rect[0], 0.0);
    assert_eq!(r.guides[0], Some(0.0));
    assert_eq!(r.guide_kind[0], Some(CandidateKind::Parent));
    // 형제 위 가장자리(50)에 맞춥니다.
    let r = snap(
        [100.0, 52.0, 40.0, 40.0],
        SnapMode::Move,
        6.0,
        OBJECTS,
        &set,
    )
    .unwrap();
    assert_eq!((r.rect[1], r.guides[1]), (50.0, Some(50.0)));
    assert_eq!(r.guide_kind[1], Some(CandidateKind::Sibling));
    // 부모 중심(200)과 형제 가장자리(200)가 같은 거리면 부모가 먼저입니다.
    let r = snap(
        [178.0, 120.0, 40.0, 40.0],
        SnapMode::Move,
        6.0,
        OBJECTS,
        &set,
    )
    .unwrap();
    assert_eq!(r.rect[0], 180.0);
    assert_eq!(r.guide_kind[0], Some(CandidateKind::Parent));
    // 허용 오차 밖이면 그대로입니다.
    let r = snap(
        [20.0, 120.0, 40.0, 40.0],
        SnapMode::Move,
        6.0,
        OBJECTS,
        &set,
    )
    .unwrap();
    assert_eq!(r.rect, [20.0, 120.0, 40.0, 40.0]);
    assert_eq!(r.guides, [None, None]);
}

#[test]
fn guides_then_layout_grid_win_over_objects_and_are_used_without_object_snap() {
    let lines = SnapLines {
        guides: [vec![105.0], vec![]],
        grid: [vec![102.0, 98.0], vec![61.0]],
    };
    let set = SnapCandidates::with_lines(
        vec![parent(), sibling([100.0, 60.0, 10.0, 10.0])],
        lines,
        [0.0; 4],
    );
    // 형제 가장자리(100) 1px, 그리드(102) 1px, 가이드(105) 4px: 가이드가 이깁니다.
    let r = snap([101.0, 0.0, 30.0, 30.0], SnapMode::Move, 6.0, OBJECTS, &set).unwrap();
    assert_eq!(r.rect[0], 105.0);
    assert_eq!(r.guide_kind[0], Some(CandidateKind::Guide));
    // 가이드가 허용 오차 밖이면 그리드(98, 102 중 가까운 쪽)가 형제보다 먼저입니다.
    let r = snap([99.5, 0.0, 30.0, 30.0], SnapMode::Move, 3.0, OBJECTS, &set).unwrap();
    assert_eq!(r.rect[0], 98.0);
    assert_eq!(r.guide_kind[0], Some(CandidateKind::GridLine));
    // 객체 스냅을 꺼도 선 후보는 씁니다.
    let off = SnapSettings {
        objects: false,
        pixels: false,
    };
    let r = snap([200.0, 59.0, 30.0, 30.0], SnapMode::Move, 3.0, off, &set).unwrap();
    assert_eq!(
        (r.rect[1], r.guide_kind[1]),
        (61.0, Some(CandidateKind::GridLine))
    );
}

#[test]
fn move_matches_equal_spacing_and_reports_distance() {
    // 형제 두 개 사이 간격 30. 오른쪽 형제와 28 떨어져 있으면 30으로 맞춥니다.
    let set = candidates(&[[10.0, 100.0, 40.0, 40.0], [80.0, 100.0, 40.0, 40.0]]);
    assert_eq!(set.gaps[0], vec![30.0]);
    let r = snap(
        [148.0, 100.0, 40.0, 40.0],
        SnapMode::Move,
        3.0,
        OBJECTS,
        &set,
    )
    .unwrap();
    assert_eq!(r.rect[0], 150.0);
    assert_eq!(r.spacing[0], Some(30.0));
    assert_eq!(r.guides[0], None);
    assert_eq!(r.distance[0], Some(30.0));
}

#[test]
fn pixel_snap_rounds_to_integer_parent_px_only_without_object_hit() {
    let set = candidates(&[]);
    let r = snap(
        [100.4, 200.6, 10.0, 10.0],
        SnapMode::Move,
        1.0,
        SnapSettings {
            objects: false,
            pixels: true,
        },
        &set,
    )
    .unwrap();
    assert_eq!(&r.rect[..2], &[100.0, 201.0]);
    assert_eq!(r.guides, [None, None]);
    let r = snap([0.4, 0.0, 10.0, 10.0], SnapMode::Move, 1.0, ON, &set).unwrap();
    assert_eq!((r.rect[0], r.guides[0]), (0.0, Some(0.0)));
}

fn resize(handle: ResizeHandle, shift: bool, option: bool) -> SnapMode {
    SnapMode::Resize {
        handle,
        modifiers: Modifiers { shift, option },
    }
}

#[test]
fn resize_size_match_is_not_an_alignment_guide() {
    let set = candidates(&[[300.0, 10.0, 60.0, 30.0]]);
    // 오른쪽 변을 끌어 너비 58 → 형제와 같은 60.
    let r = snap(
        [100.0, 100.0, 58.0, 20.0],
        resize(ResizeHandle::Right, false, false),
        3.0,
        OBJECTS,
        &set,
    )
    .unwrap();
    assert_eq!(r.rect, [100.0, 100.0, 60.0, 20.0]);
    assert_eq!(r.size_match[0], Some(60.0));
    assert_eq!(r.guides, [None, None]);
}

#[test]
fn resize_keeps_option_center_and_skips_shift_ratio() {
    let set = candidates(&[]);
    // 중심 기준: 오른쪽 변을 부모 오른쪽(400)에 맞추면 왼쪽 변도 같은 거리만큼 움직여 중심(300)을 지킵니다.
    let r = snap(
        [202.0, 100.0, 196.0, 20.0],
        resize(ResizeHandle::Right, false, true),
        3.0,
        OBJECTS,
        &set,
    )
    .unwrap();
    assert_eq!(r.rect, [200.0, 100.0, 200.0, 20.0]);
    assert_eq!(r.rect[0] + r.rect[2] / 2.0, 300.0);
    // 비율 유지는 스냅하지 않습니다.
    let rect = [202.0, 100.0, 196.0, 98.0];
    let r = snap(
        rect,
        resize(ResizeHandle::BottomRight, true, false),
        3.0,
        ON,
        &set,
    )
    .unwrap();
    assert_eq!(r.rect, rect);
    // 왼쪽 위 핸들은 옮기는 변(왼쪽·위)만 맞춥니다.
    let r = snap(
        [2.0, 1.0, 98.0, 99.0],
        resize(ResizeHandle::TopLeft, false, false),
        3.0,
        OBJECTS,
        &set,
    )
    .unwrap();
    assert_eq!(r.rect, [0.0, 0.0, 100.0, 100.0]);
}

#[test]
fn too_many_siblings_are_limited_to_the_nearest_instead_of_rejected() {
    let mut boxes = vec![parent()];
    for index in 0..(MAX_SIBLINGS + 100) {
        boxes.push(sibling([index as f32 * 10.0, 0.0, 5.0, 5.0]));
    }
    let set = SnapCandidates::new(boxes, [0.0, 0.0, 5.0, 5.0]);
    assert_eq!(set.boxes.len(), 1 + MAX_SIBLINGS);
    assert_eq!(set.boxes[0].kind, CandidateKind::Parent);
    assert!(set.boxes.iter().any(|c| c.rect[0] == 10.0));
    assert!(!set
        .boxes
        .iter()
        .any(|c| c.rect[0] == ((MAX_SIBLINGS + 99) as f32) * 10.0));
}

#[test]
fn invalid_input_is_rejected_and_tolerance_converts_screen_points() {
    let set = candidates(&[]);
    assert!(snap([f32::NAN, 0.0, 1.0, 1.0], SnapMode::Move, 6.0, ON, &set).is_none());
    assert!(snap([0.0, 0.0, -1.0, 1.0], SnapMode::Move, 6.0, ON, &set).is_none());
    assert!(snap([0.0, 0.0, 1.0, 1.0], SnapMode::Move, f32::NAN, ON, &set).is_none());
    assert_eq!(tolerance(6.0, 2.0), Some(3.0));
    assert_eq!(tolerance(6.0, 0.0), None);
}

#[test]
fn screen_tolerance_is_the_same_at_25_and_400_percent_zoom() {
    let set = candidates(&[[200.0, 0.0, 50.0, 50.0]]);
    for zoom in [0.25f32, 4.0] {
        let tol = tolerance(DEFAULT_TOLERANCE_PT, zoom).unwrap();
        // 화면에서 5pt 떨어진 상자는 맞고, 7pt 떨어진 상자는 맞지 않습니다.
        let near = 200.0 - 50.0 - 5.0 / zoom;
        let far = 200.0 - 50.0 - 7.0 / zoom;
        let r = snap(
            [near, 400.0, 50.0, 50.0],
            SnapMode::Move,
            tol,
            OBJECTS,
            &set,
        )
        .unwrap();
        assert_eq!(r.rect[0], 150.0, "{zoom}");
        let r = snap([far, 400.0, 50.0, 50.0], SnapMode::Move, tol, OBJECTS, &set).unwrap();
        assert_eq!(r.rect[0], far, "{zoom}");
    }
}

/// `canvas/snap-candidates-20k`: 부모 하나와 형제 2만 개(결정적 의사 난수 배치).
pub(crate) fn snap_candidates_20k() -> SnapCandidates {
    let mut boxes = vec![SnapCandidate {
        rect: [0.0, 0.0, 8000.0, 8000.0],
        kind: CandidateKind::Parent,
    }];
    let mut seed: u64 = 0x5eed_2000;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((seed >> 33) as f32) / (1u64 << 31) as f32
    };
    for _ in 0..20_000 {
        let x = (next() * 7900.0).round();
        let y = (next() * 7900.0).round();
        let w = 8.0 + (next() * 90.0).round();
        let h = 8.0 + (next() * 90.0).round();
        boxes.push(SnapCandidate {
            rect: [x, y, w, h],
            kind: CandidateKind::Sibling,
        });
    }
    let lines = SnapLines {
        guides: [vec![1000.5, 4000.5], vec![2500.5]],
        grid: [(0..12).map(|c| c as f32 * 666.0).collect(), vec![]],
    };
    SnapCandidates::with_lines(boxes, lines, [4000.0, 4000.0, 50.0, 50.0])
}

fn drag_path(step: usize) -> [f32; 4] {
    let t = step as f32;
    [
        (t * 7.37) % 7900.0,
        (t * 3.11 + 13.0) % 7900.0,
        25.0 + (step % 7) as f32,
        30.0,
    ]
}

#[test]
fn snap_candidates_20k_results_are_deterministic() {
    let a = snap_candidates_20k();
    let b = snap_candidates_20k();
    assert_eq!(a.boxes.len(), 20_001);
    for step in 0..400 {
        let rect = drag_path(step);
        let mode = if step % 2 == 0 {
            SnapMode::Move
        } else {
            resize(ResizeHandle::BottomRight, false, false)
        };
        let first = snap(rect, mode, 3.0, ON, &a).unwrap();
        assert_eq!(first, snap(rect, mode, 3.0, ON, &b).unwrap());
        assert_eq!(first, snap(rect, mode, 3.0, ON, &a).unwrap());
    }
}

#[test]
fn flow_children_get_no_object_candidates() {
    use crate::canvas::CanvasCache;
    use crate::window_session::test_fixtures::{
        identity, sample_document, saved_project_with, sid, window,
    };
    use crate::window_session::ProjectWindowSession;
    let model = sample_document();
    let dir = saved_project_with("snap-flow", &[&model]);
    let session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
    let scene = CanvasCache::default()
        .current(&session, None, &mut Default::default())
        .unwrap();
    let options = CollectOptions {
        flow_child: true,
        lines: SnapLines {
            guides: [vec![5.0], vec![]],
            grid: Default::default(),
        },
        ..CollectOptions::default()
    };
    let set = collect_in(&scene, &[sid(11)], &options).unwrap();
    assert!(set.boxes.is_empty());
    let r = snap([4.0, 0.0, 1.0, 1.0], SnapMode::Move, 3.0, ON, &set).unwrap();
    assert_eq!(r.guide_kind[0], Some(CandidateKind::Guide));
}

#[test]
fn collect_reads_parent_local_boxes_from_the_scene_and_excludes_moving_nodes() {
    use crate::canvas::CanvasCache;
    use crate::window_session::test_fixtures::{
        identity, sample_document, saved_project_with, sid, window,
    };
    use crate::window_session::ProjectWindowSession;
    use stellar_core_types::value::Size2;
    use stellar_document::frame::FramePayload;
    use stellar_document::records::NodePayload;
    let placed = |w: f32, h: f32, transform: Transform2D| {
        NodePayload::Frame(FramePayload {
            size: Size2::new(w, h).unwrap(),
            transform,
            ..FramePayload::default()
        })
    };
    let at = |x: f32, y: f32| Transform2D::new(1., 0., 0., 1., x, y).unwrap();
    let mut model = sample_document();
    // root 10(400×300) 아래 11(10,10 100×100)과 12(200,50 50×50, 90° 회전).
    model
        .set_payload(sid(10), placed(400., 300., at(0., 0.)))
        .unwrap();
    model
        .set_payload(sid(11), placed(100., 100., at(10., 10.)))
        .unwrap();
    let rotated = Transform2D::new(0., 1., -1., 0., 250., 50.).unwrap();
    model
        .set_payload(sid(12), placed(50., 20., rotated))
        .unwrap();
    let dir = saved_project_with("snap-collect", &[&model]);
    let session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
    let scene = CanvasCache::default()
        .current(&session, None, &mut Default::default())
        .unwrap();
    let set = collect(&scene, &[sid(11)]).unwrap();
    assert_eq!(set.boxes[0].kind, CandidateKind::Parent);
    assert_eq!(set.boxes[0].rect, [0.0, 0.0, 400.0, 300.0]);
    // 회전한 형제는 부모 local 경계 상자(x 230~250, y 50~100)입니다. 움직이는 11은 후보가 아닙니다.
    assert_eq!(set.boxes.len(), 2);
    let [x, y, w, h] = set.boxes[1].rect;
    assert!(
        (x - 230.0).abs() < 1e-3 && (y - 50.0).abs() < 1e-3,
        "{x} {y}"
    );
    assert!(
        (w - 20.0).abs() < 1e-3 && (h - 50.0).abs() < 1e-3,
        "{w} {h}"
    );
    // 가시 영역 밖 형제는 후보가 아닙니다.
    let far = CollectOptions {
        visible: Some([1000.0, 1000.0, 10.0, 10.0]),
        ..CollectOptions::default()
    };
    assert_eq!(collect_in(&scene, &[sid(11)], &far).unwrap().boxes.len(), 1);
    // root는 부모가 없어 스냅 후보를 만들지 않습니다.
    assert!(collect(&scene, &[sid(10)]).is_none());
}

#[test]
#[ignore = "release에서 형제 2만 개 후보의 update당 스냅 시간 측정(p99 0.35ms 예산)"]
fn measure_snap_with_20k_candidates() {
    let set = snap_candidates_20k();
    let mut times = Vec::new();
    for step in 0..5000 {
        let rect = drag_path(step);
        let mode = if step % 2 == 0 {
            SnapMode::Move
        } else {
            resize(ResizeHandle::Right, false, false)
        };
        let started = std::time::Instant::now();
        let r = snap(rect, mode, 3.0, ON, &set);
        times.push(started.elapsed());
        assert!(r.is_some());
    }
    times.sort();
    let p = |q: f64| times[((times.len() as f64 - 1.0) * q) as usize];
    eprintln!(
        "snap 2만 후보: 중앙값 {:?}, p99 {:?}, 최대 {:?}",
        p(0.5),
        p(0.99),
        times[times.len() - 1]
    );
    assert!(p(0.99) <= std::time::Duration::from_micros(350));
}

//! 부분 래스터화 회귀 테스트 (XX.4.4).
//!
//! 한 unsupported effect가 View 전체를 래스터화하지 않습니다. 검증 기준은 vector node 수 유지입니다: 래스터화되는 node는
//! effect가 걸린 가지뿐이고, 나머지 node는 SVG에 vector 요소로 남습니다.

use super::raster::{capture_of_snapshot, painted, write_raster, FakeRaster};
use super::*;
use crate::svg::raster::plan_raster;

#[test]
fn one_unsupported_effect_does_not_rasterize_the_whole_view() {
    // 검증 기준(XX.4.4): vector node count 유지. 사각형 60개 View에서 하나만 blur입니다.
    const COUNT: u64 = 60;
    const BLURRED: u64 = 31;
    let nodes: Vec<SceneNode> = (1..=COUNT)
        .map(|n| {
            let x = (n as f32) * 12.0;
            if n == BLURRED {
                SceneNode::leaf(painted(n, n, x, |p| {
                    p.layer_blur = BlurRadius::new(1.0).unwrap();
                }))
            } else {
                SceneNode::leaf(rect_spec(n, n, x))
            }
        })
        .collect();
    let capture = capture(snapshot_of(nodes), svg_request());
    let raster = FakeRaster::default();
    let document = write_raster(&capture, &raster).unwrap();
    validate(document.as_str());
    let plan = plan_raster(&capture);
    assert_eq!(plan.visible_nodes(), COUNT);
    assert_eq!(plan.rasterized_nodes(), 1);
    assert_eq!(plan.vector_nodes(), COUNT - 1);
    // SVG에도 vector 사각형 59개와 image 하나만 있습니다.
    assert_eq!(
        document.as_str().matches("<rect ").count() as u64,
        COUNT - 1
    );
    assert_eq!(document.raster_images(), 1);
    assert_eq!(document.groups(), COUNT);
    assert_eq!(
        raster.calls.borrow()[0].1,
        vec![(fx::node_id(BLURRED), false, 0)]
    );
    // 같은 View에 effect가 없으면 모두 vector입니다(기준선).
    let plain: Vec<SceneNode> = (1..=COUNT)
        .map(|n| SceneNode::leaf(rect_spec(n, n, (n as f32) * 12.0)))
        .collect();
    let baseline = write_raster(
        &capture_of_snapshot(snapshot_of(plain)),
        &FakeRaster::default(),
    )
    .unwrap();
    assert_eq!(baseline.as_str().matches("<rect ").count() as u64, COUNT);
    assert_eq!(baseline.raster_images(), 0);
}

#[test]
fn nested_effects_rasterize_only_the_affected_branch() {
    // frame(1) > [group(2) > [blur leaf(3), leaf(4)], leaf(5)], 형제 root(6): blur 가지(3)만 래스터.
    let leaf3 = painted(3, 3, 0.0, |p| p.layer_blur = BlurRadius::new(2.0).unwrap());
    let group = SceneNode::new(
        rect_spec(2, 2, 0.0),
        vec![
            Arc::new(SceneNode::leaf(leaf3)),
            Arc::new(SceneNode::leaf(rect_spec(4, 4, 30.0))),
        ],
    )
    .unwrap();
    let frame = SceneNode::new(
        rect_spec(1, 1, 0.0),
        vec![
            Arc::new(group),
            Arc::new(SceneNode::leaf(rect_spec(5, 5, 60.0))),
        ],
    )
    .unwrap();
    let capture = capture_of_snapshot(snapshot_of(vec![
        frame,
        SceneNode::leaf(rect_spec(6, 6, 90.0)),
    ]));
    let raster = FakeRaster::default();
    let document = write_raster(&capture, &raster).unwrap();
    validate(document.as_str());
    let plan = plan_raster(&capture);
    assert_eq!(plan.vector_nodes(), 5);
    assert_eq!(document.as_str().matches("<rect ").count(), 5);
    assert_eq!(raster.calls.borrow()[0].1, vec![(fx::node_id(3), false, 0)]);
}

#[test]
fn each_raster_feature_keeps_the_rest_of_the_view_vector() {
    // shadow(Content), blur(Subtree), 겹치는 앞 내용이 없는 blend(Backdrop) 각각 한 node만 래스터화합니다.
    const COUNT: u64 = 40;
    const TARGET: u64 = 20;
    let edits: [fn(&mut PaintStack); 3] = [
        |p| {
            p.shadows.push(Shadow::new(
                fx::p(1.0, 1.0),
                BlurRadius::new(1.0).unwrap(),
                fx::srgb(0.0, 0.0, 0.0, 0.5),
            ))
        },
        |p| p.layer_blur = BlurRadius::new(1.0).unwrap(),
        |p| p.blend = SceneBlendMode::Screen,
    ];
    for edit in edits {
        let nodes: Vec<SceneNode> = (1..=COUNT)
            .map(|n| {
                let x = (n as f32) * 20.0;
                if n == TARGET {
                    SceneNode::leaf(painted(n, n, x, edit))
                } else {
                    SceneNode::leaf(rect_spec(n, n, x))
                }
            })
            .collect();
        let capture = capture_of_snapshot(snapshot_of(nodes));
        let document = write_raster(&capture, &FakeRaster::default()).unwrap();
        validate(document.as_str());
        let plan = plan_raster(&capture);
        assert_eq!(plan.vector_nodes(), COUNT - 1);
        assert_eq!(document.raster_images(), 1);
        assert_eq!(
            document.as_str().matches("<rect ").count() as u64,
            COUNT - 1
        );
    }
}

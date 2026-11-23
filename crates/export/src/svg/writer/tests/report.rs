//! 래스터 report 테스트 (XX.4.3).
//!
//! 검증 기준은 report fixture 정확입니다. drop shadow(Content), 자손을 가진 layer blur(Subtrees), 앞 형제와 겹치는
//! blend(backdrop 범위)가 있는 View의 report 고정 표기가 golden과 같고, 구조 값(영역 종류·root·pixel·node 수·이유)이
//! planner 계획과 SVG 출력과 맞습니다.

use super::raster::{capture_of_snapshot, painted, write_raster, FakeRaster};
use super::*;
use crate::svg::raster::plan_raster;
use crate::svg::report::{BackdropReport, BackdropSource, RasterizedKind};

const GOLDEN_REPORT: &str = include_str!("../../../../tests/fixtures/raster_report_v1.txt");

/// 1: vector, 2: shadow, 3(자식 4): blur, 5: 6과 겹침, 6: multiply, 7: vector.
fn report_snapshot() -> SceneSnapshot {
    let shadow = painted(2, 2, 20.0, |p| {
        p.shadows.push(Shadow::new(
            fx::p(1.0, 1.0),
            BlurRadius::new(1.0).unwrap(),
            fx::srgb(0.0, 0.0, 0.0, 0.5),
        ));
    });
    let blur = painted(3, 3, 40.0, |p| p.layer_blur = BlurRadius::new(1.0).unwrap());
    let blurred =
        SceneNode::new(blur, vec![Arc::new(SceneNode::leaf(rect_spec(4, 4, 42.0)))]).unwrap();
    snapshot_of(vec![
        SceneNode::leaf(rect_spec(1, 1, 0.0)),
        SceneNode::leaf(shadow),
        blurred,
        SceneNode::leaf(rect_spec(5, 5, 70.0)),
        SceneNode::leaf(painted(6, 6, 75.0, |p| p.blend = SceneBlendMode::Multiply)),
        SceneNode::leaf(rect_spec(7, 7, 100.0)),
    ])
}

#[test]
fn report_lists_rasterized_subtrees_and_reasons() {
    let capture = capture_of_snapshot(report_snapshot());
    let document = write_raster(&capture, &FakeRaster::default()).unwrap();
    let report = document.report();
    let text = report.to_text();
    if std::env::var_os("STELLAR_EXPORT_UPDATE_GOLDEN").is_some() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/raster_report_v1.txt"
        );
        std::fs::write(path, &text).unwrap();
        return;
    }
    assert_eq!(text, GOLDEN_REPORT);
    // 보이는 7개 중 shadow 1 + blur 가지 2 + blend 범위 2가 래스터, 1·7이 vector입니다.
    assert_eq!(report.visible_nodes, 7);
    assert_eq!(report.rasterized_nodes, 5);
    assert_eq!(report.vector_nodes, 2);
    assert!(!report.is_fully_vector());
    let plan = plan_raster(&capture);
    assert_eq!(report.rasterized.len(), plan.boundaries().len());
    assert_eq!(report.rasterized.len(), document.raster_images());
    let [shadow, blur, blend] = report.rasterized.as_slice() else {
        panic!("영역 3개: {report:?}");
    };
    assert_eq!(shadow.kind, RasterizedKind::Content);
    assert_eq!(shadow.roots, vec![fx::node_id(2)]);
    assert_eq!(shadow.reasons[0].feature, SvgFeature::DropShadow);
    assert_eq!(
        shadow.reasons[0].css,
        SvgFeature::DropShadow.css_longhands()
    );
    assert_eq!(blur.kind, RasterizedKind::Subtrees { parent: None });
    assert_eq!(blur.roots, vec![fx::node_id(3)]);
    assert_eq!(blur.rasterized_nodes, 2);
    assert_eq!(blur.reasons[0].feature, SvgFeature::LayerBlur);
    assert_eq!(blend.roots, vec![fx::node_id(5), fx::node_id(6)]);
    assert_eq!(
        blend.reasons[0].backdrop,
        Some(BackdropReport {
            isolation: None,
            widest: Some(BackdropSource::Node(fx::node_id(5))),
        })
    );
    for (region, boundary) in report.rasterized.iter().zip(plan.boundaries()) {
        assert_eq!(region.pixels, boundary.pixels());
        assert!(!region.reasons.is_empty());
        assert!(region.reasons.iter().all(|r| !r.why.is_empty()));
    }
}

#[test]
fn vector_exports_report_no_rasterized_regions() {
    let document = write_snapshot(fx::basic_snapshot(&Basic::default())).unwrap();
    let report = document.report();
    assert!(report.is_fully_vector());
    assert_eq!(report.rasterized_nodes, 0);
    assert_eq!(report.vector_nodes, report.visible_nodes);
    assert_eq!(report.visible_nodes, 4);
    assert_eq!(report.to_text(), "nodes visible=4 vector=4 rasterized=0\n");
}

//! SVG vector 합성 효과 테스트 (XX.3.4).
//!
//! scene이 표현하는 clip·mask·opacity를 vector로 기록하는지 고정합니다.
//!
//! - golden: Inside·Center·Outside stroke, even-odd path Outside stroke, 반투명 색, group opacity, 둥근
//!   clip 안 중첩 clip을 가진 View의 SVG가 golden과 byte 단위로 같고 구조 검사를 통과합니다.
//! - mask: Outside stroke마다 node당 `<mask>` 하나를 쓰고, mask 영역이 가장 넓은 Outside stroke를 덮습니다.
//! - browser render: golden을 WebKit(macOS Quick Look)으로 렌더한 결과는 보고서(`docs/task-reports/XX.3.4.md`)에
//!   pixel 표본과 함께 기록합니다.

use super::*;
use stellar_scene::geometry::PathCommand;
use stellar_scene::node::PathPrimitive;

const GOLDEN_VECTOR: &str = include_str!("../../../../tests/fixtures/vector_effects_v1.svg");

fn stroke(color: Color, width: f32, align: StrokeAlign) -> PaintLayer {
    PaintLayer::Stroke(
        StrokeLayer::new(Paint::Solid(color), width, align, StrokeCap::Butt).unwrap(),
    )
}

fn fill(color: Color) -> PaintLayer {
    PaintLayer::Fill(Paint::Solid(color))
}

fn spec(
    n: u64,
    z: u64,
    content: SceneContent,
    transform: Transform2D,
    clip: ClipStackRef,
    paint: PaintStack,
) -> SceneNodeSpec {
    SceneNodeSpec {
        source: fx::node_id(n),
        content,
        world_transform: transform,
        layout_rect: fx::rect(0.0, 0.0, 10.0, 10.0),
        clip,
        paint: PaintStackRef::new(paint),
        z: ZOrder(z),
        hit: HitTestMeta::INTERACTIVE,
    }
}

fn layers(layers: Vec<PaintLayer>) -> PaintStack {
    PaintStack {
        layers,
        ..PaintStack::empty()
    }
}

fn ring() -> PathGeometry {
    // 바깥 사각형과 안쪽 사각형(같은 방향): even-odd면 구멍, non-zero면 채움.
    PathGeometry::new(vec![
        PathCommand::MoveTo(fx::p(0.0, 0.0)),
        PathCommand::LineTo(fx::p(30.0, 0.0)),
        PathCommand::LineTo(fx::p(30.0, 30.0)),
        PathCommand::LineTo(fx::p(0.0, 30.0)),
        PathCommand::Close,
        PathCommand::MoveTo(fx::p(10.0, 10.0)),
        PathCommand::LineTo(fx::p(20.0, 10.0)),
        PathCommand::LineTo(fx::p(20.0, 20.0)),
        PathCommand::LineTo(fx::p(10.0, 20.0)),
        PathCommand::Close,
    ])
    .unwrap()
}

/// Frame(id 1, 둥근 clip) 안에 Outside stroke 삼각형(2), 반투명 group(3) 안의 중첩 clip 사각형(4), even-odd
/// 고리(5)가 있는 View. 고리는 Frame clip 밖까지 stroke가 나가 clip이 자릅니다.
pub(super) fn vector_snapshot() -> SceneSnapshot {
    let frame_rect = fx::rect(0.0, 0.0, 120.0, 90.0);
    let radii = CornerRadii::new(12.0, 12.0, 12.0, 12.0).unwrap();
    let frame_clip = ClipStackRef::none().push(ClipShape::Rect {
        rect: frame_rect,
        radii,
        transform: fx::translate(10.0, 10.0),
    });
    let frame = spec(
        1,
        1,
        SceneContent::Rect(RectPrimitive {
            rect: frame_rect,
            radii,
        }),
        fx::translate(10.0, 10.0),
        ClipStackRef::none(),
        layers(vec![
            fill(fx::srgb(0.95, 0.95, 0.9, 1.0)),
            stroke(fx::srgb(0.2, 0.2, 0.2, 1.0), 3.0, StrokeAlign::Inside),
        ]),
    );
    let triangle = spec(
        2,
        2,
        SceneContent::Path(PathPrimitive {
            geometry: fx::triangle(),
            fill_rule: FillRule::NonZero,
        }),
        fx::translate(20.0, 20.0),
        frame_clip.clone(),
        layers(vec![
            fill(fx::srgb(1.0, 0.8, 0.0, 1.0)),
            stroke(fx::srgb(0.0, 0.0, 1.0, 0.5), 4.0, StrokeAlign::Outside),
            stroke(fx::srgb(0.0, 0.0, 0.0, 1.0), 1.0, StrokeAlign::Center),
        ]),
    );
    let inner_clip = frame_clip.push(ClipShape::Rect {
        rect: fx::rect(0.0, 0.0, 30.0, 20.0),
        radii: CornerRadii::ZERO,
        transform: fx::translate(70.0, 20.0),
    });
    let group = spec(
        3,
        3,
        SceneContent::Rect(RectPrimitive {
            rect: fx::rect(0.0, 0.0, 30.0, 20.0),
            radii: CornerRadii::ZERO,
        }),
        fx::translate(70.0, 20.0),
        frame_clip.clone(),
        PaintStack {
            opacity: Opacity::new(0.5).unwrap(),
            ..layers(vec![fill(fx::srgb(0.0, 0.6, 0.0, 1.0))])
        },
    );
    let clipped = spec(
        4,
        4,
        SceneContent::Rect(RectPrimitive {
            rect: fx::rect(0.0, 0.0, 40.0, 40.0),
            radii: CornerRadii::ZERO,
        }),
        fx::translate(80.0, 25.0),
        inner_clip,
        layers(vec![fill(fx::srgb(0.8, 0.0, 0.0, 0.75))]),
    );
    let ring = spec(
        5,
        5,
        SceneContent::Path(PathPrimitive {
            geometry: ring(),
            fill_rule: FillRule::EvenOdd,
        }),
        fx::translate(100.0, 65.0),
        frame_clip,
        layers(vec![
            fill(fx::srgb(0.5, 0.0, 0.5, 1.0)),
            stroke(fx::srgb(0.0, 0.5, 0.5, 1.0), 2.0, StrokeAlign::Outside),
            stroke(fx::srgb(0.0, 0.0, 0.0, 1.0), 6.0, StrokeAlign::Outside),
        ]),
    );
    let group = SceneNode::new(group, vec![Arc::new(SceneNode::leaf(clipped))]).unwrap();
    let frame = SceneNode::new(
        frame,
        vec![
            Arc::new(SceneNode::leaf(triangle)),
            Arc::new(group),
            Arc::new(SceneNode::leaf(ring)),
        ],
    )
    .unwrap();
    snapshot_of(vec![frame])
}

#[test]
fn vector_effects_match_golden() {
    let document = write_snapshot(vector_snapshot()).unwrap();
    if std::env::var_os("STELLAR_EXPORT_UPDATE_GOLDEN").is_some() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/vector_effects_v1.svg"
        );
        std::fs::write(path, document.as_str()).unwrap();
        return;
    }
    assert_eq!(document.as_str(), GOLDEN_VECTOR);
    let ids = validate(document.as_str());
    let svg = document.as_str();
    // 래스터 image 없이 모두 vector입니다.
    assert!(!svg.contains("<image"));
    assert_eq!(document.groups(), 5);
    // Outside stroke가 있는 node(2, 5)마다 mask 하나, 같은 node의 두 Outside stroke는 mask를 공유합니다.
    assert_eq!(document.mask_definitions(), 2);
    assert_eq!(svg.matches("<mask ").count(), 2);
    let ring_mask = format!("n-{}-m", fx::node_id(5));
    assert!(ids.contains(&ring_mask));
    assert_eq!(
        svg.matches(&format!("mask=\"url(#{ring_mask})\"")).count(),
        2
    );
    // mask 영역은 가장 넓은 Outside stroke(6)의 4배까지 덮고, 고리는 자기 채우기 규칙으로 잘립니다.
    assert!(svg.contains(&format!(
        "<mask id=\"{ring_mask}\" maskUnits=\"userSpaceOnUse\" x=\"-24\" y=\"-24\" width=\"78\" height=\"78\">"
    )));
    assert!(svg.contains("fill=\"rgb(0%,0%,0%)\" fill-rule=\"evenodd\"/>\n</mask>"));
    // Outside stroke는 2배 폭으로 그리고 mask가 안쪽 절반을 자릅니다.
    assert!(svg.contains("stroke-width=\"8\""));
    assert!(svg.contains("stroke-width=\"12\""));
    // group opacity와 반투명 색은 vector 속성입니다.
    assert!(svg.contains("opacity=\"0.5\">"));
    assert!(svg.contains("fill-opacity=\"0.75\""));
    assert!(svg.contains("stroke-opacity=\"0.5\""));
    // frame clip(자식 공유 1) + 중첩 clip(1) + Inside stroke 자기 clip(1).
    assert_eq!(document.clip_definitions(), 3);
}

#[test]
fn vector_effects_are_byte_identical() {
    let first = write_snapshot(vector_snapshot()).unwrap();
    let decoded =
        stellar_scene::codec::decode(&stellar_scene::codec::encode(&vector_snapshot())).unwrap();
    assert_eq!(
        write_snapshot(decoded).unwrap().as_bytes(),
        first.as_bytes()
    );
}

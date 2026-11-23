//! SVG writer 테스트 (XX.3.3).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::thread;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::reference::AssetRef;
use stellar_core_types::revision::{DocumentRevision, SceneRevision};
use stellar_core_types::shadow::{BlurRadius, Shadow};
use stellar_core_types::value::{Size2, Transform2D};
use stellar_scene::builder::{FrameClipInput, FrameInput, NodeInput, SceneBuilder, ShapeInput};
use stellar_scene::handle::SnapshotHandle;
use stellar_scene::node::{
    CornerRadii, HitTestMeta, PointerEvents, RectPrimitive, SceneNodeSpec, TextLayoutKey,
    TextPrimitive, ZOrder,
};
use stellar_scene::revision::SceneRevisionMap;
use stellar_scene::stack::{
    ClipStackRef, Opacity, Paint, PaintStack, PaintStackRef, SceneBlendMode, StrokeCap, StrokeLayer,
};
use stellar_scene::SceneSnapshot;

use super::*;
use crate::capture::{
    AssetRevision, EncodedImage, ExportAssetSource, FontRevision, ImageMedia, ResourcePins,
    SceneSource,
};
use crate::preflight::ConfirmedExport;
use crate::request::{
    ExportBackground, ExportRequest, ExportScale, ExportSource, OutputPath, TargetProfileKey,
};
use crate::test_fixtures::{self as fx, Basic};
use crate::text::NoText;

/// Text outline 테스트 (XX.3.2).
mod text;
mod vector;

const GOLDEN: &str = include_str!("../../../tests/fixtures/basic_view_v1.svg");

const PINS: ResourcePins = ResourcePins {
    fonts: FontRevision(1),
    assets: AssetRevision(3),
};

/// 고정 revision별 asset 내용.
#[derive(Default)]
struct Assets(HashMap<(AssetRef, AssetRevision), EncodedImage>);

impl ExportAssetSource for Assets {
    fn encoded_image(&self, asset: AssetRef, revision: AssetRevision) -> Option<EncodedImage> {
        self.0.get(&(asset, revision)).cloned()
    }
}

fn assets() -> Assets {
    let mut map = HashMap::new();
    map.insert(
        (fx::asset(1), AssetRevision(3)),
        EncodedImage::new(ImageMedia::Png, Arc::from(b"stellar-png".to_vec()), 60, 40).unwrap(),
    );
    map.insert(
        (fx::asset(2), AssetRevision(3)),
        EncodedImage::new(ImageMedia::Jpeg, Arc::from(b"other".to_vec()), 10, 10).unwrap(),
    );
    Assets(map)
}

fn capture(snapshot: SceneSnapshot, request: ExportRequest) -> ExportCapture {
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

/// writer 동작만 보는 테스트용 진행 허가(진단은 preflight 테스트가 검사합니다).
fn confirmed(capture: &ExportCapture) -> ConfirmedExport {
    ConfirmedExport::assume_confirmed(capture.clone())
}

fn svg_request() -> ExportRequest {
    fx::view_request(ExportFormat::Svg)
}

fn write(capture: &ExportCapture) -> Result<SvgDocument, SvgExportError> {
    write_svg(
        &confirmed(capture),
        &assets(),
        &NoText,
        &SvgWriteOptions::default(),
    )
}

fn write_snapshot(snapshot: SceneSnapshot) -> Result<SvgDocument, SvgExportError> {
    write(&capture(snapshot, svg_request()))
}

/// 출력 구조 검사: 태그 균형, id 유일, 모든 `url(#…)`이 정의된 id를 가리킴, `<text` 없음.
///
/// writer 출력에는 text node·주석·CDATA가 없고 속성 값에 `<`·`>`가 없으므로 태그 단위로 충분합니다.
fn validate(svg: &str) -> BTreeSet<String> {
    assert!(svg.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg "));
    assert!(svg.ends_with("</svg>\n"));
    assert!(!svg.contains("<text"), "text 요소 없음");
    let mut open: Vec<&str> = Vec::new();
    let mut ids = BTreeSet::new();
    let mut rest = svg;
    while let Some(start) = rest.find('<') {
        let between = &rest[..start];
        assert!(between.trim().is_empty(), "태그 밖 내용 없음: {between:?}");
        let end = rest[start..].find('>').expect("닫는 >") + start;
        let tag = &rest[start + 1..end];
        rest = &rest[end + 1..];
        if tag.starts_with('?') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            assert_eq!(open.pop(), Some(name), "닫는 태그 순서");
            continue;
        }
        let name = tag
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap();
        for part in tag.split(" id=\"").skip(1) {
            let id = &part[..part.find('"').unwrap()];
            assert!(ids.insert(id.to_string()), "id 중복: {id}");
        }
        if !tag.ends_with('/') {
            open.push(name);
        }
    }
    assert!(open.is_empty(), "닫히지 않은 태그: {open:?}");
    for part in svg.split("url(#").skip(1) {
        let id = &part[..part.find(')').unwrap()];
        assert!(ids.contains(id), "정의되지 않은 참조: {id}");
    }
    ids
}

#[test]
fn basic_view_matches_golden() {
    let document = write_snapshot(fx::basic_snapshot(&Basic::default())).unwrap();
    if std::env::var_os("STELLAR_EXPORT_UPDATE_GOLDEN").is_some() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/basic_view_v1.svg"
        );
        std::fs::write(path, document.as_str()).unwrap();
        return;
    }
    assert_eq!(document.as_str(), GOLDEN);
    let ids = validate(document.as_str());
    // Frame·Shape·Path·Image 그룹 + frame bounds clip 1개(자식 공유) + 테두리 자기 기하 clip 1개.
    assert_eq!(document.groups(), 4);
    assert_eq!(document.clip_definitions(), 2);
    assert_eq!(ids.len(), 6);
    assert!(ids.contains(&format!("n-{}", fx::node_id(1))));
    assert!(ids.contains(&format!("n-{}-g", fx::node_id(1))));
    assert!(ids.contains(&format!("n-{}-c1", fx::node_id(2))));
}

#[test]
fn same_snapshot_is_byte_identical() {
    // 검증 기준(XX.3.3): same snapshot byte-identical.
    let snapshot = fx::basic_snapshot(&Basic::default());
    let capture = capture(snapshot.clone(), svg_request());
    let first = write(&capture).unwrap();
    let second = write(&capture).unwrap();
    assert_eq!(first.as_bytes(), second.as_bytes());
    // 따로 만든 같은 내용 snapshot(다른 allocation·revision)도 같은 bytes입니다.
    let rebuilt = SceneSnapshot::new(
        DocumentRevision(41),
        SceneRevision(97),
        fx::basic_roots(&Basic::default()),
    )
    .unwrap();
    assert_eq!(
        write_snapshot(rebuilt).unwrap().as_bytes(),
        first.as_bytes()
    );
    // canonical codec round-trip(공유 구조가 모두 새 allocation)도 같은 bytes입니다.
    let decoded = stellar_scene::codec::decode(&stellar_scene::codec::encode(&snapshot)).unwrap();
    assert_eq!(
        write_snapshot(decoded).unwrap().as_bytes(),
        first.as_bytes()
    );
    // 다른 thread에서 써도 같은 bytes입니다(HashMap seed 등 실행 상태에 의존하지 않음).
    let threaded = {
        let capture = capture.clone();
        thread::spawn(move || write(&capture).unwrap().into_string())
            .join()
            .unwrap()
    };
    assert_eq!(threaded.as_bytes(), first.as_bytes());
}

#[test]
fn edits_during_export_do_not_change_the_document() {
    // XX.1.2 capture와 결합: capture 뒤 편집·asset 교체가 SVG에 섞이지 않습니다.
    let mut map = SceneRevisionMap::new();
    let base = map
        .generate(DocumentRevision(1), fx::basic_roots(&Basic::default()))
        .unwrap();
    let pinned = capture(base.clone(), svg_request());
    let before = write(&pinned).unwrap();
    let edited = map
        .generate(
            DocumentRevision(2),
            fx::basic_roots(&Basic {
                shape_fill: fx::srgb(0.0, 1.0, 0.0, 1.0),
                image: fx::asset(2),
                ..Basic::default()
            }),
        )
        .unwrap();
    let mut store = assets();
    store.0.insert(
        (fx::asset(1), AssetRevision(4)),
        EncodedImage::new(ImageMedia::Png, Arc::from(b"replaced".to_vec()), 60, 40).unwrap(),
    );
    let after = write_svg(
        &confirmed(&pinned),
        &store,
        &NoText,
        &SvgWriteOptions::default(),
    )
    .unwrap();
    assert_eq!(after.as_bytes(), before.as_bytes());
    let fresh = write(&capture(edited, svg_request())).unwrap();
    assert_ne!(fresh.as_bytes(), before.as_bytes());
    assert!(fresh.as_str().contains("rgb(0%,100%,0%)"));
    assert!(fresh.as_str().contains("data:image/jpeg"));
}

#[test]
fn selection_does_not_apply_ancestor_clips() {
    let snapshot = fx::basic_snapshot(&Basic::default());
    let request = fx::request(
        ExportSource::selection(fx::view_id(), [fx::node_id(2)]).unwrap(),
        ExportFormat::Svg,
    );
    let document = write(&capture(snapshot, request)).unwrap();
    validate(document.as_str());
    assert_eq!(document.groups(), 1);
    assert_eq!(document.clip_definitions(), 0);
    assert!(!document.as_str().contains("<clipPath"));
    // Shape(30,20) 40×30 + stroke 1.5.
    assert!(document
        .as_str()
        .contains("width=\"43\" height=\"33\" viewBox=\"28.5 18.5 43 33\""));
}

fn rect_spec(n: u64, z: u64, x: f32) -> SceneNodeSpec {
    SceneNodeSpec {
        source: fx::node_id(n),
        content: SceneContent::Rect(RectPrimitive {
            rect: fx::rect(0.0, 0.0, 10.0, 10.0),
            radii: CornerRadii::ZERO,
        }),
        world_transform: fx::translate(x, 0.0),
        layout_rect: fx::rect(0.0, 0.0, 10.0, 10.0),
        clip: ClipStackRef::none(),
        paint: PaintStackRef::new(PaintStack {
            layers: vec![PaintLayer::Fill(Paint::Solid(fx::srgb(0.5, 0.5, 0.5, 1.0)))],
            ..PaintStack::empty()
        }),
        z: ZOrder(z),
        hit: HitTestMeta::INTERACTIVE,
    }
}

fn snapshot_of(nodes: Vec<SceneNode>) -> SceneSnapshot {
    SceneSnapshot::new(
        DocumentRevision(1),
        SceneRevision(1),
        nodes.into_iter().map(Arc::new).collect(),
    )
    .unwrap()
}

#[test]
fn repeated_sources_get_distinct_safe_ids() {
    // 한 document node가 여러 scene node를 만드는 경우(component instance 등) id 충돌 없음.
    let nodes = vec![
        SceneNode::leaf(rect_spec(7, 1, 0.0)),
        SceneNode::leaf(rect_spec(7, 2, 20.0)),
        SceneNode::leaf(rect_spec(8, 3, 40.0)),
        SceneNode::leaf(rect_spec(7, 4, 60.0)),
    ];
    let document = write_snapshot(snapshot_of(nodes)).unwrap();
    let ids = validate(document.as_str());
    let base = format!("n-{}", fx::node_id(7));
    let expected: BTreeSet<String> = [
        base.clone(),
        format!("{base}-d2"),
        format!("{base}-d3"),
        format!("n-{}", fx::node_id(8)),
    ]
    .into_iter()
    .collect();
    assert_eq!(ids, expected);
    for id in &ids {
        assert!(crate::svg::format::is_safe_id(id), "{id}");
    }
}

/// outer frame(clip) > inner frame(clip, 둥근 모서리) > shape 두 개.
fn nested_clip_snapshot() -> SceneSnapshot {
    let mut builder = SceneBuilder::new();
    let mut root = builder.root_scope();
    let frame = |n: u64, x: f32, size: f32, radius: f32| FrameInput {
        node: NodeInput::new(fx::node_id(n), fx::translate(x, x)),
        size: Size2::new(size, size).unwrap(),
        corner_radii: CornerRadii::new(radius, radius, radius, radius).unwrap(),
        background: None,
        border: None,
        clip: FrameClipInput::Bounds,
    };
    let mut outer = builder
        .begin_frame(&root, &frame(1, 0.0, 100.0, 0.0))
        .unwrap();
    let mut inner = builder
        .begin_frame(outer.scope(), &frame(2, 10.0, 50.0, 5.0))
        .unwrap();
    for (n, x) in [(3u64, 0.0f32), (4, 30.0)] {
        let mut node = NodeInput::new(fx::node_id(n), fx::translate(x, 0.0));
        node.paint
            .fills
            .push(Paint::Solid(fx::srgb(0.0, 0.0, 0.0, 1.0)));
        builder
            .build_shape(
                inner.scope_mut(),
                &ShapeInput {
                    node,
                    layout_rect: fx::rect(0.0, 0.0, 40.0, 30.0),
                    geometry: fx::triangle(),
                    cap: StrokeCap::Butt,
                },
            )
            .unwrap();
    }
    builder.finish_frame(outer.scope_mut(), inner).unwrap();
    builder.finish_frame(&mut root, outer).unwrap();
    SceneSnapshot::new(DocumentRevision(1), SceneRevision(1), builder.finish(root)).unwrap()
}

#[test]
fn clip_definitions_cover_only_new_clips_and_are_shared() {
    let document = write_snapshot(nested_clip_snapshot()).unwrap();
    let text = document.as_str();
    validate(text);
    let outer_clip = format!("n-{}-c1", fx::node_id(2));
    let inner_clip = format!("n-{}-c2", fx::node_id(3));
    // inner frame 그룹은 outer clip, 두 shape 그룹은 inner clip 하나만(outer는 상속) 겁니다.
    assert_eq!(document.clip_definitions(), 2);
    assert!(text.contains(&format!(
        "<g id=\"n-{}\" clip-path=\"url(#{outer_clip})\">",
        fx::node_id(2)
    )));
    for n in [3, 4] {
        assert!(text.contains(&format!(
            "<g id=\"n-{}\" clip-path=\"url(#{inner_clip})\">",
            fx::node_id(n)
        )));
    }
    // 새 clip 정의는 조상 clip을 다시 참조하지 않습니다(이중 적용 없음).
    assert!(text.contains(&format!("<clipPath id=\"{inner_clip}\">")));
    assert_eq!(text.matches("clip-path=\"url(#").count(), 3);
}

#[test]
fn multiple_new_clips_on_one_node_form_a_chain() {
    // selection으로 outer frame을 빼면 shape는 두 clip을 모두 범위 밖으로 둡니다. 반대로 View
    // 내보내기에서 clip 두 개가 한 번에 늘어나는 node는 chain을 만듭니다.
    let outer = ClipStackRef::none().push(stellar_scene::stack::ClipShape::Rect {
        rect: fx::rect(0.0, 0.0, 100.0, 100.0),
        radii: CornerRadii::ZERO,
        transform: Transform2D::identity(),
    });
    let both = outer.push(stellar_scene::stack::ClipShape::Rect {
        rect: fx::rect(5.0, 5.0, 50.0, 50.0),
        radii: CornerRadii::ZERO,
        transform: Transform2D::identity(),
    });
    let mut spec = rect_spec(1, 1, 0.0);
    spec.clip = both;
    let document = write_snapshot(snapshot_of(vec![SceneNode::leaf(spec)])).unwrap();
    validate(document.as_str());
    let c1 = format!("n-{}-c1", fx::node_id(1));
    let c2 = format!("n-{}-c2", fx::node_id(1));
    assert!(document
        .as_str()
        .contains(&format!("<clipPath id=\"{c2}\" clip-path=\"url(#{c1})\">")));
    assert!(document
        .as_str()
        .contains(&format!("clip-path=\"url(#{c2})\">")));
}

#[test]
fn clip_must_extend_the_parent_clip() {
    let parent_clip = ClipStackRef::none().push(stellar_scene::stack::ClipShape::Rect {
        rect: fx::rect(0.0, 0.0, 100.0, 100.0),
        radii: CornerRadii::ZERO,
        transform: Transform2D::identity(),
    });
    let mut parent = rect_spec(1, 1, 0.0);
    parent.clip = parent_clip;
    // 자식이 부모 clip 없이 시작합니다(부모 clip을 벗어난 자식).
    let child = SceneNode::leaf(rect_spec(2, 2, 0.0));
    let tree = SceneNode::new(parent, vec![Arc::new(child)]).unwrap();
    assert_eq!(
        write_snapshot(snapshot_of(vec![tree])).unwrap_err(),
        SvgExportError::ClipNotNested {
            node: fx::node_id(2)
        }
    );
}

#[test]
fn unsupported_features_stop_with_typed_errors() {
    let mut shadow = rect_spec(2, 1, 0.0);
    shadow.paint = PaintStackRef::new(PaintStack {
        shadows: vec![Shadow::new(
            fx::p(1.0, 1.0),
            BlurRadius::new(2.0).unwrap(),
            fx::srgb(0.0, 0.0, 0.0, 0.5),
        )],
        ..PaintStack::empty()
    });
    let mut blend = rect_spec(3, 1, 0.0);
    blend.paint = PaintStackRef::new(PaintStack {
        blend: SceneBlendMode::Multiply,
        ..PaintStack::empty()
    });
    let cases = [
        (shadow, SvgFeature::DropShadow),
        (blend, SvgFeature::BlendMode),
    ];
    for (spec, feature) in cases {
        let source = spec.source;
        let error = write_snapshot(snapshot_of(vec![SceneNode::leaf(spec)])).unwrap_err();
        assert_eq!(
            error,
            SvgExportError::Unsupported {
                node: source,
                feature,
                support: feature.support()
            }
        );
        assert!(error.to_string().contains(&feature.to_string()));
    }
    // 숨긴 node의 미지원 기능은 출력에 영향이 없습니다.
    let mut hidden_text = SceneNodeSpec {
        content: SceneContent::Text(TextPrimitive {
            layout: TextLayoutKey::from_bytes([1; 16]),
            bounds: fx::rect(0.0, 0.0, 10.0, 10.0),
        }),
        ..rect_spec(5, 2, 0.0)
    };
    hidden_text.hit = HitTestMeta {
        visibility: Visibility::Hidden,
        pointer_events: PointerEvents::Auto,
    };
    let document = write_snapshot(snapshot_of(vec![
        SceneNode::leaf(rect_spec(6, 1, 0.0)),
        SceneNode::leaf(hidden_text),
    ]))
    .unwrap();
    assert_eq!(document.groups(), 1);
    assert!(!document.as_str().contains(&fx::node_id(5).to_string()));
}

#[test]
fn missing_assets_and_wrong_format_are_errors() {
    let missing = capture(
        fx::basic_snapshot(&Basic {
            image: fx::asset(77),
            ..Basic::default()
        }),
        svg_request(),
    );
    assert_eq!(
        write(&missing).unwrap_err(),
        SvgExportError::MissingImage {
            node: fx::node_id(4),
            asset: fx::asset(77)
        }
    );
    // 고정 revision(3)이 아닌 revision에만 있는 내용은 쓰지 않습니다.
    let mut other_revision = Assets::default();
    other_revision.0.insert(
        (fx::asset(1), AssetRevision(2)),
        EncodedImage::new(ImageMedia::Png, Arc::from(b"old".to_vec()), 1, 1).unwrap(),
    );
    let pinned = capture(fx::basic_snapshot(&Basic::default()), svg_request());
    assert!(matches!(
        write_svg(
            &confirmed(&pinned),
            &other_revision,
            &NoText,
            &SvgWriteOptions::default()
        ),
        Err(SvgExportError::MissingImage { .. })
    ));
    let png = capture(
        fx::basic_snapshot(&Basic::default()),
        fx::view_request(ExportFormat::Png),
    );
    assert_eq!(
        write(&png).unwrap_err(),
        SvgExportError::FormatMismatch {
            requested: ExportFormat::Png
        }
    );
}

#[test]
fn output_limit_is_enforced() {
    let pinned = capture(fx::basic_snapshot(&Basic::default()), svg_request());
    let full = write(&pinned).unwrap();
    let exact = SvgWriteOptions {
        max_output_bytes: full.as_bytes().len(),
    };
    assert_eq!(
        write_svg(&confirmed(&pinned), &assets(), &NoText, &exact).unwrap(),
        full
    );
    for limit in [0, 100, full.as_bytes().len() - 1] {
        assert_eq!(
            write_svg(
                &confirmed(&pinned),
                &assets(),
                &NoText,
                &SvgWriteOptions {
                    max_output_bytes: limit
                }
            )
            .unwrap_err(),
            SvgExportError::OutputTooLarge { limit }
        );
    }
    // 큰 이미지는 base64를 쓰기 전에 거부합니다.
    let mut huge = Assets::default();
    huge.0.insert(
        (fx::asset(1), AssetRevision(3)),
        EncodedImage::new(ImageMedia::Png, Arc::from(vec![0u8; 1 << 20]), 1, 1).unwrap(),
    );
    assert_eq!(
        write_svg(
            &confirmed(&pinned),
            &huge,
            &NoText,
            &SvgWriteOptions {
                max_output_bytes: 1 << 20
            }
        )
        .unwrap_err(),
        SvgExportError::OutputTooLarge { limit: 1 << 20 }
    );
}

#[test]
fn background_scale_and_opacity_are_recorded() {
    let request = ExportRequest::new(
        ExportSource::view(fx::view_id()).unwrap(),
        TargetProfileKey::new("web-modern").unwrap(),
        ExportScale::new(2.0).unwrap(),
        ExportBackground::Solid(Color::new(ColorSpace::DisplayP3, 0.0, 0.0, 1.0, 1.0).unwrap()),
        OutputPath::new("/tmp/stellar-export/out.svg").unwrap(),
        ExportFormat::Svg,
    )
    .unwrap();
    let mut translucent = rect_spec(1, 1, 0.0);
    translucent.paint = PaintStackRef::new(PaintStack {
        layers: vec![PaintLayer::Fill(Paint::Solid(fx::srgb(1.0, 0.0, 0.0, 1.0)))],
        opacity: Opacity::new(0.25).unwrap(),
        ..PaintStack::empty()
    });
    let document = write(&capture(
        snapshot_of(vec![SceneNode::leaf(translucent)]),
        request,
    ))
    .unwrap();
    validate(document.as_str());
    assert_eq!(
        document.as_str(),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\" \
             viewBox=\"0 0 10 10\">\n\
             <rect x=\"0\" y=\"0\" width=\"10\" height=\"10\" fill=\"color(display-p3 0 0 1)\"/>\n\
             <g id=\"n-{}\" opacity=\"0.25\">\n\
             <rect x=\"0\" y=\"0\" width=\"10\" height=\"10\" fill=\"rgb(100%,0%,0%)\"/>\n\
             </g>\n</svg>\n",
            fx::node_id(1)
        )
    );
}

#[test]
fn deep_trees_are_written_without_recursion() {
    let depth = 1_000u64;
    let mut node = SceneNode::leaf(rect_spec(depth, depth, 0.0));
    for n in (1..depth).rev() {
        node = SceneNode::new(rect_spec(n, n, 0.0), vec![Arc::new(node)]).unwrap();
    }
    // 깊은 tree의 drop은 이 thread가 합니다(작은 stack thread는 참조 카운트만 줄입니다).
    let handle = SnapshotHandle::new(snapshot_of(vec![node]));
    let pinned = ExportCapture::new(
        svg_request(),
        SceneSource {
            view: fx::view_id(),
            target_profile: fx::profile(),
            snapshot: handle.clone(),
        },
        PINS,
    )
    .unwrap();
    let worker = thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(move || {
            let document = write(&pinned).unwrap();
            (document.groups(), document.into_string())
        })
        .unwrap();
    let (groups, text) = worker.join().expect("작은 stack에서도 panic 없음");
    assert_eq!(groups, depth);
    assert_eq!(text.matches("<g ").count(), depth as usize);
    validate(&text);
    drop(handle);
}

#[test]
fn errors_have_user_messages() {
    for error in [
        SvgExportError::FormatMismatch {
            requested: ExportFormat::Png,
        },
        SvgExportError::ClipNotNested {
            node: fx::node_id(1),
        },
        SvgExportError::MissingImage {
            node: fx::node_id(1),
            asset: fx::asset(1),
        },
        SvgExportError::Value {
            node: fx::node_id(1),
            error: SvgValueError::NonFinite,
        },
        SvgExportError::Document(SvgValueError::NonFinite),
        SvgExportError::Id(SvgIdError::Collision { id: "a".into() }),
        SvgExportError::OutputTooLarge { limit: 1 },
    ] {
        assert!(!error.to_string().is_empty());
    }
    assert_eq!(
        SvgWriteOptions::default().max_output_bytes,
        DEFAULT_MAX_SVG_BYTES
    );
}

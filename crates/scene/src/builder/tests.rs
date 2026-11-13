//! scene builder 테스트 (XI.2.2).

use super::*;
use crate::codec;
use crate::fixtures::{id, p, rect, translate};
use crate::geometry::PathCommand;
use crate::snapshot::SceneSnapshot;
use stellar_core_types::revision::{DocumentRevision, SceneRevision};

/// node 종류별 golden(이름 → semantic hash, canonical bytes hex).
const GOLDEN: &str = include_str!("../../tests/fixtures/scene_builders_v1.golden");

fn golden() -> Vec<(String, String, String)> {
    GOLDEN
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .map(|line| {
            let mut parts = line.split_whitespace();
            let name = parts.next().expect("name").to_string();
            let hash = parts.next().expect("hash").to_string();
            let hex = parts.next().expect("hex").to_string();
            (name, hash, hex)
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
        .collect()
}

fn color(r: f32, g: f32, b: f32, a: f32) -> Color {
    Color::new(ColorSpace::Srgb, r, g, b, a).expect("color")
}

fn size(w: f32, h: f32) -> Size2 {
    Size2::new(w, h).expect("size")
}

fn triangle(w: f32, h: f32) -> PathGeometry {
    PathGeometry::new(vec![
        PathCommand::MoveTo(p(0.0, 0.0)),
        PathCommand::LineTo(p(w, 0.0)),
        PathCommand::LineTo(p(w / 2.0, h)),
        PathCommand::Close,
    ])
    .expect("triangle")
}

fn ellipse(w: f32, h: f32) -> PathGeometry {
    // document Ellipse 평가와 같은 형태(4개 cubic, κ)의 고정 입력.
    let k = 0.552_284_8f32;
    let (rx, ry) = (w / 2.0, h / 2.0);
    PathGeometry::new(vec![
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
    ])
    .expect("ellipse")
}

fn effects() -> ModifierPaint {
    ModifierPaint {
        fills: vec![Paint::Solid(color(0.0, 1.0, 0.0, 0.5))],
        strokes: vec![StrokeInput {
            paint: Paint::Solid(color(0.0, 0.0, 0.0, 1.0)),
            width: 3.0,
        }],
        shadows: vec![Shadow::new(
            p(2.0, 3.0),
            BlurRadius::new(4.0).expect("blur"),
            color(0.0, 0.0, 0.0, 0.25),
        )],
        opacity: Opacity::new(0.75).expect("opacity"),
        blend: SceneBlendMode::Multiply,
        layer_blur: BlurRadius::ZERO,
    }
}

fn frame_input(source: u64, x: f32, y: f32, clip: FrameClipInput) -> FrameInput {
    let mut node = NodeInput::new(id(source), translate(x, y));
    node.paint = effects();
    FrameInput {
        node,
        size: size(100.0, 50.0),
        corner_radii: CornerRadii::new(4.0, 4.0, 4.0, 4.0).expect("radii"),
        background: Some(color(1.0, 0.0, 0.0, 1.0)),
        border: Some(BorderInput {
            color: color(0.0, 0.0, 1.0, 1.0),
            thickness: 2.0,
        }),
        clip,
    }
}

fn shape_input(source: u64) -> ShapeInput {
    let mut node = NodeInput::new(id(source), translate(5.0, 5.0));
    node.paint = effects();
    ShapeInput {
        node,
        layout_rect: rect(0.0, 0.0, 40.0, 20.0),
        geometry: ellipse(40.0, 20.0),
        cap: StrokeCap::Round,
    }
}

fn path_input(source: u64) -> PathInput {
    let mut node = NodeInput::new(id(source), translate(-3.0, 7.5));
    node.paint.fills = vec![Paint::Solid(color(0.2, 0.4, 0.6, 1.0))];
    PathInput {
        node,
        layout_rect: rect(0.0, 0.0, 30.0, 30.0),
        geometry: triangle(30.0, 30.0),
        fill_rule: FillRule::EvenOdd,
    }
}

fn text_input(source: u64) -> TextInput {
    let mut node = NodeInput::new(id(source), translate(12.0, 0.0));
    node.paint.strokes = vec![StrokeInput {
        paint: Paint::Solid(color(1.0, 1.0, 1.0, 1.0)),
        width: 1.0,
    }];
    TextInput {
        node,
        layout: TextLayoutKey::from_bytes([0x7E; 16]),
        size: size(80.0, 18.0),
    }
}

fn image_input(source: u64) -> ImageInput {
    let mut node = NodeInput::new(id(source), translate(0.0, 24.0));
    node.paint.strokes = vec![StrokeInput {
        paint: Paint::Solid(color(0.5, 0.5, 0.5, 1.0)),
        width: 2.0,
    }];
    ImageInput {
        node,
        asset: AssetRef::new(id(0xA55E7)),
        size: size(64.0, 48.0),
        fit: ImageFit::Cover,
        crop: ImageCrop::new(0.25, 0.0, 0.5, 1.0).expect("crop"),
        sampling: ImageSampling::Nearest,
        color_space: Some(ColorSpace::DisplayP3),
    }
}

/// 이름별 golden scene을 만듭니다.
fn build(name: &str) -> SceneSnapshot {
    let mut builder = SceneBuilder::new();
    let mut root = builder.root_scope();
    match name {
        "frame" => {
            let frame = builder
                .begin_frame(&root, &frame_input(1, 10.0, 20.0, FrameClipInput::Bounds))
                .expect("frame");
            builder.finish_frame(&mut root, frame).expect("finish");
        }
        "shape" => {
            builder
                .build_shape(&mut root, &shape_input(2))
                .expect("shape");
        }
        "path" => {
            builder.build_path(&mut root, &path_input(3)).expect("path");
        }
        "text" => {
            builder.build_text(&mut root, &text_input(4)).expect("text");
        }
        "image" => {
            builder
                .build_image(&mut root, &image_input(5))
                .expect("image");
        }
        "nested" => {
            let mut outer = builder
                .begin_frame(&root, &frame_input(10, 10.0, 20.0, FrameClipInput::Bounds))
                .expect("outer");
            builder
                .build_shape(outer.scope_mut(), &shape_input(11))
                .expect("shape");
            let mut inner = builder
                .begin_frame(
                    outer.scope(),
                    &FrameInput {
                        node: NodeInput {
                            locked: true,
                            ..NodeInput::new(id(12), translate(50.0, 0.0))
                        },
                        size: size(40.0, 40.0),
                        corner_radii: CornerRadii::ZERO,
                        background: None,
                        border: None,
                        clip: FrameClipInput::Path {
                            geometry: triangle(40.0, 40.0),
                            fill_rule: FillRule::NonZero,
                        },
                    },
                )
                .expect("inner");
            builder
                .build_path(inner.scope_mut(), &path_input(13))
                .expect("path");
            let mut hidden_text = text_input(14);
            hidden_text.node.visible = false;
            builder
                .build_text(inner.scope_mut(), &hidden_text)
                .expect("text");
            builder
                .finish_frame(outer.scope_mut(), inner)
                .expect("inner");
            builder
                .build_image(outer.scope_mut(), &image_input(15))
                .expect("image");
            builder.finish_frame(&mut root, outer).expect("outer");
        }
        other => panic!("알 수 없는 golden 이름: {other}"),
    }
    SceneSnapshot::new(DocumentRevision(1), SceneRevision(1), builder.finish(root))
        .expect("snapshot")
}

#[test]
fn every_node_kind_matches_golden_scene_snapshot() {
    // 검증 기준(XI.2.2): node kind별 golden scene snapshot.
    let golden = golden();
    let names: Vec<&str> = golden.iter().map(|(name, _, _)| name.as_str()).collect();
    assert_eq!(names, ["frame", "shape", "path", "text", "image", "nested"]);
    for (name, hash, expected_hex) in &golden {
        let snapshot = build(name);
        let actual_hex = hex(&codec::encode(&snapshot));
        assert_eq!(
            &actual_hex,
            expected_hex,
            "{name} golden 불일치. 실제:\n{name} {} {actual_hex}",
            snapshot.semantic_hash()
        );
        assert_eq!(&snapshot.semantic_hash().to_string(), hash, "{name} hash");
        // golden bytes는 같은 scene으로 복원됩니다.
        assert_eq!(
            codec::decode(&unhex(expected_hex)).expect("decode"),
            snapshot
        );
    }
}

#[test]
fn frame_builds_rect_with_payload_paint_then_modifiers_and_clips_children() {
    let snapshot = build("frame");
    let frame = snapshot.root(0).expect("frame");
    assert_eq!(frame.source(), id(1));
    let expected_rect = rect(0.0, 0.0, 100.0, 50.0);
    assert_eq!(
        frame.content(),
        &SceneContent::Rect(RectPrimitive {
            rect: expected_rect,
            radii: CornerRadii::new(4.0, 4.0, 4.0, 4.0).unwrap()
        })
    );
    assert_eq!(frame.layout_rect(), expected_rect);
    assert_eq!(frame.world_transform(), translate(10.0, 20.0));
    let paint = frame.paint().get();
    // 배경 → 테두리(Inside) → modifier fill → modifier stroke(Inside).
    assert_eq!(paint.layers.len(), 4);
    assert_eq!(
        paint.layers[0],
        PaintLayer::Fill(Paint::Solid(color(1.0, 0.0, 0.0, 1.0)))
    );
    match paint.layers[1] {
        PaintLayer::Stroke(stroke) => {
            assert_eq!(stroke.width(), 2.0);
            assert_eq!(stroke.align(), StrokeAlign::Inside);
        }
        other => panic!("테두리 stroke가 아님: {other:?}"),
    }
    assert!(matches!(paint.layers[2], PaintLayer::Fill(_)));
    match paint.layers[3] {
        PaintLayer::Stroke(stroke) => assert_eq!(stroke.align(), StrokeAlign::Inside),
        other => panic!("modifier stroke가 아님: {other:?}"),
    }
    assert_eq!(paint.shadows.len(), 1);
    assert_eq!(paint.opacity.get(), 0.75);
    assert_eq!(paint.blend, SceneBlendMode::Multiply);
    // Frame 자신은 부모 clip(없음)을 받습니다.
    assert!(frame.clip().is_empty());
    assert_eq!(frame.z(), ZOrder(1));
    assert_eq!(frame.hit(), HitTestMeta::INTERACTIVE);
}

#[test]
fn nested_scope_composes_transforms_clips_visibility_and_z_order() {
    let snapshot = build("nested");
    let outer = snapshot.root(0).expect("outer");
    let shape = outer.child(0).expect("shape");
    let inner = outer.child(1).expect("inner");
    let path = inner.child(0).expect("path");
    let text = inner.child(1).expect("text");
    let image = outer.child(2).expect("image");

    // world transform = local.then(parent world).
    assert_eq!(shape.world_transform(), translate(15.0, 25.0));
    assert_eq!(inner.world_transform(), translate(60.0, 20.0));
    assert_eq!(path.world_transform(), translate(57.0, 27.5));

    // outer bounds clip은 자식에, inner path clip은 손자에 쌓입니다.
    assert_eq!(shape.clip().depth(), 1);
    assert_eq!(
        shape.clip().innermost(),
        Some(&ClipShape::Rect {
            rect: rect(0.0, 0.0, 100.0, 50.0),
            radii: CornerRadii::new(4.0, 4.0, 4.0, 4.0).unwrap(),
            transform: translate(10.0, 20.0),
        })
    );
    assert!(inner.clip().ptr_eq(shape.clip()));
    assert_eq!(path.clip().depth(), 2);
    assert!(matches!(
        path.clip().innermost(),
        Some(ClipShape::Path { transform, .. }) if *transform == translate(60.0, 20.0)
    ));

    // 잠금은 자손에게 pointer None, 숨김은 자신에게만(형제 영향 없음).
    assert_eq!(inner.hit().pointer_events, PointerEvents::None);
    assert_eq!(path.hit().pointer_events, PointerEvents::None);
    assert_eq!(text.hit().visibility, Visibility::Hidden);
    assert_eq!(path.hit().visibility, Visibility::Visible);
    assert_eq!(image.hit(), HitTestMeta::INTERACTIVE);

    // pre-order z: outer 1, shape 2, inner 3, path 4, text 5, image 6.
    let zs: Vec<u64> = snapshot.iter_preorder().map(|n| n.z().0).collect();
    assert_eq!(zs, vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn leaf_kinds_map_to_their_primitives() {
    let shape = build("shape");
    let shape = shape.root(0).unwrap();
    match shape.content() {
        SceneContent::Path(path) => {
            assert_eq!(path.fill_rule, FillRule::NonZero);
            assert_eq!(path.geometry, ellipse(40.0, 20.0));
        }
        other => panic!("shape는 path primitive: {other:?}"),
    }
    match shape.paint().get().layers[1] {
        PaintLayer::Stroke(stroke) => {
            assert_eq!(stroke.align(), StrokeAlign::Center);
            assert_eq!(stroke.cap(), StrokeCap::Round);
        }
        other => panic!("stroke가 아님: {other:?}"),
    }

    let path = build("path");
    match path.root(0).unwrap().content() {
        SceneContent::Path(primitive) => assert_eq!(primitive.fill_rule, FillRule::EvenOdd),
        other => panic!("path primitive가 아님: {other:?}"),
    }

    let text = build("text");
    let text = text.root(0).unwrap();
    assert_eq!(
        text.content().resource(),
        Some(crate::node::SceneResourceRef::TextLayout(
            TextLayoutKey::from_bytes([0x7E; 16])
        ))
    );
    assert_eq!(text.layout_rect(), rect(0.0, 0.0, 80.0, 18.0));
    match text.paint().get().layers[0] {
        PaintLayer::Stroke(stroke) => assert_eq!(stroke.align(), StrokeAlign::Center),
        other => panic!("stroke가 아님: {other:?}"),
    }

    let image = build("image");
    let image = image.root(0).unwrap();
    match image.content() {
        SceneContent::Image(primitive) => {
            assert_eq!(primitive.asset, AssetRef::new(id(0xA55E7)));
            assert_eq!(primitive.fit, ImageFit::Cover);
            assert_eq!(primitive.sampling, ImageSampling::Nearest);
            assert_eq!(primitive.color_space, Some(ColorSpace::DisplayP3));
            assert_eq!(primitive.bounds, rect(0.0, 0.0, 64.0, 48.0));
        }
        other => panic!("image primitive가 아님: {other:?}"),
    }
    match image.paint().get().layers[0] {
        PaintLayer::Stroke(stroke) => assert_eq!(stroke.align(), StrokeAlign::Inside),
        other => panic!("stroke가 아님: {other:?}"),
    }
}

#[test]
fn invalid_inputs_are_structured_errors() {
    let mut builder = SceneBuilder::new();
    let mut root = builder.root_scope();
    let huge = Transform2D::new(f32::MAX, 0.0, 0.0, f32::MAX, 0.0, 0.0).unwrap();
    let mut frame = builder
        .begin_frame(
            &root,
            &FrameInput {
                node: NodeInput::new(id(1), huge),
                ..frame_input(1, 0.0, 0.0, FrameClipInput::None)
            },
        )
        .expect("frame");
    let mut overflow = path_input(2);
    overflow.node.transform = huge;
    assert_eq!(
        builder
            .build_path(frame.scope_mut(), &overflow)
            .unwrap_err(),
        SceneBuildError::NonFiniteTransform { source: id(2) }
    );
    builder.finish_frame(&mut root, frame).expect("finish");

    let mut negative = path_input(3);
    negative.node.paint.strokes = vec![StrokeInput {
        paint: Paint::Solid(color(0.0, 0.0, 0.0, 1.0)),
        width: -1.0,
    }];
    assert_eq!(
        builder.build_path(&mut root, &negative).unwrap_err(),
        SceneBuildError::InvalidValue {
            source: id(3),
            error: SceneValueError::Negative {
                field: "stroke width"
            }
        }
    );
    // 실패한 node는 scope에 추가되지 않습니다.
    assert_eq!(root.nodes().len(), 1);
}

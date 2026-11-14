//! canonical semantic snapshot JSON (테스트 전용, VI.10.4).
//!
//! 문서의 **의미 내용 전체**를 결정론적 JSON으로 펼칩니다. 저장 형식이 아니라 round-trip·
//! reload·플랫폼 간 비교와 사람이 읽는 fixture diff 용도입니다(프로덕션 저장 경로는 chunk
//! model만 사용, VI.10.2).
//!
//! 결정론 규칙:
//!
//! - node·modifier stack은 StableId 순, 페이지·View·자식 command·modifier 항목은 문서 순서.
//! - object key는 사전순, 출력은 ASCII([`json`](super::json) 규칙).
//! - float은 저장값 그대로(평가·clamp 결과가 아님), 최단 왕복 10진 표기.
//! - runtime 핸들·캐시는 포함하지 않습니다(XIV.3.1 semantic snapshot과 같은 원칙).

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::reference::NodeRef;
use stellar_core_types::value::{Rect, Size2, Transform2D, Vec2};

use super::json::Json;
use crate::definitions::DocumentDefinition;
use crate::frame::{Clipping, FramePayload};
use crate::image::{FitMode, ImagePayload, SamplingMode};
use crate::model::{DocumentModel, ModelError};
use crate::modifier::{BlendMode, BooleanOp, ModifierPayload, ModifierStack};
use crate::path::{IdentifiedPath, PathCommand, WindingRule};
use crate::records::{DocumentRecords, NodePayload, NodeRecord};
use crate::shape::{LineCap, ShapeGeometry};
use crate::text::TextPayload;

/// snapshot 형식 이름.
pub const SNAPSHOT_FORMAT: &str = "stellar-semantic-snapshot";

/// snapshot 형식 version.
pub const SNAPSHOT_VERSION: u64 = 1;

fn id(value: StableId) -> Json {
    Json::String(value.to_string())
}

fn node_ref(value: NodeRef) -> Json {
    id(value.id())
}

fn hex(bytes: &[u8]) -> Json {
    Json::String(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn vec2(value: Vec2) -> Json {
    Json::floats(&[value.x(), value.y()])
}

fn size2(value: Size2) -> Json {
    Json::floats(&[value.width(), value.height()])
}

fn rect(value: Rect) -> Json {
    Json::floats(&[
        value.origin().x(),
        value.origin().y(),
        value.size().width(),
        value.size().height(),
    ])
}

fn transform(value: Transform2D) -> Json {
    let (a, b, c, d, tx, ty) = value.components();
    Json::floats(&[a, b, c, d, tx, ty])
}

fn color_space(value: ColorSpace) -> Json {
    Json::str(match value {
        ColorSpace::Srgb => "srgb",
        ColorSpace::DisplayP3 => "display-p3",
    })
}

fn color(value: Color) -> Json {
    Json::object([
        ("space", color_space(value.space())),
        (
            "rgba",
            Json::floats(&[value.r(), value.g(), value.b(), value.alpha()]),
        ),
    ])
}

fn optional<T>(value: Option<T>, f: impl FnOnce(T) -> Json) -> Json {
    value.map_or(Json::Null, f)
}

fn frame(payload: &FramePayload) -> Json {
    let FramePayload {
        size,
        transform: matrix,
        clipping,
        background,
        border,
        corner_radius,
        layout_mode,
    } = payload;
    let clipping = match clipping {
        Clipping::Disabled => Json::object([("kind", Json::str("disabled"))]),
        Clipping::FrameBounds => Json::object([("kind", Json::str("frame-bounds"))]),
        Clipping::Path(target) => {
            Json::object([("kind", Json::str("path")), ("target", node_ref(*target))])
        }
    };
    let (tl, tr, br, bl) = corner_radius.components();
    Json::object([
        ("size", size2(*size)),
        ("transform", transform(*matrix)),
        ("clipping", clipping),
        ("background", optional(*background, color)),
        (
            "border",
            optional(*border, |b| {
                Json::object([
                    ("color", color(b.color)),
                    ("thickness", Json::float(b.thickness())),
                ])
            }),
        ),
        ("corner_radius", Json::floats(&[tl, tr, br, bl])),
        ("layout_mode", optional(*layout_mode, id)),
    ])
}

fn line_cap(value: LineCap) -> Json {
    Json::str(match value {
        LineCap::Butt => "butt",
        LineCap::Round => "round",
        LineCap::Square => "square",
    })
}

fn shape(geometry: &ShapeGeometry) -> Json {
    let kind = ("shape", Json::str(geometry.kind().name()));
    match geometry {
        ShapeGeometry::Rectangle(p) => {
            let (tl, tr, br, bl) = p.corner_radius.components();
            Json::object([
                kind,
                ("size", size2(p.size)),
                ("corner_radius", Json::floats(&[tl, tr, br, bl])),
            ])
        }
        ShapeGeometry::Ellipse(p) => Json::object([kind, ("bounds", rect(p.bounds))]),
        ShapeGeometry::Line(p) => Json::object([
            kind,
            ("start", vec2(p.start)),
            ("end", vec2(p.end)),
            ("cap", line_cap(p.cap)),
        ]),
        ShapeGeometry::Polygon(p) => Json::object([
            kind,
            ("bounds", rect(p.bounds())),
            ("sides", Json::uint(u64::from(p.sides()))),
            ("rotation_degrees", Json::float(p.rotation_degrees())),
            ("corner_radius", Json::float(p.corner_radius())),
        ]),
    }
}

fn path(value: &IdentifiedPath) -> Json {
    let winding = match value.data().winding() {
        WindingRule::NonZero => "non-zero",
        WindingRule::EvenOdd => "even-odd",
    };
    let commands = value
        .data()
        .commands()
        .iter()
        .zip(value.ids())
        .map(|(command, ids)| {
            let point = optional(ids.point, |p| id(p.0));
            let handles = optional(ids.handles, |(h1, h2)| {
                Json::Array(vec![id(h1.0), id(h2.0)])
            });
            match command {
                PathCommand::MoveTo(to) => Json::object([
                    ("op", Json::str("move")),
                    ("to", vec2(*to)),
                    ("point", point),
                ]),
                PathCommand::LineTo(to) => Json::object([
                    ("op", Json::str("line")),
                    ("to", vec2(*to)),
                    ("point", point),
                ]),
                PathCommand::CubicTo { c1, c2, to } => Json::object([
                    ("op", Json::str("cubic")),
                    ("c1", vec2(*c1)),
                    ("c2", vec2(*c2)),
                    ("to", vec2(*to)),
                    ("point", point),
                    ("handles", handles),
                ]),
                PathCommand::Close => Json::object([("op", Json::str("close"))]),
            }
        })
        .collect();
    Json::object([
        ("winding", Json::str(winding)),
        ("commands", Json::Array(commands)),
    ])
}

fn text(value: &TextPayload) -> Json {
    Json::object([
        ("content", Json::str(value.content())),
        (
            "paragraph_styles",
            Json::Array(
                value
                    .paragraphs()
                    .iter()
                    .map(|p| Json::uint(u64::from(p.style)))
                    .collect(),
            ),
        ),
        (
            "style_runs",
            Json::Array(
                value
                    .style_runs()
                    .iter()
                    .map(|run| {
                        Json::object([
                            ("start", Json::uint(u64::from(run.range.start.0))),
                            ("end", Json::uint(u64::from(run.range.end.0))),
                            ("style", Json::uint(u64::from(run.style))),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn image(value: &ImagePayload) -> Json {
    let fit = match value.fit {
        FitMode::Fill => "fill",
        FitMode::Contain => "contain",
        FitMode::Cover => "cover",
        FitMode::None => "none",
    };
    let sampling = match value.sampling {
        SamplingMode::Linear => "linear",
        SamplingMode::Nearest => "nearest",
    };
    let (x, y, w, h) = value.crop.components();
    Json::object([
        ("asset", id(value.asset.id())),
        ("size", size2(value.size)),
        ("fit", Json::str(fit)),
        ("crop", Json::floats(&[x, y, w, h])),
        ("sampling", Json::str(sampling)),
        (
            "color_space_override",
            optional(value.color_space_override, color_space),
        ),
    ])
}

fn payload(value: &NodePayload) -> Json {
    match value {
        NodePayload::Frame(p) => frame(p),
        NodePayload::Fragment(_) => Json::object([]),
        NodePayload::Shape(p) => shape(p),
        NodePayload::Path(p) => path(p),
        NodePayload::Text(p) => text(p),
        NodePayload::Image(p) => image(p),
        NodePayload::ComponentInstance(p) => Json::object([
            ("component", id(p.component().id())),
            (
                "property_overrides",
                Json::Array(
                    p.property_overrides()
                        .map(|(key, value)| {
                            Json::object([
                                (
                                    "path",
                                    Json::Array(key.path().iter().copied().map(id).collect()),
                                ),
                                ("property", id(key.property().stable_id())),
                                ("typed_value", hex(&value.to_bytes())),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "slot_overrides",
                Json::Array(
                    p.slot_overrides()
                        .map(|(slot, content)| {
                            Json::object([
                                ("slot", id(slot.stable_id())),
                                (
                                    "content",
                                    Json::Array(
                                        content.content().iter().copied().map(id).collect(),
                                    ),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
    }
}

fn node(record: &NodeRecord) -> Json {
    Json::object([
        ("id", id(record.id)),
        ("kind", Json::str(record.kind().name())),
        ("parent", optional(record.parent, id)),
        ("order_key", hex(record.order_key.as_bytes())),
        ("visible", Json::Bool(record.visible)),
        ("locked", Json::Bool(record.locked)),
        ("name", Json::str(&record.display_name)),
        ("payload", payload(&record.payload)),
    ])
}

fn modifier_payload(value: &ModifierPayload) -> Json {
    let finite = |v: stellar_core_types::value::FiniteF32| Json::float(v.get());
    match value {
        ModifierPayload::Transform(m) => Json::object([("matrix", transform(*m))]),
        ModifierPayload::Boolean { op, operands } => Json::object([
            (
                "op",
                Json::str(match op {
                    BooleanOp::Union => "union",
                    BooleanOp::Subtract => "subtract",
                    BooleanOp::Intersect => "intersect",
                    BooleanOp::Exclude => "exclude",
                }),
            ),
            (
                "operands",
                Json::Array(operands.iter().map(|r| node_ref(*r)).collect()),
            ),
        ]),
        ModifierPayload::StrokeExpand { width } => Json::object([("width", finite(*width))]),
        ModifierPayload::Corner { radius } => Json::object([("radius", finite(*radius))]),
        ModifierPayload::Mask { source } | ModifierPayload::Clip { source } => {
            Json::object([("source", node_ref(*source))])
        }
        ModifierPayload::Fill { color: c } => Json::object([("color", color(*c))]),
        ModifierPayload::Stroke { color: c, width } => {
            Json::object([("color", color(*c)), ("width", finite(*width))])
        }
        ModifierPayload::Shadow {
            color: c,
            offset,
            blur,
        } => Json::object([
            ("color", color(*c)),
            ("offset", vec2(*offset)),
            ("blur", finite(*blur)),
        ]),
        ModifierPayload::Blur { radius } => Json::object([("radius", finite(*radius))]),
        ModifierPayload::Blend { mode } => Json::object([(
            "mode",
            Json::str(match mode {
                BlendMode::Normal => "normal",
                BlendMode::Multiply => "multiply",
                BlendMode::Screen => "screen",
                BlendMode::Overlay => "overlay",
                BlendMode::Darken => "darken",
                BlendMode::Lighten => "lighten",
            }),
        )]),
        ModifierPayload::Opacity { value } => Json::object([("value", finite(*value))]),
    }
}

fn modifier_stack(stack: &ModifierStack) -> Json {
    Json::object([
        ("owner", id(stack.node)),
        (
            "entries",
            Json::Array(
                stack
                    .entries()
                    .iter()
                    .map(|entry| {
                        Json::object([
                            ("id", id(entry.id.stable_id())),
                            ("order_key", hex(entry.order.as_bytes())),
                            ("kind", Json::str(entry.kind.name())),
                            ("enabled", Json::Bool(entry.enabled)),
                            ("payload", modifier_payload(&entry.payload)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn definition(value: &DocumentDefinition) -> Json {
    let pages = value
        .pages
        .iter()
        .map(|page| {
            let views = page
                .views
                .iter()
                .map(|view| {
                    Json::object([
                        ("id", id(view.id)),
                        ("name", Json::str(view.display_name())),
                        ("root_frame", id(view.root_frame)),
                    ])
                })
                .collect();
            Json::object([
                ("id", id(page.id)),
                ("name", Json::str(page.display_name())),
                ("views", Json::Array(views)),
            ])
        })
        .collect();
    Json::object([
        ("id", id(value.id)),
        ("name", Json::str(value.display_name())),
        ("pages", Json::Array(pages)),
    ])
}

/// 레코드의 semantic snapshot JSON 값. 입력 레코드 순서와 무관합니다.
pub fn semantic_snapshot(records: &DocumentRecords) -> Json {
    let mut sorted = records.clone();
    sorted.sort_canonical();
    Json::object([
        ("format", Json::str(SNAPSHOT_FORMAT)),
        ("version", Json::uint(SNAPSHOT_VERSION)),
        ("document", definition(&sorted.definition)),
        (
            "nodes",
            Json::Array(sorted.nodes.iter().map(node).collect()),
        ),
        (
            "modifier_stacks",
            Json::Array(sorted.modifier_stacks.iter().map(modifier_stack).collect()),
        ),
    ])
}

/// 레코드의 semantic snapshot JSON 문자열(ASCII, 마지막 줄바꿈 포함).
pub fn semantic_snapshot_json(records: &DocumentRecords) -> String {
    semantic_snapshot(records).to_pretty_string()
}

/// 런타임 모델의 semantic snapshot JSON 문자열.
pub fn model_snapshot_json(model: &DocumentModel) -> Result<String, ModelError> {
    Ok(semantic_snapshot_json(&model.to_records()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::{PageDefinition, ViewDefinition};
    use crate::fragment::FragmentPayload;
    use crate::frame::{Border, CornerRadius};
    use crate::image::CropRect;
    use crate::modifier::{ModifierId, ModifierKind};
    use crate::path::PathData;
    use crate::shape::{EllipseParams, LineParams, PolygonParams, RectangleParams};
    use crate::text::{Paragraph, StyleRun, TextRange};
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::reference::AssetRef;
    use stellar_core_types::text::UserText;
    use stellar_core_types::value::{Angle, FiniteF32};

    const FIXTURE: &str = "src/test_support/fixtures/semantic_snapshot_basic.json";

    fn sid(n: u64) -> StableId {
        StableId::new_v7(1_700_000_000_000 + n, 0x0abc, 0x0123_4567_89ab_cdef ^ n)
    }

    fn key(byte: u8) -> OrderKey {
        OrderKey::from_bytes(vec![byte]).unwrap()
    }

    fn record(n: u64, parent: Option<u64>, k: u8, name: &str, payload: NodePayload) -> NodeRecord {
        NodeRecord {
            id: sid(n),
            parent: parent.map(sid),
            order_key: key(k),
            visible: n != 15,
            locked: n == 13,
            display_name: name.to_string(),
            payload,
        }
    }

    /// 고정 fixture 문서: 모든 node 종류·Shape 4종·modifier 12종.
    fn fixture_model() -> DocumentModel {
        let mut definition = DocumentDefinition::new(sid(1), "기본 \"문서\"".to_string());
        let mut page = PageDefinition::new(sid(2), "페이지 🎨".to_string());
        page.add_view(ViewDefinition::new(sid(3), "홈".into(), sid(10)))
            .unwrap();
        page.add_view(ViewDefinition::new(sid(4), String::new(), sid(20)))
            .unwrap();
        definition.add_page(page).unwrap();
        let mut model = DocumentModel::new(definition);

        let srgb = |r, g, b, a| Color::new(ColorSpace::Srgb, r, g, b, a).unwrap();
        let p3 = Color::new(ColorSpace::DisplayP3, 1.0, 0.5, 0.25, 1.0).unwrap();
        let mut next = 100u64;
        let path = IdentifiedPath::assign(
            PathData::new(
                WindingRule::EvenOdd,
                vec![
                    PathCommand::MoveTo(Vec2::ZERO),
                    PathCommand::LineTo(Vec2::new(10.0, 0.0).unwrap()),
                    PathCommand::CubicTo {
                        c1: Vec2::new(10.0, 5.5).unwrap(),
                        c2: Vec2::new(4.5, 10.0).unwrap(),
                        to: Vec2::new(0.0, 10.0).unwrap(),
                    },
                    PathCommand::Close,
                ],
            )
            .unwrap(),
            &mut || {
                next += 1;
                sid(next)
            },
        );
        let mut image = ImagePayload::new(AssetRef::new(sid(90)), Size2::new(64.0, 48.0).unwrap());
        image.crop = CropRect::from_components(0.25, 0.0, 0.5, 1.0).unwrap();
        image.color_space_override = Some(ColorSpace::DisplayP3);
        let card = FramePayload {
            size: Size2::new(320.0, 180.5).unwrap(),
            transform: Transform2D::new(-1.0, 0.0, 0.0, 1.0, 12.0, -8.25).unwrap(),
            clipping: Clipping::Path(NodeRef::new(sid(14))),
            background: Some(srgb(1.0, 1.0, 1.0, 0.5)),
            border: Some(Border::new(p3, 1.5).unwrap()),
            corner_radius: CornerRadius::new(8.0, 8.0, 0.0, 0.0).unwrap(),
            layout_mode: Some(sid(91)),
        };
        let nodes = [
            record(
                10,
                None,
                0x80,
                "홈",
                NodePayload::Frame(FramePayload::default()),
            ),
            record(11, Some(10), 0x40, "카드", NodePayload::Frame(card)),
            record(
                12,
                Some(11),
                0x40,
                "그룹",
                NodePayload::Fragment(FragmentPayload),
            ),
            record(
                13,
                Some(12),
                0x40,
                "배경",
                NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
                    size: Size2::new(100.0, 40.0).unwrap(),
                    corner_radius: CornerRadius::new(4.0, 4.0, 4.0, 4.0).unwrap(),
                })),
            ),
            record(14, Some(11), 0x80, "곡선", NodePayload::Path(path)),
            record(
                15,
                Some(11),
                0xC0,
                "제목\t탭",
                NodePayload::Text(
                    TextPayload::new(
                        UserText::new("안녕\n👩‍💻"),
                        vec![Paragraph { style: 0 }, Paragraph { style: 1 }],
                        vec![StyleRun {
                            range: TextRange::new(0, 2).unwrap(),
                            style: 4,
                        }],
                    )
                    .unwrap(),
                ),
            ),
            record(16, Some(10), 0x80, "사진", NodePayload::Image(image)),
            record(
                17,
                Some(12),
                0x80,
                "",
                NodePayload::Shape(ShapeGeometry::Ellipse(EllipseParams {
                    bounds: Rect::new(Vec2::new(-2.0, 3.5).unwrap(), Size2::new(9.0, 9.0).unwrap()),
                })),
            ),
            record(
                20,
                None,
                0x80,
                "설정",
                NodePayload::Frame(FramePayload::default()),
            ),
            record(
                21,
                Some(20),
                0x40,
                "선",
                NodePayload::Shape(ShapeGeometry::Line(LineParams {
                    start: Vec2::ZERO,
                    end: Vec2::new(300.0, 0.0).unwrap(),
                    cap: LineCap::Round,
                })),
            ),
            record(
                22,
                Some(20),
                0x80,
                "별",
                NodePayload::Shape(ShapeGeometry::Polygon(
                    PolygonParams::new(
                        Rect::new(Vec2::ZERO, Size2::new(24.0, 24.0).unwrap()),
                        5,
                        Angle::new(-18.0).unwrap(),
                        1.25,
                    )
                    .unwrap(),
                )),
            ),
        ];
        for node in nodes {
            model.insert_node(node).unwrap();
        }

        // 모든 modifier 종류(기본값이 아닌 payload).
        let mut stack = ModifierStack::new(sid(13));
        let f = |v: f32| FiniteF32::new(v).unwrap();
        let payloads = [
            ModifierPayload::Transform(Transform2D::new(1.0, 0.0, 0.0, 1.0, 2.0, 3.0).unwrap()),
            ModifierPayload::Boolean {
                op: BooleanOp::Subtract,
                operands: vec![NodeRef::new(sid(17)), NodeRef::new(sid(999))],
            },
            ModifierPayload::StrokeExpand { width: f(2.5) },
            ModifierPayload::Corner { radius: f(6.0) },
            ModifierPayload::Mask {
                source: NodeRef::new(sid(17)),
            },
            ModifierPayload::Clip {
                source: NodeRef::new(StableId::NIL),
            },
            ModifierPayload::Fill {
                color: srgb(0.0, 0.25, 0.75, 1.0),
            },
            ModifierPayload::Stroke {
                color: p3,
                width: f(0.5),
            },
            ModifierPayload::Shadow {
                color: srgb(0.0, 0.0, 0.0, 0.25),
                offset: Vec2::new(0.0, 2.0).unwrap(),
                blur: f(8.0),
            },
            ModifierPayload::Blur { radius: f(3.0) },
            ModifierPayload::Blend {
                mode: BlendMode::Multiply,
            },
            ModifierPayload::Opacity { value: f(0.75) },
        ];
        for (i, payload) in payloads.into_iter().enumerate() {
            let mid = ModifierId::new(sid(200 + i as u64));
            stack.push_back(mid, payload.kind()).unwrap();
            stack.set_payload(mid, payload).unwrap();
            if i % 5 == 4 {
                stack.set_enabled(mid, false).unwrap();
            }
        }
        model.set_modifier_stack(stack).unwrap();
        let mut card_stack = ModifierStack::new(sid(11));
        card_stack
            .push_back(ModifierId::new(sid(300)), ModifierKind::Opacity)
            .unwrap();
        model.set_modifier_stack(card_stack).unwrap();
        model
    }

    fn fixture_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE)
    }

    #[test]
    fn fixture_snapshot_is_byte_identical() {
        // 검증 기준(VI.10.4): fixture diff가 byte-identical. 출력은 정수 연산과 f32 최단 왕복
        // 표기만으로 정해져 플랫폼과 무관합니다(ADR-0006 규칙 4).
        let snapshot = model_snapshot_json(&fixture_model()).unwrap();
        assert!(snapshot.is_ascii());
        if std::env::var_os("STELLAR_UPDATE_SNAPSHOT_FIXTURES").is_some() {
            std::fs::write(fixture_path(), &snapshot).unwrap();
        }
        let expected = std::fs::read(fixture_path()).expect("fixture 파일");
        assert!(
            snapshot.as_bytes() == expected.as_slice(),
            "semantic snapshot이 fixture와 다릅니다. 의도한 변경이면 \
             STELLAR_UPDATE_SNAPSHOT_FIXTURES=1로 갱신하세요.\n{snapshot}"
        );
    }

    #[test]
    fn snapshot_is_independent_of_record_order_and_runtime_history() {
        let model = fixture_model();
        let records = model.to_records().unwrap();
        let mut shuffled = records.clone();
        shuffled.nodes.reverse();
        shuffled.modifier_stacks.reverse();
        assert_eq!(
            semantic_snapshot_json(&shuffled),
            semantic_snapshot_json(&records)
        );
        // 레코드 → 모델 재구축(새 runtime 핸들) 후에도 같습니다.
        let (reloaded, _) = DocumentModel::from_records(shuffled).unwrap();
        assert_eq!(
            model_snapshot_json(&reloaded).unwrap(),
            model_snapshot_json(&model).unwrap()
        );
    }

    #[test]
    fn snapshot_reflects_every_semantic_change() {
        let records = fixture_model().to_records().unwrap();
        let base = semantic_snapshot_json(&records);
        let mut renamed = records.clone();
        renamed.nodes[0].display_name.push('!');
        assert_ne!(semantic_snapshot_json(&renamed), base, "이름");
        let mut hidden = records.clone();
        hidden.nodes[1].visible = !hidden.nodes[1].visible;
        assert_ne!(semantic_snapshot_json(&hidden), base, "표시 여부");
        let mut reordered = records.clone();
        reordered.nodes[2].order_key = key(0x41);
        assert_ne!(semantic_snapshot_json(&reordered), base, "순서 key");
        let mut disabled = records;
        let first = disabled.modifier_stacks[0].entries()[0].id;
        disabled.modifier_stacks[0]
            .set_enabled(first, false)
            .unwrap();
        assert_ne!(semantic_snapshot_json(&disabled), base, "modifier enabled");
    }

    fn rust_sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn production_code_does_not_use_test_support() {
        // 검증 기준(VI.10.4): 테스트 전용 — 프로덕션 코드(각 파일의 첫 `#[cfg(test)]` 앞)가
        // test_support를 참조하지 않습니다. lib.rs의 모듈 선언만 예외입니다.
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_sources(&src, &mut files);
        assert!(files.len() > 20);
        for file in files {
            let relative = file.strip_prefix(&src).unwrap();
            if relative.starts_with("test_support") || relative == std::path::Path::new("lib.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap();
            let production = text.split("#[cfg(test)]").next().unwrap_or("");
            assert!(
                !production.contains("test_support"),
                "{}의 프로덕션 코드가 test_support를 참조합니다",
                relative.display()
            );
        }
        let lib = std::fs::read_to_string(src.join("lib.rs")).unwrap();
        assert_eq!(
            lib.matches("test_support").count(),
            1,
            "lib.rs는 선언 1회만"
        );
        // 선언은 cfg(test) 또는 기본값이 꺼진 feature에서만 컴파일됩니다.
        assert!(
            lib.contains(
                "#[cfg(any(test, feature = \"test-support\"))]\n#[doc(hidden)]\npub mod test_support;"
            ),
            "lib.rs의 test_support 선언은 test-support feature로 gate돼야 합니다"
        );
        let manifest =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
                .unwrap();
        let features = manifest_section(&manifest, "features");
        assert!(features.contains("test-support = []"), "feature 정의");
        assert!(
            !features.contains("default"),
            "test-support는 기본 feature가 아닙니다"
        );
    }

    use std::path::{Path, PathBuf};

    /// workspace root Cargo.toml의 `members` 목록.
    fn workspace_members(root: &Path) -> Vec<PathBuf> {
        let text = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        let start = text.find("members").expect("members");
        let open = start + text[start..].find('[').expect("[");
        let close = open + text[open..].find(']').expect("]");
        text[open + 1..close]
            .split(',')
            .map(|m| m.trim().trim_matches('"'))
            .filter(|m| !m.is_empty())
            .map(|m| root.join(m))
            .collect()
    }

    /// `[name]` 표 본문(다음 표 머리 전까지). 없으면 빈 문자열.
    fn manifest_section(text: &str, name: &str) -> String {
        let header = format!("[{name}]");
        let mut out = String::new();
        let mut inside = false;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                inside = trimmed == header;
                continue;
            }
            if inside {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    }

    /// 파일 첫 항목이 test 조건 inner attribute(`#![cfg(test)]`, `#![cfg(any(test, feature =
    /// "test-support"))]`)인 파일은 전체가 테스트 전용입니다.
    fn file_is_test_only(text: &str) -> bool {
        text.lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with("//"))
            .is_some_and(|l| {
                l == "#![cfg(test)]" || l == "#![cfg(any(test, feature = \"test-support\"))]"
            })
    }

    /// `[features]` 본문에 test-support가 나오면, 그 정의가 `test-support = [..]` 한 줄이고
    /// 각 항목이 `<crate>/test-support`이며 `default`가 test-support를 켜지 않는지.
    fn features_only_forward_test_support(body: &str) -> bool {
        for line in body.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') || !line.contains("test-support") {
                continue;
            }
            let Some(value) = line.strip_prefix("test-support = [") else {
                return false;
            };
            let Some(items) = value.strip_suffix(']') else {
                return false;
            };
            let all_forward = items
                .split(',')
                .map(str::trim)
                .filter(|i| !i.is_empty())
                .all(|i| {
                    let i = i.trim_matches('"');
                    i.ends_with("/test-support") && !i.starts_with("dep:")
                });
            if !all_forward {
                return false;
            }
        }
        !body
            .lines()
            .map(str::trim)
            .any(|l| l.starts_with("default") && l.contains("test-support"))
    }

    /// 비테스트 소스(첫 `#[cfg(test)]` 앞)에서 `stellar_document`로 시작하는 경로·use 문이
    /// `test_support`에 닿는지. use group(`stellar_document::{.., test_support}`)도 잡도록 각
    /// 등장 위치부터 다음 `;`까지를 봅니다.
    fn references_document_test_support(production: &str) -> bool {
        production.match_indices("stellar_document").any(|(at, _)| {
            let rest = &production[at..];
            let statement = rest.split(';').next().unwrap_or(rest);
            statement.contains("test_support")
        })
    }

    #[test]
    fn workspace_production_code_does_not_use_test_support() {
        // 검증 기준(VI.10.4) 확장: workspace의 다른 모든 crate(crates/*, tools/*)의 비테스트
        // 소스가 document test_support를 참조하지 않고, 어떤 manifest도 `[dev-dependencies]`
        // 밖에서 test-support feature를 켜지 않습니다.
        let document = Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = document.join("../..");
        let members = workspace_members(&root);
        assert!(members.len() > 20, "members {}", members.len());
        assert!(members.iter().any(|m| m.ends_with("crates/project")));
        let mut scanned = 0usize;
        for member in &members {
            let manifest = std::fs::read_to_string(member.join("Cargo.toml")).unwrap();
            let is_document = member.ends_with("crates/document");
            for section in ["dependencies", "build-dependencies", "features"] {
                if is_document && section == "features" {
                    continue;
                }
                let body = manifest_section(&manifest, section);
                let offending = if section == "features" {
                    // 허용: 기본값이 아닌 `test-support` feature가 의존 crate의 test-support만
                    // 전달하는 정의(다른 crate의 테스트가 dev-dependencies로 켤 때 필요).
                    !features_only_forward_test_support(&body)
                } else {
                    body.contains("test-support")
                };
                assert!(
                    !offending,
                    "{}의 [{section}]가 test-support를 켭니다(dev-dependencies만 허용)",
                    member.display()
                );
            }
            if is_document {
                continue;
            }
            let src = member.join("src");
            if !src.is_dir() {
                continue;
            }
            let mut files = Vec::new();
            rust_sources(&src, &mut files);
            for file in files {
                let text = std::fs::read_to_string(&file).unwrap();
                if file_is_test_only(&text) {
                    continue;
                }
                let production = text.split("#[cfg(test)]").next().unwrap_or("");
                assert!(
                    !references_document_test_support(production),
                    "{}의 프로덕션 코드가 stellar_document::test_support를 참조합니다",
                    file.display()
                );
                scanned += 1;
            }
        }
        assert!(scanned > 100, "검사한 소스 {scanned}개");
        // scanner 자체 검증(positive control).
        assert!(references_document_test_support(
            "use stellar_document::test_support::json::Json;"
        ));
        assert!(references_document_test_support(
            "use stellar_document::{model::DocumentModel, test_support::semantic_snapshot};"
        ));
        assert!(!references_document_test_support(
            "use stellar_document::model::DocumentModel;\nmod test_support;"
        ));
    }
}

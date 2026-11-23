//! 쓰기 v4 golden bytes와 거부 규칙.
//!
//! golden 텍스트의 registry 의존 키(`{frame:0x0104}` 등)는 registry 조회로 채웁니다. registry가 이름을 바꾸면
//! (IX.11.2) 이 golden은 그 이름을 따라가고 나머지 bytes(값 문법·줄 순서·인용)는 고정입니다.

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::PropertyKey;
use stellar_core_types::reference::{AssetRef, ComponentRef, NodeRef};
use stellar_core_types::text::UserText;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::{FiniteF32, Size2, Transform2D, Vec2};
use stellar_document::component::definition::{ComponentPropertyId, ComponentSlotId};
use stellar_document::component::instance::{ComponentInstancePayload, OverrideKey, SlotOverride};
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::frame::{Border, Clipping, CornerRadius, FramePayload};
use stellar_document::frame_metadata::frame_property;
use stellar_document::image::ImagePayload;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierPayload, ModifierStack};
use stellar_document::modifier_metadata::modifier_property;
use stellar_document::path::{
    CommandIds, IdentifiedPath, PathCommand, PathData, PathHandleId, PathPointId, WindingRule,
};
use stellar_document::payload_metadata::{image_property, shape_property};
use stellar_document::records::{DocumentRecords, NodePayload, NodeRecord};
use stellar_document::shape::{RectangleParams, ShapeGeometry, ShapeKind};
use stellar_document::text::{Paragraph, StyleRun, TextPayload, TextRange};

use super::super::text_codec::{decode, encode};
use super::super::text_source::TextSourceError;

pub(crate) fn sid(n: u64) -> StableId {
    StableId::new_v7(n, 0, n)
}

fn record(id: u64, parent: Option<u64>, key: u8, payload: NodePayload) -> NodeRecord {
    NodeRecord {
        id: sid(id),
        parent: parent.map(sid),
        order_key: OrderKey::from_bytes(vec![key]).unwrap(),
        visible: true,
        locked: false,
        display_name: format!("n{id}"),
        payload,
    }
}

/// 모든 node 종류와 여러 modifier를 담은 문서.
pub(crate) fn golden_model() -> DocumentModel {
    let mut definition = DocumentDefinition::new(sid(1), "Doc".into());
    let mut page = PageDefinition::new(sid(2), "Page".into());
    page.add_view(ViewDefinition::new(sid(3), "View".into(), sid(10)))
        .unwrap();
    definition.add_page(page).unwrap();
    let red = Color::new(ColorSpace::Srgb, 1.0, 0.0, 0.0, 1.0).unwrap();
    let p3 = Color::new(ColorSpace::DisplayP3, 0.25, 0.5, 1.0, 0.75).unwrap();
    let frame = FramePayload {
        size: Size2::new(1440.0, 900.5).unwrap(),
        transform: Transform2D::new(1.0, 0.0, 0.0, 1.0, 12.5, -4.0).unwrap(),
        clipping: Clipping::FrameBounds,
        background: Some(p3),
        border: Some(Border::new(red, 1.5).unwrap()),
        corner_radius: CornerRadius::new(1.0, 2.0, 3.0, 4.0).unwrap(),
        layout_mode: None,
    };
    let mut instance = ComponentInstancePayload::new(ComponentRef::new(sid(90)));
    instance.set_property_override(
        OverrideKey::direct(ComponentPropertyId::new(sid(91))),
        TypedValue::Boolean(true),
    );
    instance.set_slot_override(
        ComponentSlotId::new(sid(92)),
        SlotOverride::new(vec![sid(93)]).unwrap(),
    );
    let path = IdentifiedPath::new(
        PathData::new(
            WindingRule::EvenOdd,
            vec![
                PathCommand::MoveTo(Vec2::new(0.0, 0.0).unwrap()),
                PathCommand::CubicTo {
                    c1: Vec2::new(1.0, 2.0).unwrap(),
                    c2: Vec2::new(3.0, 4.0).unwrap(),
                    to: Vec2::new(5.0, 6.0).unwrap(),
                },
                PathCommand::Close,
            ],
        )
        .unwrap(),
        vec![
            CommandIds {
                point: Some(PathPointId(sid(60))),
                handles: None,
            },
            CommandIds {
                point: Some(PathPointId(sid(61))),
                handles: Some((PathHandleId(sid(62)), PathHandleId(sid(63)))),
            },
            CommandIds {
                point: None,
                handles: None,
            },
        ],
    )
    .unwrap();
    let mut records = DocumentRecords::new(definition);
    records.nodes = vec![
        record(10, None, 0x80, NodePayload::Frame(frame)),
        record(
            11,
            Some(10),
            0x40,
            NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
                size: Size2::new(10.0, 20.0).unwrap(),
                corner_radius: CornerRadius::ZERO,
            })),
        ),
        record(
            12,
            Some(10),
            0x50,
            NodePayload::Text(
                TextPayload::new(
                    UserText::new("Hi\n둘"),
                    vec![Paragraph { style: 0 }, Paragraph { style: 1 }],
                    vec![StyleRun {
                        range: TextRange::new(0, 2).unwrap(),
                        style: 0,
                    }],
                )
                .unwrap(),
            ),
        ),
        record(13, Some(10), 0x60, NodePayload::Path(path)),
        record(
            14,
            Some(10),
            0x70,
            NodePayload::Image(ImagePayload::new(
                AssetRef::new(sid(80)),
                Size2::new(64.0, 48.0).unwrap(),
            )),
        ),
        record(15, Some(10), 0x78, NodePayload::ComponentInstance(instance)),
    ];
    let mut stack = ModifierStack::new(sid(11));
    for (n, key, payload) in [
        (20, 0x40, ModifierPayload::Fill { color: red }),
        (
            21,
            0x80,
            ModifierPayload::Shadow {
                color: p3,
                offset: Vec2::new(2.0, -3.0).unwrap(),
                blur: FiniteF32::new(4.5).unwrap(),
            },
        ),
        (
            22,
            0xc0,
            ModifierPayload::Clip {
                source: NodeRef::new(sid(13)),
            },
        ),
    ] {
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(sid(n)),
                order: OrderKey::from_bytes(vec![key]).unwrap(),
                kind: payload.kind(),
                enabled: n != 21,
                payload,
            })
            .unwrap();
    }
    records.modifier_stacks.push(stack);
    DocumentModel::from_records(records).unwrap().0
}

/// canonical writer가 키를 인용하지 않는 안전한 plain 식별자인지(writer의 인용 정책과 같음).
fn yaml_key(name: &str) -> String {
    let plain = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !matches!(
            name.to_ascii_lowercase().as_str(),
            "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "y" | "n"
        );
    if plain {
        name.to_owned()
    } else {
        format!("\"{name}\"")
    }
}

/// `{frame:0x0104}` 같은 자리를 registry 이름으로 채웁니다.
pub(crate) fn fill_registry_names(template: &str) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let end = start + rest[start..].find('}').unwrap();
        let (family, key) = rest[start + 1..end].split_once(':').unwrap();
        let key = PropertyKey(u32::from_str_radix(key.trim_start_matches("0x"), 16).unwrap());
        let name = match family {
            "frame" => frame_property(key).unwrap().name,
            "rectangle" => shape_property(ShapeKind::Rectangle, key).unwrap().name,
            "image" => image_property(key).unwrap().name,
            "modifier" => modifier_property(key).unwrap().name,
            other => panic!("{other}"),
        };
        out.push_str(&rest[..start]);
        out.push_str(&yaml_key(name));
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

#[test]
fn view_v4_golden_bytes_use_registry_names_and_round_trip() {
    let expected = fill_registry_names(include_str!("golden/view-v4.stellar.yaml.in"));
    let model = golden_model();
    let text = encode(&model).unwrap();
    assert_eq!(text, expected);
    let decoded = decode(&text).unwrap();
    assert_eq!(decoded.to_records().unwrap(), model.to_records().unwrap());
    assert_eq!(encode(&decoded).unwrap(), text);
}

#[test]
fn unknown_keys_values_and_unrepresentable_meaning_are_rejected() {
    let text = encode(&golden_model()).unwrap();
    let width = yaml_key(frame_property(PropertyKey(0x0108)).unwrap().name);
    let cases = [
        // 모르는 키.
        text.replacen("      clipping:", "      future-clipping:", 1),
        format!("{text}future: none\n"),
        // 모르는 값과 canonical이 아닌 숫자 표기.
        text.replacen("\"1440px\"", "\"1440.0px\"", 1),
        text.replacen("\"1440px\"", "\"1440\"", 1),
        // ADR-0075 정규형이 아닌 색 표기(8비트 정확 sRGB는 `#rrggbb`).
        text.replacen("\"#ff0000\"", "\"color(srgb 1 0 0)\"", 1),
        text.replacen("\"#ff0000\"", "\"#FF0000\"", 1),
        text.replacen("\"#ff0000\"", "\"#f00\"", 1),
        text.replacen("\"#ff0000\"", "red", 1),
        text.replacen("border-top-style: solid", "border-top-style: dashed", 1),
        // 변마다 다른 테두리는 모델이 표현하지 않습니다.
        text.replacen(
            "border-left-width: \"1.5px\"",
            "border-left-width: \"2px\"",
            1,
        ),
        // 빠진 키.
        text.replacen(&format!("      {width}: \"1440px\"\n"), "", 1),
        // modifier는 order 순서로 씁니다.
        text.replacen("order: \"40\"", "order: d0", 1),
        // 같은 번호의 옛 문법.
        text.replacen("schema_version: \"4\"", "schema_version: \"3\"", 1),
        // registry 이름과 다른 옛 파일 키.
        text.replacen("border-top-width:", "border-width:", 1),
    ];
    for case in cases {
        assert_ne!(case, text);
        assert!(decode(&case).is_err(), "{case}");
    }
}

#[test]
fn unrepresentable_typed_values_refuse_to_write() {
    let mut records = golden_model().to_records().unwrap();
    let node = records
        .nodes
        .iter_mut()
        .find(|n| matches!(n.payload, NodePayload::ComponentInstance(_)))
        .unwrap();
    let NodePayload::ComponentInstance(instance) = &mut node.payload else {
        unreachable!()
    };
    instance.set_property_override(
        OverrideKey::direct(ComponentPropertyId::new(sid(99))),
        TypedValue::Typography(
            stellar_core_types::typed_value::TypographyValue::from_record(
                &stellar_core_types::typography::Typography::default(),
            )
            .unwrap(),
        ),
    );
    let model = DocumentModel::from_records(records).unwrap().0;
    assert!(matches!(
        encode(&model),
        Err(TextSourceError::Unsupported(_))
    ));
}

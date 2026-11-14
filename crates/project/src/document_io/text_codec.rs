//! ADR-0059 첫 텍스트 원본 범위: 단일 문서·페이지·View와 Frame/Fragment.
//!
//! 모든 저장 속성을 명시하며, 미지원 레코드나 키는 생략하지 않고 거부합니다.

use std::fmt::Display;
use std::str::FromStr;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::NodeRef;
use stellar_core_types::value::{Size2, Transform2D};
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::{Border, Clipping, CornerRadius, FramePayload};
use stellar_document::model::DocumentModel;
use stellar_document::records::{DocumentRecords, NodePayload, NodeRecord};
use stellar_document::validate::validate_document;

use super::text_source::TextSourceError;
use crate::yaml::{load_restricted, write_canonical_checked, YamlValue};

fn invalid(error: impl Display) -> TextSourceError {
    TextSourceError::Invalid(error.to_string())
}

fn scalar(value: impl Display) -> YamlValue {
    YamlValue::scalar(value.to_string())
}

fn mapping<const N: usize>(fields: [(&str, YamlValue); N]) -> YamlValue {
    YamlValue::Mapping(
        fields
            .into_iter()
            .map(|(key, value)| (scalar(key), value))
            .collect(),
    )
}

fn floats(values: &[f32]) -> YamlValue {
    scalar(
        values
            .iter()
            .map(f32::to_string)
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn optional<T>(value: Option<T>, encode: impl FnOnce(T) -> YamlValue) -> YamlValue {
    value.map_or_else(|| scalar("none"), encode)
}

fn color(value: Color) -> YamlValue {
    mapping([
        (
            "space",
            scalar(match value.space() {
                ColorSpace::Srgb => "srgb",
                ColorSpace::DisplayP3 => "display-p3",
            }),
        ),
        (
            "rgba",
            floats(&[value.r(), value.g(), value.b(), value.alpha()]),
        ),
    ])
}

fn frame(payload: &FramePayload) -> YamlValue {
    let FramePayload {
        size,
        transform,
        clipping,
        background,
        border,
        corner_radius,
        layout_mode,
    } = payload;
    let (a, b, c, d, tx, ty) = transform.components();
    let (tl, tr, br, bl) = corner_radius.components();
    mapping([
        ("size", floats(&[size.width(), size.height()])),
        ("transform", floats(&[a, b, c, d, tx, ty])),
        (
            "clipping",
            match clipping {
                Clipping::Disabled => mapping([("kind", scalar("disabled"))]),
                Clipping::FrameBounds => mapping([("kind", scalar("frame-bounds"))]),
                Clipping::Path(target) => {
                    mapping([("kind", scalar("path")), ("target", scalar(target.id()))])
                }
            },
        ),
        ("background", optional(*background, color)),
        (
            "border",
            optional(*border, |value| {
                mapping([
                    ("color", color(value.color)),
                    ("thickness", scalar(value.thickness())),
                ])
            }),
        ),
        ("corner_radius", floats(&[tl, tr, br, bl])),
        ("layout_mode", optional(*layout_mode, scalar)),
    ])
}

fn node(record: &NodeRecord) -> Result<YamlValue, TextSourceError> {
    let (kind, payload) = match &record.payload {
        NodePayload::Frame(value) => ("frame", frame(value)),
        NodePayload::Fragment(_) => ("fragment", scalar("none")),
        other => {
            return Err(TextSourceError::Unsupported(format!(
                "node {}: {} payload",
                record.id,
                other.kind().name()
            )))
        }
    };
    let order_key: String = record
        .order_key
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(mapping([
        ("id", scalar(record.id)),
        ("parent", optional(record.parent, scalar)),
        ("order_key", scalar(order_key)),
        ("visible", scalar(record.visible)),
        ("locked", scalar(record.locked)),
        ("display_name", scalar(&record.display_name)),
        ("kind", scalar(kind)),
        ("payload", payload),
    ]))
}

/// 지원 범위를 검증한 뒤 결정론적인 제한 YAML로 기록합니다.
pub fn encode(model: &DocumentModel) -> Result<String, TextSourceError> {
    let records = model.to_records().map_err(invalid)?;
    validate_document(&records).map_err(invalid)?;
    let [page] = records.definition.pages.as_slice() else {
        return Err(TextSourceError::Unsupported(
            "정확히 하나의 페이지가 필요합니다".into(),
        ));
    };
    let [view] = page.views.as_slice() else {
        return Err(TextSourceError::Unsupported(
            "정확히 하나의 View가 필요합니다".into(),
        ));
    };
    if !records.modifier_stacks.is_empty() {
        return Err(TextSourceError::Unsupported("modifier stack".into()));
    }
    let value = mapping([
        ("schema_version", scalar(1)),
        (
            "document",
            mapping([
                ("id", scalar(records.definition.id)),
                ("display_name", scalar(records.definition.display_name())),
            ]),
        ),
        (
            "page",
            mapping([
                ("id", scalar(page.id)),
                ("display_name", scalar(page.display_name())),
            ]),
        ),
        (
            "view",
            mapping([
                ("id", scalar(view.id)),
                ("display_name", scalar(view.display_name())),
                ("root_frame", scalar(view.root_frame)),
            ]),
        ),
        (
            "nodes",
            YamlValue::Sequence(records.nodes.iter().map(node).collect::<Result<_, _>>()?),
        ),
    ]);
    write_canonical_checked(&value).map_err(invalid)
}

fn fields(value: &YamlValue, required: &[&str]) -> Result<(), TextSourceError> {
    let pairs = value
        .as_mapping()
        .ok_or_else(|| invalid("mapping이 필요합니다"))?;
    for (key, _) in pairs {
        let name = text(key)?;
        if !required.contains(&name) {
            return Err(TextSourceError::Unsupported(format!(
                "알 수 없는 키: {name}"
            )));
        }
    }
    for name in required {
        field(value, name)?;
    }
    Ok(())
}

fn field<'a>(value: &'a YamlValue, key: &str) -> Result<&'a YamlValue, TextSourceError> {
    value
        .get(key)
        .ok_or_else(|| invalid(format!("필수 키 누락: {key}")))
}

fn text(value: &YamlValue) -> Result<&str, TextSourceError> {
    value
        .as_scalar()
        .ok_or_else(|| invalid("scalar가 필요합니다"))
}

fn parse<T: FromStr>(value: &YamlValue) -> Result<T, TextSourceError> {
    text(value)?
        .parse()
        .map_err(|_| invalid("scalar 값 형식이 잘못되었습니다"))
}

fn parse_optional<T>(
    value: &YamlValue,
    decode: impl FnOnce(&YamlValue) -> Result<T, TextSourceError>,
) -> Result<Option<T>, TextSourceError> {
    if value.as_scalar() == Some("none") {
        Ok(None)
    } else {
        decode(value).map(Some)
    }
}

fn parse_floats<const N: usize>(value: &YamlValue) -> Result<[f32; N], TextSourceError> {
    let mut result = [0.0_f32; N];
    let mut values = text(value)?.split_whitespace();
    for slot in &mut result {
        let number = values
            .next()
            .ok_or_else(|| invalid("숫자 개수가 부족합니다"))?;
        *slot = number
            .parse()
            .map_err(|_| invalid("float 형식이 잘못되었습니다"))?;
        if !slot.is_finite() {
            return Err(invalid("float은 유한해야 합니다"));
        }
    }
    if values.next().is_some() {
        return Err(invalid("숫자 개수가 너무 많습니다"));
    }
    Ok(result)
}

fn parse_color(value: &YamlValue) -> Result<Color, TextSourceError> {
    fields(value, &["space", "rgba"])?;
    let space = match text(field(value, "space")?)? {
        "srgb" => ColorSpace::Srgb,
        "display-p3" => ColorSpace::DisplayP3,
        _ => return Err(TextSourceError::Unsupported("알 수 없는 색공간".into())),
    };
    let [r, g, b, a] = parse_floats(field(value, "rgba")?)?;
    Color::new(space, r, g, b, a).map_err(invalid)
}

fn parse_border(value: &YamlValue) -> Result<Border, TextSourceError> {
    fields(value, &["color", "thickness"])?;
    Border::new(
        parse_color(field(value, "color")?)?,
        parse(field(value, "thickness")?)?,
    )
    .map_err(invalid)
}

fn parse_clipping(value: &YamlValue) -> Result<Clipping, TextSourceError> {
    match text(field(value, "kind")?)? {
        "disabled" => {
            fields(value, &["kind"])?;
            Ok(Clipping::Disabled)
        }
        "frame-bounds" => {
            fields(value, &["kind"])?;
            Ok(Clipping::FrameBounds)
        }
        "path" => {
            fields(value, &["kind", "target"])?;
            Ok(Clipping::Path(NodeRef::new(parse(field(
                value, "target",
            )?)?)))
        }
        _ => Err(TextSourceError::Unsupported(
            "알 수 없는 clipping 방식".into(),
        )),
    }
}

fn parse_frame(value: &YamlValue) -> Result<FramePayload, TextSourceError> {
    fields(
        value,
        &[
            "size",
            "transform",
            "clipping",
            "background",
            "border",
            "corner_radius",
            "layout_mode",
        ],
    )?;
    let [width, height] = parse_floats(field(value, "size")?)?;
    let [a, b, c, d, tx, ty] = parse_floats(field(value, "transform")?)?;
    let [tl, tr, br, bl] = parse_floats(field(value, "corner_radius")?)?;
    Ok(FramePayload {
        size: Size2::new(width, height).map_err(invalid)?,
        transform: Transform2D::new(a, b, c, d, tx, ty).map_err(invalid)?,
        clipping: parse_clipping(field(value, "clipping")?)?,
        background: parse_optional(field(value, "background")?, parse_color)?,
        border: parse_optional(field(value, "border")?, parse_border)?,
        corner_radius: CornerRadius::new(tl, tr, br, bl).map_err(invalid)?,
        layout_mode: parse_optional(field(value, "layout_mode")?, parse)?,
    })
}

fn parse_order_key(value: &YamlValue) -> Result<OrderKey, TextSourceError> {
    let hex = text(value)?;
    if !hex.is_ascii() || hex.len() % 2 != 0 {
        return Err(invalid("OrderKey는 2자리씩 구분되는 hex여야 합니다"));
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|offset| u8::from_str_radix(&hex[offset..offset + 2], 16).map_err(invalid))
        .collect::<Result<_, _>>()?;
    OrderKey::from_bytes(bytes).map_err(invalid)
}

fn parse_node(value: &YamlValue) -> Result<NodeRecord, TextSourceError> {
    fields(
        value,
        &[
            "id",
            "parent",
            "order_key",
            "visible",
            "locked",
            "display_name",
            "kind",
            "payload",
        ],
    )?;
    let payload = field(value, "payload")?;
    let payload = match text(field(value, "kind")?)? {
        "frame" => NodePayload::Frame(parse_frame(payload)?),
        "fragment" if payload.as_scalar() == Some("none") => NodePayload::Fragment(FragmentPayload),
        "fragment" => return Err(invalid("Fragment payload는 none이어야 합니다")),
        kind => {
            return Err(TextSourceError::Unsupported(format!(
                "node payload: {kind}"
            )))
        }
    };
    Ok(NodeRecord {
        id: parse(field(value, "id")?)?,
        parent: parse_optional(field(value, "parent")?, parse)?,
        order_key: parse_order_key(field(value, "order_key")?)?,
        visible: parse(field(value, "visible")?)?,
        locked: parse(field(value, "locked")?)?,
        display_name: text(field(value, "display_name")?)?.to_owned(),
        payload,
    })
}

/// 제한 YAML을 엄격하게 해석하고 문서 validator로 runtime 모델을 재구축합니다.
pub fn decode(source: &str) -> Result<DocumentModel, TextSourceError> {
    let value = load_restricted(source).map_err(invalid)?;
    fields(
        &value,
        &["schema_version", "document", "page", "view", "nodes"],
    )?;
    if text(field(&value, "schema_version")?)? != "1" {
        return Err(TextSourceError::Unsupported(
            "text source schema_version".into(),
        ));
    }
    let document = field(&value, "document")?;
    let page = field(&value, "page")?;
    let view = field(&value, "view")?;
    fields(document, &["id", "display_name"])?;
    fields(page, &["id", "display_name"])?;
    fields(view, &["id", "display_name", "root_frame"])?;
    let mut definition = DocumentDefinition::new(
        parse(field(document, "id")?)?,
        text(field(document, "display_name")?)?.to_owned(),
    );
    let mut page_definition = PageDefinition::new(
        parse(field(page, "id")?)?,
        text(field(page, "display_name")?)?.to_owned(),
    );
    page_definition
        .add_view(ViewDefinition::new(
            parse(field(view, "id")?)?,
            text(field(view, "display_name")?)?.to_owned(),
            parse(field(view, "root_frame")?)?,
        ))
        .map_err(invalid)?;
    definition.add_page(page_definition).map_err(invalid)?;
    let nodes = field(&value, "nodes")?
        .as_sequence()
        .ok_or_else(|| invalid("nodes는 sequence여야 합니다"))?;
    let records = DocumentRecords {
        definition,
        nodes: nodes.iter().map(parse_node).collect::<Result<_, _>>()?,
        modifier_stacks: Vec::new(),
    };
    DocumentModel::from_records(records)
        .map(|(model, _)| model)
        .map_err(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::ids::StableId;
    use stellar_document::modifier::{ModifierId, ModifierKind, ModifierStack};

    fn sid(value: u64) -> StableId {
        StableId::new_v7(value, 0, value)
    }

    fn model() -> DocumentModel {
        let mut definition = DocumentDefinition::new(sid(1), "문서\n이름".into());
        let mut page = PageDefinition::new(sid(2), "none".into());
        page.add_view(ViewDefinition::new(sid(3), "".into(), sid(10)))
            .unwrap();
        definition.add_page(page).unwrap();
        let color = Color::new(ColorSpace::DisplayP3, -0.125, 1.25, 0.1, 0.5).unwrap();
        let frame = FramePayload {
            size: Size2::new(0.1, f32::MIN_POSITIVE).unwrap(),
            transform: Transform2D::new(-1.0, 0.25, -0.75, 1.0, 10.0, -20.0).unwrap(),
            clipping: Clipping::Path(NodeRef::new(sid(99))),
            background: Some(color),
            border: Some(Border::new(color, 0.3).unwrap()),
            corner_radius: CornerRadius::new(1.0, 2.0, 3.0, 4.0).unwrap(),
            layout_mode: Some(sid(100)),
        };
        let mut records = DocumentRecords::new(definition);
        records.nodes = vec![
            NodeRecord {
                id: sid(11),
                parent: Some(sid(10)),
                order_key: OrderKey::from_bytes(vec![0, 0, 127]).unwrap(),
                visible: false,
                locked: true,
                display_name: "null: \"자식\"".into(),
                payload: NodePayload::Fragment(FragmentPayload),
            },
            NodeRecord {
                id: sid(10),
                parent: None,
                order_key: OrderKey::between(None, None),
                visible: true,
                locked: false,
                display_name: "root".into(),
                payload: NodePayload::Frame(frame),
            },
        ];
        DocumentModel::from_records(records).unwrap().0
    }

    #[test]
    fn all_supported_properties_round_trip_with_stable_text() {
        let model = model();
        let first = encode(&model).unwrap();
        let decoded = decode(&first).unwrap();
        assert_eq!(decoded.to_records().unwrap(), model.to_records().unwrap());
        assert_eq!(encode(&decoded).unwrap(), first);
        assert!(
            first.find(&sid(10).to_string()).unwrap() < first.rfind(&sid(11).to_string()).unwrap()
        );
    }

    #[test]
    fn omitted_unknown_and_invalid_fields_never_default() {
        let source = encode(&model()).unwrap();
        for changed in [
            source.replacen("schema_version: \"1\"", "schema_version: \"2\"", 1),
            source.replacen("layout_mode:", "unknown_property:", 1),
            source.replacen("kind: fragment", "kind: shape", 1),
            source.replacen("visible: \"true\"", "visible: yes", 1),
            format!("{source}modifier_stacks: []\n"),
        ] {
            assert_ne!(changed, source);
            assert!(decode(&changed).is_err(), "{changed}");
        }
    }

    #[test]
    fn modifiers_and_extra_pages_are_rejected_before_encoding() {
        let mut model = model();
        let mut stack = ModifierStack::new(sid(10));
        stack
            .push_back(ModifierId::new(sid(101)), ModifierKind::Opacity)
            .unwrap();
        model.set_modifier_stack(stack).unwrap();
        assert!(matches!(
            encode(&model),
            Err(TextSourceError::Unsupported(_))
        ));

        let mut records = model.to_records().unwrap();
        records.modifier_stacks.clear();
        records
            .definition
            .add_page(PageDefinition::new(sid(102), String::new()))
            .unwrap();
        let (model, _) = DocumentModel::from_records(records).unwrap();
        assert!(matches!(
            encode(&model),
            Err(TextSourceError::Unsupported(_))
        ));
    }
}

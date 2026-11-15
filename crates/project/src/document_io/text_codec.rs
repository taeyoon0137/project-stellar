//! ADR-0059 단일 View 텍스트 원본. 독립 컴포넌트는 정의 ID로 참조합니다.
//!
//! 모든 저장 속성을 명시하며, 미지원 레코드나 키는 생략하지 않고 거부합니다.

use std::fmt::Display;
use std::str::FromStr;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{ComponentRef, NodeRef};
use stellar_core_types::text::UserText;
use stellar_core_types::typed_value::TypographyValue;
use stellar_core_types::typography::{FontSize, FontStyle, LetterSpacing, LineHeight, Typography};
use stellar_core_types::value::{FiniteF32, Size2, Transform2D};
use stellar_document::component::instance::ComponentInstancePayload;
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::{Border, Clipping, CornerRadius, FramePayload};
use stellar_document::model::DocumentModel;
use stellar_document::modifier::{
    ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
};
use stellar_document::records::{DocumentRecords, NodePayload, NodeRecord};
use stellar_document::shape::ShapeKind;
use stellar_document::text::{
    LineHeight as ParagraphLineHeight, Paragraph, ParagraphStyle, StyleRun, TextAlignment,
    TextPayload, TextRange, TextStyle, WrappingMode,
};
use stellar_document::text_styles::DocumentTextStyles;
use stellar_document::validate::validate_document;

use super::text_source::TextSourceError;
use crate::yaml::{load_restricted, write_canonical_checked, YamlValue};

mod document_font;
mod shape;

pub(super) fn invalid(error: impl Display) -> TextSourceError {
    TextSourceError::Invalid(error.to_string())
}

pub(super) fn scalar(value: impl Display) -> YamlValue {
    YamlValue::scalar(value.to_string())
}

pub(super) fn mapping<const N: usize>(fields: [(&str, YamlValue); N]) -> YamlValue {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FrameDimensions {
    Paired,
    Longhands,
}

pub(super) fn frame(payload: &FramePayload, dimensions: FrameDimensions) -> YamlValue {
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
    let transform = ("transform", floats(&[a, b, c, d, tx, ty]));
    let clipping = (
        "clipping",
        match clipping {
            Clipping::Disabled => mapping([("kind", scalar("disabled"))]),
            Clipping::FrameBounds => mapping([("kind", scalar("frame-bounds"))]),
            Clipping::Path(target) => {
                mapping([("kind", scalar("path")), ("target", scalar(target.id()))])
            }
        },
    );
    let background = ("background", optional(*background, color));
    let border = (
        "border",
        optional(*border, |value| {
            mapping([
                ("color", color(value.color)),
                ("thickness", scalar(value.thickness())),
            ])
        }),
    );
    let corner_radius = ("corner_radius", floats(&[tl, tr, br, bl]));
    let layout_mode = ("layout_mode", optional(*layout_mode, scalar));
    match dimensions {
        FrameDimensions::Paired => mapping([
            ("size", floats(&[size.width(), size.height()])),
            transform,
            clipping,
            background,
            border,
            corner_radius,
            layout_mode,
        ]),
        FrameDimensions::Longhands => mapping([
            background,
            border,
            clipping,
            corner_radius,
            ("height", scalar(size.height())),
            layout_mode,
            transform,
            ("width", scalar(size.width())),
        ]),
    }
}

fn text_payload(payload: &TextPayload) -> Result<YamlValue, TextSourceError> {
    // TextRange의 public 필드로 직접 만든 역전 범위도 쓰기 전에 거부합니다.
    if payload
        .style_runs()
        .iter()
        .any(|run| run.range.start > run.range.end)
    {
        return Err(invalid("Text run의 start는 end 이하여야 합니다"));
    }
    Ok(mapping([
        ("content", scalar(payload.content())),
        (
            "paragraphs",
            YamlValue::Sequence(
                payload
                    .paragraphs()
                    .iter()
                    .map(|paragraph| mapping([("style", scalar(paragraph.style))]))
                    .collect(),
            ),
        ),
        (
            "style_runs",
            YamlValue::Sequence(
                payload
                    .style_runs()
                    .iter()
                    .map(|run| {
                        mapping([
                            ("start", scalar(run.range.start.0)),
                            ("end", scalar(run.range.end.0)),
                            ("style", scalar(run.style)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]))
}

pub(super) fn node(
    record: &NodeRecord,
    dimensions: FrameDimensions,
) -> Result<YamlValue, TextSourceError> {
    let (kind, payload) = match &record.payload {
        NodePayload::Frame(value) => ("frame", frame(value, dimensions)),
        NodePayload::Fragment(_) => ("fragment", scalar("none")),
        NodePayload::Text(value) => ("text", text_payload(value)?),
        other => {
            return Err(TextSourceError::Unsupported(format!(
                "node {}: {} payload",
                record.id,
                other.kind().name()
            )))
        }
    };
    Ok(node_fields(record, kind, payload))
}

fn order_key(key: &OrderKey) -> YamlValue {
    scalar(
        key.as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
    )
}

fn node_fields(record: &NodeRecord, kind: &str, payload: YamlValue) -> YamlValue {
    mapping([
        ("id", scalar(record.id)),
        ("parent", optional(record.parent, scalar)),
        ("order_key", order_key(&record.order_key)),
        ("visible", scalar(record.visible)),
        ("locked", scalar(record.locked)),
        ("display_name", scalar(&record.display_name)),
        ("kind", scalar(kind)),
        ("payload", payload),
    ])
}

/// 배치 명령의 단일 활성 평행이동만 표현합니다. 다른 modifier를 좌표로 축약하지 않습니다.
fn view_node(
    record: &NodeRecord,
    stack: Option<&ModifierStack>,
    document_font: bool,
) -> Result<YamlValue, TextSourceError> {
    if let NodePayload::Shape(geometry) = &record.payload {
        return Ok(node_fields(
            record,
            geometry.kind().name(),
            shape::encode(geometry, stack)?,
        ));
    }
    if let (true, NodePayload::Text(text)) = (document_font, &record.payload) {
        document_font::check_text(text)?;
        let YamlValue::Mapping(mut fields) = text_payload(text)? else {
            unreachable!()
        };
        fields.push((
            scalar("placement"),
            stack
                .map(translation)
                .transpose()?
                .unwrap_or_else(|| scalar("none")),
        ));
        return Ok(node_fields(record, "text", YamlValue::Mapping(fields)));
    }
    let NodePayload::ComponentInstance(instance) = &record.payload else {
        if stack.is_some() {
            return Err(TextSourceError::Unsupported("modifier stack".into()));
        }
        return node(record, FrameDimensions::Paired);
    };
    if !instance.has_no_overrides() {
        return Err(TextSourceError::Unsupported(
            "component instance override".into(),
        ));
    }
    let component = instance.component().id();
    if component == StableId::NIL {
        return Err(invalid("빈 component 참조"));
    }
    let placement = stack
        .map(translation)
        .transpose()?
        .unwrap_or_else(|| scalar("none"));
    Ok(node_fields(
        record,
        "component-instance",
        mapping([("component", scalar(component)), ("placement", placement)]),
    ))
}

fn translation(stack: &ModifierStack) -> Result<YamlValue, TextSourceError> {
    let [entry] = stack.entries() else {
        return Err(TextSourceError::Unsupported(
            "복합 배치 modifier stack".into(),
        ));
    };
    let ModifierPayload::Transform(matrix) = entry.payload else {
        return Err(TextSourceError::Unsupported("배치 이외 modifier".into()));
    };
    let (a, b, c, d, x, y) = matrix.components();
    if !entry.enabled || (a, b, c, d) != (1., 0., 0., 1.) {
        return Err(TextSourceError::Unsupported(
            "배치는 활성 평행이동이어야 합니다".into(),
        ));
    }
    Ok(mapping([
        ("id", scalar(entry.id.stable_id())),
        ("order_key", order_key(&entry.order)),
        ("position", floats(&[x, y])),
    ]))
}

/// `initial_typography_id`가 표현하는 v2 표. 새 문서의 생성 기본값과 독립적인 포맷 상수입니다.
fn v2_text_styles(resource: StableId) -> DocumentTextStyles {
    let typography = TypographyValue::from_record(&Typography {
        families: Vec::new(),
        size: FontSize::Medium,
        weight: FiniteF32::new(400.0).expect("v2 font weight"),
        style: FontStyle::Normal,
        stretch_percent: FiniteF32::new(100.0).expect("v2 font stretch"),
        line_height: LineHeight::Normal,
        letter_spacing: LetterSpacing::Normal,
    })
    .expect("v2 typography");
    let paragraph = ParagraphStyle {
        typography: None,
        alignment: TextAlignment::Start,
        wrapping: WrappingMode::Wrap,
        line_height: ParagraphLineHeight::Auto,
    };
    DocumentTextStyles {
        default_typography: Some(resource),
        default_fill: None,
        typography: [(resource, typography)].into(),
        fonts: Default::default(),
        paragraphs: [(0, paragraph)].into(),
        runs: [(0, TextStyle::new(None, None, vec![], vec![]))].into(),
    }
}

/// 지원 범위를 검증한 뒤 결정론적인 제한 YAML로 기록합니다.
pub fn encode(model: &DocumentModel) -> Result<String, TextSourceError> {
    let records = model.to_records().map_err(invalid)?;
    validate_document(&records).map_err(invalid)?;
    let definition = &records.definition;
    let id = scalar(definition.id);
    let name = scalar(definition.display_name());
    let styles = &definition.text_styles;
    let (version, document) = if styles.is_empty() {
        (1, mapping([("id", id), ("display_name", name)]))
    } else if let Some(resource) = styles
        .default_typography
        .filter(|id| *styles == v2_text_styles(*id))
    {
        (
            2,
            mapping([
                ("id", id),
                ("display_name", name),
                ("initial_typography_id", scalar(resource)),
            ]),
        )
    } else {
        (
            3,
            mapping([
                ("id", id),
                ("display_name", name),
                ("document_font", document_font::encode(styles)?),
            ]),
        )
    };
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
    let value = mapping([
        ("schema_version", scalar(version)),
        ("document", document),
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
            YamlValue::Sequence(
                records
                    .nodes
                    .iter()
                    .map(|record| view_node(record, model.modifier_stack(record.id), version == 3))
                    .collect::<Result<_, _>>()?,
            ),
        ),
    ]);
    write_canonical_checked(&value).map_err(invalid)
}

pub(super) fn fields(value: &YamlValue, required: &[&str]) -> Result<(), TextSourceError> {
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

pub(super) fn field<'a>(value: &'a YamlValue, key: &str) -> Result<&'a YamlValue, TextSourceError> {
    value
        .get(key)
        .ok_or_else(|| invalid(format!("필수 키 누락: {key}")))
}

pub(super) fn text(value: &YamlValue) -> Result<&str, TextSourceError> {
    value
        .as_scalar()
        .ok_or_else(|| invalid("scalar가 필요합니다"))
}

pub(super) fn parse<T: FromStr>(value: &YamlValue) -> Result<T, TextSourceError> {
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

fn parse_frame(
    value: &YamlValue,
    dimensions: FrameDimensions,
) -> Result<FramePayload, TextSourceError> {
    let required: &[&str] = match dimensions {
        FrameDimensions::Paired => &[
            "size",
            "transform",
            "clipping",
            "background",
            "border",
            "corner_radius",
            "layout_mode",
        ],
        FrameDimensions::Longhands => &[
            "background",
            "border",
            "clipping",
            "corner_radius",
            "height",
            "layout_mode",
            "transform",
            "width",
        ],
    };
    fields(value, required)?;
    let [width, height] = match dimensions {
        FrameDimensions::Paired => parse_floats(field(value, "size")?)?,
        FrameDimensions::Longhands => [
            parse(field(value, "width")?)?,
            parse(field(value, "height")?)?,
        ],
    };
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
    if hex.len() % 2 != 0 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid("OrderKey는 2자리씩 구분되는 hex여야 합니다"));
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|offset| u8::from_str_radix(&hex[offset..offset + 2], 16).map_err(invalid))
        .collect::<Result<_, _>>()?;
    OrderKey::from_bytes(bytes).map_err(invalid)
}

fn parse_text_payload(value: &YamlValue) -> Result<TextPayload, TextSourceError> {
    let paragraphs = field(value, "paragraphs")?
        .as_sequence()
        .ok_or_else(|| invalid("paragraphs는 sequence여야 합니다"))?
        .iter()
        .map(|paragraph| {
            fields(paragraph, &["style"])?;
            Ok(Paragraph {
                style: parse(field(paragraph, "style")?)?,
            })
        })
        .collect::<Result<Vec<_>, TextSourceError>>()?;
    let runs = field(value, "style_runs")?
        .as_sequence()
        .ok_or_else(|| invalid("style_runs는 sequence여야 합니다"))?
        .iter()
        .map(|run| {
            fields(run, &["start", "end", "style"])?;
            let start = parse(field(run, "start")?)?;
            let end = parse(field(run, "end")?)?;
            Ok(StyleRun {
                range: TextRange::new(start, end)
                    .ok_or_else(|| invalid("Text run의 start는 end 이하여야 합니다"))?,
                style: parse(field(run, "style")?)?,
            })
        })
        .collect::<Result<Vec<_>, TextSourceError>>()?;
    // 저장된 run의 순서·겹침·빈 범위는 모델의 원본 의미이므로 정규화하지 않습니다.
    TextPayload::new(
        UserText::new(text(field(value, "content")?)?),
        paragraphs,
        runs,
    )
    .map_err(invalid)
}

pub(super) fn parse_node(
    value: &YamlValue,
    dimensions: FrameDimensions,
) -> Result<NodeRecord, TextSourceError> {
    let payload = field(value, "payload")?;
    let payload = match text(field(value, "kind")?)? {
        "frame" => NodePayload::Frame(parse_frame(payload, dimensions)?),
        "fragment" if payload.as_scalar() == Some("none") => NodePayload::Fragment(FragmentPayload),
        "fragment" => return Err(invalid("Fragment payload는 none이어야 합니다")),
        "text" => {
            fields(payload, &["content", "paragraphs", "style_runs"])?;
            NodePayload::Text(parse_text_payload(payload)?)
        }
        kind => {
            return Err(TextSourceError::Unsupported(format!(
                "node payload: {kind}"
            )))
        }
    };
    parse_node_fields(value, payload)
}

fn parse_node_fields(
    value: &YamlValue,
    payload: NodePayload,
) -> Result<NodeRecord, TextSourceError> {
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

fn parse_view_node(
    value: &YamlValue,
    stacks: &mut Vec<ModifierStack>,
    document_font: bool,
) -> Result<NodeRecord, TextSourceError> {
    let kind = text(field(value, "kind")?)?;
    if let Some(shape_kind) = ShapeKind::ALL
        .into_iter()
        .find(|shape| shape.name() == kind)
    {
        return shape::decode(value, shape_kind, stacks);
    }
    if document_font && kind == "text" {
        let payload = field(value, "payload")?;
        fields(
            payload,
            &["content", "paragraphs", "style_runs", "placement"],
        )?;
        let placement = field(payload, "placement")?;
        let text = parse_text_payload(payload)?;
        document_font::check_text(&text)?;
        let record = parse_node_fields(value, NodePayload::Text(text))?;
        if placement.as_scalar() != Some("none") {
            stacks.push(parse_translation(record.id, placement)?);
        }
        return Ok(record);
    }
    if kind != "component-instance" {
        return parse_node(value, FrameDimensions::Paired);
    }
    let payload = field(value, "payload")?;
    fields(payload, &["component", "placement"])?;
    let component = parse(field(payload, "component")?)?;
    if component == StableId::NIL {
        return Err(invalid("빈 component 참조"));
    }
    let instance = ComponentInstancePayload::new(ComponentRef::new(component));
    let record = parse_node_fields(value, NodePayload::ComponentInstance(instance))?;
    let placement = field(payload, "placement")?;
    if placement.as_scalar() != Some("none") {
        stacks.push(parse_translation(record.id, placement)?);
    }
    Ok(record)
}

fn parse_translation(
    owner: StableId,
    placement: &YamlValue,
) -> Result<ModifierStack, TextSourceError> {
    fields(placement, &["id", "order_key", "position"])?;
    let [x, y] = parse_floats(field(placement, "position")?)?;
    let mut stack = ModifierStack::new(owner);
    stack
        .insert(ModifierEntry {
            id: ModifierId::new(parse(field(placement, "id")?)?),
            order: parse_order_key(field(placement, "order_key")?)?,
            enabled: true,
            kind: ModifierKind::Transform,
            payload: ModifierPayload::Transform(
                Transform2D::new(1., 0., 0., 1., x, y).map_err(invalid)?,
            ),
        })
        .map_err(invalid)?;
    Ok(stack)
}

/// 제한 YAML을 엄격하게 해석하고 문서 validator로 runtime 모델을 재구축합니다.
pub fn decode(source: &str) -> Result<DocumentModel, TextSourceError> {
    let value = load_restricted(source).map_err(invalid)?;
    fields(
        &value,
        &["schema_version", "document", "page", "view", "nodes"],
    )?;
    let document = field(&value, "document")?;
    let version = text(field(&value, "schema_version")?)?;
    let styles = match version {
        "1" => {
            fields(document, &["id", "display_name"])?;
            DocumentTextStyles::default()
        }
        "2" => {
            fields(document, &["id", "display_name", "initial_typography_id"])?;
            v2_text_styles(parse(field(document, "initial_typography_id")?)?)
        }
        "3" => {
            fields(document, &["id", "display_name", "document_font"])?;
            document_font::decode(field(document, "document_font")?)?
        }
        _ => {
            return Err(TextSourceError::Unsupported(
                "text source schema_version".into(),
            ))
        }
    };
    let page = field(&value, "page")?;
    let view = field(&value, "view")?;
    fields(page, &["id", "display_name"])?;
    fields(view, &["id", "display_name", "root_frame"])?;
    let mut definition = DocumentDefinition::new(
        parse(field(document, "id")?)?,
        text(field(document, "display_name")?)?.to_owned(),
    );
    definition.text_styles = styles;
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
    let mut modifier_stacks = Vec::new();
    let records = DocumentRecords {
        definition,
        nodes: nodes
            .iter()
            .map(|node| parse_view_node(node, &mut modifier_stacks, version == "3"))
            .collect::<Result<_, _>>()?,
        modifier_stacks,
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

    #[test]
    fn unplaced_instances_and_nondefault_placement_order_round_trip_only_in_views() {
        let mut model = model();
        let mut node = model.remove_leaf(sid(11)).unwrap();
        node.payload = NodePayload::ComponentInstance(ComponentInstancePayload::new(
            ComponentRef::new(sid(20)),
        ));
        model.insert_node(node).unwrap();
        let bare = encode(&model).unwrap();
        assert!(bare.contains("placement: none"));
        assert_eq!(
            decode(&bare).unwrap().to_records().unwrap(),
            model.to_records().unwrap()
        );

        let mut stack = ModifierStack::new(sid(11));
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(sid(21)),
                order: OrderKey::from_bytes(vec![0, 222]).unwrap(),
                enabled: true,
                kind: ModifierKind::Transform,
                payload: ModifierPayload::Transform(
                    Transform2D::new(1., 0., 0., 1., -4.25, 0.).unwrap(),
                ),
            })
            .unwrap();
        model.set_modifier_stack(stack.clone()).unwrap();
        let placed = encode(&model).unwrap();
        assert_eq!(
            decode(&placed).unwrap().to_records().unwrap(),
            model.to_records().unwrap()
        );
        // 독립 정의에서 공유하는 기존 reader는 View의 새로운 노드 종류를 받지 않습니다.
        let yaml = load_restricted(&placed).unwrap();
        let value = field(&yaml, "nodes")
            .unwrap()
            .as_sequence()
            .unwrap()
            .last()
            .unwrap();
        assert!(matches!(
            parse_node(value, FrameDimensions::Paired),
            Err(TextSourceError::Unsupported(_))
        ));
        for bad in [
            placed.replace("position: \"-4.25 0\"", "position: \"NaN 0\""),
            placed.replace("position:", "matrix:"),
            placed.replace(&sid(21).to_string(), &sid(11).to_string()),
            placed.replace(&sid(20).to_string(), &StableId::NIL.to_string()),
            placed.replace("placement:", "slot_overrides: []\n      placement:"),
        ] {
            assert_ne!(bad, placed);
            assert!(decode(&bad).is_err());
        }
        let modifier = ModifierId::new(sid(21));
        stack.set_enabled(modifier, false).unwrap();
        model.set_modifier_stack(stack.clone()).unwrap();
        assert!(matches!(
            encode(&model),
            Err(TextSourceError::Unsupported(_))
        ));
        stack.set_enabled(modifier, true).unwrap();
        stack
            .set_payload(
                modifier,
                ModifierPayload::Transform(Transform2D::new(1., 0.5, 0., 1., 4., -1.).unwrap()),
            )
            .unwrap();
        model.set_modifier_stack(stack.clone()).unwrap();
        assert!(matches!(
            encode(&model),
            Err(TextSourceError::Unsupported(_))
        ));
        stack
            .push_back(ModifierId::new(sid(22)), ModifierKind::Transform)
            .unwrap();
        model.set_modifier_stack(stack).unwrap();
        assert!(matches!(
            encode(&model),
            Err(TextSourceError::Unsupported(_))
        ));
    }

    #[test]
    fn instance_property_and_slot_overrides_cannot_be_written_as_plain_references() {
        use stellar_document::component::{
            definition::{ComponentPropertyId, ComponentSlotId},
            instance::{OverrideKey, SlotOverride},
        };
        let mut model = model();
        let mut node = model.remove_leaf(sid(11)).unwrap();
        let mut property = ComponentInstancePayload::new(ComponentRef::new(sid(20)));
        property.set_property_override(
            OverrideKey::direct(ComponentPropertyId::new(sid(21))),
            stellar_core_types::typed_value::TypedValue::Boolean(false),
        );
        let mut slot = ComponentInstancePayload::new(ComponentRef::new(sid(20)));
        slot.set_slot_override(ComponentSlotId::new(sid(22)), SlotOverride::empty());
        for instance in [property, slot] {
            node.payload = NodePayload::ComponentInstance(instance);
            model.insert_node(node).unwrap();
            assert!(matches!(
                encode(&model),
                Err(TextSourceError::Unsupported(_))
            ));
            node = model.remove_leaf(sid(11)).unwrap();
        }
    }
}

#[cfg(test)]
mod order_key_tests {
    use super::*;

    #[test]
    fn order_key_accepts_only_hex_digit_pairs() {
        for bad in ["+1", "1+", " 1", "abc", "１２"] {
            assert!(parse_order_key(&YamlValue::scalar(bad)).is_err(), "{bad}");
        }
        assert_eq!(
            parse_order_key(&YamlValue::scalar("0aB1"))
                .unwrap()
                .as_bytes(),
            &[0x0a, 0xb1]
        );
    }
}

//! 단일 View 텍스트 원본(ADR-0059 방향, ADR-0061 형식). 독립 컴포넌트는 정의 ID로 참조합니다.
//!
//! 쓰기 버전은 View `schema_version: "2"` 하나입니다. 커밋된 v1은 읽기 전용입니다. 영속 속성 하나를
//! 키 하나·한 줄로 쓰고, 파일에 없는 값을 버전 번호에서 복원하지 않습니다. 미지원 레코드나 키는
//! 생략하지 않고 거부합니다.

use std::fmt::Display;
use std::str::FromStr;

use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{ComponentRef, NodeRef};
use stellar_core_types::text::UserText;
use stellar_core_types::value::{Size2, Transform2D};
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
use stellar_document::text::{Paragraph, StyleRun, TextPayload, TextRange};
use stellar_document::text_styles::DocumentTextStyles;
#[cfg(test)]
use stellar_document::validate::validate_document;

use super::text_source::TextSourceError;
#[cfg(test)]
use crate::yaml::write_canonical_checked;
use crate::yaml::{load_restricted, YamlValue};

pub(crate) mod document_font;
mod shape;

/// View 원본의 유일한 쓰기 버전.
const VIEW_VERSION: &str = "2";

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

fn space_name(space: ColorSpace) -> &'static str {
    match space {
        ColorSpace::Srgb => "srgb",
        ColorSpace::DisplayP3 => "display-p3",
    }
}

/// 원본 한 줄 색: 색 공간과 비연관 RGBA. CLI·MCP 조회도 이 표현을 그대로 씁니다.
pub fn color_source_value(value: Color) -> String {
    format!(
        "{} {} {} {} {}",
        space_name(value.space()),
        value.r(),
        value.g(),
        value.b(),
        value.alpha()
    )
}

fn color_line(value: Option<Color>) -> YamlValue {
    optional(value, |color| scalar(color_source_value(color)))
}

/// Frame payload 문법.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Grammar {
    /// 커밋된 View v1과 컴포넌트 v1·v2. 읽기 전용입니다(`size`, `corner_radius`, 색·clipping mapping).
    Legacy,
    /// ADR-0061 결정 2: 영속 속성 하나 = 키 하나·한 줄(View v2, 컴포넌트 v3).
    Current,
}

/// 한 속성 한 줄 문법의 Frame 키. 파일의 줄 순서입니다(ADR-0061 결정 2).
///
/// 모양(배경·테두리·모서리)과 크기 키는 이름 오름차순이고, `clipping`과 `transform`은 끝에 둡니다. Git은 바로 이웃한
/// 두 줄을 서로 다른 브랜치가 바꾸면 충돌로 남기므로, 가장 자주 따로 바뀌는 이동(`transform`)이 크기(`width`)
/// 줄과 이웃하지 않고 테두리 색과 두께 사이에도 모서리 줄이 오게 한 순서입니다.
const FRAME_KEYS: [&str; 12] = [
    "background",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
    "border-color",
    "border-top-left-radius",
    "border-top-right-radius",
    "border-width",
    "height",
    "layout-mode",
    "width",
    "clipping",
    "transform",
];

/// Frame의 모든 영속 속성을 각각 한 줄로 씁니다. 각 줄이 영속 속성 하나의 병합 단위입니다. 파일 키와 속성 패널
/// 행·웹 CSS 출력 이름의 대응은 `docs/spec/text-view-source-v1.md`의 대응 표에 있습니다(ADR-0061 결정 2).
///
/// 내부 모델의 묶음 key `border`(0x0105, 색과 두께)는 `border-color`·`border-width` 두 줄로 나눠 씁니다. 테두리가
/// 없으면 두 줄이 모두 `none`입니다.
pub(super) fn frame(payload: &FramePayload) -> YamlValue {
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
    let clipping = match clipping {
        Clipping::Disabled => "disabled".to_owned(),
        Clipping::FrameBounds => "frame-bounds".to_owned(),
        Clipping::Path(target) => format!("path {}", target.id()),
    };
    mapping([
        ("background", color_line(*background)),
        ("border-bottom-left-radius", scalar(bl)),
        ("border-bottom-right-radius", scalar(br)),
        ("border-color", color_line(border.map(|value| value.color))),
        ("border-top-left-radius", scalar(tl)),
        ("border-top-right-radius", scalar(tr)),
        (
            "border-width",
            optional(*border, |value| scalar(value.thickness())),
        ),
        ("height", scalar(size.height())),
        ("layout-mode", optional(*layout_mode, scalar)),
        ("width", scalar(size.width())),
        ("clipping", scalar(clipping)),
        ("transform", floats(&[a, b, c, d, tx, ty])),
    ])
}

#[cfg_attr(not(test), allow(dead_code))]
fn text_payload(payload: &TextPayload) -> Result<Vec<(YamlValue, YamlValue)>, TextSourceError> {
    // TextRange의 public 필드로 직접 만든 역전 범위도 쓰기 전에 거부합니다.
    if payload
        .style_runs()
        .iter()
        .any(|run| run.range.start > run.range.end)
    {
        return Err(invalid("Text run의 start는 end 이하여야 합니다"));
    }
    let YamlValue::Mapping(fields) = mapping([
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
    ]) else {
        unreachable!("mapping")
    };
    Ok(fields)
}

/// Frame·Fragment 레코드. 컴포넌트 원본(v3)과 View 원본(v2)이 공유합니다.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn node(record: &NodeRecord) -> Result<YamlValue, TextSourceError> {
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
    Ok(node_fields(record, kind, payload))
}

#[cfg_attr(not(test), allow(dead_code))]
fn order_key(key: &OrderKey) -> YamlValue {
    scalar(
        key.as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
    )
}

#[cfg_attr(not(test), allow(dead_code))]
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
#[cfg_attr(not(test), allow(dead_code))]
fn placement(stack: Option<&ModifierStack>) -> Result<YamlValue, TextSourceError> {
    let Some(stack) = stack else {
        return Ok(scalar("none"));
    };
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
        ("translate-x", scalar(x)),
        ("translate-y", scalar(y)),
    ]))
}

#[cfg_attr(not(test), allow(dead_code))]
fn view_node(
    record: &NodeRecord,
    stack: Option<&ModifierStack>,
) -> Result<YamlValue, TextSourceError> {
    match &record.payload {
        NodePayload::Shape(geometry) => Ok(node_fields(
            record,
            geometry.kind().name(),
            shape::encode(geometry, stack)?,
        )),
        NodePayload::Text(text) => {
            let mut fields = text_payload(text)?;
            fields.push((scalar("placement"), placement(stack)?));
            Ok(node_fields(record, "text", YamlValue::Mapping(fields)))
        }
        NodePayload::ComponentInstance(instance) => {
            if !instance.has_no_overrides() {
                return Err(TextSourceError::Unsupported(
                    "component instance override".into(),
                ));
            }
            let component = instance.component().id();
            if component == StableId::NIL {
                return Err(invalid("빈 component 참조"));
            }
            Ok(node_fields(
                record,
                "component-instance",
                mapping([
                    ("component", scalar(component)),
                    ("placement", placement(stack)?),
                ]),
            ))
        }
        _ if stack.is_some() => Err(TextSourceError::Unsupported("modifier stack".into())),
        _ => node(record),
    }
}

/// 지원 범위를 검증한 뒤 결정론적인 제한 YAML(View v4, ADR-0077)로 기록합니다.
pub fn encode(model: &DocumentModel) -> Result<String, TextSourceError> {
    super::source_v4::view::encode(model)
}

/// 읽기 전용 View v2로 기록합니다(v2 읽기 회귀 테스트 전용).
#[cfg(test)]
fn encode_v2(model: &DocumentModel) -> Result<String, TextSourceError> {
    let records = model.to_records().map_err(invalid)?;
    validate_document(&records).map_err(invalid)?;
    let definition = &records.definition;
    let [page] = definition.pages.as_slice() else {
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
        ("schema_version", scalar(VIEW_VERSION)),
        (
            "document",
            mapping([
                ("id", scalar(definition.id)),
                ("display_name", scalar(definition.display_name())),
                (
                    "document_font",
                    document_font::encode(&definition.text_styles)?,
                ),
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
            YamlValue::Sequence(
                records
                    .nodes
                    .iter()
                    .map(|record| view_node(record, model.modifier_stack(record.id)))
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

fn finite(value: &YamlValue) -> Result<f32, TextSourceError> {
    let [number] = parse_floats(value)?;
    Ok(number)
}

fn parse_color_line(value: &YamlValue) -> Result<Color, TextSourceError> {
    let (space, rgba) = text(value)?
        .split_once(' ')
        .ok_or_else(|| invalid("색은 색 공간과 RGBA가 필요합니다"))?;
    let space = match space {
        "srgb" => ColorSpace::Srgb,
        "display-p3" => ColorSpace::DisplayP3,
        _ => return Err(TextSourceError::Unsupported("알 수 없는 색공간".into())),
    };
    let [r, g, b, a] = parse_floats(&YamlValue::scalar(rgba))?;
    Color::new(space, r, g, b, a).map_err(invalid)
}

fn parse_clipping_line(value: &str) -> Result<Clipping, TextSourceError> {
    match value.split_once(' ') {
        None if value == "disabled" => Ok(Clipping::Disabled),
        None if value == "frame-bounds" => Ok(Clipping::FrameBounds),
        Some(("path", target)) => {
            Ok(Clipping::Path(NodeRef::new(target.parse().map_err(
                |_| invalid("clipping path 대상 형식이 잘못되었습니다"),
            )?)))
        }
        _ => Err(TextSourceError::Unsupported(
            "알 수 없는 clipping 방식".into(),
        )),
    }
}

/// `border-color`·`border-width` 두 줄을 내부 묶음 Border 하나로 합칩니다. 한 줄만 `none`이면 거부합니다.
fn parse_border_lines(value: &YamlValue) -> Result<Option<Border>, TextSourceError> {
    let color = parse_optional(field(value, "border-color")?, parse_color_line)?;
    let width = parse_optional(field(value, "border-width")?, finite)?;
    match (color, width) {
        (None, None) => Ok(None),
        (Some(color), Some(width)) => Border::new(color, width).map(Some).map_err(invalid),
        _ => Err(invalid(
            "border-color와 border-width는 함께 none이거나 함께 값이어야 합니다",
        )),
    }
}

fn parse_frame(value: &YamlValue) -> Result<FramePayload, TextSourceError> {
    fields(value, &FRAME_KEYS)?;
    let length = |key| finite(field(value, key)?);
    let [a, b, c, d, tx, ty] = parse_floats(field(value, "transform")?)?;
    Ok(FramePayload {
        size: Size2::new(length("width")?, length("height")?).map_err(invalid)?,
        transform: Transform2D::new(a, b, c, d, tx, ty).map_err(invalid)?,
        clipping: parse_clipping_line(text(field(value, "clipping")?)?)?,
        background: parse_optional(field(value, "background")?, parse_color_line)?,
        border: parse_border_lines(value)?,
        corner_radius: CornerRadius::new(
            length("border-top-left-radius")?,
            length("border-top-right-radius")?,
            length("border-bottom-right-radius")?,
            length("border-bottom-left-radius")?,
        )
        .map_err(invalid)?,
        layout_mode: parse_optional(field(value, "layout-mode")?, parse)?,
    })
}

/// 읽기 전용 v1 색 mapping.
fn parse_legacy_color(value: &YamlValue) -> Result<Color, TextSourceError> {
    fields(value, &["space", "rgba"])?;
    let space = match text(field(value, "space")?)? {
        "srgb" => ColorSpace::Srgb,
        "display-p3" => ColorSpace::DisplayP3,
        _ => return Err(TextSourceError::Unsupported("알 수 없는 색공간".into())),
    };
    let [r, g, b, a] = parse_floats(field(value, "rgba")?)?;
    Color::new(space, r, g, b, a).map_err(invalid)
}

fn parse_legacy_border(value: &YamlValue) -> Result<Border, TextSourceError> {
    fields(value, &["color", "thickness"])?;
    Border::new(
        parse_legacy_color(field(value, "color")?)?,
        parse(field(value, "thickness")?)?,
    )
    .map_err(invalid)
}

fn parse_legacy_clipping(value: &YamlValue) -> Result<Clipping, TextSourceError> {
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

fn parse_legacy_frame(value: &YamlValue) -> Result<FramePayload, TextSourceError> {
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
        clipping: parse_legacy_clipping(field(value, "clipping")?)?,
        background: parse_optional(field(value, "background")?, parse_legacy_color)?,
        border: parse_optional(field(value, "border")?, parse_legacy_border)?,
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

/// Frame·Fragment 레코드와 읽기 전용 v1 Text를 해석합니다.
pub(super) fn parse_node(
    value: &YamlValue,
    grammar: Grammar,
) -> Result<NodeRecord, TextSourceError> {
    let payload = field(value, "payload")?;
    let payload = match (text(field(value, "kind")?)?, grammar) {
        ("frame", Grammar::Legacy) => NodePayload::Frame(parse_legacy_frame(payload)?),
        ("frame", Grammar::Current) => NodePayload::Frame(parse_frame(payload)?),
        ("fragment", _) if payload.as_scalar() == Some("none") => {
            NodePayload::Fragment(FragmentPayload)
        }
        ("fragment", _) => return Err(invalid("Fragment payload는 none이어야 합니다")),
        ("text", Grammar::Legacy) => {
            fields(payload, &["content", "paragraphs", "style_runs"])?;
            NodePayload::Text(parse_text_payload(payload)?)
        }
        (kind, _) => {
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

fn parse_placement(
    owner: StableId,
    placement: &YamlValue,
    stacks: &mut Vec<ModifierStack>,
) -> Result<(), TextSourceError> {
    if placement.as_scalar() == Some("none") {
        return Ok(());
    }
    fields(
        placement,
        &["id", "order_key", "translate-x", "translate-y"],
    )?;
    let x = finite(field(placement, "translate-x")?)?;
    let y = finite(field(placement, "translate-y")?)?;
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
    stacks.push(stack);
    Ok(())
}

fn parse_view_node(
    value: &YamlValue,
    stacks: &mut Vec<ModifierStack>,
) -> Result<NodeRecord, TextSourceError> {
    let kind = text(field(value, "kind")?)?;
    if let Some(shape_kind) = ShapeKind::ALL
        .into_iter()
        .find(|shape| shape.name() == kind)
    {
        return shape::decode(value, shape_kind, stacks);
    }
    let payload = field(value, "payload")?;
    match kind {
        "text" => {
            fields(
                payload,
                &["content", "paragraphs", "style_runs", "placement"],
            )?;
            let text = parse_text_payload(payload)?;
            let record = parse_node_fields(value, NodePayload::Text(text))?;
            parse_placement(record.id, field(payload, "placement")?, stacks)?;
            Ok(record)
        }
        "component-instance" => {
            fields(payload, &["component", "placement"])?;
            let component = parse(field(payload, "component")?)?;
            if component == StableId::NIL {
                return Err(invalid("빈 component 참조"));
            }
            let instance = ComponentInstancePayload::new(ComponentRef::new(component));
            let record = parse_node_fields(value, NodePayload::ComponentInstance(instance))?;
            parse_placement(record.id, field(payload, "placement")?, stacks)?;
            Ok(record)
        }
        _ => parse_node(value, Grammar::Current),
    }
}

/// 제한 YAML을 엄격하게 해석하고 문서 validator로 runtime 모델을 재구축합니다.
///
/// `"2"`가 유일한 쓰기 버전입니다. `"1"`은 커밋된 기존 원본의 읽기 전용 문법입니다(문서 style 표·
/// 도형·인스턴스·배치 없음). 같은 번호를 재정의하기 전 작업 트리에서만 쓰인 문법(`initial_typography_id`,
/// `schema_version: "3"`)은 거부합니다.
pub fn decode(source: &str) -> Result<DocumentModel, TextSourceError> {
    let value = load_restricted(source).map_err(invalid)?;
    if value.get("schema_version").and_then(YamlValue::as_scalar) == Some(super::source_v4::VERSION)
    {
        return super::source_v4::view::decode(&value);
    }
    fields(
        &value,
        &["schema_version", "document", "page", "view", "nodes"],
    )?;
    let document = field(&value, "document")?;
    let current = match text(field(&value, "schema_version")?)? {
        "1" => {
            fields(document, &["id", "display_name"])?;
            false
        }
        VIEW_VERSION => {
            fields(document, &["id", "display_name", "document_font"])?;
            true
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
    definition.text_styles = if current {
        document_font::decode(field(document, "document_font")?)?
    } else {
        DocumentTextStyles::default()
    };
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
    let nodes = nodes
        .iter()
        .map(|node| {
            if current {
                parse_view_node(node, &mut modifier_stacks)
            } else {
                parse_node(node, Grammar::Legacy)
            }
        })
        .collect::<Result<_, _>>()?;
    let records = DocumentRecords {
        definition,
        nodes,
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
        let first = encode_v2(&model).unwrap();
        assert!(first.starts_with("schema_version: \"2\"\n"));
        let decoded = decode(&first).unwrap();
        assert_eq!(decoded.to_records().unwrap(), model.to_records().unwrap());
        assert_eq!(encode_v2(&decoded).unwrap(), first);
        assert!(
            first.find(&sid(10).to_string()).unwrap() < first.rfind(&sid(11).to_string()).unwrap()
        );
        // 영속 속성 하나 = 한 줄. 묶음 값이나 여러 줄 색 mapping이 없습니다.
        for line in [
            "      background: \"display-p3 -0.125 1.25 0.1 0.5\"\n",
            "      border-color: \"display-p3 -0.125 1.25 0.1 0.5\"\n",
            "      border-width: \"0.3\"\n",
            "      border-bottom-left-radius: \"4\"\n",
            "      border-bottom-right-radius: \"3\"\n",
            "      border-top-left-radius: \"1\"\n",
            "      border-top-right-radius: \"2\"\n",
            &format!("      clipping: \"path {}\"\n", sid(99)),
            "      width: \"0.1\"\n",
        ] {
            assert!(first.contains(line), "{line}\n{first}");
        }
        for bundled in [
            "size:",
            "corner_radius:",
            "space:",
            "rgba:",
            "kind: path",
            "border:",
        ] {
            assert!(!first.contains(bundled), "{bundled}");
        }
        // 이동(transform)은 크기 줄과 이웃하지 않고, 테두리 색과 두께 사이에는 다른 줄이 있습니다.
        let keys: Vec<&str> = first
            .lines()
            .skip_while(|line| *line != "    payload:")
            .skip(1)
            .take(FRAME_KEYS.len())
            .map(|line| line.trim().split_once(':').unwrap().0)
            .collect();
        assert_eq!(keys, FRAME_KEYS);
        let at = |key| FRAME_KEYS.iter().position(|k| *k == key).unwrap();
        assert!(at("transform").abs_diff(at("width")) > 1);
        assert!(at("transform").abs_diff(at("height")) > 1);
        assert!(at("border-color").abs_diff(at("border-width")) > 1);
    }

    #[test]
    fn border_lines_must_be_both_none_or_both_values() {
        let source = encode_v2(&model()).unwrap();
        for half in [
            source.replacen(
                "border-color: \"display-p3 -0.125 1.25 0.1 0.5\"",
                "border-color: none",
                1,
            ),
            source.replacen("border-width: \"0.3\"", "border-width: none", 1),
            source.replacen("border-width: \"0.3\"", "border: \"0.3\"", 1),
        ] {
            assert_ne!(half, source);
            assert!(decode(&half).is_err(), "{half}");
        }
        let none = source
            .replacen(
                "border-color: \"display-p3 -0.125 1.25 0.1 0.5\"",
                "border-color: none",
                1,
            )
            .replacen("border-width: \"0.3\"", "border-width: none", 1);
        let records = decode(&none).unwrap().to_records().unwrap();
        assert!(records.nodes.iter().all(|node| match &node.payload {
            NodePayload::Frame(frame) => frame.border.is_none(),
            _ => true,
        }));
    }

    #[test]
    fn legacy_v1_is_read_only_and_redefined_numbers_reject_old_syntax() {
        let current = encode_v2(&model()).unwrap();
        // 커밋된 v1 문법: 묶음 Frame 값과 문서 style 표 없음.
        let legacy = format!(
            "schema_version: \"1\"\ndocument:\n  id: \"{}\"\n  display_name: Doc\npage:\n  id: \"{}\"\n  display_name: Page\nview:\n  id: \"{}\"\n  display_name: View\n  root_frame: \"{}\"\nnodes:\n  -\n    id: \"{}\"\n    parent: none\n    order_key: \"80\"\n    visible: \"true\"\n    locked: \"false\"\n    display_name: Root\n    kind: frame\n    payload:\n      size: \"10 20\"\n      transform: \"1 0 0 1 0 0\"\n      clipping:\n        kind: frame-bounds\n      background:\n        space: srgb\n        rgba: \"1 0 0 1\"\n      border: none\n      corner_radius: \"1 2 3 4\"\n      layout_mode: none\n",
            sid(1),
            sid(2),
            sid(3),
            sid(10),
            sid(10)
        );
        let opened = decode(&legacy).unwrap();
        let rewritten = encode_v2(&opened).unwrap();
        assert!(rewritten.starts_with("schema_version: \"2\"\n"));
        assert!(rewritten.contains("      background: \"srgb 1 0 0 1\"\n"));
        assert!(rewritten.contains("  document_font: none\n"));
        for rejected in [
            legacy.replacen("schema_version: \"1\"", "schema_version: \"2\"", 1),
            current.replacen("schema_version: \"2\"", "schema_version: \"1\"", 1),
            current.replacen("schema_version: \"2\"", "schema_version: \"3\"", 1),
            current.replacen(
                "  document_font: none\n",
                &format!("  initial_typography_id: \"{}\"\n", sid(6)),
                1,
            ),
        ] {
            assert!(decode(&rejected).is_err(), "{rejected}");
        }
    }

    #[test]
    fn omitted_unknown_and_invalid_fields_never_default() {
        let source = encode_v2(&model()).unwrap();
        for changed in [
            source.replacen("layout-mode:", "unknown_property:", 1),
            source.replacen("kind: fragment", "kind: shape", 1),
            source.replacen("visible: \"true\"", "visible: yes", 1),
            source.replacen("      width: \"0.1\"\n", "", 1),
            source.replacen("width: \"0.1\"", "width: NaN", 1),
            source.replacen("background: \"display-p3", "background: \"oklch", 1),
            source.replacen("clipping: \"path", "clipping: \"mask", 1),
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
            encode_v2(&model),
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
            encode_v2(&model),
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
        let bare = encode_v2(&model).unwrap();
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
        let placed = encode_v2(&model).unwrap();
        assert!(
            placed.contains("        translate-x: \"-4.25\"\n        translate-y: \"0\"\n"),
            "{placed}"
        );
        assert_eq!(
            decode(&placed).unwrap().to_records().unwrap(),
            model.to_records().unwrap()
        );
        // 독립 정의에서 공유하는 Frame reader는 View의 다른 노드 종류를 받지 않습니다.
        let yaml = load_restricted(&placed).unwrap();
        let value = field(&yaml, "nodes")
            .unwrap()
            .as_sequence()
            .unwrap()
            .last()
            .unwrap();
        assert!(matches!(
            parse_node(value, Grammar::Current),
            Err(TextSourceError::Unsupported(_))
        ));
        for bad in [
            placed.replace("translate-x: \"-4.25\"", "translate-x: NaN"),
            placed.replace("        translate-x: \"-4.25\"\n", ""),
            placed.replace("translate-x: \"-4.25\"", "position: \"-4.25 0\""),
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
            encode_v2(&model),
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
            encode_v2(&model),
            Err(TextSourceError::Unsupported(_))
        ));
        stack
            .push_back(ModifierId::new(sid(22)), ModifierKind::Transform)
            .unwrap();
        model.set_modifier_stack(stack).unwrap();
        assert!(matches!(
            encode_v2(&model),
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
                encode_v2(&model),
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

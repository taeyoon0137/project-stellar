//! 단일 View 원본 쓰기 v4(`stellar text-source export|check`, ADR-0077 결정 2).
//!
//! 문서 하나·페이지 하나·View 하나를 파일 하나에 씁니다. node는 모든 종류와 modifier stack을 v4 문법으로 씁니다.
//! Text style 표는 catalog 항목과 같은 문법의 `text-styles` 블록에 두며, 등록 글꼴은 asset catalog가 필요해
//! 이 단일 파일에서는 거부합니다(다중 파일 원본 [`super::project`]는 catalog를 씁니다).

use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::model::DocumentModel;
use stellar_document::records::DocumentRecords;
use stellar_document::text_styles::DocumentTextStyles;
use stellar_document::validate::validate_document;

use super::super::text_codec::{field, fields, mapping, scalar, text};
use super::super::text_source::TextSourceError;
use super::css::{self, invalid, unsupported};
use super::{node, styles, VERSION};
use crate::yaml::{write_canonical_checked, YamlValue};

fn styles_value(
    document: stellar_core_types::ids::StableId,
    table: &DocumentTextStyles,
) -> Result<YamlValue, TextSourceError> {
    if table.is_empty() {
        return Ok(scalar("none"));
    }
    if !table.fonts.is_empty() {
        return Err(unsupported(
            "등록 글꼴은 다중 파일 원본의 asset catalog에 둡니다",
        ));
    }
    styles::text_styles_value(document, table)
}

/// 지원 범위를 검증한 뒤 결정론적인 View v4로 씁니다.
pub(crate) fn encode(model: &DocumentModel) -> Result<String, TextSourceError> {
    let records = model.to_records().map_err(invalid)?;
    validate_document(&records).map_err(invalid)?;
    let definition = &records.definition;
    let [page] = definition.pages.as_slice() else {
        return Err(unsupported("정확히 하나의 페이지가 필요합니다"));
    };
    let [view] = page.views.as_slice() else {
        return Err(unsupported("정확히 하나의 View가 필요합니다"));
    };
    let value = mapping([
        ("schema_version", scalar(VERSION)),
        (
            "document",
            mapping([
                ("id", scalar(css::id(definition.id))),
                ("display_name", scalar(definition.display_name())),
                (
                    "text-styles",
                    styles_value(definition.id, &definition.text_styles)?,
                ),
            ]),
        ),
        (
            "page",
            mapping([
                ("id", scalar(css::id(page.id))),
                ("display_name", scalar(page.display_name())),
            ]),
        ),
        (
            "view",
            mapping([
                ("id", scalar(css::id(view.id))),
                ("display_name", scalar(view.display_name())),
                ("root_frame", scalar(css::id(view.root_frame))),
            ]),
        ),
        (
            "nodes",
            YamlValue::Sequence(
                records
                    .nodes
                    .iter()
                    .map(|record| node::node(record, Some(model.modifier_stack(record.id))))
                    .collect::<Result<_, _>>()?,
            ),
        ),
    ]);
    write_canonical_checked(&value).map_err(invalid)
}

/// View v4를 읽어 문서 validator로 모델을 재구축합니다.
pub(crate) fn decode(value: &YamlValue) -> Result<DocumentModel, TextSourceError> {
    fields(
        value,
        &["schema_version", "document", "page", "view", "nodes"],
    )?;
    let document = field(value, "document")?;
    fields(document, &["id", "display_name", "text-styles"])?;
    let page = field(value, "page")?;
    fields(page, &["id", "display_name"])?;
    let view = field(value, "view")?;
    fields(view, &["id", "display_name", "root_frame"])?;
    let id = |v: &YamlValue, key| css::parse_id(text(field(v, key)?)?);
    let mut definition = DocumentDefinition::new(
        id(document, "id")?,
        text(field(document, "display_name")?)?.to_owned(),
    );
    let table = field(document, "text-styles")?;
    if table.as_scalar() != Some("none") {
        let (owner, parsed) = styles::parse_text_styles_value(table, &Default::default())?;
        if owner != definition.id {
            return Err(invalid("text-styles의 document가 문서 id와 다릅니다"));
        }
        if parsed.is_empty() {
            return Err(invalid("빈 Text style 표는 none으로 씁니다"));
        }
        definition.text_styles = parsed;
    }
    let mut page_definition = PageDefinition::new(
        id(page, "id")?,
        text(field(page, "display_name")?)?.to_owned(),
    );
    page_definition
        .add_view(ViewDefinition::new(
            id(view, "id")?,
            text(field(view, "display_name")?)?.to_owned(),
            id(view, "root_frame")?,
        ))
        .map_err(invalid)?;
    definition.add_page(page_definition).map_err(invalid)?;
    let mut records = DocumentRecords::new(definition);
    let mut last = None;
    for item in field(value, "nodes")?
        .as_sequence()
        .ok_or_else(|| invalid("nodes는 sequence여야 합니다"))?
    {
        let (record, stack) = node::parse_node(item, true)?;
        if last.is_some_and(|prev| prev >= record.id) {
            return Err(invalid("node는 StableId 오름차순, 중복 없이 씁니다"));
        }
        last = Some(record.id);
        records.nodes.push(record);
        records.modifier_stacks.extend(stack);
    }
    DocumentModel::from_records(records)
        .map(|(model, _)| model)
        .map_err(invalid)
}

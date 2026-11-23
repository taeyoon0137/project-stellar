//! `guides`: 저장된 문서의 Frame 가이드·레이아웃 그리드와 View 변수·Variant를 조회합니다(XII.10.19·XII.10.20·XVI.11.1,
//! ADR-0126).
//!
//! 조회는 MCP `document.guides`와 같은 ops 함수(`stellar_ops::guides::read_guides`)입니다. 줄 문장은 텍스트 원본 `guides`
//! 목록과 같고, 길이는 문서 CSS 길이 formatter로 씁니다.

use std::path::Path;

use stellar_ops::core_types::ids::StableId;
use stellar_ops::guides::{read_guides, GuidesReadError, GuidesReport};

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Success};
use crate::views;

const USAGE: &str = "guides에는 <프로젝트 경로> <문서 id> [--node <Frame id>]가 필요합니다";

fn parse_id(value: &str, kind: &str) -> Result<StableId, CliError> {
    value
        .parse()
        .map_err(|_| CliError::usage(format!("{kind}는 UUID 형식이어야 합니다: {value}")))
}

fn read_error(error: GuidesReadError, context: &str) -> CliError {
    let error = match error {
        GuidesReadError::Open(e) => views::open_error(&e),
        GuidesReadError::Document(e) => views::document_error(&e),
        GuidesReadError::NotAFrame(id) => CliError::new(
            ExitStatus::NoInput,
            "not-a-frame",
            format!("node {id}가 문서의 Frame이 아닙니다"),
        ),
    };
    error.with_context(context.to_string())
}

/// `guides <경로> <문서 id> [--node <Frame id>]`.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    let mut positional = Vec::new();
    let mut node = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--node" if node.is_none() => {
                let value = iter.next().ok_or_else(|| CliError::usage(USAGE))?;
                node = Some(parse_id(value, "node id")?);
            }
            other if other.starts_with('-') => {
                return Err(CliError::usage(format!(
                    "알 수 없거나 중복된 옵션: {other}"
                )))
            }
            other => positional.push(other),
        }
    }
    let [path, document] = positional.as_slice() else {
        return Err(CliError::usage(USAGE));
    };
    let document = parse_id(document, "문서 id")?;
    let report = read_guides(Path::new(path), document, node).map_err(|e| read_error(e, path))?;
    Ok(Success {
        result: report_json(&report),
        diagnostics: Vec::new(),
    })
}

fn tracks(values: &[(String, String)]) -> JsonValue {
    JsonValue::Array(
        values
            .iter()
            .map(|(start, end)| {
                JsonValue::Array(vec![
                    JsonValue::string(start.clone()),
                    JsonValue::string(end.clone()),
                ])
            })
            .collect(),
    )
}

fn strings(values: &[String]) -> JsonValue {
    JsonValue::Array(values.iter().cloned().map(JsonValue::string).collect())
}

/// 조회 결과 JSON(MCP `document.guides`와 같은 field 이름).
pub fn report_json(report: &GuidesReport) -> JsonValue {
    JsonValue::object(vec![
        ("revision", views::revision(report.revision)),
        ("document", views::id(report.document)),
        (
            "frames",
            JsonValue::Array(
                report
                    .frames
                    .iter()
                    .map(|frame| {
                        JsonValue::object(vec![
                            ("owner", views::id(frame.owner)),
                            (
                                "guides",
                                JsonValue::Array(
                                    frame
                                        .guides
                                        .iter()
                                        .map(|guide| {
                                            JsonValue::object(vec![
                                                ("id", views::id(guide.id)),
                                                ("axis", JsonValue::string(guide.axis)),
                                                (
                                                    "position",
                                                    JsonValue::string(guide.position.clone()),
                                                ),
                                                ("line", JsonValue::string(guide.line.clone())),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
                            (
                                "grids",
                                JsonValue::Array(
                                    frame
                                        .grids
                                        .iter()
                                        .map(|grid| {
                                            JsonValue::object(vec![
                                                ("line", JsonValue::string(grid.line.clone())),
                                                ("visible", JsonValue::Bool(grid.visible)),
                                                ("columns", tracks(&grid.columns)),
                                                ("rows", tracks(&grid.rows)),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "views",
            JsonValue::Array(
                report
                    .views
                    .iter()
                    .map(|view| {
                        JsonValue::object(vec![
                            ("view", views::id(view.view)),
                            (
                                "variables",
                                JsonValue::Array(
                                    view.variables
                                        .iter()
                                        .map(|row| {
                                            JsonValue::object(vec![
                                                ("id", views::id(row.id)),
                                                ("name", JsonValue::string(row.name.clone())),
                                                (
                                                    "value_type",
                                                    JsonValue::string(row.value_type.clone()),
                                                ),
                                                (
                                                    "default",
                                                    row.default
                                                        .clone()
                                                        .map_or(JsonValue::Null, JsonValue::string),
                                                ),
                                                ("allowed", strings(&row.allowed)),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
                            (
                                "variants",
                                JsonValue::Array(
                                    view.variants
                                        .iter()
                                        .map(|row| {
                                            JsonValue::object(vec![
                                                ("id", views::id(row.id)),
                                                ("when", strings(&row.when)),
                                                (
                                                    "property_patches",
                                                    JsonValue::int(row.property_patches as i64),
                                                ),
                                                (
                                                    "structure_patches",
                                                    JsonValue::int(row.structure_patches as i64),
                                                ),
                                                (
                                                    "slot_changes",
                                                    JsonValue::int(row.slot_changes as i64),
                                                ),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
                            ("diagnostics", strings(&view.diagnostics)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

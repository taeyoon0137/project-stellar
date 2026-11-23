//! `context`: 저장된 문서의 Component Context 선언·진단과 위치별 해석을 조회합니다(XXX.7.11, ADR-0114 결정 6).
//!
//! 조회는 MCP `document.context`와 같은 ops 함수(`stellar_ops::context::read_context`)입니다. 값·식·타입 문장은
//! `stellar apply`의 Context 편집 입력과 같은 문법이라 출력을 그대로 다시 입력으로 쓸 수 있습니다.

use std::path::Path;

use stellar_ops::context::{
    read_context, ContextReadError, ContextReport, ContextSite, ContextSiteReport, ContextSource,
};
use stellar_ops::core_types::ids::StableId;

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Success};
use crate::views;

const USAGE: &str =
    "context에는 <프로젝트 경로> <문서 id> [--node <node id> [--instances <id,…>]]가 필요합니다";

fn parse_id(value: &str, kind: &str) -> Result<StableId, CliError> {
    value
        .parse()
        .map_err(|_| CliError::usage(format!("{kind}는 UUID 형식이어야 합니다: {value}")))
}

fn read_error(error: ContextReadError, context: &str) -> CliError {
    let error = match error {
        ContextReadError::Open(e) => views::open_error(&e),
        ContextReadError::Document(e) => views::document_error(&e),
        ContextReadError::Components(e) => crate::text_source::operation_error(&e),
        ContextReadError::Site(e) => CliError::new(
            ExitStatus::NoInput,
            "unknown-site",
            format!("위치를 해석할 수 없습니다: {e}"),
        ),
    };
    error.with_context(context.to_string())
}

/// `context <경로> <문서 id> [--node <node id> [--instances <id,…>]]`.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    let mut positional = Vec::new();
    let mut node = None;
    let mut instances = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--node" if node.is_none() => {
                let value = iter.next().ok_or_else(|| CliError::usage(USAGE))?;
                node = Some(parse_id(value, "node id")?);
            }
            "--instances" if instances.is_none() => {
                let value = iter.next().ok_or_else(|| CliError::usage(USAGE))?;
                instances = Some(
                    value
                        .split(',')
                        .map(|id| parse_id(id, "인스턴스 id"))
                        .collect::<Result<Vec<_>, _>>()?,
                );
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
    if instances.is_some() && node.is_none() {
        return Err(CliError::usage("--instances는 --node와 함께 씁니다"));
    }
    let document = parse_id(document, "문서 id")?;
    let site = node.map(|node| ContextSite::virtual_node(instances.unwrap_or_default(), node));
    let report =
        read_context(Path::new(path), document, site.as_ref()).map_err(|e| read_error(e, path))?;
    Ok(Success {
        result: report_json(&report),
        diagnostics: Vec::new(),
    })
}

fn ids(values: &[StableId]) -> JsonValue {
    JsonValue::Array(values.iter().map(|id| views::id(*id)).collect())
}

fn opt_text(value: &Option<String>) -> JsonValue {
    value
        .as_ref()
        .map_or(JsonValue::Null, |text| JsonValue::string(text.clone()))
}

fn source_json(source: &ContextSource) -> JsonValue {
    match source {
        ContextSource::Default => JsonValue::object(vec![("kind", JsonValue::string("default"))]),
        ContextSource::Provision {
            provision,
            provider,
            instances,
        } => JsonValue::object(vec![
            ("kind", JsonValue::string("provision")),
            ("provision", views::id(*provision)),
            ("provider", views::id(*provider)),
            ("instances", ids(instances)),
        ]),
    }
}

fn site_json(site: &ContextSite) -> JsonValue {
    JsonValue::object(vec![
        ("node", views::id(site.node)),
        ("instances", ids(&site.instances)),
    ])
}

fn members_json(entries: &[(StableId, Vec<StableId>)], owner: &'static str) -> JsonValue {
    JsonValue::Array(
        entries
            .iter()
            .map(|(id, keys)| JsonValue::object(vec![(owner, views::id(*id)), ("keys", ids(keys))]))
            .collect(),
    )
}

fn site_report_json(report: &ContextSiteReport) -> JsonValue {
    JsonValue::object(vec![
        ("site", site_json(&report.site)),
        (
            "resolutions",
            JsonValue::Array(
                report
                    .resolutions
                    .iter()
                    .map(|row| {
                        JsonValue::object(vec![
                            ("key", views::id(row.key)),
                            ("name", JsonValue::string(row.name.clone())),
                            ("value", opt_text(&row.value)),
                            ("error", opt_text(&row.error)),
                            ("source", source_json(&row.source)),
                            (
                                "path",
                                JsonValue::Array(row.path.iter().map(site_json).collect()),
                            ),
                            ("via_slots", ids(&row.via_slots)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "candidates",
            JsonValue::Array(
                report
                    .candidates
                    .iter()
                    .map(|row| {
                        JsonValue::object(vec![
                            ("key", views::id(row.key)),
                            ("name", JsonValue::string(row.name.clone())),
                            ("value_type", JsonValue::string(row.value_type.clone())),
                            ("needs_declaration", JsonValue::Bool(row.needs_declaration)),
                            ("source", source_json(&row.source)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// 조회 결과 JSON(MCP `document.context`와 같은 field 이름).
pub fn report_json(report: &ContextReport) -> JsonValue {
    JsonValue::object(vec![
        ("revision", views::revision(report.revision)),
        ("document", views::id(report.document)),
        (
            "keys",
            JsonValue::Array(
                report
                    .keys
                    .iter()
                    .map(|row| {
                        JsonValue::object(vec![
                            ("id", views::id(row.id)),
                            ("name", JsonValue::string(row.name.clone())),
                            ("scope", JsonValue::string(row.scope.clone())),
                            ("value_type", JsonValue::string(row.value_type.clone())),
                            ("default", JsonValue::string(row.default.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "provisions",
            JsonValue::Array(
                report
                    .provisions
                    .iter()
                    .map(|row| {
                        JsonValue::object(vec![
                            ("id", views::id(row.id)),
                            ("owner", JsonValue::string(row.owner.clone())),
                            ("provider", views::id(row.provider)),
                            ("key", views::id(row.key)),
                            ("value", JsonValue::string(row.value.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "requirements",
            members_json(&report.requirements, "component"),
        ),
        ("slot_keys", members_json(&report.slot_keys, "slot")),
        (
            "diagnostics",
            JsonValue::Array(
                report
                    .diagnostics
                    .iter()
                    .map(|row| {
                        JsonValue::object(vec![
                            ("code", JsonValue::string(row.code)),
                            ("severity", JsonValue::string(row.severity)),
                            ("message", JsonValue::string(row.message.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "site",
            report
                .site
                .as_ref()
                .map_or(JsonValue::Null, site_report_json),
        ),
    ])
}

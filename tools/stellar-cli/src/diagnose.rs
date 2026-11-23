//! `diagnose`: 저장 프로젝트의 무결성 진단을 필터·page로 읽습니다(XXV.3.1, ADR-0095).
//!
//! 수집은 `stellar_ops::integrity::diagnose_snapshot`, 필터·pagination은 `DiagnosticIndex::page`입니다. 이 명령은
//! 읽기 전용이며 파일을 쓰지 않습니다. 진단이 있어도 성공(0)입니다. 판정 실패가 아니라 목록이기 때문입니다.
//!
//! cursor는 프로젝트 revision token과 index cursor를 함께 담습니다. 프로젝트가 바뀌면 이전 cursor는 거부합니다.

use std::collections::BTreeSet;
use std::path::Path;

use stellar_ops::core_types::ids::StableId;
use stellar_ops::integrity::diagnose_snapshot;
use stellar_ops::integrity::diagnostic::{Diagnostic, DiagnosticKind, Severity};
use stellar_ops::integrity::index::{Cursor, DiagnosticFilter, PageError, MAX_PAGE_LIMIT};
use stellar_ops::query::property_path_label;
use stellar_ops::snapshot::open_snapshot;

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Success};
use crate::views;

/// 기본 page 크기.
pub const DEFAULT_LIMIT: usize = 1000;

const USAGE: &str = "diagnose에는 <프로젝트 경로>와 선택 [--kind <종류>]... [--severity info|warning|error] [--document <id>|--project] [--source <id>] [--limit <n>] [--cursor <c>]가 필요합니다";

struct Args<'a> {
    path: &'a str,
    filter: DiagnosticFilter,
    limit: usize,
    cursor: Option<&'a str>,
}

fn parse_id(value: &str, what: &str) -> Result<StableId, CliError> {
    value
        .parse()
        .map_err(|_| CliError::usage(format!("{what}는 UUID 형식이어야 합니다: {value}")))
}

fn parse_args(args: &[String]) -> Result<Args<'_>, CliError> {
    let mut path = None;
    let mut filter = DiagnosticFilter::default();
    let mut limit = DEFAULT_LIMIT;
    let mut cursor = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = |name: &str| {
            iter.next()
                .map(String::as_str)
                .ok_or_else(|| CliError::usage(format!("{name}에는 값이 필요합니다")))
        };
        match arg.as_str() {
            "--kind" => {
                let name = value("--kind")?;
                let kind = DiagnosticKind::from_name(name).ok_or_else(|| {
                    let names: Vec<&str> = DiagnosticKind::ALL.iter().map(|k| k.name()).collect();
                    CliError::usage(format!(
                        "알 수 없는 진단 종류: {name} (가능: {})",
                        names.join(", ")
                    ))
                })?;
                filter.kinds.insert(kind);
            }
            "--severity" => {
                let name = value("--severity")?;
                filter.min_severity = Some(Severity::from_name(name).ok_or_else(|| {
                    CliError::usage(format!("심각도는 info|warning|error입니다: {name}"))
                })?);
            }
            "--document" => {
                if filter.document.is_some() {
                    return Err(CliError::usage(
                        "--document·--project는 하나만 줄 수 있습니다",
                    ));
                }
                filter.document = Some(Some(parse_id(value("--document")?, "문서 id")?));
            }
            "--project" => {
                if filter.document.is_some() {
                    return Err(CliError::usage(
                        "--document·--project는 하나만 줄 수 있습니다",
                    ));
                }
                filter.document = Some(None);
            }
            "--source" => filter.source = Some(parse_id(value("--source")?, "source id")?),
            "--limit" => {
                let text = value("--limit")?;
                limit = text
                    .parse::<usize>()
                    .ok()
                    .filter(|n| (1..=MAX_PAGE_LIMIT).contains(n))
                    .ok_or_else(|| {
                        CliError::usage(format!(
                            "--limit는 1..={MAX_PAGE_LIMIT} 정수입니다: {text}"
                        ))
                    })?;
            }
            "--cursor" => cursor = Some(value("--cursor")?),
            option if option.starts_with('-') => {
                return Err(CliError::usage(format!("알 수 없는 옵션: {option}")))
            }
            positional if path.is_none() => path = Some(positional),
            extra => return Err(CliError::usage(format!("인자가 너무 많습니다: {extra}"))),
        }
    }
    Ok(Args {
        path: path.ok_or_else(|| CliError::usage(USAGE))?,
        filter,
        limit,
        cursor,
    })
}

fn diagnostic_json(d: &Diagnostic) -> JsonValue {
    let remediation = d
        .remediation
        .iter()
        .map(|action| {
            JsonValue::object(vec![
                ("action", JsonValue::string(action.name())),
                ("form", JsonValue::string(action.descriptor().form.name())),
            ])
        })
        .collect();
    JsonValue::object(vec![
        ("id", JsonValue::string(d.id().to_string())),
        ("kind", JsonValue::string(d.kind().name())),
        ("severity", JsonValue::string(d.severity.name())),
        ("code", JsonValue::string(d.code())),
        ("origin", JsonValue::string(d.key.origin.clone())),
        ("document", views::opt_id(d.document())),
        ("source", views::opt_id(d.source())),
        (
            "property",
            d.property().map_or(JsonValue::Null, |p| {
                JsonValue::string(property_path_label(p))
            }),
        ),
        ("message", JsonValue::string(d.message.clone())),
        ("remediation", JsonValue::Array(remediation)),
    ])
}

fn stale(message: &str) -> CliError {
    CliError::new(ExitStatus::DataErr, "stale-cursor", message.to_string())
}

/// `diagnose <경로> [옵션]`.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    let args = parse_args(args)?;
    let context = args.path.to_string();
    let snapshot = open_snapshot(Path::new(args.path))
        .map_err(|e| views::open_error(&e).with_context(context.clone()))?;
    let revision = snapshot.revision().token();
    let cursor = match args.cursor {
        None => None,
        Some(token) => {
            let (project, index) = token
                .split_once('~')
                .ok_or_else(|| CliError::usage(format!("cursor 형식이 아닙니다: {token}")))?;
            if project != revision {
                return Err(stale(
                    "cursor 이후 프로젝트가 바뀌었습니다. 처음부터 다시 읽으세요",
                ));
            }
            Some(
                Cursor::parse(index)
                    .ok_or_else(|| CliError::usage(format!("cursor 형식이 아닙니다: {token}")))?,
            )
        }
    };
    let collected = diagnose_snapshot(&snapshot);
    let index = collected.engine.index();
    let page = index
        .page(&args.filter, cursor.as_ref(), args.limit)
        .map_err(|e| match e {
            PageError::Stale => stale(&e.to_string()),
            PageError::InvalidLimit(_) => CliError::usage(e.to_string()),
        })?;
    let summary = index.summary();
    let by_kind = DiagnosticKind::ALL
        .iter()
        .filter(|k| summary.kind(**k) > 0)
        .map(|k| (k.name(), views::count(summary.kind(*k))))
        .collect();
    let by_severity = Severity::ALL
        .iter()
        .map(|s| (s.name(), views::count(summary.severity(*s))))
        .collect();
    let unreadable = collected
        .unreadable
        .iter()
        .map(|(document, message)| {
            JsonValue::object(vec![
                ("document", views::id(*document)),
                ("message", JsonValue::string(message.clone())),
            ])
        })
        .collect();
    let kinds: BTreeSet<&str> = args.filter.kinds.iter().map(|k| k.name()).collect();
    Ok(Success {
        result: JsonValue::object(vec![
            ("revision", views::revision(snapshot.revision())),
            (
                "documents_checked",
                views::count(collected.documents_checked),
            ),
            ("unreadable", JsonValue::Array(unreadable)),
            (
                "summary",
                JsonValue::object(vec![
                    ("total", views::count(summary.total())),
                    ("by_kind", JsonValue::object(by_kind)),
                    ("by_severity", JsonValue::object(by_severity)),
                ]),
            ),
            (
                "filter",
                JsonValue::object(vec![
                    (
                        "kinds",
                        JsonValue::Array(kinds.into_iter().map(JsonValue::string).collect()),
                    ),
                    (
                        "min_severity",
                        args.filter
                            .min_severity
                            .map_or(JsonValue::Null, |s| JsonValue::string(s.name())),
                    ),
                    ("matching", views::count(index.count(&args.filter))),
                ]),
            ),
            (
                "items",
                JsonValue::Array(page.items.iter().map(diagnostic_json).collect()),
            ),
            (
                "next_cursor",
                page.next.map_or(JsonValue::Null, |c| {
                    JsonValue::string(format!("{revision}~{}", c.token()))
                }),
            ),
        ]),
        diagnostics: Vec::new(),
    })
}

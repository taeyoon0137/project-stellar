//! `export-web`: 저장된 View 하나의 고정 Free Frame HTML+CSS 문서를 JSON 결과에 담습니다.

use std::path::Path;

use stellar_ops::core_types::ids::StableId;
use stellar_ops::snapshot::open_snapshot;
use stellar_ops::web_frame::{export_web_frame, HtmlError};

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Diagnostic, Severity, Success};
use crate::views;

/// `export-web <프로젝트 경로> <문서 id> <View id>`.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    let [path, document, view] = args else {
        return Err(CliError::usage(
            "export-web에는 <프로젝트 경로> <문서 id> <View id>가 필요합니다",
        ));
    };
    if let Some(option) = args.iter().find(|s| s.starts_with('-')) {
        return Err(CliError::usage(format!("알 수 없는 옵션: {option}")));
    }
    let parse_id = |value: &str, kind: &str| -> Result<StableId, CliError> {
        value
            .parse()
            .map_err(|_| CliError::usage(format!("{kind}는 UUID 형식이어야 합니다: {value}")))
    };
    let document = parse_id(document, "문서 id")?;
    let view = parse_id(view, "View id")?;
    let snapshot =
        open_snapshot(Path::new(path)).map_err(|e| views::open_error(&e).with_context(path))?;
    let report = export_web_frame(&snapshot, document, view).map_err(|e| {
        let error = match e {
            HtmlError::Unsaved => CliError::new(
                ExitStatus::NoInput,
                "unsaved-project",
                "저장된 generation이 없습니다",
            ),
            HtmlError::UnsupportedSchema(version) => CliError::new(
                ExitStatus::DataErr,
                "unsupported-schema",
                format!("generation schema {version}은 지원하지 않습니다"),
            ),
            HtmlError::Document(e) => views::document_error(&e),
            HtmlError::UnknownView(id) => CliError::new(
                ExitStatus::NoInput,
                "unknown-view",
                format!("문서 {document}에 View {id}가 없습니다"),
            ),
        };
        error.with_context(path)
    })?;
    let mut fields = vec![
        ("revision", views::revision(report.revision)),
        ("document", views::id(report.document)),
        ("view", views::id(report.view)),
        ("root_frame", views::id(report.root_frame)),
        ("format", JsonValue::string("fixed-free-frame-html")),
        (
            "html",
            report
                .html
                .as_ref()
                .map_or(JsonValue::Null, JsonValue::string),
        ),
    ];
    if let Some(hash) = report.component_source_hash {
        fields.push(("component_source_hash", JsonValue::string(hash.to_string())));
    }
    let result = JsonValue::object(fields);
    let diagnostics: Vec<_> = report
        .diagnostics
        .into_iter()
        .map(|d| Diagnostic {
            severity: if d.error {
                Severity::Error
            } else {
                Severity::Warning
            },
            code: d.code,
            message: d.message,
        })
        .collect();
    if report.html.is_some() {
        Ok(Success {
            result,
            diagnostics,
        })
    } else {
        let mut error = CliError::new(
            ExitStatus::Failure,
            "web-export-failed",
            "지정 View를 고정 Free Frame HTML+CSS로 내보내지 못했습니다",
        )
        .with_result(result)
        .with_context(path);
        error.diagnostics = diagnostics;
        Err(error)
    }
}

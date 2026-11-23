//! opt-in 단일 View 텍스트 export·읽기 전용 검사 (ADR-0059).

use std::path::Path;

use stellar_ops::text_source::{self, TextSourceOperationError};

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Success};
use crate::views;

pub(crate) fn operation_error(error: &TextSourceOperationError) -> CliError {
    let (status, code) = match error {
        TextSourceOperationError::Open(error) => return views::open_error(error),
        TextSourceOperationError::Document(error) => return views::document_error(error),
        TextSourceOperationError::Unsupported(_) => {
            (ExitStatus::Unavailable, "text-source-unsupported")
        }
        TextSourceOperationError::Invalid(_) => (ExitStatus::DataErr, "text-source-invalid"),
        TextSourceOperationError::Changed => (ExitStatus::TempFail, "text-source-changed"),
        TextSourceOperationError::Io(kind) => (ExitStatus::from_io_kind(*kind), "text-source-io"),
        TextSourceOperationError::Apply(error) => return crate::apply::apply_error(error),
    };
    CliError::new(status, code, error.to_string())
}

/// `query <design 폴더> --source-span <node StableId>`: 다중 파일 원본에서 node의 파일·줄 범위(V.10.10).
pub fn query_source_span(args: &[String]) -> Result<Success, CliError> {
    let usage = || CliError::usage("query <design 폴더> --source-span <node StableId>");
    let [design, flag, node] = args else {
        return Err(usage());
    };
    if flag != "--source-span" {
        return Err(usage());
    }
    let id: stellar_ops::core_types::ids::StableId = node
        .parse()
        .map_err(|_| CliError::usage("node는 StableId여야 합니다"))?;
    let span = text_source::source_span(Path::new(design), id)
        .map_err(|error| operation_error(&error).with_context(design.clone()))?
        .ok_or_else(|| {
            CliError::new(
                ExitStatus::NoInput,
                "source-span-not-found",
                format!("텍스트 원본에 node {id}가 없습니다"),
            )
        })?;
    Ok(Success {
        result: JsonValue::object(vec![
            ("node", views::id(id)),
            ("file", JsonValue::string(span.file)),
            ("start_line", views::count(span.start_line as usize)),
            ("end_line", views::count(span.end_line as usize)),
        ]),
        diagnostics: Vec::new(),
    })
}

fn project_success(
    operation: &str,
    outcome: stellar_ops::text_source::TextProjectOutcome,
) -> Success {
    Success {
        result: JsonValue::object(vec![
            ("operation", JsonValue::string(operation)),
            ("document", views::id(outcome.document)),
            ("file_count", views::count(outcome.file_count)),
            (
                "source_hash",
                JsonValue::string(outcome.source_hash.to_string()),
            ),
            ("node_count", views::count(outcome.node_count)),
            ("component_count", views::count(outcome.component_count)),
            ("image_count", views::count(outcome.image_count)),
        ]),
        diagnostics: Vec::new(),
    }
}

/// `text-source adopt <프로젝트>`: 텍스트 원본이 없는 chunk 프로젝트를 앱 창 열기와 같은 검증된 변환으로 원본으로
/// 옮깁니다(ADR-0113 결정 3).
fn adopt(project: &str) -> Result<Success, CliError> {
    use stellar_ops::snapshot::save::{adopt, AdoptError};
    let lock = stellar_ops::lock::LockPolicy::Required {
        dir: stellar_ops::lock::default_lock_dir()
            .map_err(|e| crate::apply::apply_error(&stellar_ops::apply::ApplyError::Lock(e)))?,
    };
    let outcome = adopt(Path::new(project), &lock).map_err(|error| {
        let message = error.to_string();
        let (status, code) = match error {
            AdoptError::Open(e) => return views::open_error(&e).with_context(project.to_owned()),
            AdoptError::Lock(e) => {
                return crate::apply::apply_error(&stellar_ops::apply::ApplyError::Lock(e))
                    .with_context(project.to_owned())
            }
            AdoptError::AlreadyText => (ExitStatus::Failure, "text-source-present"),
            AdoptError::Unsaved => (ExitStatus::NoInput, "unsaved-project"),
            AdoptError::Unsupported(_) => (ExitStatus::Unavailable, "text-source-unsupported"),
            AdoptError::Save(stellar_ops::snapshot::save::SaveError::ConcurrentWrite) => {
                (ExitStatus::TempFail, "concurrent-write")
            }
            AdoptError::Save(_) => (ExitStatus::IoErr, "text-source-io"),
        };
        CliError::new(status, code, message).with_context(project.to_owned())
    })?;
    Ok(Success {
        result: JsonValue::object(vec![
            ("operation", JsonValue::string("adopt")),
            ("revision", JsonValue::string(outcome.revision.token())),
            ("file_count", views::count(outcome.files)),
            ("written", views::count(outcome.written)),
        ]),
        diagnostics: Vec::new(),
    })
}

/// `text-source export <프로젝트> <새 파일>`, `check <파일>`, 다중 파일 원본(V.10.4)
/// `export-project <프로젝트> <새 design 폴더>`, `check-project <design 폴더>`, 원본 전환 `adopt <프로젝트>`.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    if args.iter().any(|arg| arg.starts_with('-')) {
        return Err(CliError::usage("text-source는 옵션을 받지 않습니다"));
    }
    match args {
        [operation, project] if operation == "adopt" => return adopt(project),
        [operation, project, output] if operation == "export-project" => {
            let outcome = text_source::export_project(Path::new(project), Path::new(output))
                .map_err(|error| {
                    operation_error(&error).with_context(format!("{project} -> {output}"))
                })?;
            return Ok(project_success("export-project", outcome));
        }
        [operation, design] if operation == "check-project" => {
            let outcome = text_source::check_project(Path::new(design))
                .map_err(|error| operation_error(&error).with_context(design.clone()))?;
            return Ok(project_success("check-project", outcome));
        }
        _ => {}
    }
    let (operation, outcome, context) = match args {
        [operation, project, output] if operation == "export" => (
            "export",
            text_source::export(Path::new(project), Path::new(output)),
            format!("{project} -> {output}"),
        ),
        [operation, path] if operation == "check" => {
            ("check", text_source::check(Path::new(path)), path.clone())
        }
        _ => {
            return Err(CliError::usage(
                "text-source export <프로젝트> <새 파일> | check <파일> | export-project <프로젝트> <새 design 폴더> | check-project <design 폴더> | adopt <프로젝트>",
            ))
        }
    };
    let outcome = outcome.map_err(|error| operation_error(&error).with_context(context))?;
    Ok(Success {
        result: JsonValue::object(vec![
            ("operation", JsonValue::string(operation)),
            ("document", views::id(outcome.document)),
            (
                "source_hash",
                JsonValue::string(outcome.source_hash.to_string()),
            ),
            ("node_count", views::count(outcome.node_count)),
        ]),
        diagnostics: Vec::new(),
    })
}

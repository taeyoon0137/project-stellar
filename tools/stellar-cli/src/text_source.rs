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

/// `text-source export <프로젝트> <새 파일>` 또는 `text-source check <파일>`.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    if args.iter().any(|arg| arg.starts_with('-')) {
        return Err(CliError::usage("text-source는 옵션을 받지 않습니다"));
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
                "text-source export <프로젝트> <새 파일> | check <파일>",
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

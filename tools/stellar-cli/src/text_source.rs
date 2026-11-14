//! opt-in 단일 View 텍스트 export·검사·명령 적용 (ADR-0059).

use std::path::Path;

use stellar_ops::apply::ApplyMode;
use stellar_ops::lock::{default_lock_dir, LockPolicy};
use stellar_ops::text_source::{self, TextSourceOperationError};

use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Success};
use crate::views;

pub(crate) fn operation_error(error: TextSourceOperationError) -> CliError {
    let (status, code) = match &error {
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

/// `text-source export/check/apply`를 실행합니다.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    if args.first().map(String::as_str) == Some("apply") {
        return run_apply(&args[1..]);
    }
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
                "text-source export <프로젝트> <새 파일> | check <파일> | apply <파일> <입력|-> [--dry-run]",
            ))
        }
    };
    let outcome = outcome.map_err(|error| operation_error(error).with_context(context))?;
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

fn run_apply(args: &[String]) -> Result<Success, CliError> {
    let mut positional = Vec::new();
    let mut dry_run = false;
    for arg in args {
        match arg.as_str() {
            "--dry-run" if !dry_run => dry_run = true,
            other if other.starts_with('-') && other != "-" => {
                return Err(CliError::usage(format!(
                    "알 수 없거나 중복된 옵션: {other}"
                )));
            }
            other => positional.push(other),
        }
    }
    let [path, input] = positional.as_slice() else {
        return Err(CliError::usage(
            "text-source apply <파일> <입력|-> [--dry-run]",
        ));
    };
    let doc = crate::apply::parse_envelope(
        &crate::apply::read_input(input)?,
        "stellar-text-apply",
        "base_source_hash",
    )?;
    let base_source_hash = crate::apply::text_field(&doc, "base_source_hash", "입력")?
        .parse::<text_source::ContentHash>()
        .map_err(|_| {
            CliError::new(
                ExitStatus::DataErr,
                "invalid-transaction",
                "base_source_hash 형식 오류",
            )
        })?;
    let request = text_source::TextSourceApplyRequest {
        base_source_hash,
        document: crate::apply::id_field(&doc, "document", "입력")?,
        commands: crate::apply::parse_commands(&doc)?,
    };
    let mode = if dry_run {
        ApplyMode::DryRun
    } else {
        ApplyMode::Write
    };
    let lock = if dry_run {
        LockPolicy::None
    } else {
        LockPolicy::Required {
            dir: default_lock_dir().map_err(|error| crate::project_cmds::lock_error(&error))?,
        }
    };
    let outcome = text_source::apply(Path::new(path), &request, mode, &lock)
        .map_err(|e| operation_error(e).with_context(*path))?;
    Ok(Success {
        result: JsonValue::object(vec![
            ("operation", JsonValue::string("apply")),
            (
                "mode",
                JsonValue::string(if dry_run { "dry-run" } else { "write" }),
            ),
            ("document", views::id(outcome.document)),
            (
                "base_source_hash",
                JsonValue::string(outcome.base_source_hash.to_string()),
            ),
            (
                "source_hash",
                JsonValue::string(outcome.source_hash.to_string()),
            ),
            ("node_count", views::count(outcome.node_count)),
            ("command_count", views::count(outcome.command_count)),
            ("written", JsonValue::Bool(outcome.written)),
            ("changes", views::diff_report(&outcome.diff)),
        ]),
        diagnostics: Vec::new(),
    })
}

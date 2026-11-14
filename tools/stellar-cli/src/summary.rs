//! `stellar diff-summary` (XXIV.3.2).
//!
//! `stellar diff-summary <base> <target> [--output <파일>] [--include-content]`는 같은 프로젝트 두
//! 상태의 semantic diff 요약을 만듭니다. 기본은 redaction(`stellar_ops::summary`)이며 프로젝트
//! 텍스트·이름이 될 수 있는 값은 종류만 남깁니다. `--include-content`를 명시할 때만 값을 포함합니다.
//! 프로젝트 이름·경로·repository URL은 어느 경우에도 넣지 않습니다.
//!
//! - `--output`이 없으면 요약 object가 결과 envelope의 `result`입니다.
//! - `--output`이 있으면 요약 JSON 한 줄을 그 파일에 임시 파일 + rename으로 기록하고, 결과에는
//!   artifact 크기·hash만 넣습니다(경로는 stderr 문맥에만). 입력 프로젝트 폴더 안에는 쓰지 않습니다.
//!
//! artifact 형식(`stellar-diff-summary` v1): `format`, `format_version`, `redaction`,
//! `base_revision`, `target_revision`, `total`, `by_kind`, `by_document`, `redacted_values`, `changes`.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use stellar_ops::compare::semantic_diff_projects;
use stellar_ops::summary::{artifact_hash, summarize, DiffSummary, Redaction, SummaryValue};

use crate::exit::ExitStatus;
use crate::json::{self, JsonValue};
use crate::output::{CliError, Success};
use crate::views;

/// artifact 형식 식별자.
pub const FORMAT: &str = "stellar-diff-summary";
/// artifact 형식 버전.
pub const FORMAT_VERSION: i64 = 1;

fn value_json(value: Option<&SummaryValue>) -> JsonValue {
    match value {
        None => JsonValue::Null,
        Some(v) => match &v.value {
            None => JsonValue::object(vec![
                ("type", JsonValue::string(v.type_name)),
                ("redacted", JsonValue::Bool(true)),
            ]),
            Some(inner) => JsonValue::object(vec![
                ("type", JsonValue::string(v.type_name)),
                ("value", views::value_inner(inner)),
            ]),
        },
    }
}

fn position_json(position: Option<&stellar_ops::diff::DiffPosition>) -> JsonValue {
    position.map_or(JsonValue::Null, |p| {
        JsonValue::object(vec![
            ("parent", views::opt_id(p.parent)),
            (
                "order_key",
                JsonValue::opt_string(p.order_key_hex.as_deref()),
            ),
        ])
    })
}

/// 요약 JSON(artifact 본문).
pub fn summary_json(summary: &DiffSummary) -> JsonValue {
    JsonValue::object(vec![
        ("format", JsonValue::string(FORMAT)),
        ("format_version", JsonValue::int(FORMAT_VERSION)),
        (
            "redaction",
            JsonValue::string(match summary.redaction {
                Redaction::ProjectContent => "project-content",
                Redaction::None => "none",
            }),
        ),
        ("base_revision", views::revision(summary.base_revision)),
        ("target_revision", views::revision(summary.target_revision)),
        ("total", views::count(summary.entries.len())),
        (
            "by_kind",
            JsonValue::Object(
                summary
                    .by_kind
                    .iter()
                    .map(|(k, n)| (k.to_string(), views::count(*n)))
                    .collect(),
            ),
        ),
        (
            "by_document",
            JsonValue::Object(
                summary
                    .by_document
                    .iter()
                    .map(|(id, n)| (id.to_string(), views::count(*n)))
                    .collect(),
            ),
        ),
        ("redacted_values", views::count(summary.redacted_values)),
        (
            "changes",
            JsonValue::Array(
                summary
                    .entries
                    .iter()
                    .map(|e| {
                        JsonValue::object(vec![
                            ("kind", JsonValue::string(e.kind)),
                            ("document", views::opt_id(e.document)),
                            ("object", views::id(e.object)),
                            (
                                "object_kind",
                                JsonValue::opt_string(e.object_kind.as_deref()),
                            ),
                            ("path", JsonValue::opt_string(e.path.as_deref())),
                            ("old", value_json(e.old.as_ref())),
                            ("new", value_json(e.new.as_ref())),
                            ("from", position_json(e.from.as_ref())),
                            ("to", position_json(e.to.as_ref())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// `path`가 `root` 아래인지(둘 다 존재하는 경로 기준 canonical 비교).
fn inside(path: &Path, root: &Path) -> bool {
    match (std::fs::canonicalize(path), std::fs::canonicalize(root)) {
        (Ok(path), Ok(root)) => path.starts_with(root),
        _ => false,
    }
}

/// artifact를 임시 파일 + rename으로 기록합니다.
fn write_artifact(output: &Path, bytes: &[u8], roots: &[&Path]) -> Result<(), CliError> {
    let context = output.display().to_string();
    let parent = match output.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let file_name = output
        .file_name()
        .ok_or_else(|| CliError::usage(format!("--output은 파일 경로여야 합니다: {context}")))?;
    if roots.iter().any(|root| inside(&parent, root)) {
        return Err(CliError::new(
            ExitStatus::CantCreate,
            "output-inside-project",
            "artifact를 입력 프로젝트 폴더 안에 쓰지 않습니다",
        )
        .with_context(context));
    }
    let io_error = |e: std::io::Error| {
        CliError::new(
            ExitStatus::from_io_kind(e.kind()),
            "artifact-write",
            format!("artifact를 기록할 수 없습니다: {:?}", e.kind()),
        )
        .with_context(context.clone())
    };
    let mut temp_name = file_name.to_os_string();
    temp_name.push(format!(".tmp-{}", std::process::id()));
    let temp = parent.join(temp_name);
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, output)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(io_error(e));
    }
    Ok(())
}

/// `diff-summary <base> <target> [--output <파일>] [--include-content]`.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    let mut positional = Vec::new();
    let mut output = None;
    let mut include_content = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--include-content" if !include_content => include_content = true,
            "--output" if output.is_none() => {
                let value = iter
                    .next()
                    .ok_or_else(|| CliError::usage("--output에 파일 경로가 필요합니다"))?;
                output = Some(PathBuf::from(value));
            }
            other if other.starts_with('-') && other != "-" => {
                return Err(CliError::usage(format!(
                    "알 수 없거나 중복된 옵션: {other}"
                )))
            }
            other => positional.push(Path::new(other)),
        }
    }
    let [base, target] = positional.as_slice() else {
        return Err(CliError::usage(
            "diff-summary에는 <base 경로> <target 경로>가 필요합니다",
        ));
    };
    let context = format!("{} -> {}", base.display(), target.display());
    let diff = semantic_diff_projects(base, target)
        .map_err(|e| crate::project_cmds::compare_error_public(&e).with_context(context.clone()))?;
    let redaction = if include_content {
        Redaction::None
    } else {
        Redaction::ProjectContent
    };
    let summary = summarize(
        &diff.report,
        diff.base_revision,
        diff.target_revision,
        redaction,
    );
    let document = summary_json(&summary);
    let Some(output) = output else {
        return Ok(Success {
            result: document,
            diagnostics: Vec::new(),
        });
    };
    let mut bytes = json::write(&document).into_bytes();
    bytes.push(b'\n');
    write_artifact(&output, &bytes, &[base, target])?;
    Ok(Success {
        result: JsonValue::object(vec![
            (
                "artifact",
                JsonValue::object(vec![
                    ("format", JsonValue::string(FORMAT)),
                    ("bytes", views::count(bytes.len())),
                    ("hash", JsonValue::string(artifact_hash(&bytes))),
                ]),
            ),
            (
                "redaction",
                document
                    .get("redaction")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            ),
            ("total", views::count(summary.entries.len())),
            ("redacted_values", views::count(summary.redacted_values)),
        ]),
        diagnostics: Vec::new(),
    })
}

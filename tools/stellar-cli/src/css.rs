//! `css`: 저장된 node 속성을 CSS 선언 블록으로 복사하고, 선언 블록을 붙여넣습니다(IX.11.15).
//! 앱 속성 패널과 같은 core API(`stellar_ops::css`)를 씁니다. 속성 이름은 CSS longhand(또는 Stellar 전용 이름)입니다.

use std::io::Read;
use std::path::Path;

use stellar_ops::apply::{ApplyError, ApplyMode};
use stellar_ops::core_types::ids::StableId;
use stellar_ops::css::{copy, paste, CssError};
use stellar_ops::lock::{default_lock_dir, LockPolicy};

use crate::apply::apply_error;
use crate::exit::ExitStatus;
use crate::json::JsonValue;
use crate::output::{CliError, Diagnostic, Severity, Success};
use crate::views;

/// 붙여넣기 입력 상한(byte). ops 공용 선언 블록 상한입니다.
const MAX_INPUT: u64 = stellar_ops::css::MAX_DECLARATION_BYTES as u64;

fn parse_id(value: &str, kind: &str) -> Result<StableId, CliError> {
    value
        .parse()
        .map_err(|_| CliError::usage(format!("{kind}는 UUID 형식이어야 합니다: {value}")))
}

fn css_error(error: CssError, context: &str) -> CliError {
    let error = match error {
        CssError::Open(e) => views::open_error(&e),
        CssError::Document(e) => views::document_error(&e),
        CssError::UnknownNode { document, node } => CliError::new(
            ExitStatus::NoInput,
            "unknown-node",
            format!("문서 {document}에 node {node}가 없습니다"),
        ),
        CssError::Paste(e) => CliError::new(ExitStatus::DataErr, e.code, e.message),
        CssError::Ids(e) => CliError::new(
            ExitStatus::IoErr,
            "id-generation-failed",
            format!("새 ID를 만들지 못했습니다: {e}"),
        ),
        CssError::Apply(e) => apply_error(&e),
    };
    error.with_context(context.to_string())
}

fn read_text(source: &str) -> Result<String, CliError> {
    let mut bytes = Vec::new();
    let result = if source == "-" {
        std::io::stdin().take(MAX_INPUT + 1).read_to_end(&mut bytes)
    } else {
        std::fs::File::open(source).and_then(|f| f.take(MAX_INPUT + 1).read_to_end(&mut bytes))
    };
    result.map_err(|e| {
        CliError::new(
            ExitStatus::NoInput,
            "input-unreadable",
            format!("선언 블록을 읽지 못했습니다: {e}"),
        )
    })?;
    if bytes.len() as u64 > MAX_INPUT {
        return Err(CliError::new(
            ExitStatus::DataErr,
            "input-too-large",
            format!("선언 블록은 {MAX_INPUT} byte까지입니다"),
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        CliError::new(
            ExitStatus::DataErr,
            "input-not-utf8",
            "선언 블록이 UTF-8이 아닙니다",
        )
    })
}

/// `css copy <경로> <문서 id> <node id>` | `css paste <경로> <문서 id> <node id>[,<node id>…] <입력 파일|-> [--dry-run]`.
pub fn run(args: &[String]) -> Result<Success, CliError> {
    match args.split_first() {
        Some((sub, rest)) if sub == "copy" => run_copy(rest),
        Some((sub, rest)) if sub == "paste" => run_paste(rest),
        _ => Err(CliError::usage(
            "css에는 copy <경로> <문서 id> <node id> 또는 paste <경로> <문서 id> <node id,…> <입력 파일|-> [--dry-run]이 필요합니다",
        )),
    }
}

fn run_copy(args: &[String]) -> Result<Success, CliError> {
    let [path, document, node] = args else {
        return Err(CliError::usage(
            "css copy에는 <프로젝트 경로> <문서 id> <node id>가 필요합니다",
        ));
    };
    if let Some(option) = args.iter().find(|a| a.starts_with('-')) {
        return Err(CliError::usage(format!("알 수 없는 옵션: {option}")));
    }
    let document = parse_id(document, "문서 id")?;
    let node = parse_id(node, "node id")?;
    let copied = copy(Path::new(path), document, node).map_err(|e| css_error(e, path))?;
    let declarations = copied
        .declarations
        .iter()
        .map(|d| {
            JsonValue::object(vec![
                ("name", JsonValue::string(d.name)),
                ("value", JsonValue::string(d.value.clone())),
                ("stellar_only", JsonValue::Bool(d.stellar_only)),
            ])
        })
        .collect();
    Ok(Success {
        result: JsonValue::object(vec![
            ("revision", views::revision(copied.revision)),
            ("document", views::id(copied.document)),
            ("node", views::id(copied.node)),
            ("text", JsonValue::string(copied.text)),
            ("declarations", JsonValue::Array(declarations)),
        ]),
        diagnostics: Vec::new(),
    })
}

fn run_paste(args: &[String]) -> Result<Success, CliError> {
    let mut positional = Vec::new();
    let mut dry_run = false;
    for arg in args {
        match arg.as_str() {
            "--dry-run" if !dry_run => dry_run = true,
            other if other.starts_with('-') && other != "-" => {
                return Err(CliError::usage(format!(
                    "알 수 없거나 중복된 옵션: {other}"
                )))
            }
            other => positional.push(other),
        }
    }
    let [path, document, nodes, source] = positional.as_slice() else {
        return Err(CliError::usage(
            "css paste에는 <프로젝트 경로> <문서 id> <node id,…> <입력 파일 또는 ->가 필요합니다",
        ));
    };
    let document = parse_id(document, "문서 id")?;
    let nodes: Vec<StableId> = nodes
        .split(',')
        .map(|n| parse_id(n.trim(), "node id"))
        .collect::<Result<_, _>>()?;
    let text = read_text(source)?;
    let mode = if dry_run {
        ApplyMode::DryRun
    } else {
        ApplyMode::Write
    };
    let lock = match mode {
        ApplyMode::DryRun => LockPolicy::None,
        ApplyMode::Write => LockPolicy::Required {
            dir: default_lock_dir().map_err(|e| apply_error(&ApplyError::Lock(e)))?,
        },
    };
    let pasted = paste(Path::new(path), document, &nodes, &text, mode, &lock)
        .map_err(|e| css_error(e, path))?;
    let applied = pasted
        .applied
        .iter()
        .map(|a| {
            JsonValue::object(vec![
                ("name", JsonValue::string(a.name)),
                ("value", JsonValue::string(a.value.clone())),
            ])
        })
        .collect();
    let skipped = pasted
        .skipped
        .iter()
        .map(|s| {
            JsonValue::object(vec![
                ("name", JsonValue::string(s.name.clone())),
                ("value", JsonValue::string(s.value.clone())),
                ("code", JsonValue::string(s.code)),
                ("message", JsonValue::string(s.message.clone())),
            ])
        })
        .collect();
    let diagnostics = pasted
        .skipped
        .iter()
        .map(|s| Diagnostic {
            severity: Severity::Warning,
            code: s.code,
            message: if s.name.is_empty() {
                format!("건너뜀: {}", s.message)
            } else {
                format!("건너뜀 {}: {}", s.name, s.message)
            },
        })
        .collect();
    let mut fields = vec![
        (
            "mode",
            JsonValue::string(match mode {
                ApplyMode::DryRun => "dry-run",
                ApplyMode::Write => "write",
            }),
        ),
        ("document", views::id(document)),
        ("base_revision", views::revision(pasted.base_revision)),
        ("applied", JsonValue::Array(applied)),
        ("skipped", JsonValue::Array(skipped)),
    ];
    match &pasted.outcome {
        Some(outcome) => {
            fields.push(("revision", views::revision(outcome.revision)));
            fields.push(("command_count", views::count(outcome.command_count)));
            fields.push(("changes", views::diff_report(&outcome.diff)));
        }
        None => {
            fields.push(("revision", views::revision(pasted.base_revision)));
            fields.push(("command_count", views::count(0)));
            fields.push(("changes", JsonValue::Null));
        }
    }
    Ok(Success {
        result: JsonValue::object(fields),
        diagnostics,
    })
}

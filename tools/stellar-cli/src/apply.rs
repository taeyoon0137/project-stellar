//! `stellar apply` (XXIV.4.4).
//!
//! 파일이나 표준 입력에서 CommandTransaction JSON을 읽어 `stellar-ops`의 [`apply`]로 적용합니다.
//!
//! ## 입력 형식(`stellar-apply` v1)
//!
//! ```json
//! {"format":"stellar-apply","format_version":1,
//!  "base_revision":"g1:sha256:<64 hex>","document":"<문서 StableId>",
//!  "commands":[{"kind":"set-visible","node":"<id>","visible":false}]}
//! ```
//!
//! | `kind` | 필드 |
//! |---|---|
//! | `set-visible` | `node`, `visible`(bool) |
//! | `set-locked` | `node`, `locked`(bool) |
//! | `rename` | `node`, `name`(string) |
//! | `delete-subtree` | `root` |
//! | `move-node` | `node`, `new_parent`, `order_key`(OrderKey bytes 16진수) |
//! | 그 밖의 closed enum 종류 이름(`insert-node` 등, VII.1.2) | `canonical`(commands canonical bytes 16진수) |
//!
//! 모든 object는 표에 있는 key만 가질 수 있습니다. 등록되지 않은 `kind`는 `unknown-command`(65)로
//! 거부하고 아무것도 쓰지 않습니다. `--dry-run`은 파일을 쓰지 않고 semantic diff만 출력합니다.
//!
//! 쓰기는 OS 사용자 데이터의 프로젝트별 저장 잠금을 잡은 뒤에 generation을 비교하고 교체합니다
//! (XXIV.4.5, `stellar_ops::lock`). 잠금이 사용 중이면 `save-lock-busy`(75, 재시도 가능)로 끝나고
//! 아무것도 쓰지 않습니다. OS 사용자 데이터 위치(`HOME`)를 알 수 없으면 `no-user-data-dir`(78)입니다.

use std::io::Read;
use std::path::Path;

use stellar_ops::apply::{apply, ApplyError, ApplyMode, ApplyRequest, CommandInput, MAX_COMMANDS};
use stellar_ops::core_types::ids::StableId;
use stellar_ops::lock::{default_lock_dir, LockPolicy};
use stellar_ops::revision::ProjectRevision;

use crate::exit::ExitStatus;
use crate::json::{self, JsonLimits, JsonValue};
use crate::output::{CliError, Diagnostic, Severity, Success};
use crate::views;

/// 입력 형식 식별자.
pub const FORMAT: &str = "stellar-apply";
/// 입력 형식 버전.
pub const FORMAT_VERSION: i64 = 1;
/// 입력 최대 byte 수.
pub const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;

fn invalid(code: &'static str, message: impl Into<String>) -> CliError {
    CliError::new(ExitStatus::DataErr, code, message)
}

fn read_input(source: &str) -> Result<Vec<u8>, CliError> {
    let mut bytes = Vec::new();
    let limit = (MAX_INPUT_BYTES as u64) + 1;
    let result = if source == "-" {
        std::io::stdin().lock().take(limit).read_to_end(&mut bytes)
    } else {
        std::fs::File::open(source).and_then(|f| f.take(limit).read_to_end(&mut bytes))
    };
    result.map_err(|e| {
        let status = ExitStatus::from_io_kind(e.kind());
        CliError::new(status, "input-read", "입력을 읽을 수 없습니다").with_context(source)
    })?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(invalid(
            "input-too-large",
            format!("입력이 {MAX_INPUT_BYTES} bytes를 넘습니다"),
        ));
    }
    Ok(bytes)
}

fn only_keys(value: &JsonValue, allowed: &[&str], at: &str) -> Result<(), CliError> {
    let pairs = value
        .as_object()
        .ok_or_else(|| invalid("invalid-transaction", format!("{at}는 object여야 합니다")))?;
    for (key, _) in pairs {
        if !allowed.contains(&key.as_str()) {
            return Err(invalid(
                "invalid-transaction",
                format!("{at}에 허용되지 않은 key {key:?}가 있습니다"),
            ));
        }
    }
    Ok(())
}

fn field<'a>(value: &'a JsonValue, key: &str, at: &str) -> Result<&'a JsonValue, CliError> {
    value
        .get(key)
        .ok_or_else(|| invalid("invalid-transaction", format!("{at}에 {key}가 없습니다")))
}

fn text_field<'a>(value: &'a JsonValue, key: &str, at: &str) -> Result<&'a str, CliError> {
    field(value, key, at)?.as_str().ok_or_else(|| {
        invalid(
            "invalid-transaction",
            format!("{at}.{key}는 문자열이어야 합니다"),
        )
    })
}

fn id_field(value: &JsonValue, key: &str, at: &str) -> Result<StableId, CliError> {
    text_field(value, key, at)?
        .parse::<StableId>()
        .map_err(|_| {
            invalid(
                "invalid-transaction",
                format!("{at}.{key}는 StableId여야 합니다"),
            )
        })
}

fn bool_field(value: &JsonValue, key: &str, at: &str) -> Result<bool, CliError> {
    field(value, key, at)?.as_bool().ok_or_else(|| {
        invalid(
            "invalid-transaction",
            format!("{at}.{key}는 bool이어야 합니다"),
        )
    })
}

fn hex_field(value: &JsonValue, key: &str, at: &str) -> Result<Vec<u8>, CliError> {
    let text = text_field(value, key, at)?;
    let bad = || {
        invalid(
            "invalid-transaction",
            format!("{at}.{key}는 소문자 16진수여야 합니다"),
        )
    };
    if text.len() % 2 != 0
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(bad());
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| bad()))
        .collect()
}

fn command_input(value: &JsonValue, index: usize) -> Result<CommandInput, CliError> {
    let at = format!("commands[{index}]");
    let kind = text_field(value, "kind", &at)?;
    let input = match kind {
        "set-visible" => {
            only_keys(value, &["kind", "node", "visible"], &at)?;
            CommandInput::SetVisible {
                node: id_field(value, "node", &at)?,
                visible: bool_field(value, "visible", &at)?,
            }
        }
        "set-locked" => {
            only_keys(value, &["kind", "node", "locked"], &at)?;
            CommandInput::SetLocked {
                node: id_field(value, "node", &at)?,
                locked: bool_field(value, "locked", &at)?,
            }
        }
        "rename" => {
            only_keys(value, &["kind", "node", "name"], &at)?;
            CommandInput::Rename {
                node: id_field(value, "node", &at)?,
                name: text_field(value, "name", &at)?.to_string(),
            }
        }
        "delete-subtree" => {
            only_keys(value, &["kind", "root"], &at)?;
            CommandInput::DeleteSubtree {
                root: id_field(value, "root", &at)?,
            }
        }
        "move-node" => {
            only_keys(value, &["kind", "node", "new_parent", "order_key"], &at)?;
            CommandInput::MoveNode {
                node: id_field(value, "node", &at)?,
                new_parent: id_field(value, "new_parent", &at)?,
                order_key: hex_field(value, "order_key", &at)?,
            }
        }
        other => {
            if value.get("canonical").is_none() {
                return Err(CliError::new(
                    ExitStatus::DataErr,
                    "unknown-command",
                    format!("{at}: 알 수 없는 command 종류 {other:?}"),
                ));
            }
            only_keys(value, &["kind", "canonical"], &at)?;
            CommandInput::Canonical {
                kind: other.to_string(),
                bytes: hex_field(value, "canonical", &at)?,
            }
        }
    };
    Ok(input)
}

/// 입력 JSON을 적용 요청으로 해석합니다.
pub fn parse_request(bytes: &[u8]) -> Result<ApplyRequest, CliError> {
    let limits = JsonLimits {
        max_bytes: MAX_INPUT_BYTES,
        max_depth: 16,
    };
    let doc = json::parse_bytes(bytes, limits).map_err(|e| {
        invalid(
            "invalid-json",
            format!("입력 JSON 오류({}) offset {}", e.kind.code(), e.offset),
        )
    })?;
    only_keys(
        &doc,
        &[
            "format",
            "format_version",
            "base_revision",
            "document",
            "commands",
        ],
        "입력",
    )?;
    if text_field(&doc, "format", "입력")? != FORMAT {
        return Err(invalid(
            "invalid-transaction",
            format!("입력.format은 {FORMAT:?}이어야 합니다"),
        ));
    }
    if field(&doc, "format_version", "입력")?.as_i64() != Some(FORMAT_VERSION) {
        return Err(invalid(
            "invalid-transaction",
            format!("입력.format_version은 {FORMAT_VERSION}이어야 합니다"),
        ));
    }
    let base_revision = text_field(&doc, "base_revision", "입력")?
        .parse::<ProjectRevision>()
        .map_err(|e| invalid("invalid-transaction", format!("입력.base_revision: {e}")))?;
    let document = id_field(&doc, "document", "입력")?;
    let items = field(&doc, "commands", "입력")?
        .as_array()
        .ok_or_else(|| invalid("invalid-transaction", "입력.commands는 배열이어야 합니다"))?;
    if items.len() > MAX_COMMANDS {
        return Err(invalid(
            "invalid-transaction",
            format!(
                "command {}개는 상한 {MAX_COMMANDS}개를 넘습니다",
                items.len()
            ),
        ));
    }
    let commands = items
        .iter()
        .enumerate()
        .map(|(i, item)| command_input(item, i))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ApplyRequest {
        base_revision,
        document,
        commands,
    })
}

fn apply_error(error: &ApplyError) -> CliError {
    let message = error.to_string();
    match error {
        ApplyError::Open(e) => views::open_error(e),
        ApplyError::BaseRevisionMismatch { .. } | ApplyError::ConcurrentWrite => {
            CliError::new(ExitStatus::DataErr, "base-revision-mismatch", message)
        }
        ApplyError::EmptyTransaction
        | ApplyError::TooManyCommands { .. }
        | ApplyError::InvalidCommand { .. } => invalid("invalid-transaction", message),
        ApplyError::UnknownCommand { .. } => invalid("unknown-command", message),
        ApplyError::Document(e) => views::document_error(e),
        ApplyError::Rejected { .. } => {
            CliError::new(ExitStatus::Failure, "rejected-command", message)
        }
        ApplyError::InvalidResult(_) => {
            CliError::new(ExitStatus::Failure, "invalid-result", message)
        }
        ApplyError::Projection(_) => CliError::new(ExitStatus::Software, "projection", message),
        ApplyError::Lock(lock) => crate::project_cmds::lock_error(lock),
        ApplyError::Save(_) => CliError::new(ExitStatus::CantCreate, "save", "저장 실패")
            .with_diagnostic(Diagnostic {
                severity: Severity::Error,
                code: "save",
                message,
            }),
    }
}

/// `apply <경로> <입력 파일|-> [--dry-run]`.
pub fn run(args: &[String]) -> Result<Success, CliError> {
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
    let (path, source) = match positional.as_slice() {
        [path, source] => (Path::new(*path), *source),
        _ => {
            return Err(CliError::usage(
                "apply에는 <프로젝트 경로>와 <입력 파일 또는 ->가 필요합니다",
            ))
        }
    };
    let context = path.display().to_string();
    let request = parse_request(&read_input(source)?)?;
    let mode = if dry_run {
        ApplyMode::DryRun
    } else {
        ApplyMode::Write
    };
    // 쓰기 모드만 OS 사용자 데이터의 저장 잠금을 씁니다(XXIV.4.5). dry-run은 잠금이 필요 없습니다.
    let lock = match mode {
        ApplyMode::DryRun => LockPolicy::None,
        ApplyMode::Write => LockPolicy::Required {
            dir: default_lock_dir().map_err(|e| apply_error(&ApplyError::Lock(e)))?,
        },
    };
    let outcome = apply(path, &request, mode, &lock)
        .map_err(|e| apply_error(&e).with_context(context.clone()))?;
    Ok(Success {
        result: JsonValue::object(vec![
            (
                "mode",
                JsonValue::string(match outcome.mode {
                    ApplyMode::DryRun => "dry-run",
                    ApplyMode::Write => "write",
                }),
            ),
            ("document", views::id(outcome.document)),
            ("base_revision", views::revision(outcome.base_revision)),
            ("revision", views::revision(outcome.revision)),
            ("command_count", views::count(outcome.command_count)),
            ("written_chunks", views::count(outcome.written_chunks)),
            ("changes", views::diff_report(&outcome.diff)),
        ]),
        diagnostics: Vec::new(),
    })
}

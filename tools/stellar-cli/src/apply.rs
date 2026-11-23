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
//! | `set-frame-size` | `node`, `width`, `height`(비음수 유한 f32 JSON 숫자) |
//! | `set-frame-width`·`set-frame-height` | `node`, 해당 축의 `width` 또는 `height` |
//! | `set-frame-transform` | `node`, `a`, `b`, `c`, `d`, `tx`, `ty`(유한 f32 JSON 숫자) |
//! | `delete-subtree` | `root` |
//! | `move-node` | `node`, `new_parent`, `order_key`(OrderKey bytes 16진수) |
//! | `insert-component-instance` | `parent`, `component`, `source_hash`(정의 원본 파일 hash), `x`, `y`(유한 f32 JSON 숫자) |
//! | `set-property` | `node`, `property`(registry 이름 `opacity`·`border-top-left-radius` 또는 numeric key `0x0303`), `value`(CSS 값 텍스트) |
//! | 그 밖의 closed enum 종류 이름(`insert-node` 등, VII.1.2) | `canonical`(commands canonical bytes 16진수) |
//!
//! 모든 object는 표에 있는 key만 가질 수 있습니다. 등록되지 않은 `kind`는 `unknown-command`(65)로
//! 거부하고 아무것도 쓰지 않습니다. `--dry-run`은 파일을 쓰지 않고 semantic diff만 출력합니다.
//!
//! `insert-component-instance`는 프로젝트 `design/components`의 정의 `component`를 참조하는 인스턴스를
//! `parent`의 마지막 자식으로 넣고 위치 `(x, y)`를 Transform modifier 하나로 저장합니다. `source_hash`는
//! `stellar query . components`가 보고한 그 정의 원본 파일의 hash이며, 적용할 때 원본이 없거나 hash가
//! 다르면 `component-source-changed`(65)로 거부하고 아무것도 쓰지 않습니다. 다른 command와 같은
//! transaction에 들어가 함께 적용되거나 함께 거부됩니다.
//!
//! `set-property`는 속성 패널·`stellar css paste`와 같은 선언 붙여넣기 경로로 앞 command까지 적용한 문서에서
//! command가 됩니다. 받지 못한 값·속성은 그 이유 코드(`unsupported-value`, `not-applicable` 등)와 함께
//! `invalid-command`(65)이고, 묶음 key(`0x0106` 등)는 `bundle-key-reference`입니다. 값이 이미 같으면 command가
//! 없고 저장하지 않습니다.
//!
//! 잠긴 노드와 잠긴 조상 아래 노드를 바꾸는 요청은 앱과 같은 규칙으로 `rejected-command`이고 아무것도
//! 쓰지 않습니다(D8). 잠금 해제는 그 노드의 `set-locked`로 하며, 같은 요청에서 그 노드의 다른 값은 바꾸지
//! 못합니다.
//!
//! 쓰기는 OS 사용자 데이터의 프로젝트별 저장 잠금을 잡은 뒤에 generation을 비교하고 교체합니다
//! (XXIV.4.5, `stellar_ops::lock`). 잠금이 사용 중이면 `save-lock-busy`(75, 재시도 가능)로 끝나고
//! 아무것도 쓰지 않습니다. OS 사용자 데이터 위치(`HOME`)를 알 수 없으면 `no-user-data-dir`(78)입니다.

use std::io::Read;
use std::path::Path;

use stellar_ops::apply::{
    apply, ApplyError, ApplyMode, ApplyRequest, CommandInput, ComponentInstanceInsert, MAX_COMMANDS,
};
use stellar_ops::core_types::ids::StableId;
use stellar_ops::core_types::value::{FiniteF32, Size2, Transform2D, Vec2};
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
/// command object 하나가 가질 수 있는 JSON 값 수의 최댓값. object 자신 1개와 key·값 쌍 수입니다.
/// 가장 큰 `set-frame-transform`이 `kind`·`node`·`a`·`b`·`c`·`d`·`tx`·`ty` 8쌍이라 9입니다.
/// `command_input`에 key가 더 많은 종류를 넣으면 이 값도 함께 올립니다.
const MAX_COMMAND_JSON_VALUES: usize = 9;
/// 최상위 envelope의 JSON 값 수. object 1개와 `format`·`format_version`·`base_revision`·
/// `document`·`commands` 5쌍입니다.
const ENVELOPE_JSON_VALUES: usize = 6;
/// JSON reader 값 개수 상한. command 상한([`MAX_COMMANDS`]) 안의 입력을 JSON 단계에서 거부하지
/// 않도록 정합니다. 상한보다 하나 많은 입력도 JSON 단계를 지나 command 수 검사에서
/// `TooManyCommands`로 거부되도록 command 하나를 더 셉니다.
const MAX_INPUT_JSON_VALUES: usize =
    (MAX_COMMANDS + 1) * MAX_COMMAND_JSON_VALUES + ENVELOPE_JSON_VALUES;

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

fn f32_field(value: &JsonValue, key: &str, at: &str) -> Result<FiniteF32, CliError> {
    let bad = || {
        invalid(
            "invalid-transaction",
            format!("{at}.{key}는 유한 f32 범위의 JSON 숫자여야 합니다"),
        )
    };
    let JsonValue::Number(number) = field(value, key, at)? else {
        return Err(bad());
    };
    // JSON 원문에서 직접 f32로 변환해 f64를 거치는 이중 반올림을 피합니다.
    let number = number.as_str().parse::<f32>().map_err(|_| bad())?;
    FiniteF32::new(number).map_err(|_| bad())
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
        "replace-text" => {
            only_keys(value, &["kind", "node", "start", "end", "text"], &at)?;
            let index = |key| {
                field(value, key, &at)?
                    .as_i64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| {
                        invalid(
                            "invalid-transaction",
                            format!("{at}.{key}는 음이 아닌 u32 정수여야 합니다"),
                        )
                    })
            };
            CommandInput::ReplaceText(
                stellar_ops::apply::ReplaceText::new(
                    id_field(value, "node", &at)?,
                    index("start")?,
                    index("end")?,
                    text_field(value, "text", &at)?.into(),
                )
                .ok_or_else(|| {
                    invalid(
                        "invalid-transaction",
                        "Text 범위의 시작은 끝 이하여야 합니다",
                    )
                })?,
            )
        }
        "set-frame-size" => {
            only_keys(value, &["kind", "node", "width", "height"], &at)?;
            let size = Size2::new(
                f32_field(value, "width", &at)?.get(),
                f32_field(value, "height", &at)?.get(),
            )
            .map_err(|error| invalid("invalid-transaction", format!("{at}: {error}")))?;
            CommandInput::SetFrameSize {
                node: id_field(value, "node", &at)?,
                size,
            }
        }
        "set-frame-width" | "set-frame-height" => {
            let field = if kind == "set-frame-width" {
                "width"
            } else {
                "height"
            };
            only_keys(value, &["kind", "node", field], &at)?;
            let dimension = f32_field(value, field, &at)?;
            if dimension.get() < 0.0 {
                return Err(invalid(
                    "invalid-transaction",
                    format!("{at}.{field}는 음수일 수 없습니다"),
                ));
            }
            let node = id_field(value, "node", &at)?;
            if field == "width" {
                CommandInput::SetFrameWidth {
                    node,
                    width: dimension,
                }
            } else {
                CommandInput::SetFrameHeight {
                    node,
                    height: dimension,
                }
            }
        }
        "set-frame-transform" => {
            only_keys(
                value,
                &["kind", "node", "a", "b", "c", "d", "tx", "ty"],
                &at,
            )?;
            let transform = Transform2D::new(
                f32_field(value, "a", &at)?.get(),
                f32_field(value, "b", &at)?.get(),
                f32_field(value, "c", &at)?.get(),
                f32_field(value, "d", &at)?.get(),
                f32_field(value, "tx", &at)?.get(),
                f32_field(value, "ty", &at)?.get(),
            )
            .map_err(|error| invalid("invalid-transaction", format!("{at}: {error}")))?;
            CommandInput::SetFrameTransform {
                node: id_field(value, "node", &at)?,
                transform,
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
        "insert-component-instance" => {
            only_keys(
                value,
                &["kind", "parent", "component", "source_hash", "x", "y"],
                &at,
            )?;
            let source_hash = text_field(value, "source_hash", &at)?
                .parse()
                .map_err(|_| {
                    invalid(
                        "invalid-transaction",
                        format!("{at}.source_hash는 sha256:<64자리 소문자 16진수>여야 합니다"),
                    )
                })?;
            let position = Vec2::new(
                f32_field(value, "x", &at)?.get(),
                f32_field(value, "y", &at)?.get(),
            )
            .map_err(|_| {
                invalid(
                    "invalid-transaction",
                    format!("{at}: 위치는 유한해야 합니다"),
                )
            })?;
            CommandInput::InsertComponentInstance(ComponentInstanceInsert {
                component: id_field(value, "component", &at)?,
                source_hash,
                parent: id_field(value, "parent", &at)?,
                position,
            })
        }
        "set-property" => {
            only_keys(value, &["kind", "node", "property", "value"], &at)?;
            CommandInput::SetProperty {
                node: id_field(value, "node", &at)?,
                property: text_field(value, "property", &at)?.to_string(),
                value: text_field(value, "value", &at)?.to_string(),
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
        max_nodes: MAX_INPUT_JSON_VALUES,
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
        return Err(apply_error(&ApplyError::TooManyCommands {
            count: items.len(),
        }));
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

pub(crate) fn apply_error(error: &ApplyError) -> CliError {
    let message = error.to_string();
    match error {
        ApplyError::Open(e) => views::open_error(e),
        ApplyError::BaseRevisionMismatch { .. } => {
            CliError::new(ExitStatus::DataErr, "base-revision-mismatch", message)
        }
        // 계획 뒤 다른 writer(저장 잠금을 쓰지 않는 writer)가 current를 바꿨습니다. base 불일치와
        // 같이 다시 읽고 재시도할 데이터 오류지만 코드를 나눠 잠금 경합과 구분합니다(XXIV.4.5).
        ApplyError::ConcurrentWrite => {
            CliError::new(ExitStatus::DataErr, "concurrent-write", message)
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
        // 원본을 다시 읽고(`query . components`) 새 source_hash로 재시도할 데이터 오류입니다.
        ApplyError::ComponentSourceChanged { .. } => {
            CliError::new(ExitStatus::DataErr, "component-source-changed", message)
        }
        ApplyError::ComponentSource(e) => crate::text_source::operation_error(e),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_write_is_distinguished_from_base_mismatch() {
        // 저장 잠금(XXIV.4.5)을 쓰는 headless writer끼리는 계획 뒤 교체(ConcurrentWrite)가 생기지
        // 않아야 합니다. 동시 쓰기 test가 그 경우를 base 불일치 재시도와 섞지 않도록 오류 코드를
        // 나눕니다. 둘 다 최신 revision으로 다시 읽어야 하는 데이터 오류(65)입니다.
        let concurrent = apply_error(&ApplyError::ConcurrentWrite);
        assert_eq!(concurrent.status, ExitStatus::DataErr);
        assert_eq!(concurrent.code, "concurrent-write");
        let mismatch = apply_error(&ApplyError::BaseRevisionMismatch {
            expected: ProjectRevision::Unsaved,
            found: ProjectRevision::Unsaved,
        });
        assert_eq!(mismatch.status, ExitStatus::DataErr);
        assert_eq!(mismatch.code, "base-revision-mismatch");
    }

    /// 가장 큰 command(`set-frame-transform`)를 `count`개 담은 입력.
    fn transform_request(count: usize) -> Vec<u8> {
        let node = StableId::from_bytes([7; 16]);
        let command = format!(
            r#"{{"kind":"set-frame-transform","node":"{node}","a":1,"b":0,"c":0,"d":1,"tx":2.5,"ty":-3}}"#
        );
        let mut text = format!(
            r#"{{"format":"{FORMAT}","format_version":{FORMAT_VERSION},"base_revision":"g0","document":"{node}","commands":["#
        );
        for i in 0..count {
            if i > 0 {
                text.push(',');
            }
            text.push_str(&command);
        }
        text.push_str("]}");
        text.into_bytes()
    }

    #[test]
    fn command_limit_is_enforced_by_count_not_by_json_value_limit() {
        // 상한 안의 입력은 JSON 값 개수 제한에 걸리지 않고(invalid-json이 아님), 상한을 하나 넘는
        // 입력은 command 수 검사가 거부합니다.
        let at_limit = transform_request(MAX_COMMANDS);
        assert!(at_limit.len() <= MAX_INPUT_BYTES);
        let request = parse_request(&at_limit).expect("command 10,000개는 해석됩니다");
        assert_eq!(request.commands.len(), MAX_COMMANDS);

        let over = transform_request(MAX_COMMANDS + 1);
        assert!(over.len() <= MAX_INPUT_BYTES);
        let error = parse_request(&over).expect_err("command 10,001개는 거부됩니다");
        assert_eq!(error.status, ExitStatus::DataErr);
        assert_eq!(error.code, "invalid-transaction");
        assert_eq!(
            error.message,
            ApplyError::TooManyCommands {
                count: MAX_COMMANDS + 1
            }
            .to_string()
        );
    }

    #[test]
    fn insert_component_instance_input_is_parsed_with_its_source_hash() {
        let id = StableId::from_bytes([7; 16]);
        let hash = format!("sha256:{}", "ab".repeat(32));
        let parse = |command: String| {
            parse_request(
                format!(
                    r#"{{"format":"{FORMAT}","format_version":{FORMAT_VERSION},"base_revision":"g0","document":"{id}","commands":[{command}]}}"#
                )
                .as_bytes(),
            )
        };
        let request = parse(format!(
            r#"{{"kind":"insert-component-instance","parent":"{id}","component":"{id}","source_hash":"{hash}","x":2.5,"y":-3}}"#
        ))
        .unwrap();
        assert_eq!(
            request.commands,
            [CommandInput::InsertComponentInstance(
                ComponentInstanceInsert {
                    component: id,
                    source_hash: hash.parse().unwrap(),
                    parent: id,
                    position: Vec2::new(2.5, -3.).unwrap(),
                }
            )]
        );
        for bad in [
            format!(
                r#"{{"kind":"insert-component-instance","parent":"{id}","component":"{id}","source_hash":"ab","x":0,"y":0}}"#
            ),
            format!(
                r#"{{"kind":"insert-component-instance","parent":"{id}","component":"{id}","source_hash":"{hash}","x":0}}"#
            ),
            format!(
                r#"{{"kind":"insert-component-instance","parent":"{id}","component":"{id}","source_hash":"{hash}","x":0,"y":0,"name":"Card"}}"#
            ),
        ] {
            assert_eq!(parse(bad).unwrap_err().code, "invalid-transaction");
        }
        let changed = apply_error(&ApplyError::ComponentSourceChanged {
            index: 0,
            component: id,
        });
        assert_eq!(changed.status, ExitStatus::DataErr);
        assert_eq!(changed.code, "component-source-changed");
        // set-property는 registry 이름·numeric key와 CSS 값 텍스트를 그대로 받습니다.
        let request = parse(format!(
            r#"{{"kind":"set-property","node":"{id}","property":"0x0303","value":"4px"}}"#
        ))
        .unwrap();
        assert_eq!(
            request.commands,
            [CommandInput::SetProperty {
                node: id,
                property: "0x0303".into(),
                value: "4px".into(),
            }]
        );
        for bad in [
            format!(r#"{{"kind":"set-property","node":"{id}","property":"width"}}"#),
            format!(r#"{{"kind":"set-property","node":"{id}","property":"width","value":1}}"#),
            format!(
                r#"{{"kind":"set-property","node":"{id}","property":"width","value":"1px","key":"0x0108"}}"#
            ),
        ] {
            assert_eq!(parse(bad).unwrap_err().code, "invalid-transaction");
        }
    }
}

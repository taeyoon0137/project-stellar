//! machine-readable 결과 envelope와 stdout/stderr 역할 (XXIV.1.2).
//!
//! ## stdout/stderr 계약
//!
//! - **stdout**: 명령이 실행되면(성공·실패 모두) 결과 envelope JSON 문서 **정확히 한 개**와 개행
//!   하나만 씁니다. 다른 텍스트는 쓰지 않습니다. 예외는 사용자가 요청한 도움말(`help`)뿐입니다.
//! - **stderr**: 사람이 읽는 진단만 씁니다. 형식은 계약이 아니며 경로 같은 지역 정보를 담을 수
//!   있습니다. stdout JSON에는 경로를 넣지 않습니다. 줄 접두어는 실행 이름 `stellar`입니다
//!   (XXIV.4.1, [`PROGRAM`]). manifest 값·key, 경로, 인자 echo처럼 신뢰할 수 없는 값은
//!   [`escape_for_terminal`]로 제어 문자·bidi 서식 문자를 이스케이프해 진단 한 건이 한 줄을 넘지
//!   않고 터미널 escape sequence를 보내지 않습니다.
//! - schema 식별자 `stellar-cli/result`는 실행 이름과 무관한 계약이라 XXIV.4.1에서도 바꾸지 않습니다.
//! - 사용법 오류(64)는 명령이 실행되지 않은 상태이므로 stdout에 아무것도 쓰지 않고 stderr로만
//!   안내합니다.
//!
//! ## envelope 스키마(v1)
//!
//! 모든 key가 항상 이 순서로 존재합니다(`schema/cli-result.v1.schema.json`).
//!
//! | key | 값 |
//! |---|---|
//! | `schema` | `"stellar-cli/result"` |
//! | `schema_version` | `1` |
//! | `command` | 실행한 명령 이름 |
//! | `ok` | 성공이면 `true` |
//! | `exit_code` | 프로세스 종료 코드와 같은 값 |
//! | `exit_name` | [`ExitStatus::name`] |
//! | `result` | 명령별 결과 object, 없으면 `null`. 도메인 실패(1)도 판정 근거 object를 담을 수 있습니다(XXIV.1.3) |
//! | `error` | `{"code", "message"}`, 성공이면 `null` |
//! | `diagnostics` | `{"severity", "code", "message"}` 배열(비어 있을 수 있음) |

use crate::cli::PROGRAM;
use crate::exit::ExitStatus;
use crate::json::JsonValue;

/// envelope 스키마 식별자.
pub const SCHEMA_ID: &str = "stellar-cli/result";
/// envelope 스키마 버전. 호환되지 않는 변경에서만 올립니다.
pub const SCHEMA_VERSION: i64 = 1;
/// envelope key(고정 순서).
pub const ENVELOPE_KEYS: [&str; 9] = [
    "schema",
    "schema_version",
    "command",
    "ok",
    "exit_code",
    "exit_name",
    "result",
    "error",
    "diagnostics",
];

/// 진단 심각도.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// 오류.
    Error,
    /// 경고.
    Warning,
    /// 정보.
    Info,
}

impl Severity {
    /// JSON 표기.
    pub fn name(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

/// 명령 진단 한 건.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// 심각도.
    pub severity: Severity,
    /// 고정 진단 코드(kebab-case).
    pub code: &'static str,
    /// 사람이 읽는 설명(경로 미포함).
    pub message: String,
}

/// 명령 성공 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Success {
    /// 명령별 결과 object.
    pub result: JsonValue,
    /// 경고·정보 진단.
    pub diagnostics: Vec<Diagnostic>,
}

/// 명령 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    /// 종료 상태(0이 아님).
    pub status: ExitStatus,
    /// 고정 오류 코드(kebab-case).
    pub code: &'static str,
    /// 사람이 읽는 설명(경로 미포함, stdout JSON에 들어감).
    pub message: String,
    /// 원인 진단.
    pub diagnostics: Vec<Diagnostic>,
    /// stderr 전용 부가 정보(경로 등). stdout JSON에는 넣지 않습니다.
    pub context: Option<String>,
    /// 도메인 실패(1)에서도 판정 근거를 담는 결과 object(XXIV.1.3). 없으면 envelope `result`는 null.
    pub result: Option<Box<JsonValue>>,
}

impl CliError {
    /// 진단 없는 오류를 만듭니다.
    pub fn new(status: ExitStatus, code: &'static str, message: impl Into<String>) -> CliError {
        CliError {
            status,
            code,
            message: message.into(),
            diagnostics: Vec::new(),
            context: None,
            result: None,
        }
    }

    /// 사용법 오류(64)를 만듭니다.
    pub fn usage(message: impl Into<String>) -> CliError {
        CliError::new(ExitStatus::Usage, "usage", message)
    }

    /// 원인 진단을 덧붙입니다.
    pub fn with_diagnostic(mut self, diagnostic: Diagnostic) -> CliError {
        self.diagnostics.push(diagnostic);
        self
    }

    /// stderr 전용 부가 정보를 덧붙입니다.
    pub fn with_context(mut self, context: impl Into<String>) -> CliError {
        self.context = Some(context.into());
        self
    }

    /// 실패 envelope의 `result`에 결과 object를 담습니다(경로 미포함이어야 함).
    pub fn with_result(mut self, result: JsonValue) -> CliError {
        self.result = Some(Box::new(result));
        self
    }
}

fn diagnostics_json(diagnostics: &[Diagnostic]) -> JsonValue {
    JsonValue::Array(
        diagnostics
            .iter()
            .map(|d| {
                JsonValue::object(vec![
                    ("severity", JsonValue::string(d.severity.name())),
                    ("code", JsonValue::string(d.code)),
                    ("message", JsonValue::string(d.message.clone())),
                ])
            })
            .collect(),
    )
}

fn envelope(
    command: &str,
    status: ExitStatus,
    result: JsonValue,
    error: JsonValue,
    diagnostics: &[Diagnostic],
) -> JsonValue {
    JsonValue::object(vec![
        ("schema", JsonValue::string(SCHEMA_ID)),
        ("schema_version", JsonValue::int(SCHEMA_VERSION)),
        ("command", JsonValue::string(command)),
        ("ok", JsonValue::Bool(status == ExitStatus::Ok)),
        ("exit_code", JsonValue::int(i64::from(status.code()))),
        ("exit_name", JsonValue::string(status.name())),
        ("result", result),
        ("error", error),
        ("diagnostics", diagnostics_json(diagnostics)),
    ])
}

/// 성공 envelope.
pub fn success_envelope(command: &str, success: &Success) -> JsonValue {
    envelope(
        command,
        ExitStatus::Ok,
        success.result.clone(),
        JsonValue::Null,
        &success.diagnostics,
    )
}

/// 실패 envelope.
pub fn error_envelope(command: &str, error: &CliError) -> JsonValue {
    envelope(
        command,
        error.status,
        error.result.as_deref().cloned().unwrap_or(JsonValue::Null),
        JsonValue::object(vec![
            ("code", JsonValue::string(error.code)),
            ("message", JsonValue::string(error.message.clone())),
        ]),
        &error.diagnostics,
    )
}

/// 제어 문자는 아니지만 터미널 줄을 시각적으로 위조할 수 있는 문자인지 확인합니다: Unicode bidi
/// 서식 문자(U+061C, U+200E, U+200F, U+202A\~U+202E, U+2066\~U+2069)와 줄·문단 구분자(U+2028,
/// U+2029).
fn is_bidi_or_line_separator(c: char) -> bool {
    matches!(
        c,
        '\u{61c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

/// stderr에 쓸 텍스트에서 터미널 제어·bidi 문자를 `\u{..}`·`\n` 같은 표기로 바꿉니다.
///
/// manifest 값·key, 경로, 인자처럼 신뢰할 수 없는 값이 터미널 escape sequence(창 제목 변경, 화면
/// 지우기)나 개행으로 진단 줄을 위조하지 못하게 합니다. 제어 문자(`char::is_control`: ESC·BEL·
/// CR·LF·DEL 등)는 `char::escape_default`로, bidi 서식 문자와 줄·문단 구분자는 `\u{..}`로 씁니다.
/// 규칙은 stellar-mcp 로그의 이스케이프와 같습니다.
pub fn escape_for_terminal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() {
            out.extend(c.escape_default());
        } else if is_bidi_or_line_separator(c) {
            out.push_str(&format!("\\u{{{:x}}}", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out
}

/// stderr에 쓸 사람용 진단 줄. message·context·진단 message는 [`escape_for_terminal`]을 거칩니다.
pub fn stderr_text(command: &str, error: Option<&CliError>, diagnostics: &[Diagnostic]) -> String {
    let mut out = String::new();
    if let Some(e) = error {
        out.push_str(&format!(
            "{PROGRAM} {command}: 오류({}): {}",
            e.code,
            escape_for_terminal(&e.message)
        ));
        if let Some(context) = &e.context {
            out.push_str(&format!(" [{}]", escape_for_terminal(context)));
        }
        out.push('\n');
    }
    for d in diagnostics {
        out.push_str(&format!(
            "{PROGRAM} {command}: {}({}): {}\n",
            d.severity.name(),
            d.code,
            escape_for_terminal(&d.message)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::write;

    #[test]
    fn envelopes_have_all_keys_in_fixed_order() {
        let ok = success_envelope(
            "version",
            &Success {
                result: JsonValue::object(vec![]),
                diagnostics: Vec::new(),
            },
        );
        let err = error_envelope(
            "inspect",
            &CliError::new(ExitStatus::NoInput, "no-input", "없음"),
        );
        for value in [&ok, &err] {
            let keys: Vec<&str> = value
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, _)| k.as_str())
                .collect();
            assert_eq!(keys, ENVELOPE_KEYS);
        }
        assert_eq!(ok.get("ok"), Some(&JsonValue::Bool(true)));
        assert_eq!(err.get("exit_code").and_then(JsonValue::as_i64), Some(66));
        assert_eq!(
            write(&err),
            r#"{"schema":"stellar-cli/result","schema_version":1,"command":"inspect","ok":false,"exit_code":66,"exit_name":"noinput","result":null,"error":{"code":"no-input","message":"없음"},"diagnostics":[]}"#
        );
    }

    #[test]
    fn context_goes_to_stderr_only() {
        let error = CliError::new(ExitStatus::NoInput, "no-input", "입력 없음")
            .with_context("/private/path")
            .with_diagnostic(Diagnostic {
                severity: Severity::Error,
                code: "missing",
                message: "자세히".into(),
            });
        assert!(!write(&error_envelope("inspect", &error)).contains("/private/path"));
        let text = stderr_text("inspect", Some(&error), &error.diagnostics);
        assert!(text.contains("/private/path"));
        assert!(text.contains("error(missing): 자세히"));
    }

    /// 신뢰할 수 없는 저장소 값(manifest 값·key, 디렉터리 이름)이 stderr에 그대로 나가지 않습니다.
    const HOSTILE: &str =
        "\u{1b}]0;PWNED\u{7}\u{1b}[2J\r\nstellar inspect: 오류(fake): 위조\u{202e}lmth\u{2066}";

    #[test]
    fn stderr_text_escapes_terminal_control_and_bidi_characters() {
        let error = CliError::new(ExitStatus::DataErr, "manifest-schema", HOSTILE)
            .with_context(format!("/tmp/{HOSTILE}"))
            .with_diagnostic(Diagnostic {
                severity: Severity::Error,
                code: "schema",
                message: HOSTILE.into(),
            });
        let text = stderr_text("inspect", Some(&error), &error.diagnostics);
        assert_eq!(
            text.matches('\n').count(),
            2,
            "오류 한 줄 + 진단 한 줄: {text:?}"
        );
        for forbidden in ['\u{1b}', '\u{7}', '\r', '\u{202e}', '\u{2066}'] {
            assert!(!text.contains(forbidden), "{forbidden:?}: {text:?}");
        }
        assert!(
            text.contains("\\u{1b}]0;PWNED\\u{7}\\u{1b}[2J\\r\\n"),
            "{text}"
        );
        assert!(text.contains("\\u{202e}lmth\\u{2066}"), "{text}");
        for line in text.lines() {
            assert!(line.starts_with("stellar inspect: "), "{line}");
        }
    }

    #[test]
    fn terminal_escape_keeps_printable_text() {
        assert_eq!(escape_for_terminal("경로 /a b/c.yaml"), "경로 /a b/c.yaml");
        assert_eq!(
            escape_for_terminal("\u{200e}\u{200f}\u{61c}\u{2028}\u{2029}\u{7f}"),
            "\\u{200e}\\u{200f}\\u{61c}\\u{2028}\\u{2029}\\u{7f}"
        );
    }
}

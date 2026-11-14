//! machine-readable 결과 envelope와 stdout/stderr 역할 (XXIV.1.2).
//!
//! ## stdout/stderr 계약
//!
//! - **stdout**: 명령이 실행되면(성공·실패 모두) 결과 envelope JSON 문서 **정확히 한 개**와 개행
//!   하나만 씁니다. 다른 텍스트는 쓰지 않습니다. 예외는 사용자가 요청한 도움말(`help`)뿐입니다.
//! - **stderr**: 사람이 읽는 진단만 씁니다. 형식은 계약이 아니며 경로 같은 지역 정보를 담을 수
//!   있습니다. stdout JSON에는 경로를 넣지 않습니다. 줄 접두어는 실행 이름 `stellar`입니다
//!   (XXIV.4.1, [`PROGRAM`]).
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

/// stderr에 쓸 사람용 진단 줄.
pub fn stderr_text(command: &str, error: Option<&CliError>, diagnostics: &[Diagnostic]) -> String {
    let mut out = String::new();
    if let Some(e) = error {
        out.push_str(&format!(
            "{PROGRAM} {command}: 오류({}): {}",
            e.code, e.message
        ));
        if let Some(context) = &e.context {
            out.push_str(&format!(" [{context}]"));
        }
        out.push('\n');
    }
    for d in diagnostics {
        out.push_str(&format!(
            "{PROGRAM} {command}: {}({}): {}\n",
            d.severity.name(),
            d.code,
            d.message
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
}

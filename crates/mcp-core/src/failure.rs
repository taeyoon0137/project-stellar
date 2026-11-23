//! 요청 실패: 결과 종류·종료 코드·고정 문구 (XIX.9.9, ADR-0085 결정 3·7).
//!
//! 실패는 [`ResultKind`]와 Stellar 고정 문구로만 만듭니다. 문구에 프로젝트 문자열을 넣지 않습니다(XIX.9.6).
//!
//! - tool 실행 실패(인자 오류 포함)는 MCP 명세 2025-11-25 "Tools" Error Handling대로 `isError: true` 결과입니다
//!   ([`crate::result::tool_failure`]).
//! - 모르는 tool과 resource·목록 실패는 JSON-RPC `error`이고 `data`에 같은 `kind`·`exit`를 둡니다.

pub use stellar_agent_protocol::outcome::{ExitCode, ResultKind};

use crate::json::{JsonNumber, JsonValue};
use crate::jsonrpc::{codes, RpcError};

/// 서버 정의 실패 code(JSON-RPC 서버 범위 -32000~-32099).
pub const SERVER_FAILURE: i64 = -32000;

/// 종료 코드 JSON `{"code", "name"}`.
pub fn exit_json(exit: ExitCode) -> JsonValue {
    JsonValue::object(vec![
        (
            "code",
            JsonValue::Number(JsonNumber::from_u64(u64::from(exit.code))),
        ),
        ("name", JsonValue::string(exit.name)),
    ])
}

/// 종류와 고정 문구로 실패를 만듭니다.
pub fn fail(kind: ResultKind, message: &'static str) -> RpcError {
    fail_text(kind, message)
}

/// [`fail`]과 같되 문구가 Stellar 고정 형식의 `Display`(page·scope 오류처럼 프로젝트 문자열이 없는 값)일 때 씁니다.
pub(crate) fn fail_text(kind: ResultKind, message: impl Into<String>) -> RpcError {
    let code = match kind {
        ResultKind::InvalidArguments
        | ResultKind::Stale
        | ResultKind::UnknownTool
        | ResultKind::OutOfScope
        | ResultKind::TooLarge => codes::INVALID_PARAMS,
        _ => SERVER_FAILURE,
    };
    RpcError::new(code, message).with_data(JsonValue::object(vec![
        ("kind", JsonValue::string(kind.name())),
        ("exit", exit_json(kind.exit())),
    ]))
}

/// 실패의 결과 종류. 종류 없는 protocol 오류(params 형식 등)는 `None`입니다.
pub fn kind_of(error: &RpcError) -> Option<ResultKind> {
    error
        .data
        .as_ref()
        .and_then(|d| d.get("kind"))
        .and_then(JsonValue::as_str)
        .and_then(ResultKind::from_name)
}

/// 실패 data에 보조 field를 더합니다.
pub trait WithDetail {
    /// `data`에 `key`를 더합니다.
    fn with_detail(self, key: &str, value: JsonValue) -> Self;
}

impl WithDetail for RpcError {
    fn with_detail(mut self, key: &str, value: JsonValue) -> Self {
        match &mut self.data {
            Some(JsonValue::Object(fields)) => fields.push((key.to_string(), value)),
            _ => self.data = Some(JsonValue::object(vec![(key, value)])),
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_carries_kind_and_exit() {
        let error = fail(ResultKind::Stale, "고정 문구").with_detail("index", JsonValue::int(2));
        assert_eq!(kind_of(&error), Some(ResultKind::Stale));
        let data = error.data.unwrap();
        assert_eq!(
            data.get("exit")
                .and_then(|e| e.get("code"))
                .and_then(JsonValue::as_i64),
            Some(65)
        );
        assert_eq!(data.get("index"), Some(&JsonValue::int(2)));
        assert_eq!(kind_of(&RpcError::new(-32602, "x")), None);
    }
}

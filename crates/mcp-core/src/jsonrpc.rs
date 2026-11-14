//! JSON-RPC 2.0 메시지 모델 (XIX.1.1).
//!
//! MCP가 사용하는 JSON-RPC 2.0 규칙으로 들어온 메시지를 분류합니다.
//!
//! - 최상위는 object 하나입니다. batch(array)는 지원하지 않습니다.
//! - `"jsonrpc"`는 정확히 `"2.0"`입니다.
//! - request `id`는 문자열 또는 정수입니다. `null`·소수·bool·object는 거부합니다.
//! - `params`가 있으면 object 또는 array입니다.
//! - `method`가 있고 `id`가 있으면 request, `id`가 없으면 notification입니다.
//! - `method` 없이 `result`·`error`가 있으면 client가 보낸 response입니다.

use crate::json::{JsonNumber, JsonValue};

/// JSON-RPC protocol 버전 문자열.
pub const JSONRPC_VERSION: &str = "2.0";

/// JSON-RPC 2.0 표준 오류 코드와 서버 정의 코드.
pub mod codes {
    /// 잘못된 JSON.
    pub const PARSE_ERROR: i64 = -32700;
    /// JSON-RPC request 형식 위반.
    pub const INVALID_REQUEST: i64 = -32600;
    /// 알 수 없는 method.
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// method 인자 오류.
    pub const INVALID_PARAMS: i64 = -32602;
    /// 서버 내부 오류.
    pub const INTERNAL_ERROR: i64 = -32603;
    /// initialize 전 요청(서버 정의 범위 -32000~-32099).
    pub const SERVER_NOT_INITIALIZED: i64 = -32002;
}

/// request id. 원문 표기를 그대로 되돌려 보냅니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestId {
    /// 정수 id(원문 표기 보존).
    Number(JsonNumber),
    /// 문자열 id.
    String(String),
}

impl RequestId {
    /// JSON 값으로 변환합니다.
    pub fn to_json(&self) -> JsonValue {
        match self {
            RequestId::Number(n) => JsonValue::Number(n.clone()),
            RequestId::String(s) => JsonValue::string(s.clone()),
        }
    }
}

/// JSON-RPC 오류 object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    /// 오류 코드([`codes`]).
    pub code: i64,
    /// 사람이 읽는 설명.
    pub message: String,
    /// 부가 데이터.
    pub data: Option<JsonValue>,
}

impl RpcError {
    /// 부가 데이터 없는 오류를 만듭니다.
    pub fn new(code: i64, message: impl Into<String>) -> RpcError {
        RpcError {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// 부가 데이터를 붙입니다.
    pub fn with_data(mut self, data: JsonValue) -> RpcError {
        self.data = Some(data);
        self
    }

    fn to_json(&self) -> JsonValue {
        let mut pairs = vec![
            ("code", JsonValue::int(self.code)),
            ("message", JsonValue::string(self.message.clone())),
        ];
        if let Some(data) = &self.data {
            pairs.push(("data", data.clone()));
        }
        JsonValue::object(pairs)
    }
}

/// 분류한 수신 메시지.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// 응답이 필요한 request.
    Request {
        /// request id.
        id: RequestId,
        /// method 이름.
        method: String,
        /// 인자(object 또는 array).
        params: Option<JsonValue>,
    },
    /// 응답하지 않는 notification.
    Notification {
        /// method 이름.
        method: String,
        /// 인자(object 또는 array).
        params: Option<JsonValue>,
    },
    /// client가 보낸 response(서버가 보낸 request에 대한 응답).
    Response {
        /// 응답 id(`null`이면 없음).
        id: Option<RequestId>,
    },
}

/// 형식 위반. `id`를 알 수 있으면 함께 담습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    /// 위반 메시지의 id(알 수 없으면 `None` → 응답 id `null`).
    pub id: Option<RequestId>,
    /// 응답할 오류.
    pub error: RpcError,
}

fn invalid(id: Option<RequestId>, message: &str) -> Invalid {
    Invalid {
        id,
        error: RpcError::new(codes::INVALID_REQUEST, message),
    }
}

/// 파싱한 JSON 값을 JSON-RPC 메시지로 분류합니다.
///
/// 값의 소유권을 받아 `id`·`method`·`params`를 복제하지 않고 옮깁니다. 큰 `params`도 메모리에 한
/// 벌만 있습니다(IPC-05). reader가 중복 key를 거부하므로 각 member는 한 번만 나타납니다.
pub fn classify(value: JsonValue) -> Result<Incoming, Invalid> {
    let pairs = match value {
        JsonValue::Object(pairs) => pairs,
        JsonValue::Array(_) => return Err(invalid(None, "batch 요청은 지원하지 않습니다")),
        _ => return Err(invalid(None, "JSON-RPC 메시지는 object여야 합니다")),
    };
    let (mut id, mut jsonrpc, mut params, mut method) = (None, None, None, None);
    let mut has_result_or_error = false;
    for (key, item) in pairs {
        match key.as_str() {
            "id" => id = Some(item),
            "jsonrpc" => jsonrpc = Some(item),
            "params" => params = Some(item),
            "method" => method = Some(item),
            "result" | "error" => has_result_or_error = true,
            _ => {}
        }
    }
    let id = match id {
        None => None,
        Some(JsonValue::String(s)) => Some(RequestId::String(s)),
        Some(JsonValue::Number(n)) if n.is_integer() => Some(RequestId::Number(n)),
        Some(JsonValue::Null) => return Err(invalid(None, "id는 null일 수 없습니다")),
        Some(_) => return Err(invalid(None, "id는 문자열 또는 정수여야 합니다")),
    };
    if jsonrpc.as_ref().and_then(JsonValue::as_str) != Some(JSONRPC_VERSION) {
        return Err(invalid(id, "jsonrpc는 \"2.0\"이어야 합니다"));
    }
    let params = match params {
        None => None,
        Some(p @ (JsonValue::Object(_) | JsonValue::Array(_))) => Some(p),
        Some(_) => return Err(invalid(id, "params는 object 또는 array여야 합니다")),
    };
    match method {
        Some(JsonValue::String(method)) => Ok(match id {
            Some(id) => Incoming::Request { id, method, params },
            None => Incoming::Notification { method, params },
        }),
        Some(_) => Err(invalid(id, "method는 문자열이어야 합니다")),
        None if has_result_or_error => Ok(Incoming::Response { id }),
        None => Err(invalid(id, "method가 없습니다")),
    }
}

/// 성공 response.
pub fn response_ok(id: &RequestId, result: JsonValue) -> JsonValue {
    JsonValue::object(vec![
        ("jsonrpc", JsonValue::string(JSONRPC_VERSION)),
        ("id", id.to_json()),
        ("result", result),
    ])
}

/// 오류 response. id를 알 수 없으면 `null`입니다.
pub fn response_err(id: Option<&RequestId>, error: &RpcError) -> JsonValue {
    JsonValue::object(vec![
        ("jsonrpc", JsonValue::string(JSONRPC_VERSION)),
        ("id", id.map_or(JsonValue::Null, RequestId::to_json)),
        ("error", error.to_json()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{parse, write, JsonLimits};

    fn classify_text(text: &str) -> Result<Incoming, Invalid> {
        classify(parse(text, JsonLimits::default()).unwrap())
    }

    #[test]
    fn requests_notifications_and_responses() {
        match classify_text(r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#).unwrap() {
            Incoming::Request { id, method, params } => {
                assert_eq!(write(&id.to_json()), "7");
                assert_eq!(method, "ping");
                assert_eq!(params, None);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            classify_text(r#"{"jsonrpc":"2.0","id":"a","method":"x","params":[1]}"#),
            Ok(Incoming::Request {
                id: RequestId::String(_),
                ..
            })
        ));
        assert!(matches!(
            classify_text(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
            Ok(Incoming::Notification { .. })
        ));
        assert_eq!(
            classify_text(r#"{"jsonrpc":"2.0","id":3,"result":{}}"#),
            Ok(Incoming::Response {
                id: Some(RequestId::Number(JsonNumber::from_i64(3)))
            })
        );
        // 큰 정수 id도 원문 그대로 되돌립니다.
        let big = classify_text(r#"{"jsonrpc":"2.0","id":123456789012345678901,"method":"x"}"#);
        let Ok(Incoming::Request { id, .. }) = big else {
            panic!("{big:?}")
        };
        assert_eq!(write(&id.to_json()), "123456789012345678901");
    }

    #[test]
    fn invalid_messages_are_rejected_with_known_id() {
        let cases: &[(&str, Option<&str>)] = &[
            ("[]", None),
            (r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#, None),
            ("1", None),
            (r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#, None),
            (r#"{"jsonrpc":"2.0","id":1.5,"method":"ping"}"#, None),
            (r#"{"jsonrpc":"2.0","id":true,"method":"ping"}"#, None),
            (r#"{"jsonrpc":"1.0","id":5,"method":"ping"}"#, Some("5")),
            (r#"{"id":5,"method":"ping"}"#, Some("5")),
            (r#"{"jsonrpc":"2.0","id":"q","method":3}"#, Some("\"q\"")),
            (
                r#"{"jsonrpc":"2.0","id":8,"method":"ping","params":"x"}"#,
                Some("8"),
            ),
            (r#"{"jsonrpc":"2.0","id":9}"#, Some("9")),
        ];
        for (text, id) in cases {
            let err = classify_text(text).unwrap_err();
            assert_eq!(err.error.code, codes::INVALID_REQUEST, "{text}");
            assert_eq!(
                err.id.map(|i| write(&i.to_json())).as_deref(),
                *id,
                "{text}"
            );
        }
    }

    #[test]
    fn responses_are_well_formed() {
        let id = RequestId::String("x".into());
        assert_eq!(
            write(&response_ok(&id, JsonValue::object(vec![]))),
            r#"{"jsonrpc":"2.0","id":"x","result":{}}"#
        );
        let error = RpcError::new(codes::METHOD_NOT_FOUND, "없음")
            .with_data(JsonValue::object(vec![("method", JsonValue::string("m"))]));
        assert_eq!(
            write(&response_err(None, &error)),
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32601,"message":"없음","data":{"method":"m"}}}"#
        );
    }
}

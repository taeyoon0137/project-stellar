//! MCP stdio 서버 루프 (XIX.1.1).
//!
//! stdin에서 frame을 읽어 JSON-RPC로 분류하고 응답을 stdout에 씁니다. stdout에는 응답 frame만
//! 쓰고, 진행 상황·오류 로그는 [`Logger`](stderr)로만 씁니다.
//!
//! 수명주기:
//!
//! 1. `AwaitingInitialize`: `initialize` request와 `ping`만 처리합니다. 다른 request는
//!    [`codes::SERVER_NOT_INITIALIZED`]로 거부합니다.
//! 2. `Initialized`: `initialize` 응답 후. client의 `notifications/initialized`를 기다립니다.
//! 3. `Ready`: 정상 운용. 현재는 `ping`만 제공하며 그 밖의 method는
//!    [`codes::METHOD_NOT_FOUND`]입니다(resources·tools는 XIX.2).
//!
//! notification에는 응답하지 않습니다. client가 보낸 response는 서버가 request를 보내지 않으므로
//! 로그만 남기고 무시합니다.
//!
//! `initialize`의 `protocolVersion`은 `config/mcp-protocol.yaml`에 고정한 목록으로 협상합니다
//! (XIX.1.5, [`crate::protocol`]). 지원하지 않는 version은 `-32602`로 거부하고 initialize 전
//! 상태를 유지합니다.

use std::io::{self, BufRead, Write};

use crate::framing::{read_frame, write_frame, Frame};
use crate::json::{self, JsonLimits, JsonValue};
use crate::jsonrpc::{classify, codes, response_err, response_ok, Incoming, RpcError};
use crate::log::Logger;
use crate::protocol::{ProtocolConfig, ProtocolConfigError, PREFERRED_VERSION};

/// 서버 이름(`serverInfo.name`).
pub const SERVER_NAME: &str = "stellar-mcp";
/// 선호 MCP protocol version. 코드에 값을 두지 않고 빌드 시 `config/mcp-protocol.yaml`의 첫
/// 항목에서 가져옵니다(XIX.1.5).
pub const PROTOCOL_VERSION: &str = PREFERRED_VERSION;
/// 수신 메시지 최대 byte 수.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
/// 수신 JSON 최대 중첩 깊이.
pub const MAX_MESSAGE_DEPTH: usize = 64;

/// 서버 수명주기 단계.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// `initialize` 대기.
    AwaitingInitialize,
    /// `initialize` 응답 후, `notifications/initialized` 대기.
    Initialized,
    /// 정상 운용.
    Ready,
}

/// 한 stdio 연결의 서버 상태.
#[derive(Debug)]
pub struct Server {
    phase: Phase,
    protocol: Result<ProtocolConfig, ProtocolConfigError>,
}

impl Default for Server {
    fn default() -> Self {
        Server::new()
    }
}

impl Server {
    /// 내장 protocol config를 쓰는 `initialize` 대기 상태의 서버.
    ///
    /// 내장 config가 잘못되면 initialize가 `-32603`으로 실패합니다. 실행 파일은 시작 시 config를
    /// 먼저 검사해 이 경우 종료 코드 78로 끝납니다.
    pub fn new() -> Server {
        Server {
            phase: Phase::AwaitingInitialize,
            protocol: ProtocolConfig::builtin(),
        }
    }

    /// 주어진 protocol config를 쓰는 서버.
    pub fn with_config(config: ProtocolConfig) -> Server {
        Server {
            phase: Phase::AwaitingInitialize,
            protocol: Ok(config),
        }
    }

    /// 현재 단계.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// frame 하나를 처리하고, 보낼 응답이 있으면 반환합니다.
    pub fn handle_frame<L: Write>(
        &mut self,
        bytes: &[u8],
        log: &mut Logger<L>,
    ) -> Option<JsonValue> {
        let limits = JsonLimits {
            max_bytes: MAX_MESSAGE_BYTES,
            max_depth: MAX_MESSAGE_DEPTH,
        };
        let value = match json::parse_bytes(bytes, limits) {
            Ok(value) => value,
            Err(e) => {
                log.warn(&format!("JSON 파싱 실패: {e}"));
                let error = RpcError::new(
                    codes::PARSE_ERROR,
                    format!("JSON 파싱 실패({})", e.kind.code()),
                )
                .with_data(JsonValue::object(vec![(
                    "offset",
                    JsonValue::int(i64::try_from(e.offset).unwrap_or(i64::MAX)),
                )]));
                return Some(response_err(None, &error));
            }
        };
        match classify(&value) {
            Err(invalid) => {
                log.warn(&format!("잘못된 request: {}", invalid.error.message));
                Some(response_err(invalid.id.as_ref(), &invalid.error))
            }
            Ok(Incoming::Response { .. }) => {
                log.info("client response는 처리하지 않습니다(무시)");
                None
            }
            Ok(Incoming::Notification { method, .. }) => {
                self.notification(&method, log);
                None
            }
            Ok(Incoming::Request { id, method, params }) => {
                Some(match self.request(&method, params.as_ref(), log) {
                    Ok(result) => response_ok(&id, result),
                    Err(error) => {
                        log.warn(&format!("request 거부({}): {method}", error.code));
                        response_err(Some(&id), &error)
                    }
                })
            }
        }
    }

    fn request<L: Write>(
        &mut self,
        method: &str,
        params: Option<&JsonValue>,
        log: &mut Logger<L>,
    ) -> Result<JsonValue, RpcError> {
        match method {
            "ping" => Ok(JsonValue::Object(Vec::new())),
            "initialize" => self.initialize(params, log),
            _ if self.phase == Phase::AwaitingInitialize => Err(RpcError::new(
                codes::SERVER_NOT_INITIALIZED,
                "initialize 전에는 ping만 허용합니다",
            )),
            _ => Err(
                RpcError::new(codes::METHOD_NOT_FOUND, "알 수 없는 method입니다").with_data(
                    JsonValue::object(vec![("method", JsonValue::string(method))]),
                ),
            ),
        }
    }

    fn initialize<L: Write>(
        &mut self,
        params: Option<&JsonValue>,
        log: &mut Logger<L>,
    ) -> Result<JsonValue, RpcError> {
        if self.phase != Phase::AwaitingInitialize {
            return Err(RpcError::new(
                codes::INVALID_REQUEST,
                "이미 initialize되었습니다",
            ));
        }
        let Some(params) = params.filter(|p| p.as_object().is_some()) else {
            return Err(RpcError::new(
                codes::INVALID_PARAMS,
                "initialize params는 object여야 합니다",
            ));
        };
        let Some(requested) = params.get("protocolVersion").and_then(JsonValue::as_str) else {
            return Err(RpcError::new(
                codes::INVALID_PARAMS,
                "initialize params에 protocolVersion 문자열이 필요합니다",
            ));
        };
        let config = self.protocol.as_ref().map_err(|e| {
            RpcError::new(
                codes::INTERNAL_ERROR,
                format!("protocol version config 오류: {e}"),
            )
        })?;
        let version = match config.negotiate(requested) {
            Ok(version) => version.to_string(),
            Err(error) => {
                log.warn("지원하지 않는 protocol version 요청을 거부합니다");
                return Err(error);
            }
        };
        log.info(&format!("initialize 수락(protocol version {version})"));
        self.phase = Phase::Initialized;
        Ok(JsonValue::object(vec![
            ("protocolVersion", JsonValue::string(version)),
            ("capabilities", JsonValue::Object(Vec::new())),
            (
                "serverInfo",
                JsonValue::object(vec![
                    ("name", JsonValue::string(SERVER_NAME)),
                    ("version", JsonValue::string(env!("CARGO_PKG_VERSION"))),
                ]),
            ),
        ]))
    }

    fn notification<L: Write>(&mut self, method: &str, log: &mut Logger<L>) {
        match (method, self.phase) {
            ("notifications/initialized", Phase::Initialized) => {
                self.phase = Phase::Ready;
                log.info("client 초기화 완료");
            }
            ("notifications/initialized", _) => {
                log.warn("initialize 응답 전의 initialized notification을 무시합니다");
            }
            _ => log.info(&format!("notification 무시: {method}")),
        }
    }
}

/// 입력 끝까지 frame을 처리합니다. 입출력 오류만 `Err`로 반환합니다.
pub fn serve<R: BufRead, W: Write, L: Write>(
    input: &mut R,
    output: &mut W,
    log: &mut Logger<L>,
) -> io::Result<()> {
    let mut server = Server::new();
    log.info("stdio JSON-RPC 대기");
    loop {
        match read_frame(input, MAX_MESSAGE_BYTES)? {
            Frame::Eof => {
                log.info("입력 종료: 서버를 끝냅니다");
                return Ok(());
            }
            Frame::TooLarge { discarded } => {
                log.warn(&format!(
                    "최대 크기를 넘는 메시지를 버렸습니다({discarded} bytes)"
                ));
                let error = RpcError::new(
                    codes::INVALID_REQUEST,
                    format!("메시지가 최대 크기({MAX_MESSAGE_BYTES} bytes)를 넘습니다"),
                );
                write_frame(output, &response_err(None, &error))?;
            }
            Frame::Message(bytes) => {
                if bytes.iter().all(|b| matches!(b, b' ' | b'\t' | b'\r')) {
                    continue;
                }
                if let Some(response) = server.handle_frame(&bytes, log) {
                    write_frame(output, &response)?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{parse, write};

    fn handle(server: &mut Server, text: &str) -> Option<String> {
        let mut log = Logger::new(Vec::new());
        server
            .handle_frame(text.as_bytes(), &mut log)
            .map(|v| write(&v))
    }

    const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#;

    #[test]
    fn lifecycle_reaches_ready() {
        let mut server = Server::new();
        assert_eq!(server.phase(), Phase::AwaitingInitialize);
        let reply = handle(&mut server, INIT).unwrap();
        let doc = parse(&reply, JsonLimits::default()).unwrap();
        let result = doc.get("result").unwrap();
        assert_eq!(
            result.get("protocolVersion").and_then(JsonValue::as_str),
            Some(PROTOCOL_VERSION)
        );
        assert_eq!(
            result
                .get("serverInfo")
                .and_then(|s| s.get("name"))
                .and_then(JsonValue::as_str),
            Some(SERVER_NAME)
        );
        assert_eq!(server.phase(), Phase::Initialized);
        assert_eq!(
            handle(
                &mut server,
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
            ),
            None
        );
        assert_eq!(server.phase(), Phase::Ready);
    }

    #[test]
    fn requests_before_initialize_are_rejected_except_ping() {
        let mut server = Server::new();
        assert_eq!(
            handle(&mut server, r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).unwrap(),
            r#"{"jsonrpc":"2.0","id":1,"result":{}}"#
        );
        let reply = handle(
            &mut server,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        );
        assert!(reply.unwrap().contains("-32002"));
        // initialized notification이 먼저 와도 단계는 바뀌지 않습니다.
        handle(
            &mut server,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        );
        assert_eq!(server.phase(), Phase::AwaitingInitialize);
    }

    #[test]
    fn initialize_params_are_validated() {
        for text in [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":[]}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":7}}"#,
        ] {
            let mut server = Server::new();
            let reply = handle(&mut server, text).unwrap();
            assert!(reply.contains("-32602"), "{text}: {reply}");
            assert_eq!(server.phase(), Phase::AwaitingInitialize);
        }
        let mut server = Server::new();
        handle(&mut server, INIT);
        assert!(handle(&mut server, INIT).unwrap().contains("-32600"));
    }

    #[test]
    fn serve_writes_only_frames_to_output_and_logs_to_stderr() {
        let input = format!(
            "{INIT}\n\n{{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}}\n{}\nnot json\n",
            "x".repeat(MAX_MESSAGE_BYTES + 1)
        );
        let mut reader = io::Cursor::new(input.into_bytes());
        let mut output = Vec::new();
        let mut log = Logger::new(Vec::new());
        serve(&mut reader, &mut output, &mut log).unwrap();
        let output = String::from_utf8(output).unwrap();
        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 3, "{output}");
        assert!(lines[0].contains("\"result\""));
        assert!(lines[1].contains("-32600"), "크기 초과");
        assert!(lines[2].contains("-32700"), "파싱 실패");
        for line in lines {
            let doc = parse(line, JsonLimits::default()).unwrap();
            assert_eq!(doc.get("jsonrpc").and_then(JsonValue::as_str), Some("2.0"));
        }
        let log = String::from_utf8(log.into_inner()).unwrap();
        assert!(log.lines().all(|l| l.starts_with("stellar-mcp: ")), "{log}");
        assert!(log.contains("입력 종료"));
    }

    #[test]
    fn negotiation_uses_the_given_config_and_keeps_state_on_rejection() {
        let config =
            ProtocolConfig::from_config_str("supported_versions: 2030-01-01, 2029-06-30").unwrap();
        let mut server = Server::with_config(config);
        // 내장 선호 version이라도 이 config에 없으면 거부하고 initialize 전 상태를 유지합니다.
        let reply = handle(&mut server, INIT).unwrap();
        assert!(reply.contains("-32602") && reply.contains("2030-01-01, 2029-06-30"));
        assert_eq!(server.phase(), Phase::AwaitingInitialize);
        let retry = INIT.replace("2025-11-25", "2029-06-30");
        let reply = handle(&mut server, &retry).unwrap();
        assert!(
            reply.contains("\"protocolVersion\":\"2029-06-30\""),
            "{reply}"
        );
        assert_eq!(server.phase(), Phase::Initialized);
    }

    #[test]
    fn broken_config_fails_initialize_without_panic() {
        let mut server = Server {
            phase: Phase::AwaitingInitialize,
            protocol: Err(ProtocolConfigError::Empty),
        };
        let reply = handle(&mut server, INIT).unwrap();
        assert!(reply.contains("-32603"), "{reply}");
        assert_eq!(server.phase(), Phase::AwaitingInitialize);
    }
}

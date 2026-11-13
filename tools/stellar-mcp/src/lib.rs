//! stellar-mcp — MCP stdio 브리지 (XIX.1.1).
//!
//! AI client(외부 agent CLI)와 stdin/stdout JSON-RPC 2.0으로 통신하는 L2 브리지입니다. 브리지는
//! headless 도구(`stellar-cli`)의 기능을 감싸 노출할 수 있지만, headless 도구는 이 브리지에
//! 의존하지 않습니다. network port를 열지 않습니다.
//!
//! stdout은 protocol 메시지 전용입니다. 로그는 모두 [`log::Logger`]로 stderr에 씁니다.

/// 엄격한 JSON reader·writer (XIX.1.1).
pub mod json;

/// MCP stdio transport 프레이밍 (XIX.1.1).
pub mod framing;

/// JSON-RPC 2.0 메시지 모델 (XIX.1.1).
pub mod jsonrpc;

/// stderr 전용 로그 (XIX.1.1).
pub mod log;

/// MCP stdio 서버 루프 (XIX.1.1).
pub mod server;

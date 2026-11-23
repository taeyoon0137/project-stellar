//! stellar-mcp — MCP stdio 브리지 (XIX.1.1).
//!
//! AI client(외부 agent CLI)와 stdin/stdout JSON-RPC 2.0으로 통신하는 L2 브리지입니다. 브리지는
//! headless 도구(`stellar-cli`)의 기능을 감싸 노출할 수 있지만, headless 도구는 이 브리지에
//! 의존하지 않습니다. network port를 열지 않습니다.
//!
//! stdout은 protocol 메시지 전용입니다. 로그는 모두 [`log::Logger`]로 stderr에 씁니다.

/// 엄격한 JSON reader·writer (XIX.1.1).
pub mod json;

/// MCP resource·tool catalog 정의 (XIX.2).
pub mod catalog;

/// MCP client 등록 정보 (XIX.9.8).
pub mod connection_info;

/// 실행 인자 해석과 bridge 세션 실행. 실행 파일은 이 진입점만 부릅니다 (XIX.9.1).
pub mod entry;

/// L2 공용 정의(catalog·권한·schema·상한) crate (XIX.9.1, ADR-0005 §8).
pub use stellar_agent_protocol as agent_protocol;

/// 종료 코드: XXIV.1.2 표와 같은 값·이름 (XIX.1.7).
pub mod exit;

/// 실행 중 앱 연결과 브리지 측 인증 (XIX.1.7). macOS 전용(ADR-0006).
#[cfg(target_os = "macos")]
pub mod attach;

/// MCP stdio transport 프레이밍 (XIX.1.1).
pub mod framing;

/// JSON-RPC 2.0 메시지 모델 (XIX.1.1).
pub mod jsonrpc;

/// 브리지↔앱 로컬 IPC: endpoint registry (XIX.1.2), 사용자·nonce handshake (XIX.1.3).
///
/// 앱 runtime과 같은 구현을 쓰도록 `stellar-local-ipc` crate(ADR-0005 §8)에 있습니다. 기존 경로
/// `stellar_mcp::ipc`를 유지하려고 다시 내보냅니다. macOS 전용(ADR-0006).
pub use stellar_local_ipc as ipc;

/// stderr 전용 로그 (XIX.1.1).
pub mod log;

/// 부모 프로세스 종료 감지 (XIX.1.9). macOS 전용(ADR-0006).
#[cfg(target_os = "macos")]
pub mod parent;

/// MCP protocol version 고정과 협상 (XIX.1.5).
pub mod protocol;

/// MCP stdio 서버 루프 (XIX.1.1).
pub mod server;

/// tool 결과 envelope: 프로젝트 문자열은 data로만 (XIX.9.6).
pub mod result;

/// agent command proposal 입력 해석: closed enum command만 (XIX.5.1).
pub mod proposal;

/// 실행 인자로 고정한 신뢰 프로젝트 루트의 읽기 전용 backend (XIX.9.3).
pub mod headless;

/// 테스트 fixture 전용 재노출. 제품 빌드에는 없습니다.
#[cfg(feature = "test-support")]
pub use stellar_ops::test_support;

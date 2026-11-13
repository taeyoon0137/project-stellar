//! stellar-local-ipc — 브리지↔Stellar 앱 로컬 IPC (ADR-0005 §5, §8).
//!
//! `stellar-mcp` 브리지와 실행 중 앱 runtime이 함께 쓰는 로컬 제어 채널입니다. network port를
//! 열거나 연결하지 않고 사용자 전용 디렉터리의 Unix domain socket만 사용합니다. 허용 직접 의존은
//! core-types뿐이며 document·commands·ops에 도달하지 않습니다(crate graph 정책). macOS 전용입니다
//! (ADR-0006): 다른 OS에서는 빈 crate로 빌드됩니다.
//!
//! XIX.1.2(registry)·XIX.1.3(handshake) 구현은 처음에 `tools/stellar-mcp`에 있었고, 앱 runtime이
//! 같은 코드를 쓰도록 이 crate로 옮겼습니다.

/// OS 사용자 식별과 사용자별 디렉터리 (XIX.1.2).
#[cfg(target_os = "macos")]
pub mod os;

/// 실행 중 Stellar 앱 endpoint registry (XIX.1.2).
#[cfg(target_os = "macos")]
pub mod registry;

/// IPC session nonce (XIX.1.3).
#[cfg(target_os = "macos")]
pub mod nonce;

/// 같은 OS 사용자 확인과 session nonce handshake (XIX.1.3).
#[cfg(target_os = "macos")]
pub mod handshake;

/// 연결 상대 자격 증명: effective uid와 audit token (XIX.1.6).
#[cfg(target_os = "macos")]
pub mod peer;

/// 연결 상대 코드 서명 신원 검사 (XIX.1.6).
#[cfg(target_os = "macos")]
pub mod code_identity;

/// IPC 연결 상대 인증 기준 (XIX.1.6).
#[cfg(target_os = "macos")]
pub mod auth;

/// 앱 측 IPC endpoint와 IPC I/O 스레드 (XIX.1.6).
#[cfg(target_os = "macos")]
pub mod listener;

/// 앱 측 IPC test double (XIX.1.2, XIX.1.3).
#[cfg(all(test, target_os = "macos"))]
pub(crate) mod test_support;

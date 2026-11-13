//! 브리지↔Stellar 앱 로컬 IPC (XIX.1.2, XIX.1.3).
//!
//! network port를 열거나 연결하지 않습니다. 사용자 전용 디렉터리의 Unix domain socket만
//! 사용합니다. macOS 전용입니다(ADR-0006).

/// OS 사용자 식별과 사용자별 디렉터리 (XIX.1.2).
pub mod os;

/// 실행 중 Stellar 앱 endpoint registry (XIX.1.2).
pub mod registry;

/// IPC session nonce (XIX.1.3).
pub mod nonce;

/// 같은 OS 사용자 확인과 session nonce handshake (XIX.1.3).
pub mod handshake;

/// 앱 측 IPC test double (XIX.1.2, XIX.1.3).
#[cfg(test)]
pub(crate) mod test_support;

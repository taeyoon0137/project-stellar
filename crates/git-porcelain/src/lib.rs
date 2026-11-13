//! stellar-git-porcelain — System Git porcelain 골격.
//!
//! document를 직접 수정하지 않고 snapshot 인터페이스(빈 trait)에만 의존합니다.

/// Git executable 탐색 (IV.3.1).
pub mod discovery;

/// Git 버전 probe (IV.3.2).
pub mod version;

/// 보안 Git 실행기 (IV.3.3).
pub mod runner;

/// Git 명령 결과 (IV.3.4).
pub mod command;

/// Git 설치 상태 머신 (IV.3.5).
pub mod install_state;

/// macOS Git 설치 어댑터 (IV.3.6).
pub mod install_macos;

/// Windows Git 설치 어댑터 (IV.3.7).
pub mod install_windows;

/// 설치 후 재탐지·재probe (IV.3.8).
pub mod reprobe;

/// 저장소 porcelain 연산 (IV.4.4~).
pub mod repo;

/// 프로젝트 신뢰 gate (XIV.1.1).
pub mod trust;

/// 저장소 위치 판별 (XIV.1.2).
pub mod location;

/// 테스트 공용 도구(테스트 빌드 전용).
#[cfg(test)]
mod test_support;

/// Stellar 프로젝트 snapshot 인터페이스(골격용 빈 trait).
pub trait SnapshotSource {}

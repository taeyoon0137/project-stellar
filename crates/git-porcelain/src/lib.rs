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

/// Stellar 프로젝트 snapshot 인터페이스(골격용 빈 trait).
pub trait SnapshotSource {}

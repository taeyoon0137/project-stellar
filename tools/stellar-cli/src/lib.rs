//! stellar-cli — 앱 UI와 분리된 headless 명령줄 도구 (XXIV.1.1).
//!
//! 프로젝트 파일을 core crate로 직접 검사·검증·편집하는 L1 도구입니다. 앱 UI·GUI·네트워크 없이
//! 동작하며 AI·모델·agent 코드나 의존성을 두지 않습니다. 상위 계층(MCP 브리지 등)은 이 도구를
//! 감쌀 수 있지만, 이 도구는 상위 계층에 의존하지 않습니다.
//!
//! 실행 파일(`main.rs`)은 인자를 [`cli::run`]에 넘기고 반환된 종료 코드로 끝납니다.

/// 명령 registry와 도움말 (XXIV.1.1, XXIV.1.2).
pub mod cli;

/// 명령 구현 (XXIV.1.2).
pub mod commands;

/// 종료 코드 (XXIV.1.2).
pub mod exit;

/// 엄격한 JSON reader·writer (XXIV.1.2).
pub mod json;

/// 결과 envelope와 stdout/stderr 역할 (XXIV.1.2).
pub mod output;

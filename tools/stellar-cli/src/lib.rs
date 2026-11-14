//! stellar-cli — 앱 UI와 분리된 headless 명령줄 도구 (XXIV.1.1).
//!
//! 프로젝트 파일을 `stellar-ops`(L1 공용 library)로 직접 검사·검증·편집하는 L1 도구입니다. 직접
//! 의존은 `stellar-ops`·diagnostics로 제한합니다(ADR-0005 §8, XXIV.4.3). 앱 UI·GUI·네트워크 없이
//! 동작하며 AI·모델·agent 코드나 의존성을 두지 않습니다. 상위 계층(MCP 브리지 등)은 이 도구를
//! 감쌀 수 있지만, 이 도구는 상위 계층에 의존하지 않습니다.
//!
//! 사용자가 부르는 명령 이름은 `stellar`입니다(XXIV.4.1, ADR-0005 §2). package·library 이름은
//! `stellar-cli`로 유지하고 실행 파일만 `stellar`로 빌드합니다. 실행 파일(`main.rs`)은 인자를
//! [`cli::run`]에 넘기고 반환된 종료 코드로 끝납니다.

/// 명령 registry와 도움말 (XXIV.1.1, XXIV.1.2, XXIV.4.1).
pub mod cli;

/// 명령 구현 (XXIV.1.2, XXIV.4.3).
pub mod commands;

/// `stellar apply`: CommandTransaction JSON 적용 (XXIV.4.4).
pub mod apply;

/// `validate`, `semantic-diff`, `semantic-merge`, `migrate` (XXIV.1.3), `ci` (XXIV.3.1).
pub mod project_cmds;

/// `diff-summary`: redaction 기본 semantic diff 요약 artifact (XXIV.3.2).
pub mod summary;

/// `text-source`: opt-in 단일 View 텍스트 export·읽기 전용 검사 (ADR-0059).
pub mod text_source;

/// 종료 코드 (XXIV.1.2).
pub mod exit;

/// 엄격한 JSON reader·writer (XXIV.1.2).
pub mod json;

/// 결과 envelope와 stdout/stderr 역할 (XXIV.1.2).
pub mod output;

/// `stellar-ops` 결과 → 결과 JSON 변환과 오류 분류 (XXIV.4.3).
pub mod views;

/// `export-accessibility`: 저장된 View 하나의 정적 접근성 HTML.
pub mod accessibility_html;

/// `export-web`: 저장된 고정 Free Frame View의 HTML+CSS.
pub mod web_frame;

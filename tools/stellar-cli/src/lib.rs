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

/// `diagnose`: 무결성 진단 필터·page (XXV.3.1, ADR-0095).
pub mod diagnose;

/// `diff-summary`: redaction 기본 semantic diff 요약 artifact (XXIV.3.2).
pub mod summary;

/// `text-source`: opt-in 단일 View 텍스트 export·읽기 전용 검사 (ADR-0059).
pub mod text_source;

/// `git status|stage|unstage|commit`: 의미 단위 Git 엔진 (ADR-0080).
pub mod git;

/// 종료 코드 (XXIV.1.2).
pub mod exit;

/// 엄격한 JSON reader·writer (XXIV.1.2).
pub mod json;

/// 결과 envelope와 stdout/stderr 역할 (XXIV.1.2).
pub mod output;

/// `stellar-ops` 결과 → 결과 JSON 변환과 오류 분류 (XXIV.4.3).
pub mod views;

/// `export-web`: 저장된 고정 Free Frame View의 HTML+CSS.
pub mod web_frame;

/// `export-styles`: 대상별 layout 스타일(Web 기반 규칙·요소 선언, React Native StyleSheet) (IX.11.30).
pub mod export_styles;

/// `export swiftui|appkit|compose`: 저장 View의 SwiftUI·AppKit·Compose 소스 (XXII.3·XXII.4).
pub mod export_native;

/// `export png`: 저장 View의 PNG 파일 (XX.2.3, ADR-0116).
pub mod export_png;

/// `css`: CSS 선언 블록 복사·붙여넣기 (IX.11.15).
pub mod css;

/// `context`: Component Context 선언·진단·위치별 해석 조회 (XXX.7.11, ADR-0114).
pub mod context;

/// `style-defaults`: 기본값 세트 목록·값 표·선택과 값 출처 (SD.5·SD.6, ADR-0086).
pub mod style_defaults;

/// `diagnostics`: 로컬 진단 preview·수동 번들 내보내기·금지 필드 검사·동의 (XVI.8, ADR-0109).
pub mod diagnostics;

mod component_source;
mod merge_driver;

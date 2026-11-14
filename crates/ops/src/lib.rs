//! stellar-ops — headless 조회·편집 공용 library (XXIV.4.2, ADR-0005 §1·§8).
//!
//! L1 계층입니다. 실행 중 앱 없이 core crate로 프로젝트 폴더를 직접 열어 조회·검증·편집합니다.
//! `stellar` 명령(package `stellar-cli`)과 L2 MCP backend가 같은 구현을 in-process로 호출합니다.
//!
//! - 의존 방향: 이 crate는 project, document, commands, history, storage, semantic-diff,
//!   semantic-merge, diagnostics, codegen, accessibility, target-profile에만 직접 의존할 수 있고,
//!   local-ipc·agent-protocol·mcp-core·
//!   agent-host·ffi에는 전이적으로도 도달하지 않습니다(`cargo xtask validate-crate-graph`).
//! - AI·MCP·모델·에이전트 관련 코드를 두지 않습니다.
//! - 읽기 경로([`snapshot`])는 프로젝트 파일을 만들거나 바꾸지 않습니다.
//!
//! 결과 타입은 출력 형식(JSON 등)과 무관한 Rust 값입니다. 출력 형식은 호출자가 정합니다.

/// StableId·PropertyPath 등 core 값 타입(semantic-diff 재노출 경유, 직접 edge 없음).
pub use stellar_semantic_diff::core_types;

/// 프로젝트 매니페스트·root 검사 타입(stellar-project 재노출). 상위 도구가 project crate에 직접
/// 의존하지 않고 결과를 해석할 수 있게 합니다(ADR-0005 §8 `stellar-cli` 허용 의존 ops·diagnostics).
pub mod types {
    pub use stellar_project::manifest::{Manifest, ManifestError};
    pub use stellar_project::root::{GitLink, RootError};
    pub use stellar_project::yaml::{LoaderError, LoaderErrorKind};
}

/// 디스크 generation 기준 프로젝트 revision (XXIV.4.2).
pub mod revision;

/// 프로젝트 열기 오류 (XXIV.4.2).
pub mod error;

/// 앱 없이 연 읽기 전용 프로젝트 snapshot (XXIV.4.2).
pub mod snapshot;

/// 읽기 전용 조회: 프로젝트 구조, 문서 트리 page, node 속성, token·component (XXIV.4.3).
pub mod query;

/// 문서 레코드의 semantic diff 요약 (XXIV.4.4).
pub mod diff;

/// headless 편집 적용: base revision 검사, closed enum 검증, 세대 전환 저장 (XXIV.4.4).
pub mod apply;

/// 교차 프로세스 저장 잠금 (XXIV.4.5).
pub mod lock;

/// 프로젝트 검증 (XXIV.1.3).
pub mod validate;

/// 프로젝트 사이 semantic diff와 3-way semantic merge (XXIV.1.3).
pub mod compare;

/// 선택 staging 후보의 무손실 검사와 기존 generation 파일 직렬화(XIV.5.5).
pub mod candidate;

/// 저장 스키마 migration (XXIV.1.3).
pub mod migrate;

/// CI 검사: lockfile·검증·migration 필요 여부 (XXIV.3.1).
pub mod ci;

/// redaction을 기본으로 하는 semantic diff 요약 artifact (XXIV.3.2).
pub mod summary;

/// opt-in 단일 View 텍스트 원본 export·읽기 전용 검사 (ADR-0059).
pub mod text_source;

/// 저장된 View 하나의 정적 접근성 HTML 투영(전체 코드 내보내기가 아님).
pub mod accessibility_html;

/// 테스트 전용 프로젝트 fixture 생성기. 프로덕션 경로에서 사용하지 않습니다.
#[doc(hidden)]
pub mod test_support;

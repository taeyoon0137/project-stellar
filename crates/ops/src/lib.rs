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

/// 루트로 고정하고 기존 신뢰 등록으로 확인한 읽기 전용 snapshot.
pub mod trusted;

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

/// Git 엔진: 상태·선택 stage·재검증 commit (ADR-0080).
pub mod git;

/// 저장 스키마 migration (XXIV.1.3).
pub mod migrate;

/// CI 검사: lockfile·검증·migration 필요 여부 (XXIV.3.1).
pub mod ci;

/// redaction을 기본으로 하는 semantic diff 요약 artifact (XXIV.3.2).
pub mod summary;

/// opt-in 단일 View 텍스트 원본 export·읽기 전용 검사 (ADR-0059).
pub mod text_source;

/// 저장된 고정 Free Frame View 하나의 HTML+CSS 투영.
pub mod web_frame;

/// 저장 View 내보내기의 CSS layout 계획과 React Native 스타일 객체 출력 (IX.11.30, ADR-0089).
pub mod export_css;

/// 저장 View → SwiftUI·AppKit·Compose 소스 (ADR-0107).
pub mod export_native;

/// CSS 선언 블록 복사·붙여넣기 (IX.11.15).
pub mod css;

/// 기본값 세트(ADR-0082, ADR-0086): 세트 표·해석 함수(document)와 프로젝트 선택·사용자 세트 파일(project)을
/// 상위 도구가 같은 구현으로 쓰도록 다시 냅니다.
pub mod style_defaults {
    pub use stellar_document::css::style_defaults::*;
    pub use stellar_document::layout_record;
    pub use stellar_project::manifest_style_defaults::*;
}

/// 테스트 전용 프로젝트 fixture 생성기. 프로덕션 경로에서 사용하지 않습니다.
#[doc(hidden)]
pub mod test_support;
/// 편집 command 모델 재수출. L2(mcp-core)가 commands crate에 직접 의존하지 않고 같은 lease 판정·실행 경로를
/// 씁니다(ADR-0093). 테스트 fixture도 이 경로를 씁니다.
pub use stellar_commands as commands;

/// opt-in 컴포넌트별 텍스트 원본 파일 (ADR-0059).
pub mod component_source;

/// 형식 중립 command 입력 해석과 command 종류 표: L2 MCP와 schema 생성이 쓰는 진입점 (XIX.5.1, ADR-0085).
pub mod mcp_command_input;

/// 진행 중 merge의 semantic conflict 읽기 전용 진입점: L2 MCP가 쓰는 얇은 경로 (XIX.9.3, ADR-0085).
pub mod mcp_merge;

/// 무결성 진단(ADR-0095): 공통 DiagnosticIndex·필터·pagination·remediation(`stellar-integrity`)과 저장 snapshot 수집.
/// `stellar diagnose`와 MCP `document.diagnostics`가 같은 수집 함수를 씁니다.
pub mod integrity {
    pub use stellar_integrity::*;

    use crate::core_types::ids::StableId;
    use crate::snapshot::ProjectSnapshot;
    use stellar_integrity::incremental::{origins, CheckContext, IntegrityEngine};
    use stellar_project::migration::SchemaDecision;

    /// 저장 snapshot 진단 결과.
    #[derive(Debug)]
    pub struct SnapshotDiagnostics {
        /// 진단 index를 가진 엔진.
        pub engine: IntegrityEngine,
        /// 검사한 문서 수.
        pub documents_checked: usize,
        /// 읽지 못한 문서와 이유(진단에 넣지 않고 따로 보고, `stellar validate`의 `document-load`와 같은 원인).
        pub unreadable: Vec<(StableId, String)>,
    }

    /// 저장 snapshot에서 앱 상태 없이 계산할 수 있는 진단을 모읍니다: 모든 문서의 미해결 참조(내장 checker)와 메모리
    /// migration 상태. asset store·font registry·접근성 tree·Target Profile·library cache가 필요한 생산자는 headless
    /// snapshot에 그 입력이 없어 실행하지 않습니다(ADR-0095 결정 7).
    pub fn diagnose_snapshot(snapshot: &ProjectSnapshot) -> SnapshotDiagnostics {
        let mut engine = IntegrityEngine::new();
        let mut unreadable = Vec::new();
        let mut documents_checked = 0;
        if let SchemaDecision::MigrateInMemory { from, to } = snapshot.schema() {
            engine.submit(None, origins::MIGRATION, sources::migration(from, to));
        }
        for definition in snapshot.documents() {
            match snapshot.document_records(definition.id) {
                Ok(records) => {
                    documents_checked += 1;
                    engine.check_document(definition.id, &records, &CheckContext::default());
                }
                Err(e) => unreadable.push((definition.id, e.to_string())),
            }
        }
        SnapshotDiagnostics {
            engine,
            documents_checked,
            unreadable,
        }
    }
}

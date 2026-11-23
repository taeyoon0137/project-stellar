//! stellar-bindings — Project Stellar 토큰과 통합 Binding Graph. core-types와 expressions만 직접 의존합니다.

/// 토큰 정의·이름·타입·TokenSet (VIII.5.1).
pub mod token;

/// 프로젝트·라이브러리·문서 scope 토큰 registry (VIII.5.2).
pub mod registry;

/// 토큰 dependency graph와 저장 전 cycle 거부 (VIII.5.3).
pub mod token_graph;

/// 토큰 값 평가 cache (VIII.5.4).
pub mod token_cache;

/// 토큰 생성·삭제·rename·expression 변경 command (VIII.5.5).
pub mod token_commands;

/// Value Binding 정의·직렬화·type 검증 (VIII.6.1).
pub mod binding;

/// 토큰·속성·Component Property·환경 입력 통합 dependency graph (VIII.6.2).
pub mod graph;

/// 저장소가 편집과 함께 유지하는 토큰·통합 dependency graph view (XVI.4.9).
pub mod graph_view;

/// 저장소 graph view 시험과 release 측정 (XVI.4.9).
#[cfg(test)]
mod graph_view_tests;

/// 영향받는 binding만 위상 순서로 재평가하는 scheduler (VIII.6.4).
pub mod scheduler;

/// 평가 값 provenance chain (VIII.6.5).
pub mod provenance;

/// binding 생성·교체·삭제 command (VIII.6.6).
pub mod binding_commands;

/// 환경 입력 reverse index와 환경 변경 무효화 (VIII.7.3).
pub mod environment_index;

/// Orphaned Binding validator와 binding 저장소 (VIII.8.1).
pub mod orphan;

/// live·orphan binding 저장소와 토큰 역참조·orphan target index (VIII.8.1, XVI.4.8).
pub mod store;

/// binding 저장소 index release 측정 (XVI.4.8).
#[cfg(test)]
mod store_bench;

/// orphan binding 선언 기본값 fallback (VIII.8.2).
pub mod fallback;

/// orphaned binding 재연결·명시 삭제 command (VIII.8.4).
pub mod orphan_commands;

/// 대규모 synthetic token/binding graph 생성기 (VIII.9.1).
pub mod synthetic;

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

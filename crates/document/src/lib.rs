//! stellar-document — Project Stellar 문서 모델 (구현은 후속).
//!
//! 현재는 골격이며 의존 방향 `history -> commands -> document -> core-types`를 고정합니다.

/// Generational arena (III.6.1).
pub mod arena;

/// 공통 node 필드의 SoA 저장 (III.6.2).
pub mod node_columns;

/// 문서 소유자와 읽기 전용 스냅샷 view (III.6.3).
pub mod snapshot;

/// 문서·페이지·View 정의 (VI.1.1).
pub mod definitions;

/// 열린 문서 집합 (VI.1.4).
pub mod open_set;

/// 열린 문서 수 제한 (VI.1.5).
pub mod open_limits;

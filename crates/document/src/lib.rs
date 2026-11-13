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

/// Node 종류 (VI.2.1).
pub mod node_kind;

/// parent-child 삽입·이동 validator (VI.2.3).
pub mod tree_ops;

/// 참조 inbound/outbound index (VI.2.5).
pub mod reference_index;

/// node count·depth incremental 훅 (VI.2.6).
pub mod tree_stats;

/// Frame payload (VI.3.1).
pub mod frame;

/// Frame property metadata registry (VI.3.5).
pub mod frame_metadata;

/// node 종류 capability validator (VI.3.2).
pub mod capabilities;

/// Fragment payload (VI.4).
pub mod fragment;

/// Shape payload (VI.5).
pub mod shape;

/// Path command (VI.6).
pub mod path;

/// Text payload (VI.7).
pub mod text;

/// Image payload (VI.8).
pub mod image;

/// Modifier stack (VI.9).
pub mod modifier;

/// Modifier property metadata와 무효화 class registry (VI.9.7).
pub mod modifier_metadata;

/// 영속 문서 레코드 (VI.10).
pub mod records;

/// 전체 document validator (VI.10.1).
pub mod validate;

/// 영속 레코드 canonical 인코딩과 View subtree 분할 (VI.10.2).
pub mod record_codec;

/// 런타임 문서 모델 (VI.10.2).
pub mod model;

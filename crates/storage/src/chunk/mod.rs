//! chunk 저장소 (V.2).
//!
//! 논리 chunk 식별자([`ChunkId`])와 후속 작업의 envelope·경로·저장소를 담습니다.

/// 논리 chunk 식별자 (V.2.1).
pub mod id;

/// chunk envelope header (V.2.2).
pub mod envelope;

/// chunk object 경로 생성 (V.2.3).
pub mod path;

/// immutable chunk write (V.2.4).
pub mod store;

/// chunk kind registry와 unknown 정책 (V.2.7).
pub mod kind_registry;

/// 논리 ChunkId → content hash 매핑 pointer (V.3.1).
pub mod pointer;

/// 변경 chunk만 기록하는 write planner (V.3.3).
pub mod planner;

/// chunk remap manifest (V.3.4).
pub mod remap;

/// 초기 chunk kind 정의 (V.6.1).
pub mod kinds;

/// ViewTree chunk payload 스키마 (V.6.2).
pub mod view_tree;

/// chunk 크기 예산 config reader (V.6.4).
pub mod budget;

/// 의미 경계 기반 chunk split planner (V.6.4).
pub mod split;

pub use id::ChunkId;

//! chunk 저장소 (V.2).
//!
//! 논리 chunk 식별자([`ChunkId`])와 후속 작업의 envelope·경로·저장소를 담습니다.

/// 논리 chunk 식별자 (V.2.1).
pub mod id;

/// chunk envelope header (V.2.2).
pub mod envelope;

pub use id::ChunkId;

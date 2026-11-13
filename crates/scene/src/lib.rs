//! stellar-scene — 불변 Scene 모델. core-types 의존.
//!
//! renderer-core는 crate 의존 정책(II.2.8)상 scene에만 직접 의존하므로, Scene 공개 API가 쓰는
//! core-types 기본 타입을 이 crate에서 재노출합니다.

/// Scene path geometry (XI.6.1).
pub mod geometry;

pub use stellar_core_types::value::{FiniteF32, NonFiniteError, Vec2};

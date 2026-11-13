//! stellar-scene — 불변 Scene 모델. core-types 의존.
//!
//! renderer-core는 crate 의존 정책(II.2.8)상 scene에만 직접 의존하므로, Scene 공개 API가 쓰는
//! core-types 기본 타입을 이 crate에서 재노출합니다.
//!
//! scene은 Document Node와 GPU 객체를 담지 않습니다(XI.1.1). document 계층이 평가한 값을
//! 복사해 담고, 외부 cache가 해석하는 데이터는 [`SceneResourceRef`] 식별자로만 가리킵니다.

/// Scene path geometry (XI.6.1).
pub mod geometry;

/// Scene node와 resource 참조 (XI.1.1).
pub mod node;

/// 불변 Scene Snapshot (XI.1.1).
pub mod snapshot;

/// Scene snapshot canonical serialization (XI.1.1).
pub mod codec;

pub use node::{SceneContent, SceneNode, SceneNodeSpec, SceneResourceRef};
pub use snapshot::SceneSnapshot;

pub use stellar_core_types::color::ColorSpace;
pub use stellar_core_types::value::{FiniteF32, NonFiniteError, Size2, Vec2};

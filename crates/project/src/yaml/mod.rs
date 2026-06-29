//! 제한 YAML 지원 (IV.1).
//!
//! - [`value`]: 값 트리 타입 [`YamlValue`](IV.1.1).
//! - [`loader`]: 텍스트 → 값 트리 제한 loader(IV.1.2).
//! - [`scalar`]: 스칼라의 엄격한 타입 해석(IV.1.4).
//! - canonical writer는 IV.1.5에서 추가합니다.

pub mod loader;
pub mod scalar;
pub mod value;

pub use loader::{load_restricted, LoaderError, LoaderErrorKind};
pub use value::YamlValue;

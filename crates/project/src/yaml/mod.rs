//! 제한 YAML 지원 (IV.1).
//!
//! - [`value`]: 값 트리 타입 [`YamlValue`](IV.1.1).
//! - 텍스트 → 값 트리 제한 loader는 IV.1.2, canonical writer는 IV.1.5에서 추가합니다.

pub mod value;

pub use value::YamlValue;

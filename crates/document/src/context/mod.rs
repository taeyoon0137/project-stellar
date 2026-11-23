//! Component Context (XXX.7, ADR-0025 결정 3, ADR-0099).
//!
//! Context는 경계를 암묵적으로 넘는 참조가 아니라 **선언된 인터페이스**입니다.
//!
//! - [`key`]: `ContextKey` resource(StableId·이름·TypeSpec·필수 닫힌 default·scope, XXX.7.1).

/// `ContextKey` resource (XXX.7.1).
pub mod key;

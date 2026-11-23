//! Component Context (XXX.7, ADR-0025 결정 3, ADR-0099).
//!
//! Context는 경계를 암묵적으로 넘는 참조가 아니라 **선언된 인터페이스**입니다.
//!
//! - [`key`]: `ContextKey` resource(StableId·이름·TypeSpec·필수 닫힌 default·scope, XXX.7.1).
//! - [`provision`]: node에 붙는 `ContextProvision`과 owner 범위 검사(XXX.7.2).
//! - [`set`]: key·provision·`ContextRequirement`·Slot 공개 목록을 담는 문서 resource `ContextSet`(XXX.7.2).

/// `ContextKey` resource (XXX.7.1).
pub mod key;

/// `ContextProvision` (XXX.7.2).
pub mod provision;

/// 문서 Context 요소 모음 (XXX.7.2).
pub mod set;

/// 시험용 문서·컴포넌트 구성 도우미.
#[cfg(any(test, feature = "test-support"))]
pub mod fixtures;

#[cfg(test)]
mod set_tests;

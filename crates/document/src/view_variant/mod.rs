//! View 변수와 nullable 조합형 Variant (IX.7).
//!
//! 프리뷰 runtime 상태는 저장하지 않습니다. 특정 화면 상태를 남기려면 ViewDefinition에 **View 변수**를 정의하고
//! 변수 조건에 맞는 **Variant override**를 작성합니다(기술 아키텍처 16장).
//!
//! - [`variable`]: View 변수(IX.7.1) — StableId, 타입, default, 허용 enum 값.
//! - [`condition`]: Variant 조건(IX.7.2) — 변수별 `Any` 또는 `Equals(value)`.
//! - [`resolver`]: 일치 Variant 정렬·충돌·적용(IX.7.4~IX.7.6).
//! - [`encapsulation`]: Variant override의 ComponentInstance 캡슐화 검사(IX.7.7).
//! - [`commands`]: View 변수·Variant 생성·변경·삭제 command와 orphan 진단(IX.7.8).

/// View 변수 (IX.7.1).
pub mod variable;

/// VariantCondition (IX.7.2).
pub mod condition;

/// Variant resolver (IX.7.4~IX.7.6).
pub mod resolver;

/// Variant override의 ComponentInstance 캡슐화 검사 (IX.7.7).
pub mod encapsulation;

/// View 변수·Variant 생성·변경·삭제 command (IX.7.8).
pub mod commands;

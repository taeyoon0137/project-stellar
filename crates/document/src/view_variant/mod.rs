//! View 변수와 nullable 조합형 Variant (IX.7).
//!
//! 프리뷰 runtime 상태는 저장하지 않습니다. 특정 화면 상태를 남기려면 ViewDefinition에 **View 변수**를 정의하고
//! 변수 조건에 맞는 **Variant override**를 작성합니다(기술 아키텍처 16장).
//!
//! - [`variable`]: View 변수(IX.7.1) — StableId, 타입, default, 허용 enum 값.

/// View 변수 (IX.7.1).
pub mod variable;

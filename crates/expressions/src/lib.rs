//! stellar-expressions — Project Stellar 표현식 타입 시스템. core-types만 직접 의존합니다.

/// 표현식 타입과 nullable `Optional<T>` (VIII.1.1).
pub mod types;

/// type rule 공통 오류와 피연산자 위치 (VIII.1.2).
pub mod rules;

/// 산술 type rule table (VIII.1.2).
pub mod arithmetic;

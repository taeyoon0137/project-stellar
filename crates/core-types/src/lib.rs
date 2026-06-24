//! stellar-core-types — Project Stellar 공통 코어 타입.
//!
//! 현재는 공개 모듈만 선언한 골격입니다. 구현은 후속 작업에서 채웁니다.

/// 안정 식별자·런타임 식별자 (III.1).
pub mod ids {}

/// 구조화 오류 타입 (II.3.1).
pub mod error;

/// 타입드 값과 기하 (III.3).
pub mod value {}

/// 단조 시계 추상화 (II.3.3).
pub mod clock;

/// revision newtype (II.3.4).
pub mod revision;

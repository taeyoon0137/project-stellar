//! stellar-core-types — Project Stellar 공통 코어 타입.
//!
//! 현재는 공개 모듈만 선언한 골격입니다. 구현은 후속 작업에서 채웁니다.

/// 안정 식별자·런타임 식별자 (III.1).
pub mod ids;

/// 구조화 오류 타입 (II.3.1).
pub mod error;

/// PropertyKey·PropertyPath (III.2).
pub mod property;

/// 강한 타입 참조와 미해결 참조 (III.2.3, III.2.4).
pub mod reference;

/// 타입드 값과 기하 (III.3).
pub mod value;

/// 색상과 색공간 (III.4.1).
pub mod color;

/// 그라디언트와 stop (III.4.3).
pub mod gradient;

/// 그림자와 blur parameter (III.4.4).
pub mod shadow;

/// 자식 순서 키 (III.5).
pub mod order_key;

/// OrderKey Semantic Merge fixture (III.5.4).
pub mod order_key_merge_fixtures;

/// TypedValue tagged union (III.3.5).
pub mod typed_value;

/// 문자열 값과 정규화 경계 (III.3.4).
pub mod text;

/// 단조 시계 추상화 (II.3.3).
pub mod clock;

/// revision newtype (II.3.4).
pub mod revision;

/// 프로젝트와 공통 shaper가 공유하는 font face identity.
pub mod font;

/// CSS longhand typography record.
pub mod typography;

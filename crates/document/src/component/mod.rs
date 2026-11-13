//! 컴포넌트 정의·인스턴스·Slot (Phase IX).
//!
//! 컴포넌트 정의는 scene node가 아니라 문서 resource입니다(VI.2.1). scene에는
//! [`NodeKind::ComponentInstance`](crate::node_kind::NodeKind::ComponentInstance) node만
//! 배치되고, 인스턴스는 정의를 복제하지 않고 참조합니다.
//!
//! 값은 core-types의 [`TypedValue`](stellar_core_types::typed_value::TypedValue)를
//! 사용합니다. Property default·상태 변수 initial처럼 타입이 선언된 표현식 위치는
//! `stellar-expressions`의 AST·type checker·evaluator(Phase VIII)를 그대로 씁니다. 값 binding
//! 그래프(VIII.6)는 `stellar-bindings`의 몫입니다.

mod codec;

/// ComponentDefinition resource와 정의 tree (IX.1.1).
pub mod definition;

/// 타입이 확인된 표현식 위치 (IX.1.2·IX.5 공용).
pub mod typed_expr;

/// ComponentProperty record (IX.1.2).
pub mod property;

/// ComponentInstance node payload (IX.2.1).
pub mod instance;

/// 인스턴스 expansion view (IX.2.2).
pub mod expansion;

/// ComponentSlot (IX.3.1).
pub mod slot;

/// Slot override 투영 규칙 (IX.3.2).
pub mod slot_override;

/// 가상 subtree 구조 편집 validator (IX.2.5).
pub mod mutation;

/// 명시적 detach command (IX.2.6).
pub mod detach;

/// Slot content 삽입·삭제·재정렬 command (IX.3.3).
pub mod slot_commands;

/// View Variant 희소 patch 묶음 (IX.7.3).
pub mod variant_override;

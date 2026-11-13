//! 컴포넌트 정의·인스턴스·Slot (Phase IX).
//!
//! 컴포넌트 정의는 scene node가 아니라 문서 resource입니다(VI.2.1). scene에는
//! [`NodeKind::ComponentInstance`](crate::node_kind::NodeKind::ComponentInstance) node만
//! 배치되고, 인스턴스는 정의를 복제하지 않고 참조합니다.
//!
//! 값은 core-types의 [`TypedValue`](stellar_core_types::typed_value::TypedValue)를
//! 사용합니다. 표현식·binding(Phase VIII)은 이 crate의 의존 대상이 아니므로 여기서
//! 다루지 않습니다.

mod codec;

/// ComponentDefinition resource와 정의 tree (IX.1.1).
pub mod definition;

/// ComponentInstance node payload (IX.2.1).
pub mod instance;

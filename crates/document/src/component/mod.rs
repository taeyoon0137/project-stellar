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

pub(crate) mod codec;

/// ComponentDefinition resource와 정의 tree (IX.1.1).
pub mod definition;

/// 타입이 확인된 표현식 위치 (IX.1.2·IX.5 공용).
pub mod typed_expr;

/// ComponentProperty record (IX.1.2).
pub mod property;

/// Property 생성·rename·default 변경·삭제 command (IX.1.3).
pub mod property_commands;

/// Property runtime 평가 값 instance별 캐시 (IX.1.4).
pub mod property_cache;

/// 상태 머신 이벤트 종류 (IX.5.2).
pub mod event;

/// 컴포넌트 선언형 상태 머신 (IX.5).
pub mod state_machine;

/// preview session별 runtime 상태 store (IX.5.4).
pub mod state_store;

/// 이벤트 dispatch·조건 평가·action commit (IX.5.5).
pub mod state_dispatch;

/// nested 컴포넌트 상태 캡슐화 validator (IX.5.6).
pub mod state_encapsulation;

/// 상태 머신 진단 (IX.5.7).
pub mod state_diagnostics;

/// ComponentInstance node payload (IX.2.1).
pub mod instance;

/// 인스턴스 PropertyOverride validator (IX.2.3).
pub mod override_rules;

/// 인스턴스 PropertyOverride 생성·변경·reset command (IX.2.4).
pub mod override_commands;

/// nested instance property 직접 수정 금지 (IX.4.2).
pub mod encapsulation;

/// 컴포넌트 경계를 넘는 drilling binding (IX.4.1).
pub mod drilling;

/// Property drilling 계획 (IX.4.3).
pub mod drilling_plan;

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

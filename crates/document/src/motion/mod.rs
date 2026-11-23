//! 시간축 원본 `MotionSpec` (XXX.1.1, XXX.1.6, ADR-0025 결정 2.3, ADR-0094).
//!
//! `MotionSpec`은 CSS `transition`과 같은 어휘(대상 속성, 지속 시간, 지연, timing function)로 저장하는 문서
//! 데이터입니다. 시스템 시각에 접근하지 않으며, 평가는 프리뷰 세션의 논리 시계가 맡습니다(`stellar-prototype`).

/// `MotionSpec` record와 검증.
pub mod spec;

/// MotionSpec 모음과 canonical binary 형식.
pub mod set;

/// 대상 해석·보간 끝값 검사·orphan 진단.
pub mod world;

pub use set::{MotionSet, MotionSetError};
pub use spec::{
    is_interpolable, MotionOwner, MotionSpec, MotionSpecError, MotionSpecId, MotionSpecInput,
    ReducedMotion, StartCondition, StepPosition, TimingFunction,
};
pub use world::{
    check_endpoints, check_targets, diagnose, EndpointError, LibraryWorld, MotionCheckError,
    MotionDiagnostic, MotionWorld, OrphanCause,
};

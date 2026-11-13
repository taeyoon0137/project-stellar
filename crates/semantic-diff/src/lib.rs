//! stellar-semantic-diff — Project Stellar 의미 기반 diff (Phase XIV.3). 의존: document, core-types.
//!
//! Git HEAD·Index·Working Tree에서 읽은 영속 레코드를 [`SemanticSnapshot`](snapshot::SemanticSnapshot)
//! (StableId index)으로 펼치고, 두 snapshot의 의미 차이를 `StableId + PropertyPath` 단위로
//! 계산합니다(아키텍처 §26). 하위 crate(semantic-merge)는 이 crate에만 의존하므로 공용 값
//! 타입을 [`core_types`]로 다시 내보냅니다.

/// core-types 재노출(StableId·PropertyPath·OrderKey·TypedValue 등).
pub use stellar_core_types as core_types;

/// 속성 값 (XIV.3.1).
pub mod value;

/// 속성 key (XIV.3.1).
pub mod keys;

/// SemanticSnapshot 모델 (XIV.3.1).
pub mod snapshot;

/// 영속 레코드 → snapshot 구성 (XIV.3.1).
pub mod build;

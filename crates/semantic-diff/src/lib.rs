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

/// SemanticChange 모델 (XIV.3.2).
pub mod change;

/// snapshot diff (XIV.3.3~XIV.3.8).
pub mod diff;

/// snapshot·change canonical 인코딩 (XIV.3.8).
pub mod codec;

/// diff 적용 (XIV.3.9~XIV.3.10).
pub mod apply;

/// HEAD·Index·WorkingTree semantic status 분류 (XIV.4.1~).
pub mod status;

/// 선택 변경 Staging 계획 (XIV.5.1~).
pub mod stage;

/// 변경 dependency graph (XIV.5.2).
pub mod dependency;

/// semantic snapshot 무결성 validator (XIV.5.3).
pub mod validate;

/// snapshot → 영속 문서 레코드 역투영 (XIV.5.4).
pub mod records;

/// Frame 가이드·레이아웃 그리드와 View 변수·Variant의 snapshot 투영 (ADR-0126).
pub mod guides;

mod text_styles;

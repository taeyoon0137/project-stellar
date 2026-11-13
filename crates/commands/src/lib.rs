//! stellar-commands — Project Stellar 편집 command 모델 (Phase VII).
//!
//! 의존 방향은 `history -> commands -> document -> core-types`입니다. 문서 편집은 사용자·
//! 시스템·migration·향후 agent 모두 이 crate의 같은 command 경로를 지납니다(기술 아키텍처
//! §31.4).

/// command 공개 API가 노출하는 문서 모델 타입(`DocumentModel`, `NodeRecord` 등)을 하위 crate
/// (history)가 직접 의존 edge 없이 이름 붙일 수 있도록 재수출합니다.
pub use stellar_document as document;

/// command 식별자와 출처 (VII.1.1).
pub mod id;

/// command 종류 numeric registry (VII.1.2).
pub mod kind;

/// 직렬화 가능한 closed enum command와 validate·apply·invert 인터페이스 (VII.1.2).
pub mod command;

/// command 적용 context (VII.1.2).
pub mod edit;

/// command canonical 직렬화 (VII.1.2).
pub mod codec;

/// command 대상 문서 상태(모델 + revision) (VII.1.3).
pub mod state;

/// base revision과 precondition을 포함한 command 요청 (VII.1.3).
pub mod request;

/// command 결과 change set (VII.1.4).
pub mod change;

/// 변경 전 상태로 되돌리는 transaction guard (VII.1.5).
pub mod guard;

/// 전부 성공하거나 전부 롤백하는 `CommandTransaction` (VII.1.6).
pub mod transaction;

/// command 검증용 문서 조회 helper (VII.2).
pub(crate) mod tree;

/// node 구조 command: InsertNode·DeleteSubtree (VII.2.1~).
pub mod node_ops;

/// `SetProperty` command와 속성 값 (VII.2.4).
pub mod property;

/// modifier stack command (VII.2.6).
pub mod modifier_ops;

/// node 종류 변환 command (VII.2.7).
pub mod convert;

/// `DuplicateSubtree` command (VII.2.8).
pub mod duplicate;

/// 창별 선택 상태와 selection command (VII.3).
pub mod selection;

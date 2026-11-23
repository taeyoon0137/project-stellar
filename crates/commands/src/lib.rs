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

/// 본문과 스타일을 함께 갱신하는 Text node 편집.
pub mod text_edit;

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

/// 속성 패널 값의 CSS 값 문법 parser·formatter (UX.2.2).
pub mod css_value;

/// 속성 패널 표면: 선택 행 snapshot과 CSS 값 편집 (UX.2.2).
pub mod inspector;

/// node 잠금 판정(D8): 속성 패널이 쓰고, Layers·CLI도 이 규칙으로 맞춥니다.
pub mod lock;

/// modifier stack command (VII.2.6).
pub mod modifier_ops;

/// node 종류 변환 command (VII.2.7).
pub mod convert;

/// `DuplicateSubtree` command (VII.2.8).
pub mod duplicate;

/// Page·View 추가·삭제·이름 변경 command (UX.12.1, ADR-0079).
pub mod page_view;

/// 외부에서 바뀐 문서 원본을 command 하나로 반영 (UX.4.1, ADR-0079).
pub mod snapshot_apply;

/// Path point 편집 command (XII.10.28, ADR-0079).
pub mod path_edit;

/// Component Context 편집 command (XXX.7.8, ADR-0099).
pub mod context;

/// 테스트 전용 Context 편집 command 생성기 (XXX.7.8). 프로덕션 경로에서 사용하지 않습니다.
#[cfg(any(test, feature = "test-support"))]
mod context_test_support;

/// 기본값 세트 선택과 명시 layout longhand command (SD.4, ADR-0086).
pub mod style_defaults;

/// 에이전트 승인 batch 기록: 역 transaction 보존과 값 지문 (XIX.6.1, ADR-0100).
pub mod agent_batch;

/// 에이전트 변경 되돌리기 계획과 충돌 판정 (XIX.6.8, XIX.6.9, ADR-0100).
pub mod agent_revert;

/// 에이전트 작업 그룹과 창 history 연결 지점 (XIX.6.2, ADR-0100).
pub mod agent_sink;

/// 외부 변경 batch와 에이전트 lease 겹침 판정 (XIX.7.1, ADR-0100).
pub mod agent_external;

/// 원본과 독립적인 subtree 클립보드와 Paste 계획.
pub mod paste;

/// 창별 선택 상태와 selection command (VII.3).
pub mod selection;

/// 창별 열린 탭·active view·viewport (XII.2.5).
pub mod view_tabs;

/// 테스트 전용 command 생성기 (VII.6). 프로덕션 경로에서 사용하지 않습니다.
#[doc(hidden)]
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

/// command 왕복 property test (VII.6.1).
#[cfg(test)]
mod property_tests;

/// 테스트 전용 command 실행 fuzz target (VII.6.3). 프로덕션 경로에서 사용하지 않습니다.
#[doc(hidden)]
#[cfg(any(test, feature = "test-support"))]
pub mod fuzz;

/// 테스트 전용 command sequence fuzz target과 장기 실행 harness (XVI.6.7). 프로덕션 경로에서 사용하지
/// 않습니다.
#[doc(hidden)]
#[cfg(any(test, feature = "test-support"))]
pub mod sequence_fuzz;

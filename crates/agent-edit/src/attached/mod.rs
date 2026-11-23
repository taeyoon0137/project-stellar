//! attached backend의 에이전트 편집 경로 (XIX.5.1~XIX.5.3, ADR-0093).
//!
//! 앱 창 세션이 가진 문서 상태([`DocumentState`](stellar_ops::commands::state::DocumentState)) 위에서 에이전트
//! proposal을 다룹니다.
//!
//! 1. proposal 입력은 headless dry-run과 같은 해석 결과(`stellar_ops::apply::ApplyRequest`, 브리지 쪽 해석기는
//!    `stellar_mcp::proposal::parse_transaction`)를 같은 닫힌 enum 변환(`decode_commands`)으로 command로 만듭니다(XIX.5.1).
//! 2. [`edit::preview`]는 문서 사본에 적용해 semantic diff·validation·lease 판정을 만들고, 문서 상태와 revision을
//!    바꾸지 않습니다(XIX.5.3). 사용자는 이 결과를 보고 승인합니다.
//! 3. [`edit::apply_approved`]는 같은 transaction을 lease 판정과 함께 적용합니다. lease 범위 밖 StableId를 바꾸면
//!    문서를 바꾸지 않고 거부합니다(XIX.5.2).
//!
//! 판정은 `stellar_commands::lock::lease`(사용자 편집 validator와 같은 함수)이고, diff·validation은 ops 공개
//! 함수입니다. 이 module은 이것들을 잇기만 합니다.

/// proposal 해석, 승인 전 미리보기, 승인된 적용.
pub mod edit;

/// 에이전트 변경 되돌리기 미리보기와 적용 (XIX.6.8, XIX.6.9, ADR-0100).
pub mod revert;

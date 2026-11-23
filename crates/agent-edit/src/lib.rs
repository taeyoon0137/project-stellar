//! stellar-agent-edit — 앱이 소유하는 에이전트 편집 구현 (XXIX.3.1 엔진 부분, ADR-0112).
//!
//! 에이전트가 앱 창 문서를 편집하는 동안 앱이 지켜야 하는 상태와 절차를 모읍니다. 원래 MCP 브리지 library
//! (mcp-core)에 임시로 두었던 module을 ADR-0093 결정 12와 ADR-0100 결정 3대로 옮겼습니다. 판정·기록 함수는
//! commands crate(`stellar_commands::{lock, agent_batch, agent_revert, agent_sink, agent_external}`)에 그대로 있어
//! 옮겨도 동작이 바뀌지 않습니다.
//!
//! | module | 내용 |
//! |---|---|
//! | [`permission`] | MCP 세션 권한 저장과 판정(XIX.3.2~XIX.3.4) |
//! | [`lease`] | lease 요청·보유 관리와 문서 기준 대상 확인(XIX.4.2, XIX.4.5~XIX.4.7) |
//! | [`attached`] | 승인 전 미리보기·승인된 적용(XIX.5.2, XIX.5.3), 되돌리기(XIX.6.8, XIX.6.9) |
//! | [`provenance`] | 출처 색인과 표시 상태 전이(XIX.6.7, XIX.6.10) |
//! | [`agent_history`] | 세션 완료·중지·종료 절차와 외부 변경 처리(XIX.6.2~XIX.6.6, XIX.7) |
//!
//! crate 경계(ADR-0112): runtime이 이 crate에 의존합니다. 에이전트 경로 crate(mcp-core, stellar-mcp,
//! agent-protocol, local-ipc, stellar-cli, agent-host)는 이 crate에 직접·전이로 닿지 않으므로(`FORBIDDEN_REACH`),
//! 에이전트 경로에서는 권한 응답, 출처 표시 정리, 되돌리기를 부를 수 없습니다(ADR-0005 §3). 이 crate도 MCP·runtime·
//! agent-host에 닿지 않습니다. 모든 상태는 창·앱 메모리에만 있고 직렬화 경로가 없습니다.

/// L2 공용 정의(권한 어휘·결과 종류·catalog) crate.
pub use stellar_agent_protocol as agent_protocol;

/// MCP 세션 권한 저장과 판정 (XIX.3.2~XIX.3.4, ADR-0093).
pub mod permission;

/// 에이전트 편집 lease 요청·보유 관리 (XIX.4.2, XIX.4.5~XIX.4.7, ADR-0093).
pub mod lease;

/// attached 편집 경로: 승인 전 미리보기·승인된 적용과 되돌리기 (XIX.5.2, XIX.5.3, XIX.6.8, XIX.6.9).
pub mod attached;

/// 에이전트 변경 출처 색인과 표시 상태 전이 (XIX.6.7, XIX.6.10, ADR-0100).
pub mod provenance;

/// 에이전트 출처·세션·되돌리기 test 공용 fixture.
#[cfg(test)]
pub(crate) mod provenance_fixture;

/// 에이전트 편집 세션의 완료·중지·종료 절차와 외부 변경 처리 (XIX.6.2~XIX.6.6, XIX.7, ADR-0100).
pub mod agent_history;

/// 20만 node 문서 lease·batch 단일 점유 측정(release, 기본 실행 제외).
#[cfg(test)]
mod occupancy_trace;

//! stellar-runtime — 앱 프로세스 runtime: thread 구성과 document thread event loop.
//!
//! 아키텍처 §9.2의 앱 내부 thread 모델(Native UI, Document, Render, Worker Pool)을 코어 쪽에서
//! 구성합니다. 네이티브 셸은 안정 C ABI(`stellar-ffi`)를 거쳐 이 crate에 도달하고, 이 crate는
//! ffi를 역참조하지 않습니다. crate 경계와 thread 모델 결정은 ADR-0013에 기록합니다.

/// DocumentThread event loop (XII.1.1).
pub mod document_thread;

/// UI→DocumentThread bounded command queue와 입력 우선순위 (XII.1.2).
pub mod command_queue;

/// RenderThread와 최신 scene snapshot lock-free handoff (XII.1.3).
pub mod render_thread;

/// worker pool: 작업 class와 priority tag (XII.1.4).
pub mod worker_pool;

/// worker result 출처(source revision·input hash)와 stale result 거부 (XII.1.5).
pub mod worker_result;

pub mod input_trace;

/// 셸 경계용 앱 runtime: 프로젝트·문서 session, 완료 event, 상태 snapshot.
pub mod app;

/// 창 하나의 프로젝트 편집 session (XII.2.1).
pub mod window_session;

/// 창 session 명령 (XII.6.4).
pub mod app_command;

/// 창 session을 앱 runtime 경계로 다루는 테스트.
#[cfg(test)]
mod app_window_tests;

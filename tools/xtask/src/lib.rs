#![allow(missing_docs)] // 내부 도구 crate: 공개 API 아님 (II.1.4)
//! xtask library — Project Stellar 검증 도구 모듈.
//!
//! 부트스트랩 단계의 작업 추적·정책 검증 로직을 모듈로 노출합니다.
//! binary(`main.rs`)는 이 라이브러리의 명령을 호출합니다(I.4.1 이후).

pub mod adr_policy;
pub mod check_core;
pub mod chunk_bench;
pub mod commands;
pub mod commit_message;
pub mod crate_graph;
pub mod dependencies;
pub mod ffi_header;
pub mod plan;
pub mod pr_body;
pub mod storage_fixtures;
pub mod sync;
pub mod task_report;
pub mod task_status;
pub mod versions;
pub mod yaml;

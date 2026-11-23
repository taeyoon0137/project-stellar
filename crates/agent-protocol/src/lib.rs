//! stellar-agent-protocol — L2 MCP 경계의 공용 정의 (XIX.9.1, ADR-0005 §3·§8, ADR-0085).
//!
//! `stellar-mcp-core`(MCP stdio 서버)와 이후 attached host(`stellar-runtime`이 구현할 host trait)가 함께
//! 쓰는 정의만 둡니다. JSON 직렬화와 backend 연결은 mcp-core가 맡고, 이 crate는 Rust 값과 순수 판정만
//! 가집니다.
//!
//! - 의존 방향: core-types만 직접 의존합니다. commands·document·ops·agent-host에는 전이적으로도
//!   도달하지 않습니다(`cargo xtask validate-crate-graph`).
//! - 네트워크, 파일 쓰기, 프로세스 실행이 없습니다.

/// core 값 타입(직접 edge).
pub use stellar_core_types as core_types;

/// 권한 어휘 Read·Propose·Apply·Terminal·FileDelete·GitCommit·GitPush (XIX.3.1).
pub mod permission;

/// MCP resource·tool catalog 정의 (XIX.2, XIX.9.2).
pub mod catalog;

/// xtask가 생성한 schema 산출물 (XIX.9.2).
pub mod schema;

/// MCP `initialize`의 사용 안내 (XIX.9.2).
pub mod instructions;

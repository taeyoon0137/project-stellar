//! stellar-plugin-host — 플러그인 코어 (Phase XXI, ADR-0097).
//!
//! WebView·셸 없이 검증할 수 있는 플러그인 엔진 부분입니다.
//!
//! - [`manifest`]: `.stellar-plugin` package의 `plugin.yaml` 형식(XXI.1.1).
//! - [`path`]: package 상대 경로 문법(XXI.1.1·XXI.1.2).
//! - [`permission`]: 닫힌 권한 enum(XXI.1.1, XXI.4.5).
//! - [`package`]: `.stellar-plugin` package 검증과 플러그인 디렉터리 훑기(XXI.1.2).
//! - [`admission`]: 서명 gate와 release·development 적재 정책(XXI.1.3).
//! - [`idl`]·[`bindgen`]·[`generated`]: versioned Host API IDL과 생성 Rust·TypeScript binding(XXI.4.1).
//! - [`host`]: namespace별 Host API dispatcher. 기존 ops 공개 함수만 부릅니다(XXI.4.2).
//! - [`plugin_data`]: 플러그인 전용 `plugins/<plugin-id>/` 디렉터리(XXI.6.2).
//! - [`rpc`]: RPC 오류·상한(XXI.4.3).
//! - [`session`]: 비동기 request/response·batch session과 호출량 상한(XXI.4.3).
//!
//! 의존 방향: ops(Host API가 부르는 L1 공용 진입점)와 project(제한 YAML loader)만 직접 의존합니다. MCP·agent
//! crate와 FFI에는 도달하지 않습니다. 앱 프로세스에 플러그인 native library를 적재하지 않습니다(기술 아키텍처 §33.1).

/// 엄격 JSON reader·writer(mcp-core 사본).
pub mod json;

/// `plugin.yaml` manifest 형식.
pub mod manifest;

/// package 상대 경로.
pub mod path;

/// 플러그인 권한.
pub mod permission;

/// package 검증.
pub mod package;

/// 적재 gate.
pub mod admission;

/// 이 빌드가 제공하는 Host API major version(IDL `host_api_version`).
pub use generated::host_api::HOST_API_VERSION;

/// Host API IDL 해석.
pub mod idl;

/// IDL → Rust·TypeScript binding 생성.
pub mod bindgen;

/// 생성 binding.
pub mod generated;

/// Host API dispatcher.
pub mod host;

/// 플러그인 data namespace.
pub mod plugin_data;

/// RPC 오류·상한.
pub mod rpc;

/// 비동기 RPC session.
pub mod session;

/// 생성 파일 목록(schema·binding). bindgen과 drift test가 같은 표를 씁니다.
pub mod generated_files;

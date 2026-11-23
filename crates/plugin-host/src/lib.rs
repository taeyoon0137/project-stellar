//! stellar-plugin-host — 플러그인 코어 (Phase XXI, ADR-0097).
//!
//! WebView·셸 없이 검증할 수 있는 플러그인 엔진 부분입니다.
//!
//! - [`manifest`]: `.stellar-plugin` package의 `plugin.yaml` 형식(XXI.1.1).
//! - [`path`]: package 상대 경로 문법(XXI.1.1·XXI.1.2).
//! - [`permission`]: 닫힌 권한 enum(XXI.1.1, XXI.4.5).
//! - [`package`]: `.stellar-plugin` package 검증과 플러그인 디렉터리 훑기(XXI.1.2).
//! - [`admission`]: 서명 gate와 release·development 적재 정책(XXI.1.3).
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

/// 이 빌드가 제공하는 Host API major version.
pub const HOST_API_VERSION: u32 = 1;

/// 생성 파일 목록(schema·binding). bindgen과 drift test가 같은 표를 씁니다.
pub mod generated_files;

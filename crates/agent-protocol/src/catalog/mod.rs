//! MCP resource·tool catalog의 Rust 정의 (XIX.2, XIX.9.2).
//!
//! - [`resources`]: read resource와 내용 schema (XIX.2.1).
//! - [`tools`]: tool 정의와 annotation, 필요 권한 (XIX.2.3), namespace와 기본 비활성 (XIX.2.4).
//!
//! MCP `resources/list`·`resources/templates/list`·`tools/list` 결과는 mcp-core가 이 정의에서 생성합니다.
//! 수기 목록을 따로 두지 않습니다.

/// read resource 정의 (XIX.2.1).
pub mod resources;

/// tool 정의 (XIX.2.3), namespace (XIX.2.4).
pub mod tools;

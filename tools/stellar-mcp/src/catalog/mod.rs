//! MCP resource·tool catalog 정의 (XIX.2).
//!
//! - [`resources`]: read resource와 내용 schema (XIX.2.1).
//! - [`paging`]: resource 응답의 revision과 page cursor (XIX.2.2).
//!
//! 이 모듈은 정의와 순수 판정만 담습니다. backend(headless XIX.9.3, attached XIX.9.4) 연결 전이라
//! 서버는 아직 이 catalog를 `resources/list`·`tools/list`로 광고하지 않습니다. 정의는 XIX.9.2에서
//! `stellar-agent-protocol`로 옮겨 생성합니다.

/// read resource 정의 (XIX.2.1).
pub mod resources;

/// resource 응답 revision과 page cursor (XIX.2.2).
pub mod paging;

use crate::json::{self, JsonLimits, JsonValue};

/// catalog 전체 snapshot(정의 golden fixture용): resource 정의와 내용 schema.
pub fn catalog_snapshot() -> JsonValue {
    let resources = resources::RESOURCES
        .iter()
        .map(|def| {
            JsonValue::object(vec![
                ("name", JsonValue::string(def.name)),
                ("title", JsonValue::string(def.title)),
                ("description", JsonValue::string(def.description)),
                (
                    if def.template { "uriTemplate" } else { "uri" },
                    JsonValue::string(def.uri),
                ),
                ("mimeType", JsonValue::string(resources::MIME_JSON)),
                ("availability", JsonValue::string(def.availability.name())),
                ("paginated", JsonValue::Bool(def.paginated)),
                (
                    "contentSchema",
                    json::parse(def.content_schema, JsonLimits::default())
                        .unwrap_or(JsonValue::Null),
                ),
            ])
        })
        .collect();
    JsonValue::object(vec![("resources", JsonValue::Array(resources))])
}

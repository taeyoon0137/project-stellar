//! MCP resource·tool catalog 정의 (XIX.2).
//!
//! - [`resources`]: read resource와 내용 schema (XIX.2.1).
//! - [`paging`]: resource 응답의 revision과 page cursor (XIX.2.2).
//! - [`tools`]: tool 정의와 annotation, 필요 권한 (XIX.2.3), 문서·터미널·Git namespace 분리와
//!   기본 비활성 (XIX.2.4).
//! - [`scope`]: 요청 URI·tool 인자·응답 payload의 승인 프로젝트·세션 범위 제한 (XIX.2.5).
//!
//! 이 모듈은 정의와 순수 판정만 담습니다. headless backend(XIX.9.3)는 구현한 구조·문서 resource와 저장 문서 검증 tool을
//! 이 정의에서 골라 광고합니다. 나머지 연결과 agent-protocol로의 이동(XIX.9.2)은 후속입니다.

/// read resource 정의 (XIX.2.1).
pub mod resources;

/// resource 응답 revision과 page cursor (XIX.2.2).
pub mod paging;

/// tool 정의 (XIX.2.3), namespace (XIX.2.4).
pub mod tools;

/// payload의 프로젝트·세션 범위 제한 (XIX.2.5).
pub mod scope;

use crate::json::{self, JsonLimits, JsonValue};

/// catalog 전체 snapshot(정의 golden fixture용): resource 정의·내용 schema, tool 정의·필요 권한.
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
    let tools = tools::all_tools()
        .map(|tool| {
            let mut fields = match tools::descriptor(tool) {
                JsonValue::Object(fields) => fields,
                other => vec![("descriptor".to_string(), other)],
            };
            fields.push((
                "availability".to_string(),
                JsonValue::string(tool.availability.name()),
            ));
            fields.push(("grant".to_string(), JsonValue::string(tool.grant.name())));
            fields.push((
                "namespace".to_string(),
                JsonValue::string(
                    tools::ToolNamespace::of(tool.name).map_or("", tools::ToolNamespace::prefix),
                ),
            ));
            fields.push((
                "enabledByDefault".to_string(),
                JsonValue::Bool(
                    tools::ToolNamespace::of(tool.name)
                        .is_some_and(|n| tools::EnabledNamespaces::default().includes(n)),
                ),
            ));
            JsonValue::Object(fields)
        })
        .collect();
    JsonValue::object(vec![
        ("resources", JsonValue::Array(resources)),
        ("tools", JsonValue::Array(tools)),
    ])
}

//! MCP read resource 목록 JSON (XIX.2.1, XIX.9.2).
//!
//! 정의는 `stellar-agent-protocol`(`catalog::resources`)에 있고, 이 모듈은 그 정의에서 MCP `resources/list`·
//! `resources/templates/list` 항목을 만듭니다.

pub use stellar_agent_protocol::catalog::resources::*;

use crate::json::JsonValue;

pub(crate) fn descriptor(def: &ResourceDef) -> JsonValue {
    let uri_key = if def.template { "uriTemplate" } else { "uri" };
    describe(def, uri_key, def.uri)
}

fn describe(def: &ResourceDef, key: &'static str, uri: &str) -> JsonValue {
    JsonValue::object(vec![
        (key, JsonValue::string(uri)),
        ("name", JsonValue::string(def.name)),
        ("title", JsonValue::string(def.title)),
        ("description", JsonValue::string(def.description)),
        ("mimeType", JsonValue::string(MIME_JSON)),
    ])
}

/// 고정 목록의 원래 URI도 유지하면서 페이지를 넘기는 query 문법을 광고합니다.
pub(crate) fn template_descriptor(def: &ResourceDef) -> JsonValue {
    if def.template {
        descriptor(def)
    } else {
        describe(def, "uriTemplate", &format!("{}{{?cursor,limit}}", def.uri))
    }
}

/// MCP `resources/list` 결과(`backend`가 제공하는 고정 URI resource).
pub fn resources_list(backend: Backend) -> JsonValue {
    JsonValue::object(vec![(
        "resources",
        JsonValue::Array(
            RESOURCES
                .iter()
                .filter(|r| !r.template && r.availability.includes(backend))
                .map(descriptor)
                .collect(),
        ),
    )])
}

/// MCP `resources/templates/list` 결과(`backend`가 제공하는 URI template).
pub fn templates_list(backend: Backend) -> JsonValue {
    JsonValue::object(vec![(
        "resourceTemplates",
        JsonValue::Array(
            RESOURCES
                .iter()
                .filter(|r| (r.template || r.paginated) && r.availability.includes(backend))
                .map(template_descriptor)
                .collect(),
        ),
    )])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{parse, JsonLimits};

    #[test]
    fn every_architecture_resource_is_defined_once() {
        // 아키텍처 §31.2 대표 Resources + XIX.2.1 목록.
        for name in [
            "project-structure",
            "current-document",
            "selection",
            "components",
            "tokens",
            "target-profiles",
            "git-status",
        ] {
            assert_eq!(
                RESOURCES.iter().filter(|r| r.name == name).count(),
                1,
                "{name}"
            );
        }
        let mut uris: Vec<&str> = RESOURCES.iter().map(|r| r.uri).collect();
        uris.sort_unstable();
        uris.dedup();
        assert_eq!(uris.len(), RESOURCES.len(), "URI 고유");
    }

    #[test]
    fn schemas_are_valid_json_and_require_revision() {
        for def in RESOURCES {
            let schema = parse(def.content_schema, JsonLimits::default())
                .unwrap_or_else(|e| panic!("{}: {e}", def.name));
            assert_eq!(
                schema.get("type").and_then(JsonValue::as_str),
                Some("object"),
                "{}",
                def.name
            );
            let required: Vec<&str> = schema
                .get("required")
                .and_then(JsonValue::as_array)
                .unwrap()
                .iter()
                .filter_map(JsonValue::as_str)
                .collect();
            assert!(required.contains(&"revision"), "{}", def.name);
            if def.paginated {
                assert!(required.contains(&"next_cursor"), "{}", def.name);
                let template = template_descriptor(def);
                let uri = template
                    .get("uriTemplate")
                    .and_then(JsonValue::as_str)
                    .unwrap();
                assert!(uri.ends_with("{?cursor,limit}"), "{}", def.name);
            }
            assert!(def.uri.starts_with("stellar://"), "{}", def.name);
            assert!(!def.uri.contains("project/{"), "프로젝트 선택 segment 없음");
        }
    }

    #[test]
    fn headless_lists_exclude_attached_only_resources() {
        let headless = crate::json::write(&resources_list(Backend::Headless));
        assert!(!headless.contains("stellar://selection"));
        let attached = crate::json::write(&resources_list(Backend::Attached));
        assert!(attached.contains("stellar://selection"));
        let templates = crate::json::write(&templates_list(Backend::Headless));
        assert!(templates.contains("stellar://documents/{document}"));
        assert!(!templates.contains("document/current"));
    }
}

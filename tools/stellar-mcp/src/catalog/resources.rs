//! MCP read resource 정의 (XIX.2.1).
//!
//! 아키텍처 §31.2 대표 Resources(프로젝트 구조, 현재 문서와 선택 영역, 컴포넌트, 토큰, Target
//! Profile, Git 상태)를 MCP 명세 2025-11-25 "Resources"의 resource·resource template 형태로 정의합니다.
//! 모든 내용은 `application/json` 한 개 text content이고, 내용 JSON Schema(draft 2020-12)를 함께 둡니다.
//!
//! - URI scheme은 `stellar`이며 프로젝트를 고르는 segment가 없습니다. 모든 resource는 세션에 결합된
//!   승인 프로젝트 하나 범위입니다(XIX.2.5 [`super::scope`]).
//! - `availability`는 backend(ADR-0005 §3)별 제공 여부입니다. 선택 영역과 현재 문서는 창 세션이
//!   있어야 하므로 attached 전용입니다.
//! - 큰 내용(문서 트리)은 page 단위입니다(XIX.2.2 [`super::paging`]). 모든 내용에는 revision이 있습니다.
//!
//! 이 목록은 정의입니다. backend 연결(XIX.9.3, XIX.9.4)과 `stellar-agent-protocol`로의 이동(XIX.9.2)은
//! 후속 작업입니다. 서버는 아직 `resources` capability를 광고하지 않습니다.

use crate::json::JsonValue;

/// resource 내용 MIME type.
pub const MIME_JSON: &str = "application/json";

/// resource를 제공하는 backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// headless 읽기 backend와 attached backend 모두.
    All,
    /// attached backend(실행 중 앱의 창 세션)만.
    AttachedOnly,
}

impl Availability {
    /// 고정 이름.
    pub fn name(self) -> &'static str {
        match self {
            Availability::All => "all",
            Availability::AttachedOnly => "attached-only",
        }
    }

    /// `backend`에서 제공하는지.
    pub fn includes(self, backend: Backend) -> bool {
        match self {
            Availability::All => true,
            Availability::AttachedOnly => backend == Backend::Attached,
        }
    }
}

/// 세션 backend(ADR-0005 §3, 세션 시작 시 고정).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// 앱 없이 실행한 headless 읽기 backend.
    Headless,
    /// 실행 중 앱에 붙은 attached backend.
    Attached,
}

/// resource 정의 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceDef {
    /// MCP `name`(목록 안에서 고유).
    pub name: &'static str,
    /// MCP `title`.
    pub title: &'static str,
    /// MCP `description`.
    pub description: &'static str,
    /// 고정 URI(`template`이 거짓) 또는 RFC 6570 URI template.
    pub uri: &'static str,
    /// URI template인지(`resources/templates/list`).
    pub template: bool,
    /// 제공 backend.
    pub availability: Availability,
    /// page 단위 내용인지(XIX.2.2).
    pub paginated: bool,
    /// 내용 JSON Schema(draft 2020-12) 원문.
    pub content_schema: &'static str,
}

/// 공통 revision schema 조각(XXIV.4.2 revision 표기).
macro_rules! revision_schema {
    () => {
        r#"{"type":"object","additionalProperties":false,"required":["token","generation","manifest_hash"],"properties":{"token":{"type":"string","pattern":"^g(0|[1-9][0-9]*(:sha256:[0-9a-f]{64}))$"},"generation":{"type":"integer","minimum":0},"manifest_hash":{"type":["string","null"]}}}"#
    };
}

/// 정의된 read resource(고정 순서).
pub const RESOURCES: &[ResourceDef] = &[
    ResourceDef {
        name: "project-structure",
        title: "Project structure",
        description: "문서·페이지·View 개요와 저장 chunk 요약",
        uri: "stellar://project/structure",
        template: false,
        availability: Availability::All,
        paginated: false,
        content_schema: concat!(
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"required":["revision","documents","chunks"],"properties":{"revision":"#,
            revision_schema!(),
            r#","documents":{"type":"array","items":{"type":"object","required":["id","name","pages"],"properties":{"id":{"type":"string"},"name":{"type":"string"},"pages":{"type":"array"}}}},"chunks":{"type":"array","items":{"type":"object","required":["kind","code","count"]}}}}"#
        ),
    },
    ResourceDef {
        name: "current-document",
        title: "Current document",
        description: "결합된 창의 현재 문서 트리 한 page(깊이 우선, cursor 분할)",
        uri: "stellar://document/current{?cursor,limit}",
        template: true,
        availability: Availability::AttachedOnly,
        paginated: true,
        content_schema: concat!(
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"required":["revision","document","total","offset","limit","items","next_cursor"],"properties":{"revision":"#,
            revision_schema!(),
            r#","document":{"type":"string"},"total":{"type":"integer","minimum":0},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1},"items":{"type":"array","items":{"type":"object","required":["id","parent","depth","kind"]}},"next_cursor":{"type":["string","null"]}}}"#
        ),
    },
    ResourceDef {
        name: "document",
        title: "Document",
        description: "문서 하나의 트리 한 page(깊이 우선, cursor 분할)",
        uri: "stellar://documents/{document}{?cursor,limit}",
        template: true,
        availability: Availability::All,
        paginated: true,
        content_schema: concat!(
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"required":["revision","document","total","offset","limit","items","next_cursor"],"properties":{"revision":"#,
            revision_schema!(),
            r#","document":{"type":"string"},"total":{"type":"integer","minimum":0},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1},"items":{"type":"array","items":{"type":"object","required":["id","parent","depth","kind"]}},"next_cursor":{"type":["string","null"]}}}"#
        ),
    },
    ResourceDef {
        name: "selection",
        title: "Selection",
        description: "결합된 창의 현재 선택 node 목록",
        uri: "stellar://selection",
        template: false,
        availability: Availability::AttachedOnly,
        paginated: false,
        content_schema: concat!(
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"required":["revision","document","nodes"],"properties":{"revision":"#,
            revision_schema!(),
            r#","document":{"type":["string","null"]},"nodes":{"type":"array","items":{"type":"string"}}}}"#
        ),
    },
    ResourceDef {
        name: "components",
        title: "Components",
        description: "component library 항목",
        uri: "stellar://components",
        template: false,
        availability: Availability::All,
        paginated: false,
        content_schema: concat!(
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"required":["revision","chunk_count","entries"],"properties":{"revision":"#,
            revision_schema!(),
            r#","chunk_count":{"type":"integer","minimum":0},"entries":{"type":"array"}}}"#
        ),
    },
    ResourceDef {
        name: "tokens",
        title: "Tokens",
        description: "token 집합 항목",
        uri: "stellar://tokens",
        template: false,
        availability: Availability::All,
        paginated: false,
        content_schema: concat!(
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"required":["revision","chunk_count","entries"],"properties":{"revision":"#,
            revision_schema!(),
            r#","chunk_count":{"type":"integer","minimum":0},"entries":{"type":"array"}}}"#
        ),
    },
    ResourceDef {
        name: "target-profiles",
        title: "Target profiles",
        description: "stellar.yaml의 Target Profile 목록",
        uri: "stellar://target-profiles",
        template: false,
        availability: Availability::All,
        paginated: false,
        content_schema: concat!(
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"required":["revision","profiles"],"properties":{"revision":"#,
            revision_schema!(),
            r#","profiles":{"type":"array","items":{"type":"object","required":["id","platform","minimum_os","baseline"]}}}}"#
        ),
    },
    ResourceDef {
        name: "git-status",
        title: "Git status",
        description: "작업 트리 Git 상태 요약(경로·URL 없이 semantic 변경 수)",
        uri: "stellar://git/status",
        template: false,
        availability: Availability::All,
        paginated: false,
        content_schema: concat!(
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","additionalProperties":false,"required":["revision","branch","clean","staged","unstaged","conflicted"],"properties":{"revision":"#,
            revision_schema!(),
            r#","branch":{"type":["string","null"]},"clean":{"type":"boolean"},"staged":{"type":"integer","minimum":0},"unstaged":{"type":"integer","minimum":0},"conflicted":{"type":"integer","minimum":0}}}"#
        ),
    },
];

/// 이름으로 정의를 찾습니다.
pub fn resource(name: &str) -> Option<&'static ResourceDef> {
    RESOURCES.iter().find(|r| r.name == name)
}

fn descriptor(def: &ResourceDef) -> JsonValue {
    let uri_key = if def.template { "uriTemplate" } else { "uri" };
    JsonValue::object(vec![
        (uri_key, JsonValue::string(def.uri)),
        ("name", JsonValue::string(def.name)),
        ("title", JsonValue::string(def.title)),
        ("description", JsonValue::string(def.description)),
        ("mimeType", JsonValue::string(MIME_JSON)),
    ])
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
                .filter(|r| r.template && r.availability.includes(backend))
                .map(descriptor)
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
                assert!(def.template, "page resource는 cursor 인자를 받는 template");
                assert!(def.uri.ends_with("{?cursor,limit}"), "{}", def.name);
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

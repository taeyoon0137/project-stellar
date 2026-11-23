//! MCP tool 목록 JSON (XIX.2.3, XIX.9.2).
//!
//! 정의(이름·annotation·backend·필요 권한·namespace)는 `stellar-agent-protocol`(`catalog::tools`)에 있고, 이
//! 모듈은 그 정의에서 MCP `tools/list` 항목을 만듭니다. `document.propose_commands`·`document.apply_commands`의
//! `inputSchema`는 xtask가 commands closed enum에서 생성한 `schemas/command-transaction.schema.json`입니다.

pub use stellar_agent_protocol::catalog::tools::*;

/// tool이 요구하는 권한(아키텍처 §31.3 독립 권한, XIX.3.1). 어휘는 `stellar-agent-protocol`이 정의합니다.
pub use stellar_agent_protocol::permission::Permission as RequiredGrant;

use super::resources::Backend;
use crate::json::{self, JsonLimits, JsonValue};

/// MCP `tools/list`의 tool 항목.
pub fn descriptor(tool: &ToolDef) -> JsonValue {
    JsonValue::object(vec![
        ("name", JsonValue::string(tool.name)),
        ("title", JsonValue::string(tool.title)),
        ("description", JsonValue::string(tool.description)),
        (
            "inputSchema",
            json::parse(tool.input_schema, JsonLimits::default()).unwrap_or(JsonValue::Null),
        ),
        (
            "annotations",
            JsonValue::object(vec![
                ("title", JsonValue::string(tool.title)),
                ("readOnlyHint", JsonValue::Bool(tool.read_only)),
                ("destructiveHint", JsonValue::Bool(tool.destructive)),
                ("idempotentHint", JsonValue::Bool(tool.idempotent)),
                ("openWorldHint", JsonValue::Bool(false)),
            ]),
        ),
    ])
}

/// MCP `tools/list` 결과.
pub fn tools_list(backend: Backend, enabled: EnabledNamespaces) -> JsonValue {
    JsonValue::object(vec![(
        "tools",
        JsonValue::Array(listed_tools(backend, enabled).map(descriptor).collect()),
    )])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_input_schema_is_a_strict_json_object() {
        // 검증 기준(XIX.2.3): tool inventory의 input schema가 엄격 JSON object.
        for tool in all_tools() {
            let schema = json::parse(tool.input_schema, JsonLimits::default())
                .unwrap_or_else(|e| panic!("{}: {e}", tool.name));
            assert_eq!(
                schema.get("type").and_then(JsonValue::as_str),
                Some("object"),
                "{}",
                tool.name
            );
            assert_eq!(
                schema.get("additionalProperties"),
                Some(&JsonValue::Bool(false)),
                "{}",
                tool.name
            );
        }
    }

    #[test]
    fn headless_tools_list_has_no_write_tool() {
        let listed = json::write(&tools_list(
            Backend::Headless,
            EnabledNamespaces::default().with_git().with_terminal(),
        ));
        for write in [
            "document.apply_commands",
            "document.paste_css",
            "document.render_preview",
            "git.stage",
            "git.unstage",
            "git.commit",
            "git.push",
            "terminal.run",
        ] {
            assert!(!listed.contains(&format!("\"{write}\"")), "{write}");
        }
        assert!(!listed.contains("\"destructiveHint\":true"));
    }
}

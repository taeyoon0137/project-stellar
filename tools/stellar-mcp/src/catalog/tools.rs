//! MCP tool 정의 (XIX.2.3).
//!
//! 문서 tool을 다섯 개로 분리합니다: semantic diff 생성, render preview 요청, 문서 검증, command
//! 제안, command 적용. MCP 명세 2025-11-25 "Tools"의 `name`·`title`·`description`·`inputSchema`·
//! `annotations`를 갖습니다.
//!
//! - **이름**: 명세 이름 규칙(1~128자, ASCII 영문·숫자·`_`·`-`·`.`, 대소문자 구분, 서버 안 고유)을
//!   따르고 `<namespace>.<동작>` 형태입니다([`valid_tool_name`]).
//! - **annotation**: 읽기 계열은 `readOnlyHint: true`, 문서를 바꾸는 적용은 `destructiveHint: true`입니다.
//!   annotation은 client 표시용 힌트이며 권한 판정이 아닙니다. 허용 여부는 Stellar 승인(XIX.3)이 정합니다.
//! - **backend**: headless 읽기 backend는 읽기·검증·dry-run만 제공합니다(ADR-0005 §3). render preview는
//!   attached 전용(ADR-0005 §8), command 적용은 앱 승인·lease·DocumentThread가 필요하므로 attached
//!   전용입니다.
//! - **필요 권한**: 각 tool이 요구하는 권한 이름(아키텍처 §31.3 독립 권한)을 함께 선언합니다. 권한
//!   enum과 판정은 XIX.3.1이 정의합니다.

use super::resources::{Availability, Backend};
use crate::json::{self, JsonLimits, JsonValue};

/// tool이 요구하는 권한(아키텍처 §31.3 독립 권한 이름).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RequiredGrant {
    /// 읽기.
    Read,
    /// 변경 제안.
    Propose,
    /// 변경 적용.
    Apply,
}

impl RequiredGrant {
    /// 고정 이름.
    pub fn name(self) -> &'static str {
        match self {
            RequiredGrant::Read => "read",
            RequiredGrant::Propose => "propose",
            RequiredGrant::Apply => "apply",
        }
    }
}

/// tool 정의 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolDef {
    /// MCP `name`.
    pub name: &'static str,
    /// MCP `title`.
    pub title: &'static str,
    /// MCP `description`.
    pub description: &'static str,
    /// MCP `inputSchema`(JSON Schema 원문).
    pub input_schema: &'static str,
    /// `annotations.readOnlyHint`.
    pub read_only: bool,
    /// `annotations.destructiveHint`(읽기 전용이면 의미 없음, 거짓).
    pub destructive: bool,
    /// `annotations.idempotentHint`.
    pub idempotent: bool,
    /// 제공 backend.
    pub availability: Availability,
    /// 필요 권한.
    pub grant: RequiredGrant,
}

/// command 입력 schema 조각(XXIV.4.4 `stellar-apply` command 형식과 같음).
macro_rules! commands_schema {
    () => {
        r#"{"type":"array","minItems":1,"maxItems":10000,"items":{"type":"object","required":["kind"],"properties":{"kind":{"type":"string"}}}}"#
    };
}

/// 정의된 문서 tool(고정 순서).
pub const DOCUMENT_TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "document.semantic_diff",
        title: "Semantic diff",
        description: "현재 저장된 프로젝트와 Git 기준(기본 HEAD)의 semantic diff를 만듭니다",
        input_schema: r#"{"type":"object","additionalProperties":false,"properties":{"base":{"type":"string","description":"Git revision(기본 HEAD)"}}}"#,
        read_only: true,
        destructive: false,
        idempotent: true,
        availability: Availability::All,
        grant: RequiredGrant::Read,
    },
    ToolDef {
        name: "document.render_preview",
        title: "Render preview",
        description: "결합된 창에서 node 하나의 렌더 미리보기를 요청합니다",
        input_schema: r#"{"type":"object","additionalProperties":false,"required":["document","node"],"properties":{"document":{"type":"string"},"node":{"type":"string"},"scale":{"type":"number","exclusiveMinimum":0,"maximum":4}}}"#,
        read_only: true,
        destructive: false,
        idempotent: true,
        availability: Availability::AttachedOnly,
        grant: RequiredGrant::Read,
    },
    ToolDef {
        name: "document.validate",
        title: "Validate document",
        description: "저장된 문서를 문서 validator로 검증합니다(문서를 지정하지 않으면 전체)",
        input_schema: r#"{"type":"object","additionalProperties":false,"properties":{"document":{"type":"string"}}}"#,
        read_only: true,
        destructive: false,
        idempotent: true,
        availability: Availability::All,
        grant: RequiredGrant::Read,
    },
    ToolDef {
        name: "document.propose_commands",
        title: "Propose commands",
        description: "closed enum command 묶음을 검증하고 적용하지 않은 채 semantic diff를 돌려줍니다(dry-run)",
        input_schema: concat!(
            r#"{"type":"object","additionalProperties":false,"required":["base_revision","document","commands"],"properties":{"base_revision":{"type":"string"},"document":{"type":"string"},"commands":"#,
            commands_schema!(),
            r#"}}"#
        ),
        read_only: true,
        destructive: false,
        idempotent: true,
        availability: Availability::All,
        grant: RequiredGrant::Propose,
    },
    ToolDef {
        name: "document.apply_commands",
        title: "Apply commands",
        description: "승인된 command 묶음을 앱의 lease·승인·DocumentThread 경로로 적용합니다",
        input_schema: concat!(
            r#"{"type":"object","additionalProperties":false,"required":["base_revision","document","commands"],"properties":{"base_revision":{"type":"string"},"document":{"type":"string"},"commands":"#,
            commands_schema!(),
            r#"}}"#
        ),
        read_only: false,
        destructive: true,
        idempotent: false,
        availability: Availability::AttachedOnly,
        grant: RequiredGrant::Apply,
    },
];

/// MCP 명세 tool 이름 규칙(1~128자, `[A-Za-z0-9_.-]`).
pub fn valid_tool_name(name: &str) -> bool {
    (1..=128).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

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

/// `backend`에서 제공하는 문서 tool.
pub fn document_tools(backend: Backend) -> impl Iterator<Item = &'static ToolDef> {
    DOCUMENT_TOOLS
        .iter()
        .filter(move |t| t.availability.includes(backend))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_tools_are_the_five_planned_operations() {
        // 검증 기준(XIX.2.3): tool inventory 정확.
        let names: Vec<&str> = DOCUMENT_TOOLS.iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            [
                "document.semantic_diff",
                "document.render_preview",
                "document.validate",
                "document.propose_commands",
                "document.apply_commands"
            ]
        );
        for tool in DOCUMENT_TOOLS {
            assert!(valid_tool_name(tool.name), "{}", tool.name);
            let schema = json::parse(tool.input_schema, JsonLimits::default())
                .unwrap_or_else(|e| panic!("{}: {e}", tool.name));
            assert_eq!(
                schema.get("type").and_then(JsonValue::as_str),
                Some("object")
            );
            assert_eq!(
                schema.get("additionalProperties"),
                Some(&JsonValue::Bool(false)),
                "{}",
                tool.name
            );
            // 쓰기 tool만 destructive이고, destructive는 읽기 전용일 수 없습니다.
            assert_eq!(tool.destructive, !tool.read_only, "{}", tool.name);
        }
    }

    #[test]
    fn headless_backend_lists_no_write_or_render_tool() {
        let headless: Vec<&str> = document_tools(Backend::Headless).map(|t| t.name).collect();
        assert_eq!(
            headless,
            [
                "document.semantic_diff",
                "document.validate",
                "document.propose_commands"
            ]
        );
        assert!(document_tools(Backend::Headless).all(|t| t.read_only));
        let attached: Vec<&str> = document_tools(Backend::Attached).map(|t| t.name).collect();
        assert_eq!(attached.len(), 5);
    }

    #[test]
    fn tool_name_rule_follows_the_spec() {
        for ok in ["a", "document.apply_commands", "A-Z_0.9", &"x".repeat(128)] {
            assert!(valid_tool_name(ok), "{ok}");
        }
        for bad in ["", "has space", "슬래시/", "a:b", &"x".repeat(129)] {
            assert!(!valid_tool_name(bad), "{bad}");
        }
    }
}

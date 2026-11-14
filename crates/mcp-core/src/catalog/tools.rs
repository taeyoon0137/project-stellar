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
    /// 터미널 실행(XIX.2.4).
    Terminal,
    /// Git commit(XIX.2.4).
    GitCommit,
    /// Git push(XIX.2.4).
    GitPush,
}

impl RequiredGrant {
    /// 고정 이름.
    pub fn name(self) -> &'static str {
        match self {
            RequiredGrant::Read => "read",
            RequiredGrant::Propose => "propose",
            RequiredGrant::Apply => "apply",
            RequiredGrant::Terminal => "terminal",
            RequiredGrant::GitCommit => "git-commit",
            RequiredGrant::GitPush => "git-push",
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

/// 저장 문서 검증. headless가 실제 제공하는 읽기 tool입니다.
pub const VALIDATE_DOCUMENT: ToolDef = ToolDef {
    name: "document.validate",
    title: "Validate document",
    description:
        "저장된 문서의 검증 진단을 페이지로 읽습니다(문서를 지정하지 않으면 프로젝트 전체)",
    input_schema: r#"{"type":"object","additionalProperties":false,"properties":{"document":{"type":"string"},"cursor":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":2000}}}"#,
    read_only: true,
    destructive: false,
    idempotent: true,
    availability: Availability::All,
    grant: RequiredGrant::Read,
};

/// 고정 Git HEAD 대비 저장 문서와 프로젝트 공용 컴포넌트 변경.
pub const SEMANTIC_DIFF: ToolDef = ToolDef {
    name: "document.semantic_diff",
    title: "Semantic diff",
    description:
        "같은 프로젝트의 Git HEAD와 작업 트리의 저장 문서·컴포넌트 원본 변경을 페이지로 읽습니다",
    input_schema: r#"{"type":"object","additionalProperties":false,"properties":{"base":{"type":"string","enum":["HEAD"]},"cursor":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":2000}}}"#,
    read_only: true,
    destructive: false,
    idempotent: true,
    availability: Availability::All,
    grant: RequiredGrant::Read,
};

/// 정의된 문서 tool(고정 순서).
pub const DOCUMENT_TOOLS: &[ToolDef] = &[
    SEMANTIC_DIFF,
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
    VALIDATE_DOCUMENT,
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

/// tool namespace(XIX.2.4). 문서 편집 권한과 터미널·Git 권한을 결합하지 않도록(아키텍처 §31.3)
/// namespace마다 따로 켭니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolNamespace {
    /// 문서 조회·검증·제안·적용(`document.*`).
    Document,
    /// 터미널 실행(`terminal.*`).
    Terminal,
    /// Git commit·push(`git.*`).
    Git,
}

impl ToolNamespace {
    /// 모든 namespace(고정 순서).
    pub const ALL: [ToolNamespace; 3] = [
        ToolNamespace::Document,
        ToolNamespace::Terminal,
        ToolNamespace::Git,
    ];

    /// 이름 접두어(`<prefix>.<동작>`).
    pub fn prefix(self) -> &'static str {
        match self {
            ToolNamespace::Document => "document",
            ToolNamespace::Terminal => "terminal",
            ToolNamespace::Git => "git",
        }
    }

    /// tool 이름의 namespace.
    pub fn of(name: &str) -> Option<ToolNamespace> {
        let prefix = name.split_once('.')?.0;
        ToolNamespace::ALL
            .into_iter()
            .find(|n| n.prefix() == prefix)
    }
}

/// 켜진 namespace 집합. 기본(`Default`, 모든 field false)은 문서 namespace만입니다(XIX.2.4: 초기
/// commit·push 비활성). 터미널·Git은 [`EnabledNamespaces::with_terminal`]·[`EnabledNamespaces::with_git`]로
/// 명시적으로만 켭니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EnabledNamespaces {
    terminal: bool,
    git: bool,
}

impl EnabledNamespaces {
    /// 터미널 namespace를 켭니다(명시적 선택).
    pub fn with_terminal(mut self) -> EnabledNamespaces {
        self.terminal = true;
        self
    }

    /// Git namespace를 켭니다(명시적 선택).
    pub fn with_git(mut self) -> EnabledNamespaces {
        self.git = true;
        self
    }

    /// `namespace`가 켜졌는지. 문서 namespace는 항상 켜져 있습니다.
    pub fn includes(self, namespace: ToolNamespace) -> bool {
        match namespace {
            ToolNamespace::Document => true,
            ToolNamespace::Terminal => self.terminal,
            ToolNamespace::Git => self.git,
        }
    }
}

/// 터미널·Git namespace tool(기본 비활성, attached 전용).
pub const SYSTEM_TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "terminal.run",
        title: "Run in terminal",
        description: "결합된 창의 내장 터미널에서 명령을 실행합니다(사용자 확인 필요)",
        input_schema: r#"{"type":"object","additionalProperties":false,"required":["command"],"properties":{"command":{"type":"string","minLength":1}}}"#,
        read_only: false,
        destructive: true,
        idempotent: false,
        availability: Availability::AttachedOnly,
        grant: RequiredGrant::Terminal,
    },
    ToolDef {
        name: "git.commit",
        title: "Git commit",
        description: "stage된 변경으로 commit을 만듭니다(개별 확인 필요)",
        input_schema: r#"{"type":"object","additionalProperties":false,"required":["message"],"properties":{"message":{"type":"string","minLength":1}}}"#,
        read_only: false,
        destructive: true,
        idempotent: false,
        availability: Availability::AttachedOnly,
        grant: RequiredGrant::GitCommit,
    },
    ToolDef {
        name: "git.push",
        title: "Git push",
        description: "현재 branch를 설정된 remote로 push합니다(개별 확인 필요)",
        input_schema: r#"{"type":"object","additionalProperties":false,"properties":{}}"#,
        read_only: false,
        destructive: true,
        idempotent: false,
        availability: Availability::AttachedOnly,
        grant: RequiredGrant::GitPush,
    },
];

/// 모든 정의 tool(문서 → 터미널·Git 순).
pub fn all_tools() -> impl Iterator<Item = &'static ToolDef> {
    DOCUMENT_TOOLS.iter().chain(SYSTEM_TOOLS.iter())
}

/// `backend`·`enabled`에서 노출하는 tool.
pub fn listed_tools(
    backend: Backend,
    enabled: EnabledNamespaces,
) -> impl Iterator<Item = &'static ToolDef> {
    all_tools().filter(move |t| {
        t.availability.includes(backend)
            && ToolNamespace::of(t.name).is_some_and(|n| enabled.includes(n))
    })
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
    fn permission_matrix_separates_document_terminal_and_git() {
        // 검증 기준(XIX.2.4): permission matrix. (tool, namespace, 필요 권한)
        let matrix: Vec<(&str, ToolNamespace, RequiredGrant)> = all_tools()
            .map(|t| (t.name, ToolNamespace::of(t.name).unwrap(), t.grant))
            .collect();
        assert_eq!(
            matrix,
            [
                (
                    "document.semantic_diff",
                    ToolNamespace::Document,
                    RequiredGrant::Read
                ),
                (
                    "document.render_preview",
                    ToolNamespace::Document,
                    RequiredGrant::Read
                ),
                (
                    "document.validate",
                    ToolNamespace::Document,
                    RequiredGrant::Read
                ),
                (
                    "document.propose_commands",
                    ToolNamespace::Document,
                    RequiredGrant::Propose
                ),
                (
                    "document.apply_commands",
                    ToolNamespace::Document,
                    RequiredGrant::Apply
                ),
                (
                    "terminal.run",
                    ToolNamespace::Terminal,
                    RequiredGrant::Terminal
                ),
                ("git.commit", ToolNamespace::Git, RequiredGrant::GitCommit),
                ("git.push", ToolNamespace::Git, RequiredGrant::GitPush),
            ]
        );
        // 문서 namespace tool은 터미널·Git 권한을 요구하지 않고, 그 반대도 같습니다.
        for (name, namespace, grant) in &matrix {
            let document_grant = matches!(
                grant,
                RequiredGrant::Read | RequiredGrant::Propose | RequiredGrant::Apply
            );
            assert_eq!(
                *namespace == ToolNamespace::Document,
                document_grant,
                "{name}"
            );
        }
        // 이름이 namespace 접두어로 시작하고 모두 고유합니다.
        let mut names: Vec<&str> = matrix.iter().map(|m| m.0).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), matrix.len());
        for tool in all_tools() {
            assert!(valid_tool_name(tool.name), "{}", tool.name);
        }
    }

    #[test]
    fn commit_push_and_terminal_are_disabled_by_default() {
        let names = |backend, enabled| -> Vec<&'static str> {
            listed_tools(backend, enabled).map(|t| t.name).collect()
        };
        let default = names(Backend::Attached, EnabledNamespaces::default());
        assert_eq!(default.len(), DOCUMENT_TOOLS.len());
        assert!(default.iter().all(|n| n.starts_with("document.")));
        // 문서 apply가 보여도 commit·push·terminal은 따로 켜야 합니다.
        assert!(default.contains(&"document.apply_commands"));
        let git = names(Backend::Attached, EnabledNamespaces::default().with_git());
        assert!(git.contains(&"git.commit") && git.contains(&"git.push"));
        assert!(!git.contains(&"terminal.run"));
        let terminal = names(
            Backend::Attached,
            EnabledNamespaces::default().with_terminal(),
        );
        assert!(terminal.contains(&"terminal.run") && !terminal.contains(&"git.push"));
        // headless는 켜도 터미널·Git·쓰기 tool이 없습니다.
        let headless = names(
            Backend::Headless,
            EnabledNamespaces::default().with_git().with_terminal(),
        );
        assert_eq!(
            headless,
            [
                "document.semantic_diff",
                "document.validate",
                "document.propose_commands"
            ]
        );
        let listed =
            crate::json::write(&tools_list(Backend::Attached, EnabledNamespaces::default()));
        assert!(!listed.contains("git.") && !listed.contains("terminal."));
    }

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

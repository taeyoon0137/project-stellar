//! MCP tool 정의 (XIX.2.3, XIX.2.4, XIX.9.2, ADR-0085).
//!
//! MCP 명세 2025-11-25 "Tools"의 `name`·`title`·`description`·`inputSchema`·`annotations`를 정의합니다.
//!
//! - **이름**: 명세 이름 규칙(1~128자, ASCII 영문·숫자·`_`·`-`·`.`, 대소문자 구분, 서버 안 고유)을
//!   따르고 `<namespace>.<동작>` 형태입니다([`valid_tool_name`]).
//! - **annotation**: 읽기 계열은 `readOnlyHint: true`, 상태를 바꾸는 계열은 `destructiveHint: true`입니다.
//!   annotation은 client 표시용 힌트이며 권한 판정이 아닙니다. 허용 여부는 Stellar 승인(XIX.3)이 정합니다.
//! - **backend**: headless 읽기 backend는 상태를 바꾸지 않는 권한(Read·Propose)의 tool만 제공합니다
//!   (ADR-0005 §3). 쓰기 tool은 앱 승인·lease·DocumentThread 또는 앱 Git 경로가 필요하므로 attached 전용이고
//!   headless `tools/list`에 나타나지 않습니다.
//! - **CLI·MCP 동등성(ADR-0005)**: `stellar` 명령의 읽기·검증·dry-run 능력(`query --source-span`,
//!   `export-web --frame`, `css copy`·`css paste --dry-run`, `git status`, `merge-driver resolve` 보기)을 같은
//!   ops 함수로 노출합니다. 쓰기 짝(`css paste`, `git stage|unstage|commit`)은 attached 전용으로 정의만 둡니다.
//! - **필요 권한**: 각 tool이 요구하는 [`Permission`](XIX.3.1)을 함께 선언합니다.

use super::resources::{Availability, Backend};
use crate::permission::Permission;
use crate::schema::COMMAND_TRANSACTION_SCHEMA;

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
    /// `annotations.destructiveHint`(읽기 전용이면 거짓).
    pub destructive: bool,
    /// `annotations.idempotentHint`.
    pub idempotent: bool,
    /// 제공 backend.
    pub availability: Availability,
    /// 필요 권한.
    pub grant: Permission,
}

/// page 인자 조각(`cursor`, `limit`). `limit` 상한은 mcp-core page 상한과 같아야 합니다(mcp-core 테스트가 확인).
macro_rules! page_args {
    () => {
        r#""cursor":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":2000}"#
    };
}

/// CSS 선언 블록 붙여넣기 인자 schema.
macro_rules! css_paste_schema {
    () => {
        r#"{"type":"object","additionalProperties":false,"required":["base_revision","document","nodes","declarations"],"properties":{"base_revision":{"type":"string"},"document":{"type":"string"},"nodes":{"type":"array","minItems":1,"maxItems":1000,"items":{"type":"string"}},"declarations":{"type":"string","maxLength":65536}}}"#
    };
}

/// stage·unstage 인자 schema.
macro_rules! git_selection_schema {
    () => {
        r#"{"type":"object","additionalProperties":false,"required":["items"],"properties":{"items":{"type":"array","minItems":1,"maxItems":1000,"items":{"type":"string"}},"with":{"type":"array","maxItems":1000,"items":{"type":"string"}}}}"#
    };
}

/// 고정 Git HEAD 대비 저장 문서와 프로젝트 공용 컴포넌트 변경.
pub const SEMANTIC_DIFF: ToolDef = ToolDef {
    name: "document.semantic_diff",
    title: "Semantic diff",
    description:
        "같은 프로젝트의 Git HEAD와 작업 트리의 저장 문서·컴포넌트 원본 변경을 페이지로 읽습니다",
    input_schema: concat!(
        r#"{"type":"object","additionalProperties":false,"properties":{"base":{"type":"string","enum":["HEAD"]},"#,
        page_args!(),
        r#"}}"#
    ),
    read_only: true,
    destructive: false,
    idempotent: true,
    availability: Availability::All,
    grant: Permission::Read,
};

/// 결합된 창의 렌더 미리보기(attached 전용, ADR-0005 §8).
pub const RENDER_PREVIEW: ToolDef = ToolDef {
    name: "document.render_preview",
    title: "Render preview",
    description: "결합된 창에서 node 하나의 렌더 미리보기를 요청합니다",
    input_schema: r#"{"type":"object","additionalProperties":false,"required":["document","node"],"properties":{"document":{"type":"string"},"node":{"type":"string"},"scale":{"type":"number","exclusiveMinimum":0,"maximum":4}}}"#,
    read_only: true,
    destructive: false,
    idempotent: true,
    availability: Availability::AttachedOnly,
    grant: Permission::Read,
};

/// 저장 문서 검증.
pub const VALIDATE_DOCUMENT: ToolDef = ToolDef {
    name: "document.validate",
    title: "Validate document",
    description:
        "저장된 문서의 검증 진단을 페이지로 읽습니다(문서를 지정하지 않으면 프로젝트 전체)",
    input_schema: concat!(
        r#"{"type":"object","additionalProperties":false,"properties":{"document":{"type":"string"},"#,
        page_args!(),
        r#"}}"#
    ),
    read_only: true,
    destructive: false,
    idempotent: true,
    availability: Availability::All,
    grant: Permission::Read,
};

/// closed enum command 묶음 dry-run(`stellar apply --dry-run`과 같은 ops 경로).
pub const PROPOSE_COMMANDS: ToolDef = ToolDef {
    name: "document.propose_commands",
    title: "Propose commands",
    description:
        "closed enum command 묶음을 검증하고 적용하지 않은 채 semantic diff를 돌려줍니다(dry-run)",
    input_schema: COMMAND_TRANSACTION_SCHEMA,
    read_only: true,
    destructive: false,
    idempotent: true,
    availability: Availability::All,
    grant: Permission::Propose,
};

/// 승인된 command 묶음 적용(attached 전용).
pub const APPLY_COMMANDS: ToolDef = ToolDef {
    name: "document.apply_commands",
    title: "Apply commands",
    description: "승인된 command 묶음을 앱의 lease·승인·DocumentThread 경로로 적용합니다",
    input_schema: COMMAND_TRANSACTION_SCHEMA,
    read_only: false,
    destructive: true,
    idempotent: false,
    availability: Availability::AttachedOnly,
    grant: Permission::Apply,
};

/// 다중 파일 텍스트 원본에서 node의 파일·줄 범위(`stellar query --source-span`).
pub const SOURCE_SPAN: ToolDef = ToolDef {
    name: "document.source_span",
    title: "Source span",
    description:
        "프로젝트 design 폴더의 다중 파일 텍스트 원본에서 node의 파일과 줄 범위를 찾습니다",
    input_schema: r#"{"type":"object","additionalProperties":false,"required":["node"],"properties":{"node":{"type":"string"}}}"#,
    read_only: true,
    destructive: false,
    idempotent: true,
    availability: Availability::All,
    grant: Permission::Read,
};

/// 저장 View의 HTML+CSS 투영(`stellar export-web [--frame]`). 파일을 쓰지 않고 결과만 돌려줍니다.
pub const EXPORT_WEB_FRAME: ToolDef = ToolDef {
    name: "document.export_web_frame",
    title: "Export web frame",
    description: "저장된 고정 Free Frame View(또는 그 안 Frame 하나)의 HTML+CSS와 진단을 돌려줍니다(파일을 쓰지 않음)",
    input_schema: r#"{"type":"object","additionalProperties":false,"required":["document","view"],"properties":{"document":{"type":"string"},"view":{"type":"string"},"frame":{"type":"string"}}}"#,
    read_only: true,
    destructive: false,
    idempotent: true,
    availability: Availability::All,
    grant: Permission::Read,
};

/// node 속성의 CSS longhand 선언 블록 복사(`stellar css copy`).
pub const COPY_CSS: ToolDef = ToolDef {
    name: "document.copy_css",
    title: "Copy CSS",
    description: "node의 명시 속성을 CSS longhand 선언 블록으로 돌려줍니다",
    input_schema: r#"{"type":"object","additionalProperties":false,"required":["document","node"],"properties":{"document":{"type":"string"},"node":{"type":"string"}}}"#,
    read_only: true,
    destructive: false,
    idempotent: true,
    availability: Availability::All,
    grant: Permission::Read,
};

/// CSS 선언 블록 붙여넣기 dry-run(`stellar css paste --dry-run`).
pub const PROPOSE_CSS_PASTE: ToolDef = ToolDef {
    name: "document.propose_css_paste",
    title: "Propose CSS paste",
    description: "CSS 선언 블록을 node들에 붙여넣을 때의 적용·건너뜀 선언과 semantic diff를 돌려줍니다(dry-run)",
    input_schema: css_paste_schema!(),
    read_only: true,
    destructive: false,
    idempotent: true,
    availability: Availability::All,
    grant: Permission::Propose,
};

/// CSS 선언 블록 붙여넣기 적용(attached 전용).
pub const PASTE_CSS: ToolDef = ToolDef {
    name: "document.paste_css",
    title: "Paste CSS",
    description:
        "승인된 CSS 선언 블록을 앱의 lease·승인·DocumentThread 경로로 한 transaction으로 적용합니다",
    input_schema: css_paste_schema!(),
    read_only: false,
    destructive: true,
    idempotent: false,
    availability: Availability::AttachedOnly,
    grant: Permission::Apply,
};

/// 정의된 문서 tool(고정 순서).
pub const DOCUMENT_TOOLS: &[ToolDef] = &[
    SEMANTIC_DIFF,
    RENDER_PREVIEW,
    VALIDATE_DOCUMENT,
    PROPOSE_COMMANDS,
    APPLY_COMMANDS,
    SOURCE_SPAN,
    EXPORT_WEB_FRAME,
    COPY_CSS,
    PROPOSE_CSS_PASTE,
    PASTE_CSS,
];

/// 저장 파일 기준 의미 단위 Git 상태(`stellar git status`).
pub const GIT_STATUS: ToolDef = ToolDef {
    name: "git.status",
    title: "Git status",
    description:
        "작업 트리의 원본 단위 상태와 semantic 변경 한 페이지를 읽습니다(index를 바꾸지 않음)",
    input_schema: concat!(
        r#"{"type":"object","additionalProperties":false,"properties":{"#,
        page_args!(),
        r#"}}"#
    ),
    read_only: true,
    destructive: false,
    idempotent: true,
    availability: Availability::All,
    grant: Permission::Read,
};

/// 진행 중 merge의 semantic conflict와 해결 선택지(`stellar merge-driver resolve`, `--apply` 없음).
pub const SEMANTIC_CONFLICTS: ToolDef = ToolDef {
    name: "git.semantic_conflicts",
    title: "Semantic conflicts",
    description: "진행 중인 merge·rebase의 unmerged 컴포넌트 원본에 대한 semantic merge 결과와 conflict를 읽습니다",
    input_schema: r#"{"type":"object","additionalProperties":false,"properties":{}}"#,
    read_only: true,
    destructive: false,
    idempotent: true,
    availability: Availability::All,
    grant: Permission::Read,
};

/// 터미널·Git namespace tool(고정 순서).
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
        grant: Permission::Terminal,
    },
    GIT_STATUS,
    SEMANTIC_CONFLICTS,
    ToolDef {
        name: "git.stage",
        title: "Git stage",
        description: "고른 semantic 변경·원본 단위를 index에 포함합니다(개별 확인 필요)",
        input_schema: git_selection_schema!(),
        read_only: false,
        destructive: true,
        idempotent: false,
        availability: Availability::AttachedOnly,
        grant: Permission::GitCommit,
    },
    ToolDef {
        name: "git.unstage",
        title: "Git unstage",
        description: "고른 semantic 변경·원본 단위를 index에서 뺍니다(개별 확인 필요)",
        input_schema: git_selection_schema!(),
        read_only: false,
        destructive: true,
        idempotent: false,
        availability: Availability::AttachedOnly,
        grant: Permission::GitCommit,
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
        grant: Permission::GitCommit,
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
        grant: Permission::GitPush,
    },
];

/// MCP 명세 tool 이름 규칙(1~128자, `[A-Za-z0-9_.-]`).
pub fn valid_tool_name(name: &str) -> bool {
    (1..=128).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// tool namespace(XIX.2.4). 문서 편집 권한과 터미널·Git 권한을 결합하지 않도록(아키텍처 §31.3)
/// 상태를 바꾸는 tool은 namespace마다 따로 켭니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolNamespace {
    /// 문서 조회·검증·제안·적용(`document.*`).
    Document,
    /// 터미널 실행(`terminal.*`).
    Terminal,
    /// Git 상태·stage·commit·push(`git.*`).
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
/// 터미널·commit·push 비활성). 터미널·Git의 상태를 바꾸는 tool은 [`EnabledNamespaces::with_terminal`]·
/// [`EnabledNamespaces::with_git`]로 명시적으로만 켭니다. 상태를 바꾸지 않는 읽기 tool(`git.status` 등)은
/// 문서 읽기와 같은 Read 권한이므로 namespace 스위치와 무관하게 보입니다(ADR-0085 결정 6).
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

    /// `tool`이 이 설정에서 보이는지(namespace가 켜졌거나 상태를 바꾸지 않는 tool).
    pub fn shows(self, tool: &ToolDef) -> bool {
        ToolNamespace::of(tool.name)
            .is_some_and(|n| self.includes(n) || tool.grant.is_non_mutating())
    }
}

/// 모든 정의 tool(문서 → 터미널·Git 순).
pub fn all_tools() -> impl Iterator<Item = &'static ToolDef> {
    DOCUMENT_TOOLS.iter().chain(SYSTEM_TOOLS.iter())
}

/// 이름으로 정의를 찾습니다.
pub fn tool(name: &str) -> Option<&'static ToolDef> {
    all_tools().find(|t| t.name == name)
}

/// `backend`·`enabled`에서 노출하는 tool.
pub fn listed_tools(
    backend: Backend,
    enabled: EnabledNamespaces,
) -> impl Iterator<Item = &'static ToolDef> {
    all_tools().filter(move |t| t.availability.includes(backend) && enabled.shows(t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_matrix_separates_document_terminal_and_git() {
        // 검증 기준(XIX.2.4): permission matrix. (tool, namespace, 필요 권한)
        let matrix: Vec<(&str, ToolNamespace, Permission)> = all_tools()
            .map(|t| (t.name, ToolNamespace::of(t.name).unwrap(), t.grant))
            .collect();
        for (name, namespace, grant) in &matrix {
            // 문서 namespace tool은 문서 권한만, 터미널·Git의 상태를 바꾸는 tool은 문서 권한이 아닌 권한만 씁니다.
            if *namespace == ToolNamespace::Document {
                assert!(grant.is_document(), "{name}");
            } else if !grant.is_non_mutating() {
                assert!(!grant.is_document(), "{name}");
            }
        }
        let mut names: Vec<&str> = matrix.iter().map(|m| m.0).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), matrix.len(), "이름 고유");
        for tool in all_tools() {
            assert!(valid_tool_name(tool.name), "{}", tool.name);
            // 쓰기 tool만 destructive이고, 상태를 바꾸지 않는 권한의 tool은 읽기 전용입니다.
            assert_eq!(tool.destructive, !tool.read_only, "{}", tool.name);
            assert_eq!(
                tool.read_only,
                tool.grant.is_non_mutating(),
                "{}",
                tool.name
            );
            // 상태를 바꾸는 tool은 모두 attached 전용입니다(ADR-0005 §3).
            if !tool.read_only {
                assert_eq!(
                    tool.availability,
                    Availability::AttachedOnly,
                    "{}",
                    tool.name
                );
            }
        }
    }

    #[test]
    fn headless_lists_no_write_tool_even_with_every_namespace() {
        let headless: Vec<&str> = listed_tools(
            Backend::Headless,
            EnabledNamespaces::default().with_git().with_terminal(),
        )
        .map(|t| t.name)
        .collect();
        assert_eq!(
            headless,
            [
                "document.semantic_diff",
                "document.validate",
                "document.propose_commands",
                "document.source_span",
                "document.export_web_frame",
                "document.copy_css",
                "document.propose_css_paste",
                "git.status",
                "git.semantic_conflicts",
            ]
        );
        assert!(
            listed_tools(Backend::Headless, EnabledNamespaces::default())
                .all(|t| t.read_only && t.grant.is_non_mutating())
        );
    }

    #[test]
    fn commit_push_and_terminal_are_disabled_by_default() {
        let names = |enabled| -> Vec<&'static str> {
            listed_tools(Backend::Attached, enabled)
                .map(|t| t.name)
                .collect()
        };
        let default = names(EnabledNamespaces::default());
        assert!(default.contains(&"document.apply_commands"));
        assert!(default.contains(&"git.status"), "읽기 Git tool은 보임");
        for hidden in [
            "terminal.run",
            "git.stage",
            "git.unstage",
            "git.commit",
            "git.push",
        ] {
            assert!(!default.contains(&hidden), "{hidden}");
        }
        let git = names(EnabledNamespaces::default().with_git());
        assert!(git.contains(&"git.commit") && git.contains(&"git.push"));
        assert!(!git.contains(&"terminal.run"));
        let terminal = names(EnabledNamespaces::default().with_terminal());
        assert!(terminal.contains(&"terminal.run") && !terminal.contains(&"git.push"));
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

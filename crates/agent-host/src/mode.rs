//! 에이전트 모드와 내장 세션 카탈로그 (XXIX.3.3, ADR-0005 §4.5·Security, ADR-0101 결정 2).
//!
//! 모드([`SessionMode`])는 XIX.3.1 권한 중 세션에 줄 수 있는 문서 권한(Read·Propose·Apply) 조합의 **상한**
//! 입니다. 모드는 권한을 부여하지 않습니다. 실제 허용은 XIX.3 승인(첫 연결 승인, 권한별 확인)이 따로
//! 정하고, 모드는 그 결과를 넘지 못하게만 막습니다.
//!
//! | 권한 | 읽기 전용 | 검토 후 적용(기본) | 승인 범위 내 바로 적용 |
//! |---|---|---|---|
//! | Read | 세션 | 세션 | 세션 |
//! | Propose | 상한 밖 | 세션 | 세션 |
//! | Apply | 상한 밖 | 상한 밖(batch마다 사용자 승인으로 적용) | 세션 |
//! | GitCommit·GitPush | 상한 밖 | 실행 한 번 개별 확인 | 실행 한 번 개별 확인 |
//! | Terminal·FileDelete | 내장 세션 제외 | 내장 세션 제외 | 내장 세션 제외 |
//!
//! - **내장 세션 카탈로그**: attached `tools/list`에서 Terminal·FileDelete 권한 tool을 빼고, 모드 상한 밖
//!   권한의 tool도 뺍니다([`builtin_tools`]). tool 호출은 [`builtin_tool`]이 같은 표로 거릅니다. 목록과 호출
//!   판정이 같은 함수([`grant_route`])를 씁니다.
//! - **모드 변경**: 사용자 UI만 바꿉니다. 카탈로그(agent-protocol)에는 모드를 쓰는 tool이 없으므로 그런
//!   이름의 호출은 [`ToolRefusal::Unknown`]입니다. 세션 모드 변경은
//!   [`crate::session::AgentSession::change_mode_by_user`] 하나이며, 에이전트 경로(MCP·CLI)에서 부르지
//!   않습니다(`tests/immutable_paths.rs`).

use stellar_agent_protocol::catalog::resources::Backend;
use stellar_agent_protocol::catalog::tools::{self, EnabledNamespaces, ToolDef};
use stellar_agent_protocol::permission::Permission;

pub use crate::session::SessionMode;

/// 권한 하나가 내장 세션에서 다뤄지는 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantRoute {
    /// 모드 상한 안. 승인되면 세션 동안 유지됩니다.
    Session,
    /// 실행 한 번마다 개별 확인(GitCommit·GitPush, §31.3).
    PerExecution,
    /// 모드 상한 밖.
    AboveMode,
    /// 내장 세션에서 노출하지 않음(Terminal·FileDelete, ADR-0005 §4.5).
    Excluded,
}

impl SessionMode {
    /// 모든 모드(고정 순서).
    pub const ALL: [SessionMode; 3] = [
        SessionMode::ReadOnly,
        SessionMode::ReviewThenApply,
        SessionMode::ApplyInScope,
    ];

    /// 고정 이름.
    pub fn name(self) -> &'static str {
        match self {
            SessionMode::ReadOnly => "read-only",
            SessionMode::ReviewThenApply => "review-then-apply",
            SessionMode::ApplyInScope => "apply-in-scope",
        }
    }
}

impl Default for SessionMode {
    /// 기본 모드는 검토 후 적용입니다(ADR-0005 §4.5).
    fn default() -> Self {
        SessionMode::ReviewThenApply
    }
}

/// 모드에서 권한이 다뤄지는 방식(표의 한 칸).
pub fn grant_route(mode: SessionMode, permission: Permission) -> GrantRoute {
    match permission {
        Permission::Terminal | Permission::FileDelete => GrantRoute::Excluded,
        Permission::GitCommit | Permission::GitPush => {
            if mode == SessionMode::ReadOnly {
                GrantRoute::AboveMode
            } else {
                GrantRoute::PerExecution
            }
        }
        p if mode.cap().contains(&p) => GrantRoute::Session,
        _ => GrantRoute::AboveMode,
    }
}

/// tool이 내장 세션에서 거부되는 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRefusal {
    /// 카탈로그에 없는 이름(모드·권한·신뢰를 바꾸는 tool은 정의가 없음).
    Unknown,
    /// 내장 세션에서 노출하지 않는 권한의 tool.
    Excluded(Permission),
    /// 모드 상한 밖 권한의 tool.
    AboveMode {
        /// 모드.
        mode: SessionMode,
        /// 필요 권한.
        permission: Permission,
    },
}

impl ToolRefusal {
    /// 고정 코드.
    pub fn code(self) -> &'static str {
        match self {
            ToolRefusal::Unknown => "unknown-tool",
            ToolRefusal::Excluded(_) => "excluded-from-builtin-session",
            ToolRefusal::AboveMode { .. } => "above-mode",
        }
    }
}

/// 내장 세션 attached 목록의 출발점: attached backend, 문서·Git namespace(터미널 namespace는 켜지 않음).
fn base_tools() -> impl Iterator<Item = &'static ToolDef> {
    tools::listed_tools(Backend::Attached, EnabledNamespaces::default().with_git())
}

/// 모드에서 tool의 판정.
fn judge(mode: SessionMode, tool: &'static ToolDef) -> Result<&'static ToolDef, ToolRefusal> {
    match grant_route(mode, tool.grant) {
        GrantRoute::Session | GrantRoute::PerExecution => Ok(tool),
        GrantRoute::Excluded => Err(ToolRefusal::Excluded(tool.grant)),
        GrantRoute::AboveMode => Err(ToolRefusal::AboveMode {
            mode,
            permission: tool.grant,
        }),
    }
}

/// 내장 세션 `tools/list`(카탈로그 순서).
pub fn builtin_tools(mode: SessionMode) -> Vec<&'static ToolDef> {
    base_tools().filter(|t| judge(mode, t).is_ok()).collect()
}

/// 내장 세션 `tools/call` 판정. 허용이면 정의를 돌려줍니다. 허용이어도 실제 실행은 XIX.3 승인과 lease
/// 판정을 따로 거칩니다(모드는 상한일 뿐).
pub fn builtin_tool(mode: SessionMode, name: &str) -> Result<&'static ToolDef, ToolRefusal> {
    match tools::tool(name) {
        None => Err(ToolRefusal::Unknown),
        Some(tool) => {
            // 터미널 namespace는 내장 세션에서 켜지 않으므로, attached 목록에 없는 tool은 그 권한으로
            // 거부합니다.
            if !base_tools().any(|t| t.name == tool.name) {
                return Err(ToolRefusal::Excluded(tool.grant));
            }
            judge(mode, tool)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mode_is_review_then_apply() {
        assert_eq!(SessionMode::default(), SessionMode::ReviewThenApply);
    }
}

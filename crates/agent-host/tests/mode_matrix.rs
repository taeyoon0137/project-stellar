//! 에이전트 모드와 내장 세션 카탈로그 (XXIX.3.3).
//!
//! 검증 기준: 모드별 permission matrix 통과, 내장 세션 tools/list의 terminal·파일 삭제 계열 0, tool 호출로
//! 모드 변경 시도 거부.

use stellar_agent_host::mode::*;
use stellar_agent_host::session::*;
use stellar_agent_protocol::catalog::tools;
use stellar_agent_protocol::permission::Permission;

fn route_name(route: GrantRoute) -> &'static str {
    match route {
        GrantRoute::Session => "session",
        GrantRoute::PerExecution => "per-execution",
        GrantRoute::AboveMode => "above-mode",
        GrantRoute::Excluded => "excluded",
    }
}

#[test]
fn permission_matrix_per_mode() {
    let mut lines = Vec::new();
    for mode in SessionMode::ALL {
        let row: Vec<String> = Permission::ALL
            .iter()
            .map(|p| format!("{}={}", p.name(), route_name(grant_route(mode, *p))))
            .collect();
        lines.push(format!("{}: {}", mode.name(), row.join(" ")));
    }
    assert_eq!(
        lines,
        [
            "read-only: read=session propose=above-mode apply=above-mode terminal=excluded file-delete=excluded git-commit=above-mode git-push=above-mode",
            "review-then-apply: read=session propose=session apply=above-mode terminal=excluded file-delete=excluded git-commit=per-execution git-push=per-execution",
            "apply-in-scope: read=session propose=session apply=session terminal=excluded file-delete=excluded git-commit=per-execution git-push=per-execution",
        ]
    );
    // 표의 Session 칸은 ADR-0005 §4.5 모드 상한과 같습니다.
    for mode in SessionMode::ALL {
        let session: Vec<Permission> = Permission::ALL
            .into_iter()
            .filter(|p| grant_route(mode, *p) == GrantRoute::Session)
            .collect();
        assert_eq!(session, mode.cap());
    }
}

#[test]
fn mode_is_a_ceiling_not_a_grant() {
    for mode in SessionMode::ALL {
        let mut s = AgentSession::start_by_user(UserStartRequest {
            session: AgentSessionId(1),
            mode,
            now_ms: 0,
            reconnect_grace_ms: DEFAULT_RECONNECT_GRACE_MS,
        });
        s.apply(SessionEvent::BridgeBound, 1).unwrap();
        assert_eq!(s.permissions().count(), 0, "모드는 권한을 주지 않음");
        for p in Permission::ALL {
            let result = s.apply(SessionEvent::PermissionGranted(p), 2);
            if grant_route(mode, p) == GrantRoute::Session {
                assert!(result.is_ok(), "{} {}", mode.name(), p.name());
            } else {
                assert_eq!(result, Err(TransitionError::AboveModeCap(p)));
            }
        }
        let held: Vec<Permission> = s.permissions().collect();
        assert_eq!(held, mode.cap());
    }
}

#[test]
fn builtin_catalog_has_no_terminal_or_file_delete_tools() {
    for mode in SessionMode::ALL {
        let listed = builtin_tools(mode);
        assert!(!listed.is_empty());
        for tool in &listed {
            assert!(
                !matches!(tool.grant, Permission::Terminal | Permission::FileDelete),
                "{}",
                tool.name
            );
            assert!(!tool.name.starts_with("terminal."), "{}", tool.name);
            assert!(!tool.name.contains("delete"), "{}", tool.name);
            assert!(matches!(
                grant_route(mode, tool.grant),
                GrantRoute::Session | GrantRoute::PerExecution
            ));
            // 목록과 호출 판정이 같습니다.
            assert_eq!(builtin_tool(mode, tool.name), Ok(*tool));
        }
        // 정의된 터미널·파일 삭제 계열 tool은 호출도 거부됩니다.
        for tool in tools::all_tools() {
            if matches!(tool.grant, Permission::Terminal | Permission::FileDelete) {
                assert_eq!(
                    builtin_tool(mode, tool.name),
                    Err(ToolRefusal::Excluded(tool.grant))
                );
            }
        }
    }
    assert_eq!(
        builtin_tool(SessionMode::ApplyInScope, "terminal.run"),
        Err(ToolRefusal::Excluded(Permission::Terminal))
    );
    // 읽기 전용에는 상태를 바꾸는 tool이 없습니다.
    assert!(builtin_tools(SessionMode::ReadOnly)
        .iter()
        .all(|t| t.grant == Permission::Read));
    // 검토 후 적용에는 apply tool이 없고(batch마다 사용자가 승인), Git 쓰기는 실행 한 번 확인으로 남습니다.
    let review: Vec<&str> = builtin_tools(SessionMode::ReviewThenApply)
        .iter()
        .map(|t| t.name)
        .collect();
    assert!(!review.contains(&"document.apply_commands"), "{review:?}");
    assert!(review.contains(&"document.propose_commands"), "{review:?}");
    assert!(review.contains(&"git.commit") && review.contains(&"git.push"));
    assert_eq!(
        builtin_tool(SessionMode::ReviewThenApply, "document.apply_commands"),
        Err(ToolRefusal::AboveMode {
            mode: SessionMode::ReviewThenApply,
            permission: Permission::Apply
        })
    );
}

#[test]
fn tool_calls_cannot_change_mode() {
    // 카탈로그에는 모드·권한·신뢰를 쓰는 tool이 없습니다. 그런 이름의 호출은 모르는 tool입니다.
    for tool in tools::all_tools() {
        for word in ["mode", "permission", "trust", "provenance", "revert"] {
            assert!(!tool.name.contains(word), "{}", tool.name);
            assert!(
                !tool.input_schema.contains(&format!("\"{word}\"")),
                "{}",
                tool.name
            );
        }
    }
    for name in [
        "session.set_mode",
        "agent.set_mode",
        "document.set_mode",
        "agent.mode",
        "permission.grant",
        "project.trust",
    ] {
        for mode in SessionMode::ALL {
            assert_eq!(
                builtin_tool(mode, name),
                Err(ToolRefusal::Unknown),
                "{name}"
            );
        }
    }
}

#[test]
fn user_mode_change_lowers_ceiling_and_never_raises_grants() {
    let mut s = AgentSession::start_by_user(UserStartRequest {
        session: AgentSessionId(1),
        mode: SessionMode::ApplyInScope,
        now_ms: 0,
        reconnect_grace_ms: DEFAULT_RECONNECT_GRACE_MS,
    });
    s.apply(SessionEvent::BridgeBound, 1).unwrap();
    s.apply(SessionEvent::LeaseGranted(LeaseId(4)), 1).unwrap();
    for p in [Permission::Read, Permission::Propose, Permission::Apply] {
        s.apply(SessionEvent::PermissionGranted(p), 1).unwrap();
    }
    let effects = s.change_mode_by_user(SessionMode::ReadOnly).unwrap();
    assert_eq!(
        effects,
        vec![SessionEffect::RevokePermissions(vec![
            Permission::Propose,
            Permission::Apply
        ])]
    );
    assert_eq!(s.permissions().collect::<Vec<_>>(), [Permission::Read]);
    assert_eq!(s.leases().count(), 1, "lease는 그대로");
    // 다시 올려도 권한은 생기지 않습니다.
    assert!(s
        .change_mode_by_user(SessionMode::ApplyInScope)
        .unwrap()
        .is_empty());
    assert_eq!(s.permissions().collect::<Vec<_>>(), [Permission::Read]);
    // 끝난 세션은 바꾸지 않습니다.
    s.apply(SessionEvent::UserStop, 2).unwrap();
    assert_eq!(
        s.change_mode_by_user(SessionMode::ReadOnly),
        Err(TransitionError::Ended)
    );
}

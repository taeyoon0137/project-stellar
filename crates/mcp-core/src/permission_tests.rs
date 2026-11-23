//! 권한 저장·판정 test (XIX.3.2~XIX.3.4).

use super::*;
use crate::agent_protocol::catalog::resources::Backend;
use crate::agent_protocol::catalog::tools::{listed_tools, EnabledNamespaces};

fn sid(n: u64) -> StableId {
    StableId::new_v7(n, 0, n)
}

fn key(session: u64) -> PermissionKey {
    PermissionKey::new(
        sid(1),
        PathBuf::from("/projects/a"),
        AgentSessionId(sid(session)),
    )
}

const ATTACHED: BackendPolicy = BackendPolicy::Attached { trusted: true };

fn approved_store(key: &PermissionKey) -> PermissionStore {
    let mut store = PermissionStore::new();
    store.connect(key.clone());
    store.respond_connection(key, true).unwrap();
    store
}

fn approve(store: &mut PermissionStore, key: &PermissionKey, permission: Permission) {
    let Confirmation::Pending(id) = store.request(key, permission).unwrap() else {
        panic!("대기 요청");
    };
    assert_eq!(store.respond(key, id, true), Ok(Confirmation::Approved(id)));
}

#[test]
fn permissions_do_not_survive_restart() {
    // 검증 기준(XIX.3.2): 앱을 다시 시작하면(새 저장소) 권한이 없습니다.
    let key = key(10);
    let mut store = approved_store(&key);
    approve(&mut store, &key, Permission::Apply);
    assert_eq!(
        store.decide(ATTACHED, &key, Permission::Apply),
        Decision::Allow
    );
    drop(store);
    let restarted = PermissionStore::new();
    assert_eq!(restarted.connection(&key), None);
    for permission in Permission::ALL {
        assert_eq!(
            restarted.decide(ATTACHED, &key, permission),
            Decision::AwaitingConnection,
            "{permission:?}"
        );
        assert_eq!(
            restarted.confirmation(&key, permission),
            Confirmation::NotRequested
        );
    }
}

#[test]
fn permissions_are_keyed_by_project_identity_and_session() {
    let main = key(10);
    let mut store = approved_store(&main);
    approve(&mut store, &main, Permission::Propose);
    let other_session = key(11);
    let other_worktree = PermissionKey::new(
        sid(1),
        PathBuf::from("/projects/a-worktree"),
        AgentSessionId(sid(10)),
    );
    for other in [&other_session, &other_worktree] {
        assert_eq!(
            store.decide(ATTACHED, other, Permission::Read),
            Decision::AwaitingConnection
        );
        assert_eq!(
            store.confirmation(other, Permission::Propose),
            Confirmation::NotRequested
        );
    }
    assert!(store.end_session(&main));
    assert_eq!(
        store.decide(ATTACHED, &main, Permission::Propose),
        Decision::AwaitingConnection
    );
}

#[test]
fn attached_connection_needs_first_approval_even_for_read() {
    // 검증 기준(XIX.3.3): attached 첫 연결은 승인 전 denied/prompt 상태입니다.
    let key = key(10);
    let mut store = PermissionStore::new();
    assert_eq!(store.connect(key.clone()), ConnectionApproval::Pending);
    for permission in Permission::ALL {
        assert_eq!(
            store.decide(ATTACHED, &key, permission),
            Decision::AwaitingConnection,
            "{permission:?}"
        );
    }
    assert_eq!(
        store.request(&key, Permission::Read),
        Err(PermissionError::ReadFollowsConnection)
    );
    let mut denied = PermissionStore::new();
    denied.connect(key.clone());
    denied.respond_connection(&key, false).unwrap();
    assert_eq!(
        denied.respond_connection(&key, true),
        Err(PermissionError::ConnectionAnswered),
        "첫 응답만 유효"
    );
    assert_eq!(
        denied.decide(ATTACHED, &key, Permission::Read),
        Decision::Denied(DenyReason::ConnectionDenied)
    );
    store.respond_connection(&key, true).unwrap();
    assert_eq!(
        store.decide(ATTACHED, &key, Permission::Read),
        Decision::Allow
    );
    assert_eq!(
        store.decide(ATTACHED, &key, Permission::Propose),
        Decision::AwaitingConfirmation
    );
    // 신뢰되지 않은 프로젝트는 승인과 무관하게 모두 거부합니다.
    for permission in Permission::ALL {
        assert_eq!(
            store.decide(BackendPolicy::Attached { trusted: false }, &key, permission),
            Decision::Denied(DenyReason::NotTrusted)
        );
    }
}

#[test]
fn headless_policy_allows_only_non_mutating_permissions() {
    // 검증 기준(XIX.3.3): headless untrusted는 전 tool 거부, trusted 정책 결과에 쓰기 권한 0.
    let store = PermissionStore::new();
    let key = key(10);
    let trusted = BackendPolicy::Headless { trusted: true };
    let allowed: Vec<Permission> = Permission::ALL
        .into_iter()
        .filter(|p| store.decide(trusted, &key, *p) == Decision::Allow)
        .collect();
    assert_eq!(allowed, [Permission::Read, Permission::Propose]);
    assert!(allowed.iter().all(|p| p.is_non_mutating()), "쓰기 권한 0");
    for permission in Permission::ALL {
        assert_eq!(
            store.decide(BackendPolicy::Headless { trusted: false }, &key, permission),
            Decision::Denied(DenyReason::NotTrusted)
        );
    }
    // headless `tools/list`의 모든 tool은 이 정책이 허용하는 권한만 요구합니다(목록과 정책이 같은 판정).
    let everything = EnabledNamespaces::default().with_terminal().with_git();
    for tool in listed_tools(Backend::Headless, everything) {
        assert_eq!(
            store.decide(trusted, &key, tool.grant),
            Decision::Allow,
            "{}",
            tool.name
        );
    }
}

#[test]
fn untrusted_headless_fixture_rejects_selection() {
    // 신뢰 등록이 없는 프로젝트는 headless backend를 만들 수 없으므로 어떤 tool·resource도 응답하지 않습니다.
    use stellar_ops::test_support::{scaffold_project, select_trusted_project, TempDir};
    let temp = TempDir::new("permission-untrusted");
    let project = temp.path.join("project");
    scaffold_project(&project);
    assert!(select_trusted_project(&project, temp.path.join("data")).is_err());
}

#[test]
fn confirmations_are_independent() {
    // 검증 기준(XIX.3.4): Apply 승인 한 번으로 Push·Commit·Terminal을 허용하지 않습니다(escalation fixture 실패).
    let key = key(10);
    let mut store = approved_store(&key);
    approve(&mut store, &key, Permission::Apply);
    for permission in [
        Permission::Terminal,
        Permission::FileDelete,
        Permission::GitCommit,
        Permission::GitPush,
        Permission::Propose,
    ] {
        assert_eq!(
            store.authorize(ATTACHED, &key, permission),
            Decision::AwaitingConfirmation,
            "{permission:?}"
        );
        assert_eq!(
            store.confirmation(&key, permission),
            Confirmation::NotRequested
        );
    }
    // 각 요청은 따로 상태를 돌려주고, 한 요청의 응답은 그 권한 하나만 바꿉니다.
    let ids: Vec<RequestId> = [
        Permission::GitPush,
        Permission::GitCommit,
        Permission::Terminal,
    ]
    .into_iter()
    .map(|p| match store.request(&key, p).unwrap() {
        Confirmation::Pending(id) => id,
        other => panic!("{other:?}"),
    })
    .collect();
    assert!(ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2]);
    store.respond(&key, ids[1], true).unwrap();
    assert_eq!(
        store.respond(&key, ids[1], false),
        Err(PermissionError::AlreadyAnswered(ids[1])),
        "첫 응답만 유효"
    );
    store.respond(&key, ids[2], false).unwrap();
    assert_eq!(
        store.decide(ATTACHED, &key, Permission::GitCommit),
        Decision::Allow
    );
    assert_eq!(
        store.decide(ATTACHED, &key, Permission::GitPush),
        Decision::AwaitingConfirmation
    );
    assert_eq!(
        store.decide(ATTACHED, &key, Permission::Terminal),
        Decision::Denied(DenyReason::ConfirmationDenied)
    );
    // Git commit 승인은 실행 한 번에만 쓰이고, Apply 승인은 세션 동안 남습니다.
    assert_eq!(
        store.authorize(ATTACHED, &key, Permission::GitCommit),
        Decision::Allow
    );
    assert_eq!(
        store.authorize(ATTACHED, &key, Permission::GitCommit),
        Decision::AwaitingConfirmation
    );
    assert_eq!(
        store.authorize(ATTACHED, &key, Permission::Apply),
        Decision::Allow
    );
    assert_eq!(
        store.authorize(ATTACHED, &key, Permission::Apply),
        Decision::Allow
    );
    assert_eq!(
        store.respond(&key, RequestId(999), true),
        Err(PermissionError::UnknownRequest(RequestId(999)))
    );
}

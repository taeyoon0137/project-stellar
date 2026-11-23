//! 연결 상태 판정 (XXIX.2.5): 테스트 대역 상태 fixture, 판정 불가 fail closed, 계정 식별 문자열 비보관.

mod support;

use stellar_agent_host::env::ChildEnv;
use stellar_agent_host::preflight::PreflightRefusal;
use stellar_agent_host::status::{probe_connection, spawn_probe, ConnectionState};
use stellar_agent_host::target::verify_target;
use support::*;

/// 상태 출력에 넣는 가짜 계정 식별 문자열.
const ACCOUNT: &str = "account-identifier-fixture-7f3a";

fn state_for(status_body: &str, timeout_ms: Option<u64>) -> ConnectionState {
    let s = setup(
        "status",
        "claude",
        &format!("[version]\nout 2.1.300\\n\n[status]\n{status_body}"),
    );
    let mut profile = s.profile.clone();
    if let Some(ms) = timeout_ms {
        profile.timeouts.status_ms = ms;
    }
    let v = FixtureVerifier::accepting();
    let target = verify_target(&s.target, &profile, &v).unwrap();
    let env = ChildEnv::allowlisted(&profile.env, fake_app_env(&s.home));
    let state = probe_connection(&target, &profile, &env, &s.scratch);
    assert_eq!(
        std::fs::read_dir(&s.scratch).unwrap().count(),
        0,
        "scratch 정리"
    );
    state
}

#[test]
fn fixture_states_match_judgement() {
    let printing = format!("out {{\"loggedIn\":true,\"account\":\"{ACCOUNT}\"}}\\n\n");
    assert_eq!(
        state_for(&format!("{printing}exit 0\n"), None),
        ConnectionState::Connected
    );
    assert_eq!(
        state_for(&format!("{printing}exit 1\n"), None),
        ConnectionState::NotConnected
    );
}

#[test]
fn undeterminable_fixtures_are_unusable() {
    for (body, timeout) in [
        ("exit 2\n", None),
        ("exit 127\n", None),
        ("hang\n", Some(300)),
    ] {
        let state = state_for(body, timeout);
        assert!(!state.is_usable(), "{body}: {state:?}");
        assert!(matches!(state, ConnectionState::Undeterminable(_)));
    }
}

#[test]
fn not_connected_refuses_preflight_and_leaks_no_account_text() {
    let probe = TempDir::new("status-mark");
    let mark = probe.join("ran");
    let s = setup(
        "status-pre",
        "claude",
        &format!(
            "[version]\nout 2.1.300\\n\n[status]\nout {{\"email\":\"{ACCOUNT}\"}}\\n\nexit 1\n[session]\nmark {}\n",
            mark.display()
        ),
    );
    let v = FixtureVerifier::accepting();
    let refusal = run_preflight(&s.profile, &s.target, &v, &s.home, &s.scratch).unwrap_err();
    assert_eq!(
        refusal,
        PreflightRefusal::Connection(ConnectionState::NotConnected)
    );
    assert!(!mark.exists());
    // 판정 결과·거부 이유·진단 코드 어디에도 계정 식별 문자열이 없음.
    let rendered = format!("{refusal:?} {}", refusal.code());
    assert!(!rendered.contains(ACCOUNT));
}

#[test]
fn probe_runs_off_the_calling_thread() {
    let s = setup(
        "status-worker",
        "claude",
        "[version]\nout 2.1.300\\n\n[status]\nsleep 100\nexit 0\n",
    );
    let v = FixtureVerifier::accepting();
    let target = verify_target(&s.target, &s.profile, &v).unwrap();
    let env = ChildEnv::allowlisted(&s.profile.env, fake_app_env(&s.home));
    let rx = spawn_probe(target, s.profile.clone(), env, s.scratch.clone());
    // 호출 스레드는 막히지 않습니다.
    assert!(rx.try_recv().is_err());
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap(),
        ConnectionState::Connected
    );
}

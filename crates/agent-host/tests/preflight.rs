//! 세션 실행 전 확인 (XXIX.2.7): 확인 실패 fixture에서 세션 실행 0, 정상 fixture에서 실행 1.

mod support;

use stellar_agent_host::preflight::PreflightRefusal;
use stellar_agent_host::status::ConnectionState;
use stellar_agent_host::target::SignatureError;
use support::*;

#[test]
fn failures_run_zero_sessions_and_success_runs_one() {
    let probe = TempDir::new("preflight");
    let runs = probe.join("runs");
    // session 단계가 실행되면 runs 파일이 생깁니다.
    let session = format!("[session]\nmark {}\nout {{}}\\n\nexit 0\n", runs.display());
    let cases: Vec<(&str, String, Option<SignatureError>, PreflightRefusal)> = vec![
        (
            "identity",
            format!("{OK_PROBES}{session}"),
            Some(SignatureError::RequirementFailed),
            PreflightRefusal::Target(stellar_agent_host::target::TargetError::Signature(
                SignatureError::RequirementFailed,
            )),
        ),
        (
            "version unreadable",
            format!("[version]\nout unknown\\n\n[status]\nexit 0\n{session}"),
            None,
            PreflightRefusal::VersionUndeterminable,
        ),
        (
            "version command fails",
            format!("[version]\nexit 2\n[status]\nexit 0\n{session}"),
            None,
            PreflightRefusal::VersionUndeterminable,
        ),
        (
            "not connected",
            format!("[version]\nout 2.1.300\\n\n[status]\nexit 1\n{session}"),
            None,
            PreflightRefusal::Connection(ConnectionState::NotConnected),
        ),
        (
            "status undeterminable",
            format!("[version]\nout 2.1.300\\n\n[status]\nexit 9\n{session}"),
            None,
            PreflightRefusal::Connection(ConnectionState::Undeterminable("exit-code")),
        ),
    ];
    for (name, scenario, sig, expected) in cases {
        let s = setup("preflight", "claude", &scenario);
        let v = match sig {
            Some(e) => FixtureVerifier::rejecting(e),
            None => FixtureVerifier::accepting(),
        };
        let refusal = run_preflight(&s.profile, &s.target, &v, &s.home, &s.scratch).unwrap_err();
        assert_eq!(refusal, expected, "{name}");
        assert!(!runs.exists(), "{name}: 세션 실행됨");
        assert_eq!(std::fs::read_dir(&s.scratch).unwrap().count(), 0, "{name}");
    }

    let s = setup("preflight-ok", "claude", &format!("{OK_PROBES}{session}"));
    let v = FixtureVerifier::accepting();
    let pass = run_preflight(&s.profile, &s.target, &v, &s.home, &s.scratch).unwrap();
    assert_eq!(pass.version().to_string(), "2.1.300");
    assert!(!runs.exists(), "확인만으로는 세션을 실행하지 않음");
    pass.launch("x".into(), Some(mcp()), output_for(&s.profile))
        .unwrap()
        .wait();
    assert!(runs.exists(), "정상 fixture에서 실행 1");
    assert_eq!(v.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn executable_changed_after_preflight_is_not_launched() {
    let probe = TempDir::new("preflight-toctou");
    let runs = probe.join("runs");
    let s = setup(
        "preflight-toctou",
        "claude",
        &format!("{OK_PROBES}[session]\nmark {}\n", runs.display()),
    );
    let v = FixtureVerifier::accepting();
    let pass = run_preflight(&s.profile, &s.target, &v, &s.home, &s.scratch).unwrap();
    let mut bytes = std::fs::read(&s.exe).unwrap();
    bytes.push(0);
    std::fs::write(&s.exe, bytes).unwrap();
    assert!(matches!(
        pass.launch("x".into(), Some(mcp()), output_for(&s.profile)),
        Err(stellar_agent_host::process::LaunchError::TargetChanged)
    ));
    assert!(!runs.exists());
}

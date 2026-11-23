//! launch profile fixture (XXIX.2.6).

mod support;

use stellar_agent_host::preflight::PreflightRefusal;
use stellar_agent_host::process::ProcessEndReason;
use stellar_agent_host::profile::{
    official_profiles, CliVersion, LaunchProfile, OFFICIAL_PROFILE_SOURCES,
};
use support::*;

fn report() -> String {
    std::fs::read_to_string(format!(
        "{}/../../docs/task-reports/XXIX.2.6.md",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn every_profile_has_env_names_without_values_and_report_grounds() {
    let report = report();
    let mut all: Vec<LaunchProfile> = official_profiles().unwrap();
    all.push(double_profile("claude"));
    all.push(double_profile("codex"));
    for profile in &all {
        assert!(!profile.env.is_empty(), "{}", profile.id);
        for name in &profile.env {
            assert!(!name.contains('='), "{}: 값", profile.id);
        }
    }
    // 공식 profile의 이름마다 보고서 근거(profile 절의 `NAME` 항목)가 있습니다.
    for profile in official_profiles().unwrap() {
        let section = report
            .split("### profile `")
            .find(|s| s.starts_with(&format!("{}`", profile.id)))
            .unwrap_or_else(|| panic!("보고서에 {} 절 없음", profile.id));
        for name in &profile.env {
            assert!(
                section.contains(&format!("- `{name}`")),
                "{}: {name} 근거 없음",
                profile.id
            );
        }
    }
    // 원문에도 값 field가 없음.
    for (_, source) in OFFICIAL_PROFILE_SOURCES {
        assert!(!source.contains("\"env_values\""));
    }
}

#[test]
fn values_and_unknown_keys_and_unreviewed_activation_are_rejected() {
    let base = std::fs::read_to_string(format!(
        "{}/profiles/claude-code.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let with_value = base.replacen("\"HOME\",", "\"HOME=/Users/x\",", 1);
    assert_eq!(
        LaunchProfile::parse(&with_value).unwrap_err().code,
        "env-value"
    );
    let object_env = base.replacen("\"HOME\",", "{\"HOME\":\"x\"},", 1);
    assert_eq!(
        LaunchProfile::parse(&object_env).unwrap_err().code,
        "env-value"
    );
    let unknown = base.replacen(
        "\"enabled\": false,",
        "\"enabled\": false, \"env_values\": {},",
        1,
    );
    assert_eq!(
        LaunchProfile::parse(&unknown).unwrap_err().code,
        "unknown-key"
    );
    let enabled = base.replacen("\"enabled\": false", "\"enabled\": true", 1);
    assert_eq!(
        LaunchProfile::parse(&enabled).unwrap_err().code,
        "enabled-without-terms"
    );
    let unknown_decoder = base.replacen("\"version\": 1 }", "\"version\": 9 }", 1);
    assert_eq!(
        LaunchProfile::parse(&unknown_decoder).unwrap_err().code,
        "decoder"
    );
}

#[test]
fn inactive_profile_and_out_of_range_version_are_refused() {
    let probe = TempDir::new("profile-mark");
    let scenario = format!(
        "[version]\nmark {}\nout 2.2.0 (Claude Code)\\n\n[status]\nexit 0\n[session]\nmark {}\n",
        probe.join("ran").display(),
        probe.join("ran-session").display()
    );
    let s = setup("profile-range", "claude", &scenario);
    let v = FixtureVerifier::accepting();
    // 비활성 profile: 실행 0(버전 확인도 하지 않음).
    let mut inactive = s.profile.clone();
    inactive.enabled = false;
    assert_eq!(
        run_preflight(&inactive, &s.target, &v, &s.home, &s.scratch).unwrap_err(),
        PreflightRefusal::ProfileInactive
    );
    assert!(!probe.join("ran").exists());
    // 실제 공식 profile(약관 검토 pending)도 거부.
    let official = official_profiles().unwrap().remove(0);
    assert_eq!(
        run_preflight(&official, &s.target, &v, &s.home, &s.scratch).unwrap_err(),
        PreflightRefusal::ProfileInactive
    );
    // 지원 범위 밖 버전(2.2.0 ≥ max_exclusive): 세션 실행 0.
    assert_eq!(
        run_preflight(&s.profile, &s.target, &v, &s.home, &s.scratch).unwrap_err(),
        PreflightRefusal::VersionOutOfRange(CliVersion::parse("2.2.0").unwrap())
    );
    assert!(!probe.join("ran-session").exists());
}

#[test]
fn each_profile_double_session_keeps_project_and_user_config_and_env_subset() {
    for (kind, probes) in [("claude", OK_PROBES), ("codex", OK_PROBES_CODEX)] {
        let probe = TempDir::new("profile-session");
        let env_dump = probe.join("env.txt");
        let fixture = format!(
            "{}/tests/fixtures/decoder/{}/normal.jsonl",
            env!("CARGO_MANIFEST_DIR"),
            if kind == "claude" {
                "claude-stream-json"
            } else {
                "codex-exec-jsonl"
            }
        );
        let s = setup(
            "profile-session",
            kind,
            &format!(
                "{probes}[session]\nenv-dump {}\noutfile {fixture}\nexit 0\n",
                env_dump.display()
            ),
        );
        let project_before = snapshot(&s.project);
        let home_before = snapshot(&s.home);
        let v = FixtureVerifier::accepting();
        let pass = run_preflight(&s.profile, &s.target, &v, &s.home, &s.scratch).unwrap();
        let mcp = (kind == "claude").then(mcp);
        let output = output_for(&s.profile);
        let end = pass.launch("x".into(), mcp, output.clone()).unwrap().wait();
        assert_eq!(end.reason, ProcessEndReason::Exited, "{kind}");
        assert_eq!(snapshot(&s.project), project_before, "{kind}");
        assert_eq!(snapshot(&s.home), home_before, "{kind}");
        let names = std::fs::read_to_string(&env_dump).unwrap();
        for name in names.lines() {
            assert!(s.profile.env.iter().any(|n| n == name), "{kind}: {name}");
        }
        assert!(output.decode_failure().is_none(), "{kind}");
        assert!(!output.chat_items().is_empty(), "{kind}");
    }
}

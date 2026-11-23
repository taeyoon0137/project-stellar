//! 실행 대상 지정과 신원 확인 (XXIX.2.3).
//!
//! 신원 확인 실패 fixture에서 CLI 실행 0(버전 확인 단계 포함), 확인 중 network 호출 0(서명 확인
//! `kSecCSNoNetworkAccess`, crate에 network API 없음은 `tests/inventory.rs`), CLI 실행 파일 쓰기 0.

mod support;

use std::os::unix::fs::PermissionsExt;

use stellar_agent_host::preflight::PreflightRefusal;
use stellar_agent_host::target::{
    confirm_target, verify_target, AppleStaticCodeVerifier, SignatureError, TargetError,
};
use support::*;

fn marking_scenario(mark: &std::path::Path) -> String {
    // 어느 단계든 실행되면 mark 파일이 생깁니다.
    format!(
        "[version]\nmark {m}\nout 2.1.300\\n\n[status]\nmark {m}\nexit 0\n[session]\nmark {m}\n",
        m = mark.display()
    )
}

#[test]
fn identity_failures_execute_nothing_and_write_nothing() {
    let probe = TempDir::new("target-mark");
    let mark = probe.join("ran");
    let s = setup("target", "claude", &marking_scenario(&mark));
    let exe_before = std::fs::read(&s.exe).unwrap();

    type Case<'a> = (&'a str, Box<dyn Fn() -> PreflightRefusal + 'a>);
    let cases: Vec<Case> = vec![
        (
            "unsigned",
            Box::new(|| {
                let v = FixtureVerifier::rejecting(SignatureError::Unsigned);
                run_preflight(&s.profile, &s.target, &v, &s.home, &s.scratch).unwrap_err()
            }),
        ),
        (
            "wrong team",
            Box::new(|| {
                let v = FixtureVerifier::rejecting(SignatureError::RequirementFailed);
                run_preflight(&s.profile, &s.target, &v, &s.home, &s.scratch).unwrap_err()
            }),
        ),
        (
            "real apple verifier on test double",
            Box::new(|| {
                run_preflight(
                    &s.profile,
                    &s.target,
                    &AppleStaticCodeVerifier,
                    &s.home,
                    &s.scratch,
                )
                .unwrap_err()
            }),
        ),
        (
            "wrong cli",
            Box::new(|| {
                let mut t = s.target.clone();
                t.cli = "codex".into();
                let v = FixtureVerifier::accepting();
                run_preflight(&s.profile, &t, &v, &s.home, &s.scratch).unwrap_err()
            }),
        ),
        (
            "no team id in profile",
            Box::new(|| {
                let mut p = s.profile.clone();
                p.team_id = None;
                let v = FixtureVerifier::accepting();
                run_preflight(&p, &s.target, &v, &s.home, &s.scratch).unwrap_err()
            }),
        ),
    ];
    for (name, case) in cases {
        let refusal = case();
        assert!(
            matches!(refusal, PreflightRefusal::Target(_)),
            "{name}: {refusal:?}"
        );
        assert!(!mark.exists(), "{name}: 실행됨");
    }
    assert_eq!(std::fs::read(&s.exe).unwrap(), exe_before);
}

#[test]
fn modified_or_replaced_executable_requires_reconfirmation() {
    let probe = TempDir::new("target-mod");
    let mark = probe.join("ran");
    let s = setup("target-mod", "claude", &marking_scenario(&mark));
    // 확인 뒤 파일 내용이 바뀜(크기·수정 시각 변화).
    let mut bytes = std::fs::read(&s.exe).unwrap();
    bytes.extend_from_slice(b"\0patched");
    std::fs::write(&s.exe, &bytes).unwrap();
    let v = FixtureVerifier::accepting();
    let err = verify_target(&s.target, &s.profile, &v).unwrap_err();
    assert_eq!(err, TargetError::ChangedSinceConfirmation);
    assert_eq!(v.calls.load(std::sync::atomic::Ordering::SeqCst), 0);

    // symlink로 다른 파일을 가리키게 바뀜.
    let s2 = setup("target-link", "claude", &marking_scenario(&mark));
    let link = s2.dir.join("bin/claude-link");
    std::os::unix::fs::symlink(&s2.exe, &link).unwrap();
    let confirmed = confirm_target(&link, "claude-code").unwrap();
    assert_eq!(confirmed.path, s2.exe, "정규 경로로 기록");
    std::fs::remove_file(&link).unwrap();
    let other = s2.dir.join("bin/other");
    std::fs::copy(&s2.exe, &other).unwrap();
    std::os::unix::fs::symlink(&other, &link).unwrap();
    // 기록은 정규 경로이므로 link 교체는 실행 대상을 바꾸지 못합니다.
    assert!(verify_target(&confirmed, &s2.profile, &v).is_ok());
    assert!(!mark.exists());
}

#[test]
fn unsafe_files_are_rejected_at_confirmation() {
    let s = setup("target-perm", "claude", "[session]\n");
    let mut perm = std::fs::metadata(&s.exe).unwrap().permissions();
    perm.set_mode(0o777);
    std::fs::set_permissions(&s.exe, perm).unwrap();
    assert_eq!(
        confirm_target(&s.exe, "claude-code").unwrap_err(),
        TargetError::UnsafePermissions
    );
    assert_eq!(
        confirm_target(s.dir.path(), "claude-code").unwrap_err(),
        TargetError::NotExecutableFile
    );
    let text = s.dir.join("notes.txt");
    std::fs::write(&text, "x").unwrap();
    assert_eq!(
        confirm_target(&text, "claude-code").unwrap_err(),
        TargetError::NotExecutableFile
    );
}

//! 구독 CLI 테스트 대역 self-test (XXIX.2.2).
//!
//! 대역이 XXIX.2.4~XXIX.2.13·XXIX.6 시나리오의 재료(버전·연결 상태 단계, 출력 재생, 입력 경계 조각, stderr,
//! 대기·hang, 종료 코드, 하위 프로세스, 환경·cwd·인자·stdin 기록)를 결정적으로 재현하는지 확인합니다. 대역은
//! network API를 쓰지 않고(`tests/inventory.rs`), 실제 계정·CLI 설정을 읽지 않습니다(HOME 미사용).

mod support;

use std::process::{Command, Stdio};

use support::*;

fn run_double(exe: &std::path::Path, args: &[&str], stdin: &[u8]) -> (i32, Vec<u8>, Vec<u8>) {
    use std::io::Write;
    let mut child = Command::new(exe)
        .args(args)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let out = child.wait_with_output().unwrap();
    (out.status.code().unwrap_or(-1), out.stdout, out.stderr)
}

#[test]
fn phases_replay_deterministically() {
    let dir = TempDir::new("double-phases");
    let raw = format!(
        "{}/tests/fixtures/decoder/claude-stream-json/normal.jsonl",
        env!("CARGO_MANIFEST_DIR")
    );
    let exe = install_double(
        dir.path(),
        "claude",
        &format!(
            "[version]\nout 2.1.300 (Claude Code)\\n\n[status]\nout {{\"loggedIn\":true}}\\n\nexit 1\n[session]\nread-line\nchunk 5\noutfile {raw}\nerr warn\\x1b[0m\\n\nexit 4\n"
        ),
    );
    assert_eq!(run_double(&exe, &["--version"], b"").0, 0);
    assert_eq!(
        run_double(&exe, &["--version"], b"").1,
        b"2.1.300 (Claude Code)\n"
    );
    assert_eq!(run_double(&exe, &["auth", "status"], b"").0, 1);
    let (code, stdout, stderr) = run_double(&exe, &["-p", "--verbose"], b"{}\n");
    assert_eq!(code, 4);
    assert_eq!(stdout, std::fs::read(&raw).unwrap());
    assert_eq!(stderr, b"warn\x1b[0m\n");
    // 반복해도 같은 출력.
    assert_eq!(run_double(&exe, &["-p"], b"{}\n").1, stdout);
}

#[test]
fn missing_or_malformed_scenario_fails_with_fixed_codes() {
    let dir = TempDir::new("double-bad");
    let exe = install_double(dir.path(), "claude", "[session]\nbogus\n");
    assert_eq!(run_double(&exe, &["-p"], b"").0, 98);
    std::fs::remove_file(dir.join("bin/claude.scenario")).unwrap();
    assert_eq!(run_double(&exe, &["-p"], b"").0, 97);
}

#[test]
fn double_receives_only_the_environment_it_is_given() {
    // 빈 환경으로 실행하면 대역이 보는 환경도 비어 있습니다(대역 자신이 변수를 더하지 않음).
    let dir = TempDir::new("double-env");
    let dump = dir.join("env.txt");
    let exe = install_double(
        dir.path(),
        "codex",
        &format!("[session]\nenv-dump {}\n", dump.display()),
    );
    assert_eq!(run_double(&exe, &["exec"], b"").0, 0);
    assert_eq!(std::fs::read_to_string(&dump).unwrap(), "");
}

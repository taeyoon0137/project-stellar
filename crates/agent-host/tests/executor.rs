//! macOS 실행기 (XXIX.2.4): 테스트 대역 세션에서 불변식·흔적·환경·timeout·취소·수신 정지 없음.

mod support;

use std::time::{Duration, Instant};

use stellar_agent_host::process::{process_alive, ProcessEndReason, MCP_CONFIG_FILE};
use support::*;

fn session_scenario(dir: &std::path::Path, body: &str) -> String {
    let _ = dir;
    format!("{OK_PROBES}[session]\n{body}")
}

#[test]
fn double_session_leaves_no_trace_and_changes_nothing_outside_mcp() {
    let probe = TempDir::new("exec-probe");
    let env_dump = probe.join("env.txt");
    let cwd_dump = probe.join("cwd.txt");
    let args_dump = probe.join("args.txt");
    let stdin_dump = probe.join("stdin.txt");
    let body = format!(
        "read-line\nenv-dump {}\ncwd-dump {}\nargs-dump {}\nstdin-dump {}\ntouch work-file.txt\noutfile {}/tests/fixtures/decoder/claude-stream-json/normal.jsonl\nexit 0\n",
        env_dump.display(),
        cwd_dump.display(),
        args_dump.display(),
        stdin_dump.display(),
        env!("CARGO_MANIFEST_DIR"),
    );
    let s = setup(
        "exec-trace",
        "claude",
        &session_scenario(probe.path(), &body),
    );
    let project_before = snapshot(&s.project);
    let home_before = snapshot(&s.home);
    let exe_before = std::fs::read(&s.exe).unwrap();

    let verifier = FixtureVerifier::accepting();
    let pass = run_preflight(&s.profile, &s.target, &verifier, &s.home, &s.scratch).unwrap();
    let output = output_for(&s.profile);
    let process = pass
        .launch("레이어를 정리해 줘".into(), Some(mcp()), output.clone())
        .unwrap();
    let scratch_root = process.scratch_root().to_path_buf();
    assert!(scratch_root.join(MCP_CONFIG_FILE).exists() || process.is_finished());
    let end = process.wait();
    assert_eq!(end.reason, ProcessEndReason::Exited);

    // 불변식 1·2: 프로젝트와 사용자 설정이 바뀌지 않음, CLI 실행 파일 쓰기 0.
    assert_eq!(snapshot(&s.project), project_before);
    assert_eq!(snapshot(&s.home), home_before);
    assert_eq!(std::fs::read(&s.exe).unwrap(), exe_before);
    // 실행 흔적 0: scratch(cwd, MCP 설정, CLI가 cwd에 쓴 파일) 전부 사라짐.
    assert!(!scratch_root.exists());
    assert_eq!(std::fs::read_dir(&s.scratch).unwrap().count(), 0);

    // cwd는 scratch의 work, 프로젝트가 아님.
    let cwd = std::fs::read_to_string(&cwd_dump).unwrap();
    assert!(cwd.starts_with(&*scratch_root.to_string_lossy()), "{cwd}");
    assert!(!cwd.starts_with(&*s.project.to_string_lossy()));

    // 허용 목록 밖 환경 변수 전달 0, child 환경 이름 ⊆ profile 목록.
    let names = std::fs::read_to_string(&env_dump).unwrap();
    for name in names.lines() {
        assert!(
            s.profile.env.iter().any(|n| n == name),
            "목록 밖 변수 {name}"
        );
    }
    assert!(names.lines().any(|n| n == "HOME"));
    assert!(!names.contains("SECRET") && !names.contains("API_KEY") && !names.contains("DYLD"));

    // 세션 인자: profile 명령 그대로 + MCP 설정 경로. 첫 메시지는 stdin stream JSON 한 줄.
    let args = std::fs::read_to_string(&args_dump).unwrap();
    assert!(args.lines().any(|a| a == "--strict-mcp-config"));
    assert!(args
        .lines()
        .any(|a| a.ends_with(MCP_CONFIG_FILE) && a.starts_with(&*scratch_root.to_string_lossy())));
    let stdin = std::fs::read_to_string(&stdin_dump).unwrap();
    assert_eq!(
        stdin,
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"레이어를 정리해 줘\"}}\n"
    );
    assert!(output.chat_items().len() >= 8);
}

#[test]
fn grandchild_processes_are_cleaned_up() {
    let probe = TempDir::new("exec-child");
    let pidfile = probe.join("child.pid");
    let body = format!("spawn-child {}\nout {{}}\\n\nexit 0\n", pidfile.display());
    let s = setup(
        "exec-child",
        "codex",
        &format!("{OK_PROBES_CODEX}[session]\n{body}"),
    );
    let verifier = FixtureVerifier::accepting();
    let pass = run_preflight(&s.profile, &s.target, &verifier, &s.home, &s.scratch).unwrap();
    let process = pass
        .launch("hi".into(), None, output_for(&s.profile))
        .unwrap();
    let end = process.wait();
    assert_eq!(end.reason, ProcessEndReason::Exited);
    let pid: u32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        wait_until(Duration::from_secs(5), || !process_alive(pid)),
        "하위 프로세스 {pid}가 남음"
    );
}

#[test]
fn startup_timeout_and_cancel_terminate_the_group() {
    // 시작 timeout: 첫 stdout byte 없이 대기 → profile startup(10s)보다 짧게 하려고 profile을 조정.
    let s = setup(
        "exec-timeout",
        "codex",
        &format!("{OK_PROBES_CODEX}[session]\nhang\n"),
    );
    let mut profile = s.profile.clone();
    profile.timeouts.startup_ms = 300;
    let verifier = FixtureVerifier::accepting();
    let pass = run_preflight(&profile, &s.target, &verifier, &s.home, &s.scratch).unwrap();
    let started = Instant::now();
    let end = pass
        .launch("x".into(), None, output_for(&profile))
        .unwrap()
        .wait();
    assert_eq!(end.reason, ProcessEndReason::StartupTimeout);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(std::fs::read_dir(&s.scratch).unwrap().count(), 0);

    // 취소: 출력 뒤 대기 중인 세션을 중지.
    let s = setup(
        "exec-cancel",
        "codex",
        &format!("{OK_PROBES_CODEX}[session]\nout {{\"type\":\"turn.started\"}}\\n\nhang\n"),
    );
    let pass = run_preflight(&s.profile, &s.target, &verifier, &s.home, &s.scratch).unwrap();
    let output = output_for(&s.profile);
    let process = pass.launch("x".into(), None, output.clone()).unwrap();
    assert!(wait_until(Duration::from_secs(5), || output
        .raw_stats()
        .received
        > 0));
    let handle = process.cancel_handle();
    let started = Instant::now();
    handle.cancel();
    let end = process.wait();
    assert_eq!(end.reason, ProcessEndReason::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(std::fs::read_dir(&s.scratch).unwrap().count(), 0);
}

#[test]
fn slow_ui_does_not_stop_output_reception() {
    // UI가 증분을 전혀 가져가지 않아도 32 MiB 출력을 끝까지 받고, 메모리는 원문 상한 안입니다.
    let s = setup(
        "exec-flood",
        "codex",
        &format!("{OK_PROBES_CODEX}[session]\nflood 33554432\nexit 0\n"),
    );
    let verifier = FixtureVerifier::accepting();
    let pass = run_preflight(&s.profile, &s.target, &verifier, &s.home, &s.scratch).unwrap();
    let output = output_for(&s.profile);
    let started = Instant::now();
    let end = pass
        .launch("x".into(), None, output.clone())
        .unwrap()
        .wait();
    let elapsed = started.elapsed();
    assert_eq!(end.reason, ProcessEndReason::Exited);
    assert_eq!(
        end.exit,
        Some(stellar_agent_host::process::ExitKind::Code(0))
    );
    assert_eq!(end.stdout_bytes, 33_554_432);
    let stats = output.raw_stats();
    assert_eq!(stats.received, 33_554_432);
    assert!(stats.len <= stats.cap);
    assert!(stats.dropped > 0);
    eprintln!("flood 32 MiB 수신 {elapsed:?}");
}

#[test]
fn stdin_once_profile_rejects_follow_up_and_stream_profile_accepts() {
    let probe = TempDir::new("exec-stdin");
    let dump = probe.join("stdin.txt");
    let s = setup(
        "exec-once",
        "codex",
        &format!(
            "{OK_PROBES_CODEX}[session]\nread-all\nstdin-dump {}\nexit 0\n",
            dump.display()
        ),
    );
    let verifier = FixtureVerifier::accepting();
    let pass = run_preflight(&s.profile, &s.target, &verifier, &s.home, &s.scratch).unwrap();
    let process = pass
        .launch("한 번만".into(), None, output_for(&s.profile))
        .unwrap();
    assert_eq!(
        process.send_user_message("또"),
        Err(stellar_agent_host::process::SendError::NotAccepting)
    );
    process.wait();
    assert_eq!(std::fs::read_to_string(&dump).unwrap(), "한 번만");

    let dump2 = probe.join("stdin2.txt");
    let s = setup(
        "exec-stream",
        "claude",
        &format!(
            "{OK_PROBES}[session]\nread-line\nout {{}}\\n\nread-line\nstdin-dump {}\nexit 0\n",
            dump2.display()
        ),
    );
    let pass = run_preflight(&s.profile, &s.target, &verifier, &s.home, &s.scratch).unwrap();
    let output = output_for(&s.profile);
    let process = pass
        .launch("첫째".into(), Some(mcp()), output.clone())
        .unwrap();
    assert!(wait_until(Duration::from_secs(5), || output
        .raw_stats()
        .received
        > 0));
    process.send_user_message("둘째").unwrap();
    process.wait();
    let lines: Vec<String> = std::fs::read_to_string(&dump2)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[1].contains("둘째"));
}

#[test]
fn mcp_config_carries_binding_only_in_server_env_with_private_mode() {
    use std::os::unix::fs::PermissionsExt;
    let probe = TempDir::new("exec-mcp");
    let copy = probe.join("config-copy.json");
    let mode = probe.join("args.txt");
    let s = setup(
        "exec-mcp",
        "claude",
        &format!(
            "{OK_PROBES}[session]\nargs-dump {}\nsleep 300\nout {{}}\\n\nexit 0\n",
            mode.display()
        ),
    );
    let verifier = FixtureVerifier::accepting();
    let pass = run_preflight(&s.profile, &s.target, &verifier, &s.home, &s.scratch).unwrap();
    let process = pass
        .launch("x".into(), Some(mcp()), output_for(&s.profile))
        .unwrap();
    let config = process.scratch_root().join(MCP_CONFIG_FILE);
    assert!(wait_until(Duration::from_secs(5), || config.exists()));
    let meta = std::fs::metadata(&config).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    std::fs::copy(&config, &copy).unwrap();
    process.wait();
    let text = std::fs::read_to_string(&copy).unwrap();
    assert!(text.contains("\"STELLAR_MCP_SESSION_BINDING\":\"0707"));
    assert!(text.starts_with("{\"mcpServers\":{\"stellar\":{\"type\":\"stdio\""));
    assert!(!config.exists());
}

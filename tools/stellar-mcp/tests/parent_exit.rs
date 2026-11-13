//! XIX.1.9 검증: 부모 강제 종료 fixture에서 bridge 잔존 0, stdin EOF 시 종료.
//!
//! 부모 강제 종료 fixture는 다음 구조입니다.
//!
//! ```text
//! test(stdin pipe 쓰기 끝 보유) ─ sh(→ exec sleep, 부모) ─ stellar-mcp(stdin = 그 pipe)
//! ```
//!
//! 테스트가 pipe 쓰기 끝을 계속 쥐고 있으므로 stellar-mcp의 stdin에는 EOF가 오지 않습니다. 이
//! 상태에서 부모(sh)를 SIGKILL로 강제 종료하면 stellar-mcp가 부모 종료를 직접 감지해 끝나야 합니다.

#![cfg(target_os = "macos")]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_stellar-mcp");

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!("smcp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `ps`로 본 프로세스 상태. 없으면 `None`, 있으면 상태 문자열(`Z…`는 종료된 zombie).
fn process_state(pid: u32) -> Option<String> {
    let output = Command::new("/bin/ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let state = String::from_utf8(output.stdout).unwrap().trim().to_string();
    (!state.is_empty()).then_some(state)
}

/// `pid`가 끝날 때까지(없거나 zombie) 최대 `limit` 기다립니다.
fn wait_gone(pid: u32, limit: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        match process_state(pid) {
            None => return true,
            Some(state) if state.starts_with('Z') => return true,
            Some(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    false
}

/// `path` 파일에 `needle`이 나타날 때까지 최대 `limit` 기다립니다.
fn wait_for_text(path: &PathBuf, needle: &str, limit: Duration) -> String {
    let start = Instant::now();
    loop {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        if text.contains(needle) || start.elapsed() > limit {
            return text;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn fixture_parent_force_kill_leaves_no_bridge() {
    let tmp = TempDir::new("parent-kill");
    let out = tmp.0.join("bridge.out");
    let err = tmp.0.join("bridge.err");
    // sh는 pipe(stdin)를 fd 3으로 옮겨 stellar-mcp의 stdin으로 넘기고, 자신은 pipe를 닫은 채
    // sleep으로 바뀌어 부모로 남습니다. 비대화형 sh의 background job은 기본 stdin이 /dev/null이므로
    // 명시적으로 `<&3`을 줍니다.
    let script = r#"exec 3<&0 0</dev/null
"$1" <&3 >"$2" 2>"$3" 3<&- &
echo "$!"
exec 3<&- sleep 600 >/dev/null"#;
    let mut parent = Command::new("/bin/sh")
        .args(["-c", script, "sh", BIN])
        .arg(&out)
        .arg(&err)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut pipe = parent.stdin.take().unwrap();
    let mut pid_line = String::new();
    BufReader::new(parent.stdout.take().unwrap())
        .read_line(&mut pid_line)
        .unwrap();
    let bridge: u32 = pid_line.trim().parse().unwrap();

    // bridge가 pipe로 요청을 받아 응답하는지 확인합니다(stdin은 열려 있음).
    pipe.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
        .unwrap();
    pipe.flush().unwrap();
    let reply = wait_for_text(&out, "\"id\":1", Duration::from_secs(10));
    assert_eq!(reply, "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n");
    assert!(
        process_state(bridge).is_some(),
        "강제 종료 전 bridge 실행 중"
    );

    // 부모 강제 종료(SIGKILL). 테스트는 pipe 쓰기 끝을 계속 쥐고 있어 EOF는 오지 않습니다.
    parent.kill().unwrap();
    parent.wait().unwrap();
    let gone = wait_gone(bridge, Duration::from_secs(10));
    if !gone {
        let _ = Command::new("/bin/kill")
            .args(["-9", &bridge.to_string()])
            .status();
    }
    assert!(gone, "부모 강제 종료 후 bridge 잔존");
    let log = std::fs::read_to_string(&err).unwrap();
    assert!(log.contains("부모 프로세스 감시 시작"), "{log}");
    assert!(
        log.contains("부모 프로세스 종료를 감지해 종료합니다"),
        "{log}"
    );
    assert!(
        !log.contains("입력 종료"),
        "EOF가 아니라 부모 종료로 끝남: {log}"
    );
    drop(pipe);
}

#[test]
fn stdin_eof_ends_bridge_with_exit_zero() {
    let mut child = Command::new(BIN)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdin.take());
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "stdin EOF 후 bridge 잔존"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(0));
    let output = child.wait_with_output().unwrap();
    assert!(output.stdout.is_empty());
    let log = String::from_utf8(output.stderr).unwrap();
    assert!(log.contains("입력 종료"), "{log}");
}

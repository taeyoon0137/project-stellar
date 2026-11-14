//! XIX.1.1 검증: stdio JSON-RPC protocol smoke fixture.
//!
//! 빌드된 `stellar-mcp`를 실제 프로세스로 실행해 다음을 확인합니다.
//!
//! - `tests/fixtures/protocol-smoke.in.jsonl` 입력에 대한 stdout이
//!   `protocol-smoke.out.jsonl`과 byte 단위로 같습니다.
//! - stdout의 모든 줄은 JSON-RPC 2.0 response이고, 로그는 stderr에만 있습니다.
//! - 응답은 메시지마다 즉시 flush됩니다(stdin을 닫기 전에 읽을 수 있음).
//! - 소스에 stdout으로 새는 출력 macro가 없습니다.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use stellar_mcp::json::{parse, JsonLimits, JsonValue};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run_with_input(input: &[u8], args: &[&str]) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_stellar-mcp"))
        .args(args)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let data = input.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&data);
    });
    let output = child.wait_with_output().unwrap();
    writer.join().unwrap();
    Run {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// stdout 한 줄이 JSON-RPC 2.0 response 형식인지 검사합니다.
fn assert_response_line(line: &str) {
    let doc = parse(line, JsonLimits::default()).unwrap_or_else(|e| panic!("{e}: {line}"));
    assert_eq!(
        doc.get("jsonrpc").and_then(JsonValue::as_str),
        Some("2.0"),
        "{line}"
    );
    assert!(doc.get("id").is_some(), "id 필수: {line}");
    let has_result = doc.get("result").is_some();
    let has_error = doc.get("error").is_some();
    assert!(has_result != has_error, "result·error 중 하나만: {line}");
    assert!(
        doc.get("method").is_none(),
        "서버는 request를 보내지 않음: {line}"
    );
}

#[test]
fn protocol_smoke_fixture_matches_golden_output() {
    let input = std::fs::read(Path::new(FIXTURES).join("protocol-smoke.in.jsonl")).unwrap();
    let golden =
        std::fs::read_to_string(Path::new(FIXTURES).join("protocol-smoke.out.jsonl")).unwrap();
    let run = run_with_input(&input, &[]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert_eq!(run.stdout, golden);
    for line in run.stdout.lines() {
        assert_response_line(line);
    }
    assert!(!run.stderr.is_empty(), "로그는 stderr에 있어야 함");
    for line in run.stderr.lines() {
        assert!(line.starts_with("stellar-mcp: "), "로그 형식: {line}");
    }
}

#[test]
fn responses_are_flushed_per_message() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_stellar-mcp"))
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    let mut exchange = |request: &str| -> String {
        stdin.write_all(request.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        line
    };
    let init = exchange(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
    );
    assert_response_line(init.trim_end());
    assert!(
        init.contains("\"protocolVersion\":\"2025-11-25\""),
        "{init}"
    );
    let pong = exchange(r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#);
    assert_eq!(pong, "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}\n");

    drop(stdin);
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0), "stdin 종료 시 정상 종료");
}

#[test]
fn oversized_message_is_rejected_and_stream_recovers() {
    let mut input = vec![b' '; 2 * 1024 * 1024];
    input.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n");
    let run = run_with_input(&input, &[]);
    assert_eq!(run.code, 0);
    let lines: Vec<&str> = run.stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{}", run.stdout);
    assert!(lines[0].contains("\"code\":-32600"));
    assert_eq!(lines[1], r#"{"jsonrpc":"2.0","id":1,"result":{}}"#);
}

#[test]
fn help_and_bad_arguments_never_touch_stdout() {
    let help = run_with_input(b"", &["--help"]);
    assert_eq!(help.code, 0);
    assert!(help.stdout.is_empty());
    assert!(help.stderr.contains("사용법"));

    let bad = run_with_input(b"", &["--port", "8080"]);
    assert_eq!(bad.code, 64);
    assert!(bad.stdout.is_empty());
    assert!(bad.stderr.starts_with("stellar-mcp: error: "));
}

#[test]
fn sources_have_no_stdout_print_macros() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    // 이 파일 자체가 걸리지 않도록 패턴을 조립합니다.
    let macros: Vec<String> = ["print", "println", "eprint", "eprintln", "dbg"]
        .iter()
        .map(|m| format!("{m}!("))
        .collect();
    let stdout_handle = ["io::", "stdout()"].concat();
    let mut stdout_uses = 0;
    let mut stack = vec![
        src,
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/mcp-core/src"),
    ];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for line in text.lines() {
                for m in &macros {
                    let hit = line.match_indices(m.as_str()).any(|(i, _)| {
                        // `eprintln!(` 안의 `println!(`처럼 더 긴 이름의 일부는 그 이름으로 셉니다.
                        i == 0 || !line.as_bytes()[i - 1].is_ascii_alphanumeric()
                    });
                    assert!(!hit, "{}: 출력 macro 금지: {line}", path.display());
                }
            }
            if path.file_name().is_some_and(|n| n != "main.rs") {
                assert!(
                    !text.contains(&stdout_handle),
                    "{}: stdout handle은 main.rs에서만",
                    path.display()
                );
            } else {
                stdout_uses += text.matches(&stdout_handle).count();
            }
        }
    }
    assert_eq!(stdout_uses, 1, "stdout handle은 main.rs에서 한 번만 엽니다");
}

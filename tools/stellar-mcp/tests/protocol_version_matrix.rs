//! XIX.1.5 검증: MCP protocol version matrix fixture.
//!
//! `tests/fixtures/protocol-version-matrix.jsonl`의 각 case마다 새 `stellar-mcp` 프로세스를 띄워
//! initialize를 보내고 다음을 확인합니다.
//!
//! - 첫 응답이 fixture의 `response`와 같습니다(지원 version은 그대로 협상, 미지원 version은
//!   `-32602`와 요청·지원 목록).
//! - 거부된 경우 서버는 initialize 전 상태로 남아, 같은 연결에서 지원 version으로 다시 initialize할
//!   수 있습니다. 수락된 경우 이어지는 `ping`이 성공합니다.
//! - 지원 목록은 `config/mcp-protocol.yaml`과 같습니다.

use std::io::Write;
use std::process::{Command, Stdio};

use stellar_mcp::json::{parse, write, JsonLimits, JsonValue};
use stellar_mcp::protocol::ProtocolConfig;

const MATRIX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/protocol-version-matrix.jsonl"
));

fn initialize(id: i64, version: Option<&JsonValue>) -> String {
    let mut params = Vec::new();
    if let Some(v) = version {
        params.push(("protocolVersion", v.clone()));
    }
    params.push(("capabilities", JsonValue::Object(Vec::new())));
    params.push((
        "clientInfo",
        JsonValue::object(vec![
            ("name", JsonValue::string("matrix-client")),
            ("version", JsonValue::string("1.0.0")),
        ]),
    ));
    write(&JsonValue::object(vec![
        ("jsonrpc", JsonValue::string("2.0")),
        ("id", JsonValue::int(id)),
        ("method", JsonValue::string("initialize")),
        ("params", JsonValue::object(params)),
    ]))
}

fn exchange(lines: &[String]) -> Vec<String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_stellar-mcp"))
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for line in lines {
            stdin.write_all(line.as_bytes()).unwrap();
            stdin.write_all(b"\n").unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn version_matrix_fixture() {
    let preferred = ProtocolConfig::builtin().unwrap().preferred().to_string();
    let mut accepted = 0;
    let mut rejected = 0;
    for row in MATRIX.lines().filter(|l| !l.trim().is_empty()) {
        let case = parse(row, JsonLimits::default()).unwrap();
        let name = case.get("case").and_then(JsonValue::as_str).unwrap();
        let expected = case.get("response").unwrap();
        let ok = expected.get("result").is_some();

        let follow_up = if ok {
            r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#.to_string()
        } else {
            initialize(2, Some(&JsonValue::string(preferred.clone())))
        };
        let replies = exchange(&[initialize(1, case.get("protocolVersion")), follow_up]);
        assert_eq!(replies.len(), 2, "{name}: {replies:?}");
        assert_eq!(replies[0], write(expected), "{name}");

        let second = parse(&replies[1], JsonLimits::default()).unwrap();
        assert!(second.get("result").is_some(), "{name}: 후속 요청 성공");
        if ok {
            accepted += 1;
        } else {
            // 거부 후 재시도한 initialize는 선호 version으로 수락됩니다.
            assert_eq!(
                second
                    .get("result")
                    .and_then(|r| r.get("protocolVersion"))
                    .and_then(JsonValue::as_str),
                Some(preferred.as_str()),
                "{name}"
            );
            rejected += 1;
        }
    }
    assert_eq!((accepted, rejected), (2, 9), "matrix 행 수");
}

#[test]
fn matrix_accepted_rows_equal_config_versions() {
    let config = ProtocolConfig::builtin().unwrap();
    let accepted: Vec<String> = MATRIX
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| parse(l, JsonLimits::default()).unwrap())
        .filter(|c| c.get("response").and_then(|r| r.get("result")).is_some())
        .filter_map(|c| {
            c.get("protocolVersion")
                .and_then(JsonValue::as_str)
                .map(str::to_string)
        })
        .collect();
    assert_eq!(accepted, config.supported());

    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/mcp-protocol.yaml"
    ))
    .unwrap();
    assert_eq!(ProtocolConfig::from_config_str(&text).unwrap(), config);
}

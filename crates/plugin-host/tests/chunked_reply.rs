//! 대용량 응답 조각 전송 (XXI.4.4 macOS chunked binary reply 부분).

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use stellar_plugin_host::json::{parse, JsonLimits, JsonValue};
use stellar_plugin_host::permission::Permission;
use stellar_plugin_host::rpc::{chunk_reply, join_chunks, RpcLimits};
use stellar_plugin_host::session::{envelope, HostSession, SystemClock};
use support::{json, Fixture};

fn big_limits() -> JsonLimits {
    JsonLimits {
        max_bytes: 512 * 1024 * 1024,
        ..JsonLimits::default()
    }
}

#[test]
fn chunks_split_on_utf8_boundaries_and_rejoin() {
    let id = JsonValue::int(3);
    let reply = envelope(
        &id,
        ("result", JsonValue::string("가나다라마바사".repeat(50))),
    );
    let messages = chunk_reply(&id, reply.clone(), 100);
    assert!(messages.len() > 5);
    let parsed: Vec<JsonValue> = messages.iter().map(|m| json(m)).collect();
    for message in &parsed {
        assert_eq!(message.get("id").and_then(JsonValue::as_i64), Some(3));
        let data = message
            .get("chunk")
            .and_then(|c| c.get("data"))
            .and_then(JsonValue::as_str)
            .unwrap();
        assert!(data.len() <= 100);
    }
    assert_eq!(join_chunks(&parsed).as_deref(), Some(reply.as_str()));
    // 빠진 조각·중복 조각은 이어 붙이지 않습니다.
    assert_eq!(join_chunks(&parsed[1..]), None);
    let mut doubled = parsed.clone();
    doubled[1] = parsed[0].clone();
    assert_eq!(join_chunks(&doubled), None);
    // 작은 응답은 그대로 하나입니다.
    assert_eq!(chunk_reply(&id, "{}".to_owned(), 100), ["{}"]);
}

#[test]
fn hundred_megabyte_reply_is_chunked_with_one_copy() {
    const PAYLOAD: usize = 100 * 1024 * 1024;
    let id = JsonValue::int(1);
    let reply = envelope(&id, ("result", JsonValue::string("a".repeat(PAYLOAD))));
    let reply_len = reply.len();
    let started = Instant::now();
    let messages = chunk_reply(&id, reply, RpcLimits::default().chunk_bytes);
    let elapsed = started.elapsed();
    let total: usize = messages.iter().map(String::len).sum();
    let count = messages.len();
    assert_eq!(count, reply_len.div_ceil(RpcLimits::default().chunk_bytes));
    // 조각 envelope 부가분만 늘어납니다(조각마다 100 bytes 미만).
    assert!(
        total < reply_len + count * 100,
        "total {total}, reply {reply_len}"
    );
    let parsed: Vec<JsonValue> = messages
        .iter()
        .map(|m| parse(m, big_limits()).unwrap())
        .collect();
    let joined = join_chunks(&parsed).unwrap();
    assert_eq!(joined.len(), reply_len);
    eprintln!(
        "100MiB 응답: 조각 {count}개, 전송 bytes {total}(원본 {reply_len}), 나누기 {elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(120));
}

#[test]
fn session_sends_large_results_as_chunks() {
    let fx = Fixture::new("plugin-chunked", 1);
    let limits = RpcLimits {
        chunk_bytes: 1024,
        ..RpcLimits::default()
    };
    let session = HostSession::spawn_with(
        fx.dispatcher(&[Permission::File]),
        Arc::new(SystemClock::default()),
        limits,
    );
    let text = "stellar-".repeat(2048);
    session.submit(&format!(
        r#"{{"stellar_rpc":1,"id":1,"method":"file.write","params":{{"path":"big.txt","text":"{text}"}}}}"#
    ));
    let written = json(&session.recv_timeout(Duration::from_secs(20)).unwrap());
    assert!(written.get("result").is_some());
    session.submit(r#"{"stellar_rpc":1,"id":2,"method":"file.read","params":{"path":"big.txt"}}"#);
    let first = json(&session.recv_timeout(Duration::from_secs(20)).unwrap());
    let count = first
        .get("chunk")
        .and_then(|c| c.get("count"))
        .and_then(JsonValue::as_i64)
        .unwrap() as usize;
    assert!(count > 10);
    let mut parts = vec![first];
    for _ in 1..count {
        parts.push(json(
            &session.recv_timeout(Duration::from_secs(20)).unwrap(),
        ));
    }
    let whole = json(&join_chunks(&parts).unwrap());
    assert_eq!(whole.get("id").and_then(JsonValue::as_i64), Some(2));
    assert_eq!(
        whole
            .get("result")
            .and_then(|r| r.get("text"))
            .and_then(JsonValue::as_str),
        Some(text.as_str())
    );
}

/// 생성 TypeScript client를 node로 띄워 Host session과 줄 단위로 주고받습니다. node가 없거나 TypeScript 형식 제거를
/// 지원하지 않으면 건너뜁니다.
#[test]
fn generated_typescript_client_talks_to_host_session() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ts_client/smoke.mts");
    let Ok(mut child) = Command::new("node")
        .arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    else {
        eprintln!("node가 없어 건너뜁니다");
        return;
    };
    let fx = Fixture::new("plugin-ts-client", 1);
    let limits = RpcLimits {
        chunk_bytes: 1024,
        ..RpcLimits::default()
    };
    let session = HostSession::spawn_with(
        fx.dispatcher(&[Permission::File]),
        Arc::new(SystemClock::default()),
        limits,
    );
    let mut stdin = child.stdin.take().unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut result = None;
    let mut chunked = false;
    for line in stdout.lines() {
        let line = line.unwrap();
        if let Some(rest) = line.strip_prefix("RESULT ") {
            result = Some(json(rest));
            break;
        }
        session.submit(&line);
        // 이 요청의 응답(조각이면 마지막 조각까지)을 node에 넘깁니다.
        loop {
            let reply = session.recv_timeout(Duration::from_secs(20)).expect("응답");
            writeln!(stdin, "{reply}").unwrap();
            let parsed = json(&reply);
            match parsed.get("chunk") {
                None => break,
                Some(chunk) => {
                    chunked = true;
                    let index = chunk.get("index").and_then(JsonValue::as_i64).unwrap();
                    let count = chunk.get("count").and_then(JsonValue::as_i64).unwrap();
                    if index + 1 == count {
                        break;
                    }
                }
            }
        }
    }
    let status = child.wait().unwrap();
    let Some(result) = result else {
        let mut err = String::new();
        std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut err).unwrap();
        if err.contains("ERR_UNKNOWN_FILE_EXTENSION") {
            eprintln!("node가 TypeScript 형식 제거를 지원하지 않아 건너뜁니다");
            return;
        }
        panic!("node client 실패({status}): {err}");
    };
    assert!(chunked, "file.read 응답은 조각으로 와야 합니다");
    assert_eq!(
        result.get("written").and_then(JsonValue::as_i64),
        Some(16384)
    );
    assert_eq!(
        result.get("same_text").and_then(JsonValue::as_bool),
        Some(true)
    );
    assert_eq!(
        stellar_plugin_host::json::write(result.get("batch").unwrap()),
        r#"["ok","ok","permission-denied"]"#
    );
    assert_eq!(
        result.get("denied").and_then(JsonValue::as_str),
        Some("permission-denied")
    );
    assert_eq!(
        result.get("plugin_id").and_then(JsonValue::as_str),
        Some(support::PLUGIN_ID)
    );
}

//! XIX.9.10 검증: MCP 경로 자원 상한 초과 fixture별 응답.
//!
//! - 호출량: 1초 창 100개를 넘는 요청은 처리하지 않고 `busy`(재시도 가능, `retry_after_ms`)입니다.
//! - page: `limit` 1,000 초과는 `invalid-arguments`, 기본 200입니다.
//! - 응답 크기: 1 MiB를 넘는 결과는 `too-large`입니다.
//! - 큰 문서에서 page를 끝까지 이어 읽고, 저장 뒤 이전 cursor는 `stale`로 거부합니다.
//!
//! 호출 대기 상한(30초)은 `stellar-mcp-core` 단위 test가 상한을 0으로 바꿔 확인합니다.

mod support;

use std::io::Write;
use std::time::Instant;

use stellar_mcp::agent_protocol::limits;
use support::*;

#[test]
fn burst_over_the_rate_window_is_answered_busy_without_processing() {
    let fx = Fixture::new("mcp-limit-rate");
    save_documents(&fx.project, &[&large_document(3)]);
    fx.grant();
    let mut client = fx.launch();
    client.initialize();
    // 응답을 기다리지 않고 한 번에 보냅니다. 처리 비용이 거의 없는 모르는 method로 창 안에 모두 도착하게 합니다.
    let total = limits::MAX_CALLS_PER_WINDOW + 50;
    let started = Instant::now();
    let mut batch = String::new();
    for id in 0..total {
        batch.push_str(&format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":\"stellar/unknown\"}}\n",
            1000 + id
        ));
    }
    client.send_raw(batch.as_bytes());
    let replies: Vec<J> = (0..total).map(|_| client.next_reply()).collect();
    let within_window = started.elapsed() < limits::RATE_WINDOW;
    let busy: Vec<&J> = replies
        .iter()
        .filter(|r| {
            r.get("error")
                .and_then(|e| e.get("data"))
                .and_then(|d| d.get("kind"))
                .is_some()
        })
        .collect();
    // 창 안의 처음 100개는 받고(모르는 method 오류), 넘는 요청은 busy입니다.
    for reply in &replies[..limits::MAX_CALLS_PER_WINDOW] {
        assert_eq!(
            reply.get("error").unwrap().get("code"),
            Some(&J::int(-32601)),
            "{reply:?}"
        );
    }
    if within_window {
        assert_eq!(busy.len(), 50, "창 안 초과 요청 수");
    } else {
        assert!(!busy.is_empty(), "부하로 창을 넘겨도 일부는 busy");
    }
    for reply in busy {
        assert_eq!(failure_kind(reply), "busy");
        let retry = reply
            .get("error")
            .unwrap()
            .get("data")
            .unwrap()
            .get("retry_after_ms")
            .and_then(J::as_i64)
            .unwrap();
        assert!((0..=1000).contains(&retry), "{retry}");
    }
    // ping은 세지 않습니다.
    assert!(client.call("ping", empty()).get("result").is_some());
    client.finish();
}

#[test]
fn page_limit_default_and_maximum_follow_the_contract() {
    let fx = Fixture::new("mcp-limit-page");
    let document = large_document(2_500);
    save_documents(&fx.project, &[&document]);
    fx.grant();
    let mut client = fx.launch();
    client.initialize();
    let base = format!("stellar://documents/{}", document.document_id());
    let first = resource_body(&client.read(&base));
    assert_eq!(first.get("limit"), Some(&J::int(200)), "기본 200");
    assert_eq!(array(first.get("items").unwrap()).len(), 200);
    let over = client.read(&format!("{base}?limit={}", limits::MAX_PAGE_ITEMS + 1));
    assert_eq!(failure_kind(&over), "invalid-arguments");
    let tool = client.tool(
        "document.validate",
        J::object(vec![("limit", J::int(limits::MAX_PAGE_ITEMS as i64 + 1))]),
    );
    assert_eq!(failure_kind(&tool), "invalid-arguments");
    // 큰 문서를 최대 page로 끝까지 읽습니다.
    let mut uri = format!("{base}?limit={}", limits::MAX_PAGE_ITEMS);
    let mut seen = 0;
    let mut stale_cursor = None;
    loop {
        let page = resource_body(&client.read(&uri));
        seen += array(page.get("items").unwrap()).len();
        match page.get("next_cursor").and_then(J::as_str) {
            Some(cursor) => {
                stale_cursor.get_or_insert_with(|| cursor.to_string());
                uri = format!("{base}?cursor={cursor}&limit={}", limits::MAX_PAGE_ITEMS);
            }
            None => break,
        }
    }
    assert_eq!(seen, 2_500);
    // 저장으로 revision이 바뀌면 이전 cursor는 stale입니다.
    save_documents(&fx.project, &[&large_document(2_501)]);
    let stale = client.read(&format!("{base}?cursor={}&limit=10", stale_cursor.unwrap()));
    assert_eq!(failure_kind(&stale), "stale");
    client.finish();
}

#[test]
fn oversized_result_is_too_large_and_session_continues() {
    let fx = Fixture::new("mcp-limit-size");
    let mut document = large_document(3);
    let nodes = document.to_records().unwrap().nodes;
    // 이름 하나가 page byte 상한보다 큰 node: 그 page는 만들 수 없습니다.
    document
        .set_display_name(nodes[0].id, "가".repeat(limits::MAX_PAGE_BYTES / 2))
        .unwrap();
    save_documents(&fx.project, &[&document]);
    fx.grant();
    let mut client = fx.launch();
    client.initialize();
    let uri = format!("stellar://documents/{}?limit=3", document.document_id());
    let reply = client.read(&uri);
    assert_eq!(failure_kind(&reply), "too-large");
    // 상한 초과 뒤에도 같은 세션을 계속 씁니다.
    assert!(client.read(STRUCTURE).get("result").is_some());
    client.finish();
}

/// 실측(XIX.9.10 보고서): 큰 문서의 page 읽기·검증·dry-run 왕복 시간. 기본 test에서는 돌리지 않습니다.
#[test]
#[ignore]
fn measure_round_trips() {
    let fx = Fixture::new("mcp-limit-measure");
    let document = large_document(100_000);
    save_documents(&fx.project, &[&document]);
    fx.grant();
    let mut client = fx.launch();
    client.initialize();
    let base = format!("stellar://documents/{}", document.document_id());
    let time = |client: &mut Client, label: &str, f: &dyn Fn(&mut Client) -> J| {
        let start = Instant::now();
        let reply = f(client);
        let bytes = json::write(&reply).len();
        let _ = writeln!(
            std::io::stderr(),
            "{label}: {:?}, {bytes} bytes",
            start.elapsed()
        );
    };
    time(&mut client, "structure", &|c| c.read(STRUCTURE));
    time(&mut client, "tree first page (limit 1000, cold)", &|c| {
        c.read(&format!("{base}?limit=1000"))
    });
    time(&mut client, "tree page (limit 1000, warm)", &|c| {
        c.read(&format!("{base}?limit=1000"))
    });
    time(&mut client, "validate (limit 1000)", &|c| {
        c.tool(
            "document.validate",
            J::object(vec![("limit", J::int(1000))]),
        )
    });
    let structure = resource_body(&client.read(STRUCTURE));
    let token = text(structure.get("revision").unwrap().get("token").unwrap()).to_string();
    let root = document.to_records().unwrap().nodes[0].id.to_string();
    let doc = document.document_id().to_string();
    time(&mut client, "propose rename", &|c| {
        c.tool(
            "document.propose_commands",
            J::object(vec![
                ("base_revision", J::string(&token)),
                ("document", J::string(&doc)),
                (
                    "commands",
                    J::Array(vec![J::object(vec![
                        ("kind", J::string("rename")),
                        ("node", J::string(&root)),
                        ("name", J::string("x")),
                    ])]),
                ),
            ]),
        )
    });
    client.finish();
}

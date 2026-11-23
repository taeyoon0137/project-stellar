//! `document.diagnostics`: headless MCP 무결성 진단 page가 `stellar diagnose`와 같은 ops 수집·필터를 쓰고 파일을 쓰지
//! 않는지 확인합니다(XXV.3.1, ADR-0095).

mod support;

use stellar_mcp::test_support::{document_with_unresolved_clips, random_valid_document};
use support::*;

fn page(client: &mut Client, arguments: J) -> J {
    tool_body(&client.tool("document.diagnostics", arguments))
}

#[test]
fn diagnostics_tool_filters_and_pages_the_shared_index_without_writes() {
    let fx = Fixture::new("mcp-diagnostics");
    let broken = document_with_unresolved_clips(7);
    let clean = random_valid_document(301);
    save_documents(&fx.project, &[&broken, &clean]);
    fx.grant();
    let before = file_digest(&fx.temp.path);
    let mut client = fx.launch();
    client.initialize();

    let all = page(&mut client, empty());
    assert_eq!(all.get("documents_checked"), Some(&J::int(2)));
    assert_eq!(all.get("diagnostics_total"), Some(&J::int(7)));
    assert_eq!(all.get("total"), Some(&J::int(7)));
    let first = &array(all.get("items").unwrap())[0];
    assert_eq!(first.get("kind").map(text), Some("missing-node"));
    assert_eq!(first.get("code").map(text), Some("unresolved-reference"));
    assert_eq!(first.get("severity").map(text), Some("warning"));
    assert_eq!(first.get("origin").map(text), Some("references"));
    assert!(text(first.get("property").unwrap()).contains("clipping"));
    let actions: Vec<&str> = array(first.get("remediation").unwrap())
        .iter()
        .map(|a| text(a.get("action").unwrap()))
        .collect();
    assert_eq!(actions, ["reveal-source", "reveal-property"]);

    // 3개씩 이어 읽으면 전체와 같은 순서·개수입니다.
    let mut ids = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut args = vec![
            ("document", J::string(broken.document_id().to_string())),
            ("kinds", J::Array(vec![J::string("missing-node")])),
            ("limit", J::int(3)),
        ];
        if let Some(c) = &cursor {
            args.push(("cursor", J::string(c)));
        }
        let body = page(&mut client, J::object(args));
        ids.extend(
            array(body.get("items").unwrap())
                .iter()
                .map(|i| text(i.get("id").unwrap()).to_string()),
        );
        match body.get("next_cursor").and_then(J::as_str) {
            Some(next) => cursor = Some(next.to_string()),
            None => break,
        }
    }
    let expected: Vec<String> = array(all.get("items").unwrap())
        .iter()
        .map(|i| text(i.get("id").unwrap()).to_string())
        .collect();
    assert_eq!(ids, expected);

    for (arguments, total) in [
        (
            J::object(vec![("kinds", J::Array(vec![J::string("missing-font")]))]),
            0,
        ),
        (J::object(vec![("project_only", J::Bool(true))]), 0),
        (J::object(vec![("min_severity", J::string("error"))]), 0),
        (
            J::object(vec![(
                "document",
                J::string(clean.document_id().to_string()),
            )]),
            0,
        ),
    ] {
        assert_eq!(
            page(&mut client, arguments).get("total"),
            Some(&J::int(total))
        );
    }
    for bad in [
        J::object(vec![("kinds", J::Array(vec![J::string("nope")]))]),
        J::object(vec![("min_severity", J::string("fatal"))]),
        J::object(vec![("path", J::string("."))]),
        J::object(vec![
            ("project_only", J::Bool(true)),
            ("document", J::string(broken.document_id().to_string())),
        ]),
    ] {
        assert_eq!(
            failure_kind(&client.tool("document.diagnostics", bad)),
            "invalid-arguments"
        );
    }
    client.finish();
    assert_eq!(file_digest(&fx.temp.path), before, "읽기 전용");
}

//! IX.11.28: MCP `document.copy_css`의 `layout` 값 출처(ADR-0082 결정 5, ADR-0087 결정 3).
//!
//! `tools/stellar-cli/tests/layout_provenance.rs`와 같은 fixture로 같은 golden을 비교합니다. 두 test가 한 파일과
//! 같으므로 `stellar query … node`의 `node.layout`과 MCP 결과가 같습니다.

mod support;

use support::*;

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/layout-provenance.json"
);

#[test]
fn copy_css_layout_matches_the_shared_cli_mcp_golden() {
    let fx = Fixture::new("mcp-layout-provenance");
    let model = large_document(4);
    save_documents(&fx.project, &[&model]);
    let manifest = fx.project.join("stellar.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, format!("{text}style_defaults: compact\n")).unwrap();
    let sets = fx.project.join("design/style-defaults");
    std::fs::create_dir_all(&sets).unwrap();
    std::fs::write(
        sets.join("compact.stellar.yaml"),
        "extends: react-native\nframe:\n  row-gap: 8px\n",
    )
    .unwrap();
    fx.grant();
    let before = file_digest(&fx.temp.path);

    let mut client = fx.launch();
    client.initialize();
    let frame = model.definition().pages[0].views[0].root_frame;
    let reply = client.tool(
        "document.copy_css",
        J::object(vec![
            ("document", J::string(model.document_id().to_string())),
            ("node", J::string(frame.to_string())),
        ]),
    );
    let body = tool_body(&reply);
    let layout = body.get("layout").unwrap();
    assert_eq!(
        format!("{}\n", json::write(layout)),
        std::fs::read_to_string(GOLDEN).unwrap()
    );
    client.finish();
    assert_eq!(file_digest(&fx.temp.path), before, "읽기만 함");
}

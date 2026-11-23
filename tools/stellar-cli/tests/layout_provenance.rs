//! IX.11.28: `stellar query <경로> node <문서> <node>`의 `node.layout` 값 출처(ADR-0082 결정 5, ADR-0087 결정 3).
//!
//! 같은 fixture(고정 project id, `large_document(4)`, 사용자 세트 `compact`)에서 MCP `document.copy_css`의
//! `layout`도 같은 golden(`tools/stellar-mcp/tests/fixtures/layout-provenance.json`)과 비교합니다. 두 test가 한
//! 파일과 같으므로 CLI와 MCP 결과가 같습니다. golden은 `STELLAR_CLI_BLESS=1`로만 다시 씁니다.

use std::process::{Command, Stdio};

use stellar_cli::json::{self, JsonLimits, JsonValue};
use stellar_ops::test_support::{large_document, save_documents, scaffold_project, TempDir};

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../stellar-mcp/tests/fixtures/layout-provenance.json"
);

/// MCP test와 같은 fixture: 활성 세트 `compact`(react-native를 물려받고 frame `row-gap: 8px`).
fn project(root: &std::path::Path) -> (String, String) {
    scaffold_project(root);
    let model = large_document(4);
    save_documents(root, &[&model]);
    let manifest = root.join("stellar.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, format!("{text}style_defaults: compact\n")).unwrap();
    let sets = root.join("design/style-defaults");
    std::fs::create_dir_all(&sets).unwrap();
    std::fs::write(
        sets.join("compact.stellar.yaml"),
        "extends: react-native\nframe:\n  row-gap: 8px\n",
    )
    .unwrap();
    let frame = model.definition().pages[0].views[0].root_frame;
    (model.document_id().to_string(), frame.to_string())
}

#[test]
fn query_node_layout_matches_the_shared_cli_mcp_golden() {
    let tmp = TempDir::new("cli-layout-provenance");
    let root = tmp.path.join("project");
    let (document, frame) = project(&root);
    let output = Command::new(env!("CARGO_BIN_EXE_stellar"))
        .args(["query", root.to_str().unwrap(), "node", &document, &frame])
        .env_clear()
        .env("HOME", std::env::temp_dir())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope = json::parse(
        std::str::from_utf8(&output.stdout).unwrap().trim_end(),
        JsonLimits::default(),
    )
    .unwrap();
    let layout = envelope
        .get("result")
        .and_then(|r| r.get("node"))
        .and_then(|n| n.get("layout"))
        .unwrap();
    let text = format!("{}\n", json::write(layout));
    if std::env::var_os("STELLAR_CLI_BLESS").is_some() {
        std::fs::write(GOLDEN, &text).unwrap();
    }
    assert_eq!(text, std::fs::read_to_string(GOLDEN).unwrap());

    // 출처: 세트 사슬의 값은 활성 세트 이름, 명시 값은 explicit.
    let JsonValue::Array(values) = layout.get("values").unwrap() else {
        panic!("values 배열이 아님");
    };
    let find = |name: &str| {
        let v = values
            .iter()
            .find(|v| v.get("longhand").and_then(JsonValue::as_str) == Some(name))
            .unwrap();
        (
            v.get("value").and_then(JsonValue::as_str).unwrap(),
            v.get("source").and_then(JsonValue::as_str).unwrap(),
        )
    };
    assert_eq!(find("row-gap"), ("8px", "default(compact)"));
    assert_eq!(find("flex-direction"), ("column", "default(compact)"));
    assert_eq!(find("width").1, "explicit");
}

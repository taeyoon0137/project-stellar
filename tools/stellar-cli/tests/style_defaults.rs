//! `stellar style-defaults list|show|set|resolve` (SD.5·SD.6, ADR-0082, ADR-0086).

use std::process::{Command, Output, Stdio};

use stellar_cli::json::{self, JsonValue};
use stellar_ops::style_defaults::layout_record::{LayoutLonghand, LayoutRecord};
use stellar_ops::test_support::{large_document, save_documents, scaffold_project, TempDir};

fn stellar(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stellar"))
        .args(args)
        .env_clear()
        .env("HOME", std::env::temp_dir())
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn envelope(output: &Output, exit: i32) -> JsonValue {
    assert_eq!(
        output.status.code(),
        Some(exit),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::str::from_utf8(&output.stdout).unwrap();
    json::parse(text.trim_end(), json::JsonLimits::default()).unwrap()
}

fn result(value: &JsonValue) -> &JsonValue {
    value.get("result").unwrap()
}

fn text<'a>(value: &'a JsonValue, key: &str) -> &'a str {
    value.get(key).unwrap().as_str().unwrap()
}

fn items(value: &JsonValue) -> &[JsonValue] {
    let JsonValue::Array(items) = value else {
        panic!("배열이 아님");
    };
    items
}

/// 저장된 프로젝트와 (문서 id, `display: flex`·`row-gap: 8px`를 명시한 Frame id).
fn project(tmp: &TempDir) -> (std::path::PathBuf, String, String) {
    let root = tmp.path.join("project");
    scaffold_project(&root);
    let mut model = large_document(4);
    let frame = model
        .to_records()
        .unwrap()
        .nodes
        .iter()
        .find(|n| n.kind().name() == "frame")
        .expect("Frame")
        .id;
    let mut record = LayoutRecord::new(frame);
    for (longhand, value) in [
        (LayoutLonghand::Display, "flex"),
        (LayoutLonghand::RowGap, "8px"),
        // 세트(react-native) 값과 같아도 명시로 남습니다.
        (LayoutLonghand::FlexDirection, "column"),
    ] {
        record
            .set(longhand, longhand.parse(value).unwrap())
            .unwrap();
    }
    model.set_layout_record(record).unwrap();
    save_documents(&root, &[&model]);
    (root, model.document_id().to_string(), frame.to_string())
}

fn sources(resolved: &JsonValue) -> Vec<(String, String, String)> {
    items(resolved.get("values").unwrap())
        .iter()
        .map(|v| {
            (
                text(v, "longhand").to_string(),
                text(v, "value").to_string(),
                text(v, "source").to_string(),
            )
        })
        .collect()
}

fn find<'a>(list: &'a [(String, String, String)], longhand: &str) -> &'a (String, String, String) {
    list.iter().find(|(l, _, _)| l == longhand).unwrap()
}

#[test]
fn resolve_reports_explicit_and_default_sources_and_set_switches_the_active_set() {
    let tmp = TempDir::new("cli-style-defaults");
    let (root, document, frame) = project(&tmp);
    let path = root.to_str().unwrap();

    let resolved = envelope(
        &stellar(&["style-defaults", "resolve", path, &document, &frame]),
        0,
    );
    let resolved = result(&resolved);
    assert_eq!(text(resolved, "style_defaults"), "css");
    assert_eq!(text(resolved, "class"), "frame");
    let list = sources(resolved);
    assert_eq!(list.len(), 40);
    assert_eq!(
        find(&list, "display"),
        &("display".into(), "flex".into(), "explicit".into())
    );
    assert_eq!(
        find(&list, "flex-shrink"),
        &("flex-shrink".into(), "1".into(), "default(css)".into())
    );
    assert_eq!(find(&list, "box-sizing").1, "content-box");
    // payload 크기는 명시 값.
    assert_eq!(find(&list, "width").2, "explicit");

    // 세트 바꾸기: stellar.yaml 한 줄.
    let manifest = root.join("stellar.yaml");
    let before = std::fs::read_to_string(&manifest).unwrap();
    let set = envelope(
        &stellar(&["style-defaults", "set", path, "react-native"]),
        0,
    );
    assert_eq!(text(result(&set), "previous"), "css");
    let after = std::fs::read_to_string(&manifest).unwrap();
    assert_eq!(after, format!("{before}style_defaults: react-native\n"));

    let resolved = envelope(
        &stellar(&["style-defaults", "resolve", path, &document, &frame]),
        0,
    );
    let list = sources(result(&resolved));
    assert_eq!(
        find(&list, "flex-shrink"),
        &(
            "flex-shrink".into(),
            "0".into(),
            "default(react-native)".into()
        )
    );
    assert_eq!(
        find(&list, "flex-direction"),
        &("flex-direction".into(), "column".into(), "explicit".into()),
        "세트 값과 같아도 명시"
    );
    assert_eq!(find(&list, "box-sizing").1, "border-box");

    // 없는 세트는 바꾸지 않습니다.
    let missing = stellar(&["style-defaults", "set", path, "nowhere"]);
    let value = envelope(&missing, 66);
    assert_eq!(
        value.get("error").unwrap().get("code").unwrap().as_str(),
        Some("unknown-style-defaults")
    );
    assert_eq!(std::fs::read_to_string(&manifest).unwrap(), after);
}

#[test]
fn list_and_show_include_user_sets_and_reject_cycles() {
    let tmp = TempDir::new("cli-style-defaults-user");
    let (root, _, _) = project(&tmp);
    let path = root.to_str().unwrap();
    let dir = root.join("design/style-defaults");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("compact.stellar.yaml"),
        "extends: react-native\nframe:\n  row-gap: 8px\n",
    )
    .unwrap();

    let listed = envelope(&stellar(&["style-defaults", "list", path]), 0);
    let names: Vec<&str> = items(result(&listed).get("sets").unwrap())
        .iter()
        .map(|s| text(s, "name"))
        .collect();
    assert_eq!(names, ["css", "react-native", "compact"]);
    assert_eq!(text(result(&listed), "active"), "css");

    let shown = envelope(&stellar(&["style-defaults", "show", path, "compact"]), 0);
    let chain: Vec<&str> = items(result(&shown).get("chain").unwrap())
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(chain, ["compact", "react-native", "css"]);
    let row_gap = items(result(&shown).get("values").unwrap())
        .iter()
        .find(|row| text(row, "longhand") == "row-gap")
        .unwrap();
    assert_eq!(text(row_gap, "frame"), "8px");
    assert_eq!(text(row_gap, "text"), "normal");

    envelope(&stellar(&["style-defaults", "set", path, "compact"]), 0);
    std::fs::write(dir.join("compact.stellar.yaml"), "extends: compact\n").unwrap();
    let cycle = envelope(&stellar(&["style-defaults", "show", path]), 65);
    assert_eq!(
        cycle.get("error").unwrap().get("code").unwrap().as_str(),
        Some("style-defaults-cycle")
    );
}

#[test]
fn usage_errors_go_to_stderr_with_exit_64() {
    let output = stellar(&["style-defaults", "bogus"]);
    assert_eq!(output.status.code(), Some(64));
    assert!(output.stdout.is_empty());
}

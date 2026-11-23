//! `stellar css copy|paste`와 `stellar query`·`stellar apply`의 CSS longhand 속성(IX.11.15, IX.11.22).

use std::path::Path;
use std::process::{Command, Output, Stdio};

use stellar_cli::json::{self, JsonValue};
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

fn names(list: &JsonValue) -> Vec<String> {
    let JsonValue::Array(items) = list else {
        panic!("배열이 아님");
    };
    items
        .iter()
        .map(|item| item.get("name").unwrap().as_str().unwrap().to_string())
        .collect()
}

/// 저장된 프로젝트와 (문서 id, 잠기지 않은 Frame id).
fn project(tmp: &TempDir) -> (std::path::PathBuf, String, String) {
    let root = tmp.path.join("project");
    scaffold_project(&root);
    let model = large_document(4);
    save_documents(&root, &[&model]);
    let records = model.to_records().unwrap();
    let frame = records
        .nodes
        .iter()
        .find(|n| n.kind().name() == "frame" && !n.locked)
        .expect("Frame")
        .id;
    (root, model.document_id().to_string(), frame.to_string())
}

#[test]
fn css_copy_reports_longhand_names_and_paste_applies_one_transaction() {
    let tmp = TempDir::new("cli-css");
    let (root, document, frame) = project(&tmp);
    let path = root.to_str().unwrap();
    let copied = envelope(&stellar(&["css", "copy", path, &document, &frame]), 0);
    let declarations = names(result(&copied).get("declarations").unwrap());
    for longhand in [
        "width",
        "height",
        "background-color",
        "border-top-left-radius",
        "border-bottom-left-radius",
    ] {
        assert!(
            declarations.iter().any(|n| n == longhand),
            "{longhand}: {declarations:?}"
        );
    }
    // 묶음 key 이름과 이전 modifier 이름(`fill.color` 등)은 나오지 않습니다.
    assert!(declarations
        .iter()
        .all(|n| n != "size" && n != "border-radius" && !n.contains('.')));
    let text = result(&copied).get("text").unwrap().as_str().unwrap();
    assert!(text.lines().all(|l| l.ends_with(';')), "{text}");

    let input = tmp.path.join("block.css");
    std::fs::write(
        &input,
        "width: 333px;\nborder-radius: 6px 2px;\nmargin: 4px;\nleft: calc(50% + 4px);\n",
    )
    .unwrap();
    let pasted = envelope(
        &stellar(&[
            "css",
            "paste",
            path,
            &document,
            &frame,
            input.to_str().unwrap(),
        ]),
        0,
    );
    let applied = names(result(&pasted).get("applied").unwrap());
    assert_eq!(
        applied,
        [
            "width",
            "border-top-left-radius",
            "border-top-right-radius",
            "border-bottom-right-radius",
            "border-bottom-left-radius"
        ]
    );
    let skipped = result(&pasted).get("skipped").unwrap();
    assert_eq!(names(skipped), ["margin", "left"]);
    let JsonValue::Array(skipped) = skipped else {
        unreachable!()
    };
    let codes: Vec<&str> = skipped
        .iter()
        .map(|s| s.get("code").unwrap().as_str().unwrap())
        .collect();
    assert_eq!(codes, ["unknown-property", "unsupported-value"]);
    assert_eq!(result(&pasted).get("mode").unwrap().as_str(), Some("write"));
    let diagnostics = pasted.get("diagnostics").unwrap();
    let JsonValue::Array(diagnostics) = diagnostics else {
        panic!("diagnostics");
    };
    assert_eq!(diagnostics.len(), 2, "건너뜀은 경고 진단으로도 보고");

    // 다시 복사하면 붙여넣은 값이 longhand로 보입니다.
    let after = envelope(&stellar(&["css", "copy", path, &document, &frame]), 0);
    let text = result(&after).get("text").unwrap().as_str().unwrap();
    assert!(text.contains("width: 333px;\n"), "{text}");
    assert!(text.contains("border-top-right-radius: 2px;\n"), "{text}");

    // `stellar query node`의 속성 경로는 longhand key(0x0108 width, 0x010b 모서리)이고 묶음 key(0x0101·0x0106)가
    // 아닙니다.
    let node = envelope(&stellar(&["query", path, "node", &document, &frame]), 0);
    let rendered = format!("{node:?}");
    assert!(rendered.contains("0x0108"), "{rendered}");
    assert!(rendered.contains("0x010b"));
    assert!(!rendered.contains("\"0x0101\"") && !rendered.contains("\"0x0106\""));
}

#[test]
fn css_paste_dry_run_and_errors_do_not_write() {
    let tmp = TempDir::new("cli-css-dry");
    let (root, document, frame) = project(&tmp);
    let path = root.to_str().unwrap();
    let input = tmp.path.join("block.css");
    std::fs::write(&input, "width: 12px").unwrap();
    let before = envelope(&stellar(&["css", "copy", path, &document, &frame]), 0);
    let dry = envelope(
        &stellar(&[
            "css",
            "paste",
            path,
            &document,
            &frame,
            input.to_str().unwrap(),
            "--dry-run",
        ]),
        0,
    );
    assert_eq!(result(&dry).get("mode").unwrap().as_str(), Some("dry-run"));
    let after = envelope(&stellar(&["css", "copy", path, &document, &frame]), 0);
    assert_eq!(
        result(&before).get("text"),
        result(&after).get("text"),
        "dry-run은 쓰지 않음"
    );
    std::fs::write(&input, "a { b: c").unwrap();
    let broken = stellar(&[
        "css",
        "paste",
        path,
        &document,
        &frame,
        input.to_str().unwrap(),
    ]);
    assert_eq!(broken.status.code(), Some(65));
    let usage = stellar(&["css", "copy", path]);
    assert_eq!(usage.status.code(), Some(64));
    let missing = stellar(&[
        "css",
        "copy",
        path,
        &document,
        "00000000-0000-7000-8000-000000000999",
    ]);
    assert_eq!(missing.status.code(), Some(66));
    let _ = Path::new(path);
}

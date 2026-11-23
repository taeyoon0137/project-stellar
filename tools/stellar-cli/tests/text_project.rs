//! V.10.4 다중 파일 원본 CLI: `text-source export-project|check-project`, `query --source-span`.
//!
//! 기존 generation을 내보낸 텍스트 원본의 값이 `stellar query`가 generation에서 읽은 값과 같은지 확인합니다.

use std::process::{Command, Output, Stdio};

use stellar_cli::json::{parse, JsonLimits, JsonValue};
use stellar_ops::test_support::{
    file_digest, large_document, random_valid_document, save_documents, scaffold_project, TempDir,
};

fn stellar(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stellar"))
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn result(args: &[&str], code: i32) -> JsonValue {
    let output = stellar(args);
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    parse(stdout.trim_end(), JsonLimits::default())
        .unwrap()
        .get("result")
        .cloned()
        .unwrap_or(JsonValue::Null)
}

fn string<'a>(value: &'a JsonValue, key: &str) -> &'a str {
    value.get(key).unwrap().as_str().unwrap()
}

#[test]
fn export_check_and_query_agree_on_values_and_never_touch_the_project() {
    let project = TempDir::new("text-project-cli");
    let output = TempDir::new("text-project-cli-out");
    scaffold_project(&project.path);
    let document = large_document(6);
    save_documents(&project.path, &[&document]);
    let before = file_digest(&project.path);
    let design = output.path.join("design");
    let root = project.path.to_str().unwrap();
    let exported = result(
        &[
            "text-source",
            "export-project",
            root,
            design.to_str().unwrap(),
        ],
        0,
    );
    assert_eq!(string(&exported, "operation"), "export-project");
    let checked = result(
        &["text-source", "check-project", design.to_str().unwrap()],
        0,
    );
    for key in ["document", "source_hash", "file_count", "node_count"] {
        assert_eq!(exported.get(key), checked.get(key), "{key}");
    }
    assert_eq!(file_digest(&project.path), before);
    // 다시 내보내면 같은 bytes입니다. 이미 있는 폴더에는 쓰지 않습니다.
    let again = output.path.join("again");
    let second = result(
        &[
            "text-source",
            "export-project",
            root,
            again.to_str().unwrap(),
        ],
        0,
    );
    assert_eq!(second.get("source_hash"), exported.get("source_hash"));
    let refused = stellar(&[
        "text-source",
        "export-project",
        root,
        design.to_str().unwrap(),
    ]);
    assert_ne!(refused.status.code(), Some(0));
    // 프로젝트 안으로 내보내면 두 번째 원본이 되므로 거부합니다.
    let inside = project.path.join("text");
    let inside_out = stellar(&[
        "text-source",
        "export-project",
        root,
        inside.to_str().unwrap(),
    ]);
    assert_eq!(inside_out.status.code(), Some(69));
    assert!(!inside.exists());

    // query가 generation에서 읽은 root Frame 폭과 텍스트 원본의 줄이 같은 값입니다.
    let document_id = string(&exported, "document").to_owned();
    let tree = result(&["query", root, "tree", &document_id], 0);
    let node_id = string(&tree.get("nodes").unwrap().as_array().unwrap()[0], "id").to_owned();
    let node = result(&["query", root, "node", &document_id, &node_id], 0);
    let properties = node
        .get("node")
        .unwrap()
        .get("properties")
        .unwrap()
        .as_array()
        .unwrap();
    let width = properties
        .iter()
        .find(|p| string(p, "path") == "width" && string(p, "key") == "0x0108")
        .unwrap()
        .get("value")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .clone();
    let span = result(
        &["query", design.to_str().unwrap(), "--source-span", &node_id],
        0,
    );
    let file = string(&span, "file");
    let start = span.get("start_line").unwrap().as_i64().unwrap() as usize;
    let end = span.get("end_line").unwrap().as_i64().unwrap() as usize;
    let text = std::fs::read_to_string(design.join(file)).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[start - 1], "  -");
    assert_eq!(lines[start], format!("    id: \"{node_id}\""));
    let width_line = lines[start - 1..end]
        .iter()
        .find(|line| line.trim_start().starts_with("width: "))
        .unwrap();
    let text_width: f64 = width_line
        .trim_start()
        .trim_start_matches("width: \"")
        .trim_end_matches("px\"")
        .parse()
        .unwrap();
    let query_width: f64 = stellar_cli::json::write(&width).parse().unwrap();
    assert_eq!(text_width, query_width, "{width_line}");
    // 없는 node는 찾을 수 없음입니다.
    let missing = stellar(&[
        "query",
        design.to_str().unwrap(),
        "--source-span",
        "01a318c7-1fc1-7334-93f0-000000000000",
    ]);
    assert_ne!(missing.status.code(), Some(0));
}

#[test]
fn random_corpus_projects_export_and_check() {
    for seed in [3_u64, 17, 42] {
        let project = TempDir::new("text-project-corpus");
        let output = TempDir::new("text-project-corpus-out");
        scaffold_project(&project.path);
        save_documents(&project.path, &[&random_valid_document(seed)]);
        let design = output.path.join("design");
        let exported = result(
            &[
                "text-source",
                "export-project",
                project.path.to_str().unwrap(),
                design.to_str().unwrap(),
            ],
            0,
        );
        let checked = result(
            &["text-source", "check-project", design.to_str().unwrap()],
            0,
        );
        assert_eq!(exported.get("source_hash"), checked.get("source_hash"));
    }
}

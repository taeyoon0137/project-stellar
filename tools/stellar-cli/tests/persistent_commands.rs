//! ADR-0079 command 종류를 `stellar apply`로 적용하고 저장 세대를 다시 열어 확인합니다(UX.12.1).

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use stellar_cli::json::{parse, JsonLimits, JsonValue};
use stellar_ops::commands::codec::encode_command;
use stellar_ops::commands::command::Command as EditCommand;
use stellar_ops::commands::page_view::{AddView, RenamePage};
use stellar_ops::core_types::ids::StableId;
use stellar_ops::test_support::{large_document, save_documents, scaffold_project, TempDir};

fn stellar(args: &[&str], input: Option<&str>, home: &Path) -> (i32, JsonValue) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_stellar"));
    command
        .args(args)
        .env_clear()
        .env("HOME", home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = command.spawn().unwrap();
    if let Some(input) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let envelope = parse(stdout.trim_end(), JsonLimits::default())
        .unwrap_or_else(|e| panic!("{e:?}: {stdout}"));
    (output.status.code().unwrap(), envelope)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn canonical(command: &EditCommand) -> String {
    format!(
        r#"{{"kind":"{}","canonical":"{}"}}"#,
        command.kind().name(),
        hex(&encode_command(command))
    )
}

fn result(envelope: &JsonValue) -> &JsonValue {
    envelope.get("result").unwrap()
}

fn str_of<'a>(value: &'a JsonValue, key: &str) -> &'a str {
    value.get(key).and_then(JsonValue::as_str).unwrap()
}

fn pages(root: &Path, home: &Path) -> (String, Vec<JsonValue>) {
    let (code, envelope) = stellar(&["query", root.to_str().unwrap(), "project"], None, home);
    assert_eq!(code, 0, "{envelope:?}");
    let result = result(&envelope);
    let token = str_of(result.get("revision").unwrap(), "token").to_string();
    let documents = match result.get("documents") {
        Some(JsonValue::Array(documents)) => documents.clone(),
        other => panic!("documents: {other:?}"),
    };
    match documents[0].get("pages") {
        Some(JsonValue::Array(pages)) => (token, pages.clone()),
        other => panic!("pages: {other:?}"),
    }
}

fn views(page: &JsonValue) -> Vec<JsonValue> {
    match page.get("views") {
        Some(JsonValue::Array(views)) => views.clone(),
        other => panic!("views: {other:?}"),
    }
}

#[test]
fn apply_add_view_and_rename_page_survive_reopen() {
    let dir = TempDir::new("persistent-commands-cli");
    let root = dir.path.join("project");
    let home = dir.path.join("home");
    scaffold_project(&root);
    let model = large_document(3);
    save_documents(&root, &[&model]);
    let document = model.document_id();
    let page = model.definition().pages[0].id;
    let first_view = model.definition().pages[0].views[0].id;

    let view = StableId::new_v7(5_000_000_000_000, 1, 1);
    let view_root = StableId::new_v7(5_000_000_000_001, 1, 2);
    let add_view = EditCommand::AddView(AddView::empty(page, 1, view, "두 번째 화면", view_root));
    let rename = EditCommand::RenamePage(RenamePage {
        page,
        name: "표지 페이지".into(),
    });
    let (base, before) = pages(&root, &home);
    assert_eq!(views(&before[0]).len(), 1);
    let input = format!(
        r#"{{"format":"stellar-apply","format_version":1,"base_revision":"{base}","document":"{document}","commands":[{},{}]}}"#,
        canonical(&add_view),
        canonical(&rename)
    );
    let (code, envelope) = stellar(&["apply", root.to_str().unwrap(), "-"], Some(&input), &home);
    assert_eq!(code, 0, "{envelope:?}");

    // 저장 세대를 다시 열어(별도 프로세스 query) 두 변경이 함께 들어갔는지 확인합니다.
    let (after_token, after) = pages(&root, &home);
    assert_ne!(after_token, base, "새 저장 세대");
    assert_eq!(str_of(&after[0], "name"), "표지 페이지");
    let after_views = views(&after[0]);
    assert_eq!(after_views.len(), 2);
    assert_eq!(str_of(&after_views[0], "id"), first_view.to_string());
    assert_eq!(str_of(&after_views[1], "id"), view.to_string());
    assert_eq!(str_of(&after_views[1], "name"), "두 번째 화면");
    assert_eq!(str_of(&after_views[1], "root_frame"), view_root.to_string());
    let (code, node) = stellar(
        &[
            "query",
            root.to_str().unwrap(),
            "node",
            &document.to_string(),
            &view_root.to_string(),
        ],
        None,
        &home,
    );
    assert_eq!(code, 0, "{node:?}");
    assert_eq!(
        result(&node)
            .get("node")
            .and_then(|n| n.get("kind"))
            .and_then(JsonValue::as_str),
        Some("frame")
    );

    // 같은 base로 다시 적용하면 낡은 revision이라 거부되고 아무것도 바뀌지 않습니다.
    let (code, _) = stellar(&["apply", root.to_str().unwrap(), "-"], Some(&input), &home);
    assert_ne!(code, 0);
    assert_eq!(pages(&root, &home).0, after_token);
}

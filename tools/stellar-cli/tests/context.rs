//! `stellar apply`의 Component Context 편집 입력과 `stellar context` 조회(XXX.7.8·XXX.7.11 남은 범위, ADR-0114).

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use stellar_cli::json::{parse, JsonLimits, JsonValue};
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

fn result(envelope: &JsonValue) -> &JsonValue {
    envelope.get("result").unwrap()
}

fn str_of<'a>(value: &'a JsonValue, key: &str) -> &'a str {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .unwrap_or_else(|| panic!("{key}: {value:?}"))
}

fn array<'a>(value: &'a JsonValue, key: &str) -> &'a [JsonValue] {
    match value.get(key) {
        Some(JsonValue::Array(items)) => items,
        other => panic!("{key}: {other:?}"),
    }
}

fn token(root: &Path, home: &Path) -> String {
    let (code, envelope) = stellar(&["query", root.to_str().unwrap(), "project"], None, home);
    assert_eq!(code, 0, "{envelope:?}");
    str_of(result(&envelope).get("revision").unwrap(), "token").to_string()
}

fn sid(n: u64) -> StableId {
    StableId::new_v7(6_000_000_000_000 + n, 0x0c0c, n)
}

#[test]
fn apply_edits_context_and_context_reads_it_back() {
    let dir = TempDir::new("context-cli");
    let root = dir.path.join("project");
    let home = dir.path.join("home");
    scaffold_project(&root);
    let model = large_document(3);
    save_documents(&root, &[&model]);
    let document = model.document_id();
    let view = &model.definition().pages[0].views[0];
    let (key, provision) = (sid(1), sid(2));
    let root_path = root.to_str().unwrap();

    // 1) key 하나와 View root의 provision 하나를 한 transaction으로 만듭니다.
    let commands = format!(
        r#"[{{"kind":"create-context-key","key":"{key}","name":"spacing","scope":"document {document}","value_type":"length","default":"length 8px"}},{{"kind":"create-context-provision","provision":"{provision}","owner":"view {view}","provider":"{root_frame}","key":"{key}","value":"length 12px"}}]"#,
        view = view.id,
        root_frame = view.root_frame,
    );
    let input = |base: &str, commands: &str| {
        format!(
            r#"{{"format":"stellar-apply","format_version":1,"base_revision":"{base}","document":"{document}","commands":{commands}}}"#
        )
    };
    // dry-run의 semantic diff에 Context 객체가 나타납니다.
    let base = token(&root, &home);
    let (code, envelope) = stellar(
        &["apply", root_path, "-", "--dry-run"],
        Some(&input(&base, &commands)),
        &home,
    );
    assert_eq!(code, 0, "{envelope:?}");
    let printed = stellar_cli::json::write(result(&envelope));
    assert!(printed.contains("context-key"), "{printed}");
    assert!(printed.contains("context-provision"), "{printed}");
    let (code, envelope) = stellar(
        &["apply", root_path, "-"],
        Some(&input(&base, &commands)),
        &home,
    );
    assert_eq!(code, 0, "{envelope:?}");

    // 2) 다시 연 저장 세대에서 선언을 읽습니다(값·식은 입력과 같은 문법).
    let (code, envelope) = stellar(&["context", root_path, &document.to_string()], None, &home);
    assert_eq!(code, 0, "{envelope:?}");
    let report = result(&envelope);
    let keys = array(report, "keys");
    assert_eq!(keys.len(), 1);
    assert_eq!(str_of(&keys[0], "name"), "spacing");
    assert_eq!(str_of(&keys[0], "default"), "length 8px");
    assert_eq!(str_of(&keys[0], "scope"), format!("document {document}"));
    let provisions = array(report, "provisions");
    assert_eq!(str_of(&provisions[0], "value"), "length 12px");
    assert_eq!(str_of(&provisions[0], "owner"), format!("view {}", view.id));
    assert!(report.get("site") == Some(&JsonValue::Null));

    // 3) View root 위치의 해석: provision 값과 출처, 선택기 후보.
    let (code, envelope) = stellar(
        &[
            "context",
            root_path,
            &document.to_string(),
            "--node",
            &view.root_frame.to_string(),
        ],
        None,
        &home,
    );
    assert_eq!(code, 0, "{envelope:?}");
    let site = result(&envelope).get("site").unwrap();
    let resolutions = array(site, "resolutions");
    assert_eq!(str_of(&resolutions[0], "value"), "length 12px");
    let source = resolutions[0].get("source").unwrap();
    assert_eq!(str_of(source, "kind"), "provision");
    assert_eq!(str_of(source, "provision"), provision.to_string());
    let candidates = array(site, "candidates");
    assert_eq!(str_of(&candidates[0], "name"), "spacing");
    assert_eq!(
        candidates[0].get("needs_declaration"),
        Some(&JsonValue::Bool(false)),
        "View 최상위 트리는 선언이 필요 없음"
    );

    // 4) provision을 지우면 같은 위치가 key default로 해석됩니다.
    let base = token(&root, &home);
    let delete = format!(r#"[{{"kind":"delete-context-provision","provision":"{provision}"}}]"#);
    let (code, envelope) = stellar(
        &["apply", root_path, "-"],
        Some(&input(&base, &delete)),
        &home,
    );
    assert_eq!(code, 0, "{envelope:?}");
    let (_, envelope) = stellar(
        &[
            "context",
            root_path,
            &document.to_string(),
            "--node",
            &view.root_frame.to_string(),
        ],
        None,
        &home,
    );
    let resolutions = array(result(&envelope).get("site").unwrap(), "resolutions");
    assert_eq!(str_of(&resolutions[0], "value"), "length 8px");
    assert_eq!(
        str_of(resolutions[0].get("source").unwrap(), "kind"),
        "default"
    );

    // 5) 열린 식 default·없는 위치는 거부합니다.
    let base = token(&root, &home);
    let open_default = format!(
        r#"[{{"kind":"create-context-key","key":"{}","name":"alias","scope":"project","value_type":"length","default":"context {key}"}}]"#,
        sid(3)
    );
    let (code, envelope) = stellar(
        &["apply", root_path, "-"],
        Some(&input(&base, &open_default)),
        &home,
    );
    assert_ne!(code, 0, "{envelope:?}");
    let (code, _) = stellar(
        &[
            "context",
            root_path,
            &document.to_string(),
            "--node",
            &sid(99).to_string(),
        ],
        None,
        &home,
    );
    assert_ne!(code, 0);
}

//! 실제 저장된 두 View와 실제 stellar 프로세스의 제품 경로.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use stellar_cli::json::{self, JsonValue};
use stellar_ops::snapshot::open_snapshot;
use stellar_ops::test_support::{
    accessibility as fixture, corrupt_view_tree_chunks, file_digest, save_documents,
    save_documents_with_schema, scaffold_project, TempDir,
};

fn run(path: &Path, document: &str, view: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stellar"))
        .args([
            "export-accessibility",
            path.to_str().unwrap(),
            document,
            view,
        ])
        .env_clear()
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
    assert_eq!(text.matches('\n').count(), 1);
    json::parse(text.trim_end(), json::JsonLimits::default()).unwrap()
}

fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

#[test]
fn saved_two_views_export_through_stellar_deterministically_without_git_changes() {
    let dir = TempDir::new("html-two-views");
    scaffold_project(&dir.path);
    save_documents(&dir.path, &[&fixture::document()]);
    git(&dir.path, &["init", "-q"]);
    git(&dir.path, &["add", "."]);
    git(
        &dir.path,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "fixture",
        ],
    );
    let head = git(&dir.path, &["rev-parse", "HEAD"]);
    let status = git(&dir.path, &["status", "--porcelain=v1"]);
    let before = file_digest(&dir.path);
    let expected_revision = open_snapshot(&dir.path).unwrap().revision().token();
    let document = fixture::id(1).to_string();
    let a = run(&dir.path, &document, &fixture::id(3).to_string());
    let again = run(&dir.path, &document, &fixture::id(3).to_string());
    assert_eq!(a.stdout, again.stdout);
    assert_eq!(a.stderr, again.stderr);
    let a = envelope(&a, 0);
    let result = a.get("result").unwrap();
    assert_eq!(
        result
            .get("revision")
            .unwrap()
            .get("token")
            .unwrap()
            .as_str(),
        Some(expected_revision.as_str())
    );
    assert_eq!(
        result.get("root_frame").unwrap().as_str(),
        Some(fixture::id(10).to_string().as_str())
    );
    assert_eq!(
        result.get("html").unwrap().as_str(),
        Some(include_str!("fixtures/accessibility/view-a.html"))
    );
    let diagnostics = json::write(a.get("diagnostics").unwrap());
    assert!(diagnostics.contains("view-not-selected"));
    assert!(diagnostics.contains("static-accessibility-only"));
    assert!(!result
        .get("html")
        .unwrap()
        .as_str()
        .unwrap()
        .contains("button"));
    let b = envelope(&run(&dir.path, &document, &fixture::id(4).to_string()), 0);
    assert_eq!(
        b.get("result").unwrap().get("html").unwrap().as_str(),
        Some(include_str!("fixtures/accessibility/view-b.html"))
    );
    assert_eq!(git(&dir.path, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&dir.path, &["status", "--porcelain=v1"]), status);
    assert_eq!(
        file_digest(&dir.path),
        before,
        "프로젝트·Git index·refs·objects 모두 무변경"
    );
}

#[test]
fn unsupported_data_fails_only_when_its_view_is_selected_and_keeps_revision() {
    let dir = TempDir::new("html-unsupported");
    scaffold_project(&dir.path);
    save_documents(&dir.path, &[&fixture::unsupported_document()]);
    let before = file_digest(&dir.path);
    let document = fixture::id(1).to_string();
    envelope(&run(&dir.path, &document, &fixture::id(3).to_string()), 0);
    let error = envelope(&run(&dir.path, &document, &fixture::id(4).to_string()), 1);
    assert_eq!(
        error.get("error").unwrap().get("code").unwrap().as_str(),
        Some("accessibility-export-failed")
    );
    let result = error.get("result").unwrap();
    assert_eq!(result.get("html"), Some(&JsonValue::Null));
    assert!(result.get("revision").is_some());
    let diagnostics = json::write(error.get("diagnostics").unwrap());
    assert!(diagnostics.contains("unsupported-node"));
    assert!(diagnostics.contains("Image"));
    assert!(diagnostics.contains(&fixture::id(21).to_string()));
    assert_eq!(file_digest(&dir.path), before);
}

#[test]
fn preserves_carriage_returns_and_diagnoses_null_in_the_selected_view() {
    let dir = TempDir::new("html-text-characters");
    scaffold_project(&dir.path);
    let document = fixture::id(1).to_string();
    let view = fixture::id(4).to_string();
    save_documents(
        &dir.path,
        &[&fixture::document_with_second_view_text("a\rb\r\nc")],
    );
    let result = envelope(&run(&dir.path, &document, &view), 0);
    let html = result
        .get("result")
        .unwrap()
        .get("html")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(html.contains("a&#13;b&#13;\nc"));

    save_documents(
        &dir.path,
        &[&fixture::document_with_second_view_text("a\0b")],
    );
    let before = file_digest(&dir.path);
    envelope(&run(&dir.path, &document, &fixture::id(3).to_string()), 0);
    let failure = envelope(&run(&dir.path, &document, &view), 1);
    assert_eq!(
        failure.get("result").unwrap().get("html"),
        Some(&JsonValue::Null)
    );
    let diagnostics = json::write(failure.get("diagnostics").unwrap());
    assert!(diagnostics.contains("unsupported-text-character"));
    assert!(diagnostics.contains(&fixture::id(21).to_string()));
    assert_eq!(file_digest(&dir.path), before);
}

#[test]
fn missing_unsaved_unknown_and_corrupt_inputs_have_explicit_errors_and_no_writes() {
    let dir = TempDir::new("html-errors");
    let document = fixture::id(1).to_string();
    let view = fixture::id(3).to_string();
    let unknown = fixture::id(99).to_string();
    envelope(&run(&dir.path.join("missing"), &document, &view), 66);
    scaffold_project(&dir.path);
    let before = file_digest(&dir.path);
    let error = envelope(&run(&dir.path, &document, &view), 66);
    assert_eq!(
        error.get("error").unwrap().get("code").unwrap().as_str(),
        Some("unsaved-project")
    );
    assert_eq!(file_digest(&dir.path), before);
    save_documents(&dir.path, &[&fixture::document()]);
    let before = file_digest(&dir.path);
    for (doc, view, code) in [
        (&unknown, &view, "unknown-document"),
        (&document, &unknown, "unknown-view"),
    ] {
        let error = envelope(&run(&dir.path, doc, view), 66);
        assert_eq!(
            error.get("error").unwrap().get("code").unwrap().as_str(),
            Some(code)
        );
    }
    assert_eq!(file_digest(&dir.path), before);
    assert!(corrupt_view_tree_chunks(&dir.path) > 0);
    let before = file_digest(&dir.path);
    let error = envelope(&run(&dir.path, &document, &view), 65);
    assert_eq!(
        error.get("error").unwrap().get("code").unwrap().as_str(),
        Some("document")
    );
    assert_eq!(file_digest(&dir.path), before);
}

#[test]
fn newer_generation_schema_is_not_assumed_to_be_supported() {
    let dir = TempDir::new("html-newer-schema");
    scaffold_project(&dir.path);
    save_documents_with_schema(&dir.path, &[&fixture::document()], 2);
    let before = file_digest(&dir.path);
    let error = envelope(
        &run(
            &dir.path,
            &fixture::id(1).to_string(),
            &fixture::id(3).to_string(),
        ),
        65,
    );
    assert_eq!(
        error.get("error").unwrap().get("code").unwrap().as_str(),
        Some("unsupported-schema")
    );
    assert_eq!(file_digest(&dir.path), before);
}

#[test]
fn usage_errors_emit_no_json_or_files() {
    let dir = TempDir::new("html-usage");
    for args in [
        vec!["export-accessibility"],
        vec!["export-accessibility", "p", "invalid", "invalid"],
        vec!["export-accessibility", "p", "--all", "view"],
        vec!["export-accessibility", "p", "doc", "view", "extra"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_stellar"))
            .args(args)
            .current_dir(&dir.path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(64));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("--help"));
    }
    assert!(file_digest(&dir.path).is_empty());
}

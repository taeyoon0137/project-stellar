//! 실제 저장 generation에서 opt-in export·check 명령과 파일 보존 계약을 검사합니다.

use std::process::{Command, Stdio};

use stellar_cli::json::{parse, JsonLimits, JsonValue};
use stellar_ops::test_support::{
    file_digest, large_document, random_valid_document, save_documents, scaffold_project, TempDir,
};

fn run(args: &[&str], code: i32) -> JsonValue {
    let output = Command::new(env!("CARGO_BIN_EXE_stellar"))
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(code), "{:?}", output);
    if code == 0 {
        assert!(output.stderr.is_empty());
    } else {
        assert!(!output.stderr.is_empty());
    }
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    let envelope = parse(stdout.trim_end(), JsonLimits::default()).unwrap();
    assert_eq!(
        envelope.get("command"),
        Some(&JsonValue::string("text-source"))
    );
    assert_eq!(
        envelope.get("exit_code"),
        Some(&JsonValue::int(i64::from(code)))
    );
    envelope
}

#[test]
fn export_and_check_round_trip_without_changing_generation_or_overwriting_output() {
    let project = TempDir::new("text-source-cli-project");
    let output = TempDir::new("text-source-cli-output");
    scaffold_project(&project.path);
    save_documents(&project.path, &[&large_document(4)]);
    let before = file_digest(&project.path);
    let path = output.path.join("view.stellar.yaml");
    let export = run(
        &[
            "text-source",
            "export",
            project.path.to_str().unwrap(),
            path.to_str().unwrap(),
        ],
        0,
    );
    let bytes = std::fs::read(&path).unwrap();
    let output_before = file_digest(&output.path);
    let check = run(&["text-source", "check", path.to_str().unwrap()], 0);
    let export_result = export.get("result").unwrap();
    let check_result = check.get("result").unwrap();
    assert_eq!(
        export_result.get("source_hash"),
        check_result.get("source_hash")
    );
    assert_eq!(
        export_result.get("source_hash").and_then(JsonValue::as_str),
        output_before.get("view.stellar.yaml").map(String::as_str)
    );
    assert_eq!(check_result.get("node_count"), Some(&JsonValue::int(4)));
    assert_eq!(
        export_result.get("operation"),
        Some(&JsonValue::string("export"))
    );
    assert_eq!(
        check_result.get("operation"),
        Some(&JsonValue::string("check"))
    );
    assert!(!stellar_cli::json::write(&export).contains(project.path.to_str().unwrap()));
    assert!(!stellar_cli::json::write(&check).contains(path.to_str().unwrap()));
    let overwritten = run(
        &[
            "text-source",
            "export",
            project.path.to_str().unwrap(),
            path.to_str().unwrap(),
        ],
        73,
    );
    assert_eq!(
        overwritten.get("error").unwrap().get("code"),
        Some(&JsonValue::string("text-source-io"))
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(file_digest(&output.path), output_before);
    assert_eq!(file_digest(&project.path), before);
}

#[test]
fn missing_and_invalid_sources_report_errors_without_writing() {
    let dir = TempDir::new("text-source-cli-errors");
    let path = dir.path.join("source.stellar.yaml");
    run(&["text-source", "check", path.to_str().unwrap()], 66);
    assert!(!path.exists());
    std::fs::write(&path, b"invalid text source").unwrap();
    let before = file_digest(&dir.path);
    run(&["text-source", "check", path.to_str().unwrap()], 65);
    assert_eq!(file_digest(&dir.path), before);
}

#[test]
fn unsupported_multi_document_export_preserves_every_file() {
    let project = TempDir::new("text-source-cli-unsupported");
    scaffold_project(&project.path);
    save_documents(
        &project.path,
        &[&random_valid_document(300), &random_valid_document(301)],
    );
    let before = file_digest(&project.path);
    let output = project.path.join("unsupported.stellar.yaml");
    let result = run(
        &[
            "text-source",
            "export",
            project.path.to_str().unwrap(),
            output.to_str().unwrap(),
        ],
        69,
    );
    assert_eq!(
        result.get("error").unwrap().get("code"),
        Some(&JsonValue::string("text-source-unsupported"))
    );
    assert!(!output.exists());
    assert_eq!(file_digest(&project.path), before);
}

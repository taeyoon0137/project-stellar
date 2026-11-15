//! 실제 저장 generation에서 opt-in export·check와 파일 보존 계약을 검사합니다.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use stellar_cli::json::{parse, JsonLimits, JsonValue};
use stellar_ops::core_types::ids::StableId;
use stellar_ops::test_support::{
    file_digest, large_document, random_valid_document, save_documents, scaffold_project,
    text_document, FixtureDocument, TempDir,
};

fn run(args: &[&str], code: i32) -> JsonValue {
    let output = Command::new(env!("CARGO_BIN_EXE_stellar"))
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .unwrap();
    envelope(output, code)
}

fn envelope(output: Output, code: i32) -> JsonValue {
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
        check_result.get("document"),
        Some(&JsonValue::string(
            large_document(4).document_id().to_string()
        ))
    );
    assert_eq!(export_result.get("document"), check_result.get("document"));
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

/// 단일 문서 프로젝트를 저장하고 텍스트 원본으로 내보냅니다. 반환: (프로젝트, 출력 폴더, 원본 경로).
fn exported(label: &str, model: FixtureDocument) -> (TempDir, TempDir, PathBuf) {
    let project = TempDir::new(&format!("{label}-project"));
    let source = TempDir::new(&format!("{label}-source"));
    scaffold_project(&project.path);
    save_documents(&project.path, &[&model]);
    let path = source.path.join("view.stellar.yaml");
    run(
        &[
            "text-source",
            "export",
            project.path.to_str().unwrap(),
            path.to_str().unwrap(),
        ],
        0,
    );
    (project, source, path)
}

fn source_hash(envelope: &JsonValue) -> String {
    envelope
        .get("result")
        .unwrap()
        .get("source_hash")
        .and_then(JsonValue::as_str)
        .unwrap()
        .to_string()
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.name=Test User",
            "-c",
            "user.email=test@example.com",
        ])
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .expect("임시 fixture의 Git 실행");
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn v2_source_with_initial_typography_reopens_from_a_source_only_git_clone() {
    let mut model = text_document();
    let resource = StableId::new_v7(991, 0, 77);
    model.replace_text_styles(stellar_ops::test_support::DocumentTextStyles::initial(
        resource,
    ));
    let (project, source, path) = exported("text-source-v2-git", model);
    let original = file_digest(&project.path);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(&format!("  initial_typography_id: \"{resource}\"\n")));
    let annotated = format!("# collaborator comment\n{text}");
    std::fs::write(&path, &annotated).unwrap();
    let checked = run(&["text-source", "check", path.to_str().unwrap()], 0);
    git(&source.path, &["init", "--quiet"]);
    git(&source.path, &["add", "view.stellar.yaml"]);
    git(
        &source.path,
        &["commit", "--quiet", "-m", "view with initial typography"],
    );
    let dir = TempDir::new("v2-source-only-clone");
    git(
        &dir.path,
        &["clone", "--no-local", source.path.to_str().unwrap(), "clone"],
    );
    let clone = dir.path.join("clone");
    let cloned = clone.join("view.stellar.yaml");
    assert_eq!(
        run(&["text-source", "check", cloned.to_str().unwrap()], 0),
        checked
    );
    assert_eq!(std::fs::read_to_string(cloned).unwrap(), annotated);
    assert!(!clone.join("design").exists() && !clone.join(".stellar/cache").exists());
    assert!(git(&clone, &["status", "--porcelain"]).is_empty());
    assert_eq!(file_digest(&project.path), original);
}

#[test]
fn text_node_export_and_external_content_edit_preserve_payload() {
    let (project, _source, path) = exported("text-source-cli-text", text_document());
    let project_before = file_digest(&project.path);
    let original = std::fs::read_to_string(&path).unwrap();
    assert!(original.contains("kind: text\n"), "{original}");
    assert!(original.contains("Original 한글 e\u{301} 👩‍💻"), "{original}");
    assert!(original.contains("style: \"4294967295\""), "{original}");
    let hash = source_hash(&run(&["text-source", "check", path.to_str().unwrap()], 0));

    let external = original.replace("Original 한글", "External 한글");
    assert_ne!(external, original);
    std::fs::write(&path, &external).unwrap();
    let checked = run(&["text-source", "check", path.to_str().unwrap()], 0);
    assert_ne!(source_hash(&checked), hash);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), external);
    assert_eq!(file_digest(&project.path), project_before);
}

#[test]
fn invalid_text_fields_reject_check_without_overwriting() {
    let (project, source_dir, path) =
        exported("text-source-cli-invalid-text", text_document());
    let project_before = file_digest(&project.path);
    let source = std::fs::read_to_string(&path).unwrap();
    let content_line = source
        .lines()
        .find(|line| line.starts_with("      content:"))
        .unwrap();
    let cases = [
        (
            source.replace(content_line, "      content: []"),
            65,
            "text-source-invalid",
        ),
        (
            source.replace(&format!("{content_line}\n"), ""),
            65,
            "text-source-invalid",
        ),
        (
            source.replacen("style: \"17\"", "unknown_style: \"17\"", 1),
            69,
            "text-source-unsupported",
        ),
        (
            source.replacen("start: \"8\"", "start: \"13\"", 1),
            65,
            "text-source-invalid",
        ),
        (
            source.replacen("end: \"12\"", "end: \"999\"", 1),
            65,
            "text-source-invalid",
        ),
        (
            source.replace(content_line, "      content: short"),
            65,
            "text-source-invalid",
        ),
    ];
    for (invalid, status, code) in cases {
        assert_ne!(invalid, source);
        std::fs::write(&path, &invalid).unwrap();
        let before = file_digest(&source_dir.path);
        let checked = run(&["text-source", "check", path.to_str().unwrap()], status);
        assert_eq!(
            checked.get("error").unwrap().get("code"),
            Some(&JsonValue::string(code))
        );
        assert_eq!(file_digest(&source_dir.path), before);
        assert_eq!(file_digest(&project.path), project_before);
    }
}

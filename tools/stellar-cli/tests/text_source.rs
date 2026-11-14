//! 실제 저장 generation에서 opt-in export·check·apply와 파일 보존 계약을 검사합니다.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use stellar_cli::json::{parse, JsonLimits, JsonValue};
use stellar_ops::core_types::ids::StableId;
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
    envelope(output, code)
}

fn run_with_stdin(args: &[&str], input: &str, home: Option<&Path>, code: i32) -> JsonValue {
    let mut command = Command::new(env!("CARGO_BIN_EXE_stellar"));
    command.args(args).env_clear();
    if let Some(home) = home {
        command.env("HOME", home);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    envelope(child.wait_with_output().unwrap(), code)
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

struct ApplyFixture {
    project: TempDir,
    source: TempDir,
    home: TempDir,
    path: PathBuf,
    document: StableId,
    node: StableId,
    hash: String,
}

impl ApplyFixture {
    fn new(label: &str) -> Self {
        let project = TempDir::new(&format!("{label}-project"));
        let source = TempDir::new(&format!("{label}-source"));
        let home = TempDir::new(&format!("{label}-home"));
        let model = large_document(4);
        let document = model.document_id();
        let node = model
            .to_records()
            .unwrap()
            .nodes
            .into_iter()
            .find(|node| node.parent.is_some())
            .unwrap()
            .id;
        scaffold_project(&project.path);
        save_documents(&project.path, &[&model]);
        let path = source.path.join("view.stellar.yaml");
        let exported = run(
            &[
                "text-source",
                "export",
                project.path.to_str().unwrap(),
                path.to_str().unwrap(),
            ],
            0,
        );
        let hash = exported
            .get("result")
            .unwrap()
            .get("source_hash")
            .and_then(JsonValue::as_str)
            .unwrap()
            .to_string();
        Self {
            project,
            source,
            home,
            path,
            document,
            node,
            hash,
        }
    }

    fn request(&self, commands: &str) -> String {
        transaction(&self.hash, self.document, commands)
    }
}

fn transaction(hash: &str, document: StableId, commands: &str) -> String {
    format!(
        r#"{{"format":"stellar-text-apply","format_version":1,"base_source_hash":"{hash}","document":"{document}","commands":[{commands}]}}"#
    )
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
fn apply_dry_run_then_write_checks_semantic_diff_and_readable_git_changes() {
    let fixture = ApplyFixture::new("text-source-cli-apply");
    let project_before = file_digest(&fixture.project.path);
    git(&fixture.source.path, &["init", "--quiet"]);
    git(&fixture.source.path, &["add", "--", "view.stellar.yaml"]);
    git(
        &fixture.source.path,
        &["commit", "--quiet", "-m", "fixture baseline"],
    );
    let input = fixture.request(&format!(
        r#"{{"kind":"rename","node":"{}","name":"Confirm"}},{{"kind":"set-visible","node":"{}","visible":false}}"#,
        fixture.node, fixture.node
    ));
    let request_file = fixture.home.path.join("request.json");
    std::fs::write(&request_file, &input).unwrap();
    let source_before = file_digest(&fixture.source.path);
    let home_before = file_digest(&fixture.home.path);
    let path = fixture.path.to_str().unwrap();
    let dry = run(
        &[
            "text-source",
            "apply",
            path,
            request_file.to_str().unwrap(),
            "--dry-run",
        ],
        0,
    );
    let dry_result = dry.get("result").unwrap();
    assert_eq!(
        dry_result.get("operation"),
        Some(&JsonValue::string("apply"))
    );
    assert_eq!(dry_result.get("mode"), Some(&JsonValue::string("dry-run")));
    assert_eq!(dry_result.get("written"), Some(&JsonValue::Bool(false)));
    assert_eq!(dry_result.get("command_count"), Some(&JsonValue::int(2)));
    assert_eq!(dry_result.get("node_count"), Some(&JsonValue::int(4)));
    assert_eq!(
        dry_result.get("document"),
        Some(&JsonValue::string(fixture.document.to_string()))
    );
    assert_eq!(
        dry_result.get("base_source_hash"),
        Some(&JsonValue::string(&fixture.hash))
    );
    assert_eq!(
        dry_result.get("source_hash"),
        dry_result.get("base_source_hash")
    );
    let changes = dry_result.get("changes").unwrap();
    assert_eq!(changes.get("count"), Some(&JsonValue::int(2)));
    assert_eq!(
        changes.get("by_kind").unwrap().get("set-property"),
        Some(&JsonValue::int(2))
    );
    let entries = changes.get("entries").unwrap().as_array().unwrap();
    assert_eq!(entries.len(), 2);
    for entry in entries {
        assert_eq!(
            entry.get("object"),
            Some(&JsonValue::string(fixture.node.to_string()))
        );
        assert_eq!(
            entry.get("document"),
            Some(&JsonValue::string(fixture.document.to_string()))
        );
    }
    assert!(entries.iter().any(|entry| {
        entry.get("new").and_then(|value| value.get("value")) == Some(&JsonValue::string("Confirm"))
    }));
    assert!(entries.iter().any(|entry| {
        entry.get("old").and_then(|value| value.get("value")) == Some(&JsonValue::Bool(true))
            && entry.get("new").and_then(|value| value.get("value"))
                == Some(&JsonValue::Bool(false))
    }));
    assert_eq!(file_digest(&fixture.source.path), source_before);
    assert_eq!(file_digest(&fixture.home.path), home_before);
    assert_eq!(file_digest(&fixture.project.path), project_before);

    let written = run_with_stdin(
        &["text-source", "apply", path, "-"],
        &input,
        Some(&fixture.home.path),
        0,
    );
    let result = written.get("result").unwrap();
    assert_eq!(result.get("mode"), Some(&JsonValue::string("write")));
    assert_eq!(result.get("written"), Some(&JsonValue::Bool(true)));
    assert_eq!(result.get("changes"), Some(changes));
    assert_eq!(
        result.get("base_source_hash"),
        dry_result.get("base_source_hash")
    );
    assert_ne!(result.get("source_hash"), result.get("base_source_hash"));
    let checked = run(&["text-source", "check", path], 0);
    let checked = checked.get("result").unwrap();
    assert_eq!(checked.get("source_hash"), result.get("source_hash"));
    assert_eq!(checked.get("document"), result.get("document"));
    assert_eq!(checked.get("node_count"), Some(&JsonValue::int(4)));
    assert_eq!(
        checked.get("source_hash").and_then(JsonValue::as_str),
        file_digest(&fixture.source.path)
            .get("view.stellar.yaml")
            .map(String::as_str)
    );
    let diff = git(
        &fixture.source.path,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--unified=0",
            "--",
            "view.stellar.yaml",
        ],
    );
    assert!(diff.contains("+    display_name: Confirm"), "{diff}");
    assert!(diff.contains("+    visible: \"false\""), "{diff}");
    assert!(!diff.contains("Binary files"), "{diff}");
    assert_eq!(
        git(&fixture.source.path, &["diff", "--name-only"]),
        "view.stellar.yaml\n"
    );
    assert_eq!(file_digest(&fixture.project.path), project_before);
}

#[test]
fn apply_noop_preserves_comments_and_detects_comment_only_external_edits() {
    let fixture = ApplyFixture::new("text-source-cli-noop");
    let project_before = file_digest(&fixture.project.path);
    let original = std::fs::read_to_string(&fixture.path).unwrap();
    let annotated = format!("# collaborator comment\n{original}");
    std::fs::write(&fixture.path, &annotated).unwrap();
    let commands = format!(
        r#"{{"kind":"set-visible","node":"{}","visible":true}}"#,
        fixture.node
    );
    let stale = fixture.request(&commands);
    let path = fixture.path.to_str().unwrap();
    let source_before = file_digest(&fixture.source.path);
    let rejected = run_with_stdin(
        &["text-source", "apply", path, "-"],
        &stale,
        Some(&fixture.home.path),
        75,
    );
    assert_eq!(
        rejected.get("error").unwrap().get("code"),
        Some(&JsonValue::string("text-source-changed"))
    );
    let checked = run(&["text-source", "check", path], 0);
    let hash = checked
        .get("result")
        .unwrap()
        .get("source_hash")
        .and_then(JsonValue::as_str)
        .unwrap();
    let request = transaction(hash, fixture.document, &commands);
    for dry_run in [true, false] {
        let mut args = vec!["text-source", "apply", path, "-"];
        if dry_run {
            args.push("--dry-run");
        }
        let applied = run_with_stdin(&args, &request, Some(&fixture.home.path), 0);
        let result = applied.get("result").unwrap();
        assert_eq!(result.get("written"), Some(&JsonValue::Bool(false)));
        assert_eq!(result.get("source_hash"), Some(&JsonValue::string(hash)));
        assert_eq!(
            result.get("changes").unwrap().get("count"),
            Some(&JsonValue::int(0))
        );
        assert_eq!(std::fs::read_to_string(&fixture.path).unwrap(), annotated);
        assert_eq!(file_digest(&fixture.source.path), source_before);
    }
    assert_eq!(file_digest(&fixture.project.path), project_before);
}

#[test]
fn apply_rejects_stale_hash_wrong_document_unknown_commands_and_rolls_back_transactions() {
    let fixture = ApplyFixture::new("text-source-cli-rejections");
    let project_before = file_digest(&fixture.project.path);
    let source_before = file_digest(&fixture.source.path);
    let rename = format!(
        r#"{{"kind":"rename","node":"{}","name":"must roll back"}}"#,
        fixture.node
    );
    let missing = StableId::new_v7(9_000_000, 1, 999);
    let cases = [
        (
            transaction(
                &format!("sha256:{}", "0".repeat(64)),
                fixture.document,
                &rename,
            ),
            75,
            "text-source-changed",
        ),
        (
            transaction(&fixture.hash, fixture.node, &rename),
            65,
            "text-source-invalid",
        ),
        (
            fixture.request(r#"{"kind":"unknown-command"}"#),
            65,
            "unknown-command",
        ),
        (
            fixture.request(&format!(
                r#"{rename},{{"kind":"set-visible","node":"{missing}","visible":false}}"#
            )),
            1,
            "rejected-command",
        ),
    ];
    for (request, status, code) in cases {
        for dry_run in [true, false] {
            let mut args = vec!["text-source", "apply", fixture.path.to_str().unwrap(), "-"];
            if dry_run {
                args.push("--dry-run");
            }
            let rejected = run_with_stdin(&args, &request, Some(&fixture.home.path), status);
            assert_eq!(
                rejected.get("error").unwrap().get("code"),
                Some(&JsonValue::string(code))
            );
            assert_eq!(file_digest(&fixture.source.path), source_before);
            assert_eq!(file_digest(&fixture.project.path), project_before);
        }
    }
}

#[test]
fn apply_never_discards_unknown_source_records() {
    let fixture = ApplyFixture::new("text-source-cli-future-records");
    let project_before = file_digest(&fixture.project.path);
    let original = std::fs::read_to_string(&fixture.path).unwrap();
    let future = format!("{original}future_records: keep-me\n");
    std::fs::write(&fixture.path, &future).unwrap();
    let source_before = file_digest(&fixture.source.path);
    let request = transaction(
        source_before.get("view.stellar.yaml").unwrap(),
        fixture.document,
        &format!(
            r#"{{"kind":"rename","node":"{}","name":"Edited"}}"#,
            fixture.node
        ),
    );
    for dry_run in [true, false] {
        let mut args = vec!["text-source", "apply", fixture.path.to_str().unwrap(), "-"];
        if dry_run {
            args.push("--dry-run");
        }
        let rejected = run_with_stdin(&args, &request, Some(&fixture.home.path), 69);
        assert_eq!(
            rejected.get("error").unwrap().get("code"),
            Some(&JsonValue::string("text-source-unsupported"))
        );
        assert_eq!(std::fs::read_to_string(&fixture.path).unwrap(), future);
        assert_eq!(file_digest(&fixture.source.path), source_before);
        assert_eq!(file_digest(&fixture.project.path), project_before);
    }
}

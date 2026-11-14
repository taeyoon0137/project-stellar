//! 실제 저장 generation에서 opt-in export·check·apply와 파일 보존 계약을 검사합니다.

use std::io::Write;
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
    root: StableId,
    fragment: StableId,
    hash: String,
}

impl ApplyFixture {
    fn new(label: &str) -> Self {
        Self::from_model(label, large_document(4))
    }

    fn from_model(label: &str, model: FixtureDocument) -> Self {
        let project = TempDir::new(&format!("{label}-project"));
        let source = TempDir::new(&format!("{label}-source"));
        let home = TempDir::new(&format!("{label}-home"));
        let document = model.document_id();
        let records = model.to_records().unwrap();
        let node = records
            .nodes
            .iter()
            .find(|node| node.parent.is_some())
            .unwrap()
            .id;
        let root = records
            .nodes
            .iter()
            .find(|node| node.parent.is_none())
            .unwrap()
            .id;
        let fragment = records
            .nodes
            .iter()
            .find(|node| node.payload.kind().name() == "fragment")
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
            root,
            fragment,
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
fn text_node_export_apply_and_external_content_edit_preserve_payload() {
    let model = text_document();
    let node = model.to_records().unwrap().nodes.last().unwrap().id;
    let fixture = ApplyFixture::from_model("text-source-cli-text", model);
    let path = fixture.path.to_str().unwrap();
    let project_before = file_digest(&fixture.project.path);
    let original = std::fs::read_to_string(&fixture.path).unwrap();
    assert!(original.contains("kind: text\n"), "{original}");
    assert!(original.contains("Original 한글 e\u{301} 👩‍💻"), "{original}");
    assert!(original.contains("style: \"4294967295\""), "{original}");
    git(&fixture.source.path, &["init", "--quiet"]);
    git(&fixture.source.path, &["add", "--", "view.stellar.yaml"]);
    git(
        &fixture.source.path,
        &["commit", "--quiet", "-m", "fixture baseline"],
    );
    let commands = format!(r#"{{"kind":"rename","node":"{node}","name":"Confirm"}}"#);
    let input = fixture.request(&commands);
    let source_before = file_digest(&fixture.source.path);
    let home_before = file_digest(&fixture.home.path);
    let dry = run_with_stdin(
        &["text-source", "apply", path, "-", "--dry-run"],
        &input,
        Some(&fixture.home.path),
        0,
    );
    let dry = dry.get("result").unwrap();
    assert_eq!(dry.get("written"), Some(&JsonValue::Bool(false)));
    assert_eq!(
        dry.get("source_hash"),
        Some(&JsonValue::string(&fixture.hash))
    );
    assert_eq!(
        dry.get("changes").unwrap().get("count"),
        Some(&JsonValue::int(1))
    );
    assert_eq!(file_digest(&fixture.source.path), source_before);
    assert_eq!(file_digest(&fixture.home.path), home_before);

    let written = run_with_stdin(
        &["text-source", "apply", path, "-"],
        &input,
        Some(&fixture.home.path),
        0,
    );
    let result = written.get("result").unwrap();
    assert_eq!(result.get("written"), Some(&JsonValue::Bool(true)));
    assert_eq!(result.get("changes"), dry.get("changes"));
    let expected = original.replace("display_name: TextFixture", "display_name: Confirm");
    assert_ne!(expected, original);
    assert_eq!(std::fs::read_to_string(&fixture.path).unwrap(), expected);
    let checked = run(&["text-source", "check", path], 0);
    assert_eq!(
        checked.get("result").unwrap().get("source_hash"),
        result.get("source_hash")
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
    assert!(diff.contains("-    display_name: TextFixture"), "{diff}");
    assert!(diff.contains("+    display_name: Confirm"), "{diff}");
    assert!(!diff.contains("Binary files"), "{diff}");
    assert!(!diff.contains("content:"), "{diff}");
    assert!(!diff.contains("style:"), "{diff}");

    let external = expected.replace("Original 한글", "External 한글");
    assert_ne!(external, expected);
    std::fs::write(&fixture.path, &external).unwrap();
    let checked = run(&["text-source", "check", path], 0);
    let hash = checked
        .get("result")
        .unwrap()
        .get("source_hash")
        .unwrap()
        .as_str()
        .unwrap();
    assert_ne!(Some(&JsonValue::string(hash)), result.get("source_hash"));
    let noop = run_with_stdin(
        &["text-source", "apply", path, "-"],
        &transaction(hash, fixture.document, &commands),
        Some(&fixture.home.path),
        0,
    );
    assert_eq!(
        noop.get("result").unwrap().get("written"),
        Some(&JsonValue::Bool(false))
    );
    assert_eq!(std::fs::read_to_string(&fixture.path).unwrap(), external);
    assert_eq!(file_digest(&fixture.project.path), project_before);
}

#[test]
fn invalid_text_fields_reject_check_and_apply_without_overwriting() {
    let fixture = ApplyFixture::from_model("text-source-cli-invalid-text", text_document());
    let input = fixture.request(&format!(
        r#"{{"kind":"rename","node":"{}","name":"Rejected"}}"#,
        fixture.node
    ));
    let project_before = file_digest(&fixture.project.path);
    let source = std::fs::read_to_string(&fixture.path).unwrap();
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
        std::fs::write(&fixture.path, &invalid).unwrap();
        let before = file_digest(&fixture.source.path);
        let checked = run(
            &["text-source", "check", fixture.path.to_str().unwrap()],
            status,
        );
        assert_eq!(
            checked.get("error").unwrap().get("code"),
            Some(&JsonValue::string(code))
        );
        for dry_run in [true, false] {
            let mut args = vec!["text-source", "apply", fixture.path.to_str().unwrap(), "-"];
            if dry_run {
                args.push("--dry-run");
            }
            let rejected = run_with_stdin(&args, &input, Some(&fixture.home.path), status);
            assert_eq!(
                rejected.get("error").unwrap().get("code"),
                Some(&JsonValue::string(code))
            );
            assert_eq!(file_digest(&fixture.source.path), before);
            assert_eq!(file_digest(&fixture.project.path), project_before);
        }
    }
}

#[test]
fn apply_frame_geometry_round_trips_through_cli_and_readable_git_diff() {
    let fixture = ApplyFixture::new("text-source-cli-frame-geometry");
    let project_before = file_digest(&fixture.project.path);
    git(&fixture.source.path, &["init", "--quiet"]);
    git(&fixture.source.path, &["add", "--", "view.stellar.yaml"]);
    git(
        &fixture.source.path,
        &["commit", "--quiet", "-m", "fixture baseline"],
    );
    let root = fixture.root;
    let commands = format!(
        r#"{{"kind":"set-frame-size","node":"{root}","width":320.5,"height":180}},{{"kind":"set-frame-transform","node":"{root}","a":0.5,"b":0.25,"c":-0.25,"d":0.5,"tx":12.75,"ty":-3e1}}"#
    );
    let request = fixture.request(&commands);
    let path = fixture.path.to_str().unwrap();
    let source_before = file_digest(&fixture.source.path);
    let home_before = file_digest(&fixture.home.path);
    let dry = run_with_stdin(
        &["text-source", "apply", path, "-", "--dry-run"],
        &request,
        Some(&fixture.home.path),
        0,
    );
    let dry_result = dry.get("result").unwrap();
    assert_eq!(dry_result.get("written"), Some(&JsonValue::Bool(false)));
    assert_eq!(dry_result.get("command_count"), Some(&JsonValue::int(2)));
    assert_eq!(
        dry_result.get("source_hash"),
        Some(&JsonValue::string(&fixture.hash))
    );
    let changes = dry_result.get("changes").unwrap();
    assert_eq!(changes.get("count"), Some(&JsonValue::int(2)));
    assert_eq!(
        changes.get("by_kind").unwrap().get("set-property"),
        Some(&JsonValue::int(2))
    );
    for entry in changes.get("entries").unwrap().as_array().unwrap() {
        assert_eq!(
            entry.get("object"),
            Some(&JsonValue::string(root.to_string()))
        );
    }
    assert_eq!(file_digest(&fixture.source.path), source_before);
    assert_eq!(file_digest(&fixture.home.path), home_before);
    assert_eq!(file_digest(&fixture.project.path), project_before);

    let written = run_with_stdin(
        &["text-source", "apply", path, "-"],
        &request,
        Some(&fixture.home.path),
        0,
    );
    let result = written.get("result").unwrap();
    assert_eq!(result.get("written"), Some(&JsonValue::Bool(true)));
    assert_eq!(result.get("changes"), Some(changes));
    assert_ne!(result.get("source_hash"), result.get("base_source_hash"));
    let checked = run(&["text-source", "check", path], 0);
    let checked = checked.get("result").unwrap();
    assert_eq!(checked.get("source_hash"), result.get("source_hash"));
    assert_eq!(checked.get("document"), result.get("document"));
    assert_eq!(checked.get("node_count"), Some(&JsonValue::int(4)));
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
    assert!(diff.contains("+      size: \"320.5 180\""), "{diff}");
    assert!(
        diff.contains("+      transform: \"0.5 0.25 -0.25 0.5 12.75 -30\""),
        "{diff}"
    );
    assert!(!diff.contains("Binary files"), "{diff}");
    assert_eq!(
        git(&fixture.source.path, &["diff", "--name-only"]),
        "view.stellar.yaml\n"
    );

    // 같은 값을 재적용해 디스크에서 다시 구성한 모델의 두 속성이 일치하는지 확인합니다.
    let hash = checked.get("source_hash").unwrap().as_str().unwrap();
    let source_after = file_digest(&fixture.source.path);
    let repeated = transaction(hash, fixture.document, &commands);
    let noop = run_with_stdin(
        &["text-source", "apply", path, "-"],
        &repeated,
        Some(&fixture.home.path),
        0,
    );
    let noop = noop.get("result").unwrap();
    assert_eq!(noop.get("written"), Some(&JsonValue::Bool(false)));
    assert_eq!(noop.get("source_hash"), Some(&JsonValue::string(hash)));
    assert_eq!(
        noop.get("changes").unwrap().get("count"),
        Some(&JsonValue::int(0))
    );
    assert_eq!(file_digest(&fixture.source.path), source_after);
    assert_eq!(file_digest(&fixture.project.path), project_before);
}

#[test]
fn apply_frame_geometry_rejects_invalid_fields_and_rolls_back_wrong_node_kind() {
    let fixture = ApplyFixture::new("text-source-cli-frame-rejections");
    let root = fixture.root;
    let fragment = fixture.fragment;
    let project_before = file_digest(&fixture.project.path);
    let source_before = file_digest(&fixture.source.path);
    let size = format!(r#"{{"kind":"set-frame-size","node":"{root}","width":320,"height":180}}"#);
    let transform = format!(
        r#"{{"kind":"set-frame-transform","node":"{root}","a":1,"b":0,"c":0,"d":1,"tx":12,"ty":-30}}"#
    );
    let cases = [
        (
            format!(r#"{{"kind":"set-frame-size","node":"{root}","width":-1,"height":180}}"#),
            65,
            "invalid-transaction",
        ),
        (
            format!(r#"{{"kind":"set-frame-size","node":"{root}","width":320,"height":1e39}}"#),
            65,
            "invalid-transaction",
        ),
        (
            format!(r#"{{"kind":"set-frame-size","node":"{root}","width":"320","height":180}}"#),
            65,
            "invalid-transaction",
        ),
        (
            format!(r#"{{"kind":"set-frame-size","node":"{root}","width":320}}"#),
            65,
            "invalid-transaction",
        ),
        (
            format!(
                r#"{{"kind":"set-frame-size","node":"{root}","width":320,"height":180,"extra":0}}"#
            ),
            65,
            "invalid-transaction",
        ),
        (
            format!(
                r#"{{"kind":"set-frame-transform","node":"{root}","a":1,"b":0,"c":0,"d":1,"tx":1e39,"ty":0}}"#
            ),
            65,
            "invalid-transaction",
        ),
        (
            format!(
                r#"{{"kind":"set-frame-transform","node":"{root}","a":true,"b":0,"c":0,"d":1,"tx":0,"ty":0}}"#
            ),
            65,
            "invalid-transaction",
        ),
        (
            format!(
                r#"{{"kind":"set-frame-transform","node":"{root}","a":1,"b":0,"c":0,"d":1,"tx":0}}"#
            ),
            65,
            "invalid-transaction",
        ),
        (
            format!(
                r#"{{"kind":"set-frame-transform","node":"{root}","a":1,"b":0,"c":0,"d":1,"tx":0,"ty":0,"extra":0}}"#
            ),
            65,
            "invalid-transaction",
        ),
        (
            format!(
                r#"{size},{transform},{{"kind":"set-frame-size","node":"{fragment}","width":320,"height":180}}"#
            ),
            1,
            "rejected-command",
        ),
        (
            format!(
                r#"{size},{transform},{{"kind":"set-frame-transform","node":"{fragment}","a":1,"b":0,"c":0,"d":1,"tx":12,"ty":-30}}"#
            ),
            1,
            "rejected-command",
        ),
    ];
    for (commands, status, code) in cases {
        for dry_run in [true, false] {
            let mut args = vec!["text-source", "apply", fixture.path.to_str().unwrap(), "-"];
            if dry_run {
                args.push("--dry-run");
            }
            let rejected = run_with_stdin(
                &args,
                &fixture.request(&commands),
                Some(&fixture.home.path),
                status,
            );
            assert_eq!(
                rejected.get("error").unwrap().get("code"),
                Some(&JsonValue::string(code)),
                "{commands}"
            );
            assert_eq!(file_digest(&fixture.source.path), source_before);
            assert_eq!(file_digest(&fixture.project.path), project_before);
        }
    }
}

#[test]
fn apply_after_a_source_rename_reports_the_same_missing_input_in_both_modes() {
    let fixture = ApplyFixture::new("text-source-renamed");
    let input = fixture.request(&format!(
        r#"{{"kind":"rename","node":"{}","name":"New"}}"#,
        fixture.node
    ));
    let renamed = fixture.source.path.join("renamed.stellar.yaml");
    std::fs::rename(&fixture.path, &renamed).unwrap();
    let before = file_digest(&fixture.source.path);
    for dry_run in [true, false] {
        let mut args = vec!["text-source", "apply", fixture.path.to_str().unwrap(), "-"];
        if dry_run {
            args.push("--dry-run");
        }
        let error = run_with_stdin(&args, &input, Some(&fixture.home.path), 66);
        assert_eq!(
            error.get("error").unwrap().get("code"),
            Some(&JsonValue::string("text-source-io"))
        );
    }
    assert_eq!(file_digest(&fixture.source.path), before);
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

#[test]
fn replace_text_uses_scalar_indices_and_keeps_failed_batches_off_disk() {
    let model = text_document();
    let node = model.to_records().unwrap().nodes.last().unwrap().id;
    let fixture = ApplyFixture::from_model("text-source-range-edit", model);
    let path = fixture.path.to_str().unwrap();
    let original = std::fs::read_to_string(path).unwrap();
    let project_before = file_digest(&fixture.project.path);
    let replace = |start, end, text: &str| {
        stellar_cli::json::write(&JsonValue::object(vec![
            ("kind", JsonValue::string("replace-text")),
            ("node", JsonValue::string(node.to_string())),
            ("start", JsonValue::int(start)),
            ("end", JsonValue::int(end)),
            ("text", JsonValue::string(text)),
        ]))
    };
    let edit = replace(9, 11, "새\n단락");
    for (ops, expected) in [
        (format!("{edit},{}", replace(1000, 1001, "x")), 1),
        (replace(12, 11, "x"), 65),
    ] {
        run_with_stdin(
            &["text-source", "apply", path, "-"],
            &fixture.request(&ops),
            Some(&fixture.home.path),
            expected,
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), original);
    }
    let input = fixture.request(&edit);
    let result = run_with_stdin(
        &["text-source", "apply", path, "-", "--dry-run"],
        &input,
        Some(&fixture.home.path),
        0,
    );
    assert_eq!(
        result
            .get("result")
            .unwrap()
            .get("changes")
            .unwrap()
            .get("count"),
        Some(&JsonValue::int(1))
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
    run_with_stdin(
        &["text-source", "apply", path, "-"],
        &input,
        Some(&fixture.home.path),
        0,
    );
    let yaml = std::fs::read_to_string(path).unwrap();
    assert!(yaml.contains("Original 새\\n단락 e\u{301} 👩‍💻"), "{yaml}");
    run(&["text-source", "check", path], 0);
    assert_eq!(file_digest(&fixture.project.path), project_before);
}

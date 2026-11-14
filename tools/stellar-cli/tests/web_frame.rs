//! 저장 generation → 실제 stellar → HTML golden / Chrome DOM. 브라우저 검증은 명시적으로 실행합니다.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use stellar_cli::json::{self, JsonValue};
use stellar_ops::snapshot::open_snapshot;
use stellar_ops::test_support::{
    corrupt_view_tree_chunks, file_digest, save_documents, save_documents_with_schema,
    scaffold_project, web_frame as fixture, TempDir,
};

fn run(path: &Path, document: &str, view: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stellar"))
        .args(["export-web", path.to_str().unwrap(), document, view])
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

fn html(value: &JsonValue) -> &str {
    value
        .get("result")
        .unwrap()
        .get("html")
        .unwrap()
        .as_str()
        .unwrap()
}

fn export(path: &Path, view: u64) -> JsonValue {
    envelope(
        &run(
            path,
            &fixture::id(1).to_string(),
            &fixture::id(view).to_string(),
        ),
        0,
    )
}

fn git(path: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .args(args)
        .current_dir(path)
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
fn two_selected_views_are_deterministic_and_leave_project_and_git_bytes_unchanged() {
    let dir = TempDir::new("web-two-views");
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
    let revision = open_snapshot(&dir.path).unwrap().revision().token();
    for (view, file, other_root) in [(3, "view-a.html", 20), (4, "view-b.html", 10)] {
        let args = (fixture::id(1).to_string(), fixture::id(view).to_string());
        let output = run(&dir.path, &args.0, &args.1);
        let again = run(&dir.path, &args.0, &args.1);
        assert_eq!(output.stdout, again.stdout);
        assert_eq!(output.stderr, again.stderr);
        let value = envelope(&output, 0);
        let result = value.get("result").unwrap();
        assert_eq!(
            result
                .get("revision")
                .unwrap()
                .get("token")
                .unwrap()
                .as_str(),
            Some(revision.as_str())
        );
        assert!(!html(&value).contains(&fixture::id(other_root).to_string().replace('-', "")));
        assert!(!html(&value).contains("<script>"));
        assert!(!html(&value).contains("button"));
        let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/web")
            .join(file);
        if std::env::var_os("UPDATE_WEB_GOLDENS").is_some() {
            std::fs::write(&golden, html(&value)).unwrap();
        }
        assert_eq!(html(&value), std::fs::read_to_string(golden).unwrap());
    }
    assert_eq!(git(&dir.path, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&dir.path, &["status", "--porcelain=v1"]), status);
    assert_eq!(
        file_digest(&dir.path),
        before,
        "저장 파일·Git index·refs·objects 모두 무변경"
    );
}

#[test]
fn unsupported_values_fail_only_in_the_selected_view_including_hidden_nodes() {
    let dir = TempDir::new("web-unsupported");
    scaffold_project(&dir.path);
    for (case, code) in [
        ("layout", "unsupported-layout"),
        ("clip", "unsupported-clip"),
        ("modifier", "unsupported-modifier"),
        ("text", "unsupported-node"),
        ("hidden-text", "unsupported-node"),
        ("fractional-border", "unsupported-frame-value"),
        ("oversize-border", "unsupported-frame-value"),
        ("range", "unsupported-frame-value"),
        ("matrix-range", "unsupported-frame-value"),
        ("world-range", "unsupported-frame-value"),
    ] {
        save_documents(&dir.path, &[&fixture::unsupported(case)]);
        let before = file_digest(&dir.path);
        export(&dir.path, 3);
        let value = envelope(
            &run(
                &dir.path,
                &fixture::id(1).to_string(),
                &fixture::id(4).to_string(),
            ),
            1,
        );
        assert_eq!(
            value.get("result").unwrap().get("html"),
            Some(&JsonValue::Null),
            "{case}"
        );
        assert_eq!(
            value.get("error").unwrap().get("code").unwrap().as_str(),
            Some("web-export-failed")
        );
        let diagnostics = json::write(value.get("diagnostics").unwrap());
        assert!(diagnostics.contains(code), "{case}: {diagnostics}");
        assert!(diagnostics.contains(&fixture::id(21).to_string()));
        assert_eq!(file_digest(&dir.path), before);
    }
}

#[test]
fn missing_unsaved_unknown_newer_and_corrupt_inputs_fail_without_writes() {
    let dir = TempDir::new("web-errors");
    let doc = fixture::id(1).to_string();
    let view = fixture::id(3).to_string();
    envelope(&run(&dir.path.join("missing"), &doc, &view), 66);
    scaffold_project(&dir.path);
    let before = file_digest(&dir.path);
    envelope(&run(&dir.path, &doc, &view), 66);
    assert_eq!(file_digest(&dir.path), before);
    save_documents_with_schema(&dir.path, &[&fixture::document()], 2);
    let before = file_digest(&dir.path);
    envelope(&run(&dir.path, &doc, &view), 65);
    assert_eq!(file_digest(&dir.path), before);
    save_documents(&dir.path, &[&fixture::document()]);
    let before = file_digest(&dir.path);
    envelope(&run(&dir.path, &fixture::id(99).to_string(), &view), 66);
    envelope(&run(&dir.path, &doc, &fixture::id(99).to_string()), 66);
    assert_eq!(file_digest(&dir.path), before);
    assert!(corrupt_view_tree_chunks(&dir.path) > 0);
    let before = file_digest(&dir.path);
    envelope(&run(&dir.path, &doc, &view), 65);
    assert_eq!(file_digest(&dir.path), before);
}

#[test]
fn usage_errors_emit_no_json() {
    for args in [
        vec!["export-web"],
        vec!["export-web", "p", "invalid", "invalid"],
        vec!["export-web", "p", "--all", "view"],
        vec!["export-web", "p", "doc", "view", "extra"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_stellar"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(64));
        assert!(output.stdout.is_empty());
    }
}

#[test]
#[ignore = "STELLAR_CHROME 경로로 실제 Chrome headless DOM을 검증합니다"]
fn chrome_computed_styles_bounds_stacking_and_clipping() {
    let chrome = std::env::var_os("STELLAR_CHROME").expect("STELLAR_CHROME 실행 파일 경로");
    let dir = TempDir::new("web-browser");
    let project = dir.path.join("project");
    scaffold_project(&project);
    save_documents(&project, &[&fixture::document()]);
    let before = file_digest(&project);
    let version = Command::new(&chrome).arg("--version").output().unwrap();
    assert!(version.status.success());
    eprintln!("{}", String::from_utf8_lossy(&version.stdout));
    for (view, file) in [(3, "view-a"), (4, "view-b")] {
        let value = export(&project, view);
        let probe = format!(
            "<script>const selectedView = {view};\n{}</script>\n",
            include_str!("fixtures/web/probe.js")
        );
        let page = dir.path.join(format!("{file}.html"));
        std::fs::write(
            &page,
            html(&value).replace("</body>", &format!("{probe}</body>")),
        )
        .unwrap();
        let output = Command::new(&chrome)
            .args([
                "--headless",
                "--disable-gpu",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-background-networking",
                "--force-device-scale-factor=1",
                "--window-size=1000,800",
                "--virtual-time-budget=1000",
                "--timeout=10000",
                "--dump-dom",
            ])
            .arg(format!(
                "--user-data-dir={}",
                dir.path.join(format!("chrome-{view}")).display()
            ))
            .arg(format!("file://{}", page.display()))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let dom = String::from_utf8(output.stdout).unwrap();
        let probe_json = dom
            .split("<pre id=\"stellar-web-probe\">")
            .nth(1)
            .expect("DOM probe output")
            .split("</pre>")
            .next()
            .unwrap();
        let report = json::parse(probe_json, json::JsonLimits::default()).unwrap();
        assert_eq!(
            report.get("ok"),
            Some(&JsonValue::Bool(true)),
            "{probe_json}"
        );
        eprintln!("{file}: {probe_json}");
        if let Some(evidence) = std::env::var_os("STELLAR_WEB_EVIDENCE_DIR") {
            let evidence = Path::new(&evidence);
            std::fs::create_dir_all(evidence).unwrap();
            std::fs::write(evidence.join(format!("{file}.html")), html(&value)).unwrap();
            std::fs::write(evidence.join(format!("{file}.dom.html")), &dom).unwrap();
            std::fs::write(evidence.join(format!("{file}.computed.json")), probe_json).unwrap();
            std::fs::write(evidence.join("chrome-version.txt"), &version.stdout).unwrap();
        }
    }
    assert_eq!(file_digest(&project), before);
}

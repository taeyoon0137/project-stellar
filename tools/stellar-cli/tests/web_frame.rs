//! 저장 generation → 실제 stellar → HTML golden. 브라우저 표시는 명세의 수동 확인 절차로 봅니다.

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
    let enlarged = export(&dir.path, 5);
    assert!(html(&enlarged).contains("transform: matrix(0, 1, -1, 0, 0.004999995, -0.005)"));
    assert!(html(&enlarged).contains("left: calc(1px - 1px)"));
    assert!(html(&enlarged).contains("top: calc(0px - 1px)"));
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
        ("amplified-size-rounding", "unsupported-frame-value"),
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
fn rectangle_saved_cli_preserves_geometry_modifier_order_and_read_only_scope() {
    let dir = TempDir::new("web-rectangle");
    scaffold_project(&dir.path);
    save_documents(&dir.path, &[&fixture::document()]);
    assert_eq!(
        html(&export(&dir.path, 3)),
        include_str!("fixtures/web/view-a.html")
    );
    assert_eq!(
        html(&export(&dir.path, 4)),
        include_str!("fixtures/web/view-b.html")
    );
    let mut supported_html = None;
    for case in [
        "supported",
        "disabled",
        "no-stack",
        "square",
        "after-fill",
        "double-fill",
        "unsupported",
        "hidden-unsupported",
        "singular",
        "range",
    ] {
        save_documents(&dir.path, &[&fixture::rectangle(case)]);
        let before = file_digest(&dir.path);
        // 선택 밖 Shape가 Frame 출력에 영향을 주지 않습니다. 기존 golden 그대로입니다.
        assert_eq!(
            html(&export(&dir.path, 3)),
            include_str!("fixtures/web/view-a.html")
        );
        let output = run(
            &dir.path,
            &fixture::id(1).to_string(),
            &fixture::id(4).to_string(),
        );
        let again = run(
            &dir.path,
            &fixture::id(1).to_string(),
            &fixture::id(4).to_string(),
        );
        assert_eq!(output.stdout, again.stdout);
        assert_eq!(output.stderr, again.stderr);
        let success = matches!(case, "supported" | "disabled" | "no-stack" | "square");
        let value = envelope(&output, if success { 0 } else { 1 });
        if success {
            let result = html(&value);
            assert!(result.contains("width: 40px;\n  height: 20px;"));
            let diagnostics = json::write(value.get("diagnostics").unwrap());
            assert_eq!(
                diagnostics.contains("browser-rectangle-curve"),
                case != "square"
            );
            if case == "square" {
                assert!(result.contains("border-top-left-radius: 0px;"));
            } else {
                assert!(result.contains("border-top-left-radius: 40px;"));
            }
            if case != "no-stack" {
                assert!(result.contains("left: calc(88px - 2px)"));
                assert!(result.contains("top: calc(35px - 2px)"));
                assert!(result.contains("matrix(0, 1, -1, 0, 0, 0)"));
                assert!(result.contains("background-color: color(display-p3 1.2 -0.1 0.3 / 0.25)"));
                assert!(json::write(value.get("diagnostics").unwrap())
                    .contains("browser-color-compositing"));
                if case != "square" {
                    if let Some(previous) = &supported_html {
                        assert_eq!(result, previous);
                    }
                }
                if case != "square" {
                    supported_html = Some(result.to_owned());
                }
            }
        } else {
            assert_eq!(
                value.get("result").unwrap().get("html"),
                Some(&JsonValue::Null)
            );
            let diagnostics = json::write(value.get("diagnostics").unwrap());
            let code = if matches!(case, "singular" | "range") {
                "unsupported-frame-value"
            } else {
                "unsupported-modifier"
            };
            assert!(diagnostics.contains(code), "{case}: {diagnostics}");
            assert!(diagnostics.contains(&fixture::id(21).to_string()));
        }
        assert_eq!(
            file_digest(&dir.path),
            before,
            "{case}: export writes nothing"
        );
    }
}

#[test]
fn two_instances_follow_one_source_and_keep_distinct_dom_ids() {
    let dir = TempDir::new("web-instances");
    scaffold_project(&dir.path);
    save_documents(&dir.path, &[&fixture::placed_components("supported")]);
    let folder = dir.path.join("design/components");
    std::fs::create_dir_all(&folder).unwrap();
    let component = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_stellar"))
            .arg("component-source")
            .args(args)
            .env("HOME", &dir.path)
            .output()
            .unwrap();
        envelope(&output, 0).get("result").unwrap().clone()
    };
    let get = |record: &JsonValue, key: &str| record.get(key).unwrap().as_str().unwrap().to_owned();
    let id = |n| fixture::id(n).to_string();
    let root = component(&["create", folder.to_str().unwrap(), "Card", &id(70), &id(71)]);
    let path = get(&root, "file");
    component(&[
        "add-frame",
        &path,
        &get(&root, "source_hash"),
        &id(71),
        &id(72),
        "Inset",
        "12",
        "8",
    ]);
    // 위치와 표시 여부는 YAML 원본을 직접 고칩니다(CLI는 root 크기·채우기와 자식 추가만 편집합니다).
    // 파일에는 root_node가 children보다 먼저 나옵니다.
    let rewrite = |edit: &dyn Fn(String) -> String| {
        let text = std::fs::read_to_string(&path).unwrap();
        let edited = edit(text.clone());
        assert_ne!(edited, text);
        std::fs::write(&path, edited).unwrap();
        let checked = component(&["check", folder.to_str().unwrap()]);
        get(
            &checked.get("components").unwrap().as_array().unwrap()[0],
            "source_hash",
        )
    };
    let identity = "transform: \"1 0 0 1 0 0\"";
    let hash = rewrite(&|mut text| {
        let inset = text.rfind(identity).unwrap();
        text.replace_range(inset..inset + identity.len(), "transform: \"1 0 0 1 2 3\"");
        // Root가 캔버스에 있던 좌표는 배치 좌표에 더하지 않습니다.
        text.replacen(identity, "transform: \"1 0 0 1 40 30\"", 1)
    });
    let frame = component(&["resize", &path, &hash, "90", "60"]);
    let before = file_digest(&dir.path);
    let first = export(&dir.path, 4);
    assert_eq!(first, export(&dir.path, 4));
    assert_eq!(before, file_digest(&dir.path));
    let dom_id = |n| format!("s-{}", id(n).replace('-', ""));
    let css = |page: &str, key: &str| {
        page.split_once(&format!("#{key} {{"))
            .unwrap()
            .1
            .split_once('}')
            .unwrap()
            .0
            .to_owned()
    };
    for (n, left, top) in [(21, "30", "20"), (22, "230", "-10")] {
        let unique = format!("{}-{}", dom_id(n), dom_id(72));
        assert_eq!(html(&first).matches(&format!("id=\"{unique}\"")).count(), 1);
        let root_css = css(html(&first), &dom_id(n));
        assert!(root_css.contains("width: 90px") && root_css.contains("height: 60px"));
        assert!(root_css.contains(&format!("left: calc({left}px - 2px)")));
        assert!(root_css.contains(&format!("top: calc({top}px - 2px)")));
        if n == 22 {
            assert!(root_css.contains("matrix(1, 0, 0, 1, 0.5, 0)"));
        }
        let inset_css = css(html(&first), &unique);
        assert!(inset_css.contains("width: 12px") && inset_css.contains("left: calc(2px - 0px)"));
    }
    component(&["resize", &path, &get(&frame, "source_hash"), "144", "60"]);
    let next = export(&dir.path, 4);
    assert_eq!(html(&next).matches("width: 144px").count(), 2);
    assert_eq!(
        first.get("result").unwrap().get("revision"),
        next.get("result").unwrap().get("revision")
    );
    assert_ne!(
        first.get("result").unwrap().get("component_source_hash"),
        next.get("result").unwrap().get("component_source_hash")
    );
    let visible = "visible: \"true\"";
    rewrite(&|mut text| {
        let inset = text.rfind(visible).unwrap();
        text.replace_range(inset..inset + visible.len(), "visible: \"false\"");
        text
    });
    let hidden = export(&dir.path, 4);
    for n in [21, 22] {
        assert!(
            css(html(&hidden), &format!("{}-{}", dom_id(n), dom_id(72))).contains("display: none")
        );
    }
    for (case, code) in [
        ("override", "unsupported-component-instance"),
        ("modifier", "unsupported-modifier"),
        ("singular", "unsupported-frame-value"),
    ] {
        save_documents(&dir.path, &[&fixture::placed_components(case)]);
        let failure = envelope(&run(&dir.path, &id(1), &id(4)), 1);
        assert_eq!(
            failure.get("result").unwrap().get("html"),
            Some(&JsonValue::Null)
        );
        assert!(json::write(failure.get("diagnostics").unwrap()).contains(code));
    }
    save_documents(&dir.path, &[&fixture::placed_components("hidden")]);
    assert!(css(html(&export(&dir.path, 4)), &dom_id(21)).contains("display: none"));
    std::fs::remove_file(path).unwrap();
    let missing = envelope(&run(&dir.path, &id(1), &id(4)), 1);
    assert_eq!(
        missing.get("result").unwrap().get("html"),
        Some(&JsonValue::Null)
    );
    let notes = json::write(missing.get("diagnostics").unwrap());
    assert!(notes.contains(&id(70)) && notes.contains(&id(21)));
    assert_eq!(
        html(&export(&dir.path, 3)),
        include_str!("fixtures/web/view-a.html")
    );
}

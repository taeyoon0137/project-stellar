//! 저장 generation → 실제 `stellar export-web`·`export-styles` → 기본값 세트 두 개 × 대상 두 개 golden (IX.11.30).
//!
//! - golden: `tests/export_fixtures/<세트>-web.html`(export-web HTML), `<세트>-react-native.js`(RN StyleSheet).
//!   `UPDATE_EXPORT_GOLDENS=1`이면 다시 씁니다.
//! - export-web CSS의 요소 규칙이 `style-defaults resolve`의 명시 값(layout record·payload 크기)과 같은지 확인합니다.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use stellar_cli::json::{self, JsonValue};
use stellar_ops::export_css::fixture::{self, id};
use stellar_ops::style_defaults::set_active;
use stellar_ops::test_support::{file_digest, save_documents, scaffold_project, TempDir};

fn stellar(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stellar"))
        .args(args)
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
    json::parse(text.trim_end(), json::JsonLimits::default()).unwrap()
}

fn project(name: &str, set: &str) -> TempDir {
    let dir = TempDir::new(name);
    scaffold_project(&dir.path);
    save_documents(&dir.path, &[&fixture::document()]);
    if set != "css" {
        set_active(&dir.path, set).unwrap();
    }
    dir
}

fn result<'a>(value: &'a JsonValue, key: &str) -> &'a JsonValue {
    value.get("result").unwrap().get(key).unwrap()
}

fn golden(file: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/export_fixtures")
        .join(file);
    if std::env::var_os("UPDATE_EXPORT_GOLDENS").is_some() {
        std::fs::write(&path, actual).unwrap();
    }
    assert_eq!(actual, std::fs::read_to_string(&path).unwrap(), "{file}");
}

#[test]
fn two_sets_by_two_targets_match_goldens_and_leave_project_bytes_unchanged() {
    for set in ["css", "react-native"] {
        let dir = project(&format!("export-styles-{set}"), set);
        let path = dir.path.to_str().unwrap();
        let (document, view) = (id(1).to_string(), id(3).to_string());
        let before = file_digest(&dir.path);
        let web = stellar(&["export-web", path, &document, &view]);
        assert_eq!(
            web.stdout,
            stellar(&["export-web", path, &document, &view]).stdout
        );
        let web = envelope(&web, 0);
        assert_eq!(result(&web, "format").as_str(), Some("css-layout-html"));
        golden(
            &format!("{set}-web.html"),
            result(&web, "html").as_str().unwrap(),
        );
        let rn = envelope(
            &stellar(&[
                "export-styles",
                path,
                &document,
                &view,
                "--target",
                "react-native",
            ]),
            0,
        );
        assert_eq!(
            result(&rn, "style_defaults")
                .get("active")
                .unwrap()
                .as_str(),
            Some(set)
        );
        assert_eq!(
            result(&rn, "style_defaults")
                .get("target")
                .unwrap()
                .as_str(),
            Some("react-native")
        );
        golden(
            &format!("{set}-react-native.js"),
            result(&rn, "source").as_str().unwrap(),
        );
        let styles = envelope(&stellar(&["export-styles", path, &document, &view]), 0);
        let base = result(&styles, "base_rules").as_str().unwrap();
        // 세트와 대상 기본값이 같으면(css → web) 기반 규칙이 없습니다.
        assert_eq!(base.is_empty(), set == "css");
        if !base.is_empty() {
            assert!(result(&web, "html").as_str().unwrap().contains(base));
        }
        assert_eq!(file_digest(&dir.path), before);
    }
}

#[test]
fn export_web_rules_carry_the_resolved_explicit_values() {
    for set in ["css", "react-native"] {
        let dir = project(&format!("export-styles-record-{set}"), set);
        let path = dir.path.to_str().unwrap();
        let (document, view) = (id(1).to_string(), id(3).to_string());
        let web = envelope(&stellar(&["export-web", path, &document, &view]), 0);
        let html = result(&web, "html").as_str().unwrap().to_string();
        let styles = envelope(&stellar(&["export-styles", path, &document, &view]), 0);
        let JsonValue::Array(elements) = result(&styles, "elements") else {
            panic!("elements");
        };
        let mut checked = 0;
        for element in elements {
            let node = element.get("node").unwrap().as_str().unwrap();
            let name = element.get("element").unwrap().as_str().unwrap();
            let placement = element.get("placement").unwrap().as_str().unwrap();
            let start = html.find(&format!("#{name} {{\n")).unwrap();
            let rule = &html[start..start + html[start..].find("}\n").unwrap()];
            let resolved = envelope(
                &stellar(&["style-defaults", "resolve", path, &document, node]),
                0,
            );
            assert_eq!(result(&resolved, "style_defaults").as_str(), Some(set));
            let JsonValue::Array(values) = result(&resolved, "values") else {
                panic!("values");
            };
            for value in values {
                if value.get("source").unwrap().as_str() != Some("explicit") {
                    continue;
                }
                let longhand = value.get("longhand").unwrap().as_str().unwrap();
                let text = value.get("value").unwrap().as_str().unwrap();
                // root에는 item longhand를 내지 않고, 위치 longhand는 배치 규칙이 정합니다.
                let placed = ["position", "top", "right", "bottom", "left"].contains(&longhand);
                if placed || (placement == "root" && longhand.starts_with("flex-")) {
                    continue;
                }
                assert!(
                    rule.contains(&format!("  {longhand}: {text};\n")),
                    "{set} {name} {longhand}: {text}\n{rule}"
                );
                checked += 1;
            }
        }
        assert!(checked >= 15, "{checked}");
    }
}

#[test]
fn invalid_target_and_options_are_usage_errors() {
    let dir = project("export-styles-usage", "css");
    let path = dir.path.to_str().unwrap();
    let (document, view) = (id(1).to_string(), id(3).to_string());
    for args in [
        vec![
            "export-styles",
            path,
            &document,
            &view,
            "--target",
            "android",
        ],
        vec!["export-styles", path, &document, &view, "--target"],
        vec!["export-styles", path, &document],
        vec!["export-styles", path, &document, &view, "--bogus"],
    ] {
        assert_eq!(stellar(&args).status.code(), Some(64), "{args:?}");
    }
    // View 4는 record가 없어 css 세트에서 고정 Free Frame 출력입니다.
    let value = envelope(
        &stellar(&["export-styles", path, &document, &id(4).to_string()]),
        0,
    );
    assert_eq!(
        result(&value, "format").as_str(),
        Some("fixed-free-frame-html")
    );
    let frame = envelope(
        &stellar(&[
            "export-styles",
            path,
            &document,
            &view,
            "--frame",
            &id(12).to_string(),
            "--target",
            "react-native",
        ]),
        0,
    );
    assert_eq!(
        result(&frame, "root_frame").as_str(),
        Some(id(12).to_string().as_str())
    );
}

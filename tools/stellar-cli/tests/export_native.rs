//! 저장 generation → 실제 `stellar export swiftui|appkit|compose` (XXII.3·XXII.4, ADR-0107 결정 8).
//!
//! - 세 대상 모두 소스를 내고, 같은 입력은 같은 bytes이며, 프로젝트 파일을 바꾸지 않습니다.
//! - 배치·layout 값은 `export-web`과 같은 투영입니다(flex root, grow, absolute inset, 숨김).
//! - macOS에서는 SwiftUI·AppKit 출력을 `xcrun swiftc -typecheck`로 검사합니다.

use std::process::{Command, Output, Stdio};

use stellar_cli::json::{self, JsonValue};
use stellar_ops::export_css::fixture::{self, id};
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

fn source(value: &JsonValue) -> String {
    value
        .get("result")
        .unwrap()
        .get("source")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string()
}

#[cfg(target_os = "macos")]
fn typecheck(source: &str, name: &str, sdk: &str, target: &str) {
    let dir =
        std::env::temp_dir().join(format!("stellar-cli-native-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Generated.swift");
    std::fs::write(&file, source).unwrap();
    let output = Command::new("xcrun")
        .args(["--sdk", sdk, "swiftc", "-typecheck", "-target", target])
        .arg(&file)
        .output();
    let _ = std::fs::remove_dir_all(&dir);
    let output = output.expect("xcrun 실행(DEV-MAC에는 Xcode가 필요합니다)");
    assert!(
        output.status.success(),
        "{name} typecheck 실패:\n{}\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn saved_view_exports_to_swiftui_appkit_and_compose() {
    let dir = TempDir::new("export-native-cli");
    scaffold_project(&dir.path);
    save_documents(&dir.path, &[&fixture::document()]);
    let path = dir.path.to_str().unwrap();
    let (document, view) = (id(1).to_string(), id(3).to_string());
    let before = file_digest(&dir.path);
    let cases = [
        (vec!["export", "swiftui", path, &document, &view], "ios"),
        (
            vec![
                "export",
                "swiftui",
                path,
                &document,
                &view,
                "--platform",
                "macos",
            ],
            "macos",
        ),
        (vec!["export", "appkit", path, &document, &view], "macos"),
        (vec!["export", "compose", path, &document, &view], "android"),
    ];
    let mut sources = Vec::new();
    for (args, profile) in &cases {
        let first = stellar(args);
        assert_eq!(first.stdout, stellar(args).stdout, "{args:?} 결정론");
        let value = envelope(&first, 0);
        let result = value.get("result").unwrap();
        assert_eq!(result.get("profile").unwrap().as_str(), Some(*profile));
        sources.push(source(&value));
    }
    assert_eq!(
        before,
        file_digest(&dir.path),
        "프로젝트 파일을 바꾸지 않습니다"
    );
    let [ios, mac, appkit, compose] = &sources[..] else {
        unreachable!()
    };
    // root(display: flex) → row stack, 숨긴 Frame은 view가 없습니다.
    assert!(ios.contains("struct StellarView: View"));
    assert!(ios.contains("HStack("));
    assert!(appkit.contains("final class StellarView: NSView"));
    assert!(compose.contains("fun StellarView("));
    assert!(compose.contains("    Row(\n"));
    // 명시 inset absolute Frame(14)은 overlay·Box 층.
    assert!(ios.contains(".overlay(alignment:"));
    assert!(compose.contains(".align(Alignment."));
    #[cfg(target_os = "macos")]
    {
        typecheck(
            ios,
            "ios",
            "iphonesimulator",
            "arm64-apple-ios26.0-simulator",
        );
        typecheck(mac, "mac", "macosx", "arm64-apple-macos26.0");
        typecheck(appkit, "appkit", "macosx", "arm64-apple-macos26.0");
    }
    let _ = mac;
}

#[test]
fn usage_errors_and_unknown_view_are_reported() {
    let dir = TempDir::new("export-native-usage");
    scaffold_project(&dir.path);
    save_documents(&dir.path, &[&fixture::document()]);
    let path = dir.path.to_str().unwrap();
    let document = id(1).to_string();
    let view = id(3).to_string();
    for args in [
        vec!["export"],
        vec!["export", "flutter", path, &document, &view],
        vec![
            "export",
            "compose",
            path,
            &document,
            &view,
            "--platform",
            "ios",
        ],
        vec!["export", "swiftui", path, &document],
        vec![
            "export",
            "swiftui",
            path,
            &document,
            &view,
            "--platform",
            "watchos",
        ],
    ] {
        assert_eq!(stellar(&args).status.code(), Some(64), "{args:?}");
    }
    let missing = id(99).to_string();
    let output = stellar(&["export", "swiftui", path, &document, &missing]);
    assert_eq!(output.status.code(), Some(66));
}

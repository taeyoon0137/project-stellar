//! 저장 generation → 실제 `stellar export png` (XX.2.3, ADR-0116 결정 5).
//!
//! - 저장 View를 PNG로 저장하고 결과 JSON이 크기·profile·경로를 담습니다. PNG signature와 크기가 맞습니다.
//! - 빠진 node(headless Text)는 `--omit-skipped` 없이 실패하고 파일을 쓰지 않습니다.
//! - CPU target이 그리지 않는 효과(shadow)는 실패하고 파일을 쓰지 않습니다.
//! - 프로젝트 파일은 바꾸지 않습니다.

use std::process::{Command, Output, Stdio};

use stellar_cli::json::{self, JsonValue};
use stellar_ops::export_png::fixture::{document, document_id, view_id, Variant};
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

fn project(label: &str, variant: Variant) -> TempDir {
    let dir = TempDir::new(label);
    scaffold_project(&dir.path);
    save_documents(&dir.path, &[&document(variant)]);
    dir
}

#[test]
fn saved_view_exports_to_png_file() {
    let dir = project("export-png-cli", Variant::Plain);
    let out_dir = TempDir::new("export-png-cli-out");
    let out = out_dir.path.join("view.png");
    let before = file_digest(&dir.path);
    let (path, document, view) = (
        dir.path.to_str().unwrap(),
        document_id().to_string(),
        view_id().to_string(),
    );
    let output = stellar(&[
        "export",
        "png",
        path,
        &document,
        &view,
        "--output",
        out.to_str().unwrap(),
        "--scale",
        "2",
    ]);
    let value = envelope(&output, 0);
    let result = value.get("result").unwrap();
    assert_eq!(result.get("width").unwrap().as_i64(), Some(80));
    assert_eq!(result.get("height").unwrap().as_i64(), Some(40));
    assert_eq!(result.get("profile").unwrap().as_str(), Some("srgb"));
    assert_eq!(
        result.get("path").unwrap().as_str(),
        Some(out.to_str().unwrap())
    );
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(
        result.get("bytes").unwrap().as_i64(),
        Some(bytes.len() as i64)
    );
    // 같은 입력은 같은 bytes입니다.
    let again = stellar(&[
        "export",
        "png",
        path,
        &document,
        &view,
        "--output",
        out.to_str().unwrap(),
        "--scale",
        "2",
    ]);
    envelope(&again, 0);
    assert_eq!(std::fs::read(&out).unwrap(), bytes);
    assert_eq!(
        file_digest(&dir.path),
        before,
        "프로젝트 파일을 바꾸지 않습니다"
    );
    // display-p3와 배경.
    let p3 = stellar(&[
        "export",
        "png",
        path,
        &document,
        &view,
        "--output",
        out.to_str().unwrap(),
        "--profile",
        "display-p3",
        "--background",
        "#ffffff",
    ]);
    let value = envelope(&p3, 0);
    assert_eq!(
        value
            .get("result")
            .unwrap()
            .get("profile")
            .unwrap()
            .as_str(),
        Some("display-p3")
    );
}

#[test]
fn skipped_nodes_and_unsupported_effects_fail_without_writing() {
    let dir = project("export-png-cli-text", Variant::WithText);
    let out_dir = TempDir::new("export-png-cli-text-out");
    let out = out_dir.path.join("view.png");
    let (path, document, view) = (
        dir.path.to_str().unwrap(),
        document_id().to_string(),
        view_id().to_string(),
    );
    let args = [
        "export",
        "png",
        path,
        &document,
        &view,
        "--output",
        out.to_str().unwrap(),
    ];
    let value = envelope(&stellar(&args), 1);
    let error = value.get("error").unwrap();
    assert_eq!(
        error.get("code").unwrap().as_str(),
        Some("png-skipped-nodes")
    );
    assert!(!out.exists());
    let mut confirmed = args.to_vec();
    confirmed.push("--omit-skipped");
    let value = envelope(&stellar(&confirmed), 0);
    let skipped = value
        .get("result")
        .unwrap()
        .get("skipped")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(skipped.len(), 1);
    assert_eq!(
        skipped[0].get("reason").unwrap().as_str(),
        Some("text-layout-unavailable")
    );
    assert!(out.exists());

    let dir = project("export-png-cli-shadow", Variant::WithShadow);
    let out = out_dir.path.join("shadow.png");
    let path = dir.path.to_str().unwrap();
    let value = envelope(
        &stellar(&[
            "export",
            "png",
            path,
            &document,
            &view,
            "--output",
            out.to_str().unwrap(),
        ]),
        1,
    );
    // 그리지 못하는 효과는 도메인 실패이고, 출력 파일을 만들 수 없다는 `EX_CANTCREAT`(73)가 아닙니다.
    assert_eq!(
        value.get("error").unwrap().get("code").unwrap().as_str(),
        Some("png-unsupported")
    );
    assert!(!out.exists());
}

#[test]
fn usage_errors_are_reported() {
    let output = stellar(&["export", "png"]);
    assert_eq!(output.status.code(), Some(64));
    let output = stellar(&["export", "png", "/x", "a", "b"]);
    assert_eq!(output.status.code(), Some(64));
}

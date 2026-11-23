//! 저장 View → PNG 내보내기 테스트 (XX.2.3, ADR-0116 결정 5).

use super::*;
use crate::export_png::fixture::{document, document_id, view_id, Variant};
use crate::snapshot::open_snapshot;
use crate::test_support::{save_documents, scaffold_project, TempDir};

fn project(label: &str, variant: Variant) -> TempDir {
    let dir = TempDir::new(label);
    scaffold_project(&dir.path);
    save_documents(&dir.path, &[&document(variant)]);
    dir
}

fn request(dir: &TempDir, scale: f32) -> ViewPngRequest {
    ViewPngRequest {
        document: document_id(),
        view: view_id(),
        scale,
        profile: "srgb".into(),
        background: None,
        output: dir.path.join("out.png"),
        accept_issues: false,
        omit_skipped: false,
    }
}

fn pixel(png: &stellar_export::png::DecodedPng, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * png.width + x) * 4) as usize;
    png.rgba[at..at + 4].try_into().unwrap()
}

#[test]
fn saved_view_exports_to_png_with_scene_conversion() {
    let dir = project("png-view", Variant::Plain);
    let snapshot = open_snapshot(&dir.path).unwrap();
    let outcome = export_view_png(&snapshot, &request(&dir, 2.0)).unwrap();
    assert_eq!((outcome.saved.width, outcome.saved.height), (80, 40));
    assert!(outcome.skipped.is_empty());
    assert!(outcome.accepted.is_empty());
    let bytes = std::fs::read(dir.path.join("out.png")).unwrap();
    assert_eq!(bytes.len() as u64, outcome.saved.bytes);
    let png = decode_png(&bytes, DecodeLimits::default()).unwrap();
    assert_eq!(pixel(&png, 0, 0), [255, 0, 0, 255]);
    // 파란 Frame(opacity 0.5)은 빨강 위에 선형광 반 섞임입니다: 채널 선형값 0.5 → sRGB 188.
    assert_eq!(pixel(&png, 15, 15), [188, 0, 188, 255]);
    // 파일을 쓰지 않는 렌더는 같은 bytes입니다.
    let rendered = render_view_png(&snapshot, &request(&dir, 2.0)).unwrap();
    assert_eq!(rendered.file.bytes(), &bytes[..]);
    assert_eq!(rendered.profile, "srgb");
    assert!(png_base64(&rendered.file).starts_with("iVBORw0KGgo"));
}

#[test]
fn background_and_profile_follow_request() {
    let dir = project("png-view-bg", Variant::Plain);
    let snapshot = open_snapshot(&dir.path).unwrap();
    let mut req = request(&dir, 1.0);
    req.background = Some("#00ff00".into());
    req.profile = "display-p3".into();
    let rendered = render_view_png(&snapshot, &req).unwrap();
    assert_eq!(rendered.profile, "display-p3");
    let png = decode_png(rendered.file.bytes(), DecodeLimits::default()).unwrap();
    assert_eq!((png.width, png.height), (40, 20));
    req.background = Some("초록".into());
    assert!(matches!(
        render_view_png(&snapshot, &req),
        Err(ViewPngError::InvalidRequest(_))
    ));
    req.background = None;
    req.profile = "adobe-rgb".into();
    assert!(matches!(
        render_view_png(&snapshot, &req),
        Err(ViewPngError::InvalidRequest(_))
    ));
}

#[test]
fn skipped_nodes_need_confirmation() {
    let dir = project("png-view-text", Variant::WithText);
    let snapshot = open_snapshot(&dir.path).unwrap();
    let mut req = request(&dir, 1.0);
    match export_view_png(&snapshot, &req) {
        Err(ViewPngError::SkippedNodes(nodes)) => {
            assert_eq!(nodes.len(), 1);
            assert_eq!(nodes[0].reason, SkipReason::Text);
        }
        other => panic!("{other:?}"),
    }
    assert!(!dir.path.join("out.png").exists());
    req.omit_skipped = true;
    let outcome = export_view_png(&snapshot, &req).unwrap();
    assert_eq!(outcome.skipped.len(), 1);
    assert!(dir.path.join("out.png").exists());
}

#[test]
fn unsupported_cpu_effects_fail_without_writing() {
    let dir = project("png-view-shadow", Variant::WithShadow);
    let snapshot = open_snapshot(&dir.path).unwrap();
    let error = export_view_png(&snapshot, &request(&dir, 1.0)).unwrap_err();
    assert!(error.to_string().contains("shadow"), "{error}");
    assert_eq!(error.save_failure(), Some(SaveFailure::Unsupported));
    assert!(!dir.path.join("out.png").exists());
}

#[test]
fn missing_inputs_are_reported() {
    let dir = TempDir::new("png-view-unsaved");
    scaffold_project(&dir.path);
    let snapshot = open_snapshot(&dir.path).unwrap();
    assert!(matches!(
        export_view_png(&snapshot, &request(&dir, 1.0)),
        Err(ViewPngError::Unsaved)
    ));
    let dir = project("png-view-unknown", Variant::Plain);
    let snapshot = open_snapshot(&dir.path).unwrap();
    let mut req = request(&dir, 1.0);
    req.view = document_id();
    assert!(matches!(
        export_view_png(&snapshot, &req),
        Err(ViewPngError::UnknownView(_))
    ));
    let mut req = request(&dir, 1.0);
    req.output = "relative.png".into();
    assert!(matches!(
        export_view_png(&snapshot, &req),
        Err(ViewPngError::InvalidRequest(_))
    ));
    assert_eq!(ViewPngError::Unsaved.save_failure(), None);
}

//! ADR-0059 첫 원본 경로: 모델 편집→저장→재열기→실제 Git diff와 파생 cache 복구.
mod common;

use common::{id, node, TempDir};
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::reference::NodeRef;
use stellar_core_types::value::{Size2, Transform2D};
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::{Border, Clipping, CornerRadius, FramePayload};
use stellar_document::model::DocumentModel;
use stellar_document::records::{DocumentRecords, NodePayload};
use stellar_document::text::TextPayload;
use stellar_project::document_io::text_source::{self, CacheStatus, TextSourceError};

fn fixture() -> DocumentModel {
    let mut definition = DocumentDefinition::new(id(1, 1), "문서: \"sample\"\nsecond".into());
    let mut page = PageDefinition::new(id(1, 2), "페이지".into());
    page.add_view(ViewDefinition::new(id(1, 3), "Checkout".into(), id(1, 4)))
        .unwrap();
    definition.add_page(page).unwrap();
    let color = Color::new(ColorSpace::DisplayP3, 1.25, -0.125, 0.1, 0.5).unwrap();
    let frame = FramePayload {
        size: Size2::new(320.25, 240.125).unwrap(),
        transform: Transform2D::new(-1.0, 0.1, 0.2, 1.0, 20.5, -0.0).unwrap(),
        clipping: Clipping::Path(NodeRef::new(id(1, 99))),
        background: Some(color),
        border: Some(Border::new(color, 0.125).unwrap()),
        corner_radius: CornerRadius::new(1.0, 2.0, 3.0, 4.0).unwrap(),
        layout_mode: Some(id(1, 98)),
    };
    let records = DocumentRecords {
        definition,
        nodes: vec![
            node(1, 4, None, 50, "Root", NodePayload::Frame(frame)),
            node(
                1,
                5,
                Some(4),
                50,
                "Group",
                NodePayload::Fragment(FragmentPayload),
            ),
            node(
                1,
                6,
                Some(5),
                50,
                "Button",
                NodePayload::Frame(FramePayload::default()),
            ),
        ],
        modifier_stacks: Vec::new(),
    };
    DocumentModel::from_records(records).unwrap().0
}

#[test]
fn actual_model_edit_save_reopen_and_readable_git_diff() {
    let dir = TempDir::new("stellar-text-source-edit");
    let runner =
        common::git::hermetic_runner().expect("Git is required for source collaboration test");
    common::git::init_repo(&runner, &dir.path);
    let path = dir.path.join("checkout-0123456789ab.stellar.yaml");
    let model = fixture();
    let hash = text_source::save_new(&path, &model).unwrap();
    let original = std::fs::read(&path).unwrap();
    assert!(std::str::from_utf8(&original).unwrap().contains("320.25"));
    let mut loaded = text_source::open(&path).unwrap();
    assert_eq!(loaded.source_hash, hash);
    assert_eq!(
        loaded.model.to_records().unwrap(),
        model.to_records().unwrap()
    );
    assert_eq!(text_source::save(&path, &loaded.model, hash).unwrap(), hash);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    common::git::commit_all(&runner, &dir.path, "baseline");
    loaded
        .model
        .set_display_name(id(1, 6), "Confirm".into())
        .unwrap();
    let saved = text_source::save(&path, &loaded.model, hash).unwrap();
    assert_ne!(saved, hash);
    let reopened = text_source::open(&path).unwrap();
    assert_eq!(
        reopened.model.to_records().unwrap(),
        loaded.model.to_records().unwrap()
    );
    let diff = common::git::git_ok(&runner, &dir.path, ["diff", "--no-ext-diff", "--unified=0"]);
    let diff = String::from_utf8(diff.stdout).unwrap();
    assert!(diff.contains("-    display_name: Button"), "{diff}");
    assert!(diff.contains("+    display_name: Confirm"), "{diff}");
    assert!(!diff.contains("Binary files"));
    assert!(matches!(
        text_source::save(&path, &model, hash),
        Err(TextSourceError::Changed)
    ));
    assert_eq!(text_source::open(&path).unwrap().source_hash, saved);
}

#[test]
fn missing_corrupt_and_unwritable_cache_never_own_document_data() {
    let dir = TempDir::new("stellar-text-source-cache");
    let runner = common::git::hermetic_runner().expect("Git is required for cache exclusion test");
    common::git::init_repo(&runner, &dir.path);
    let path = dir.path.join("view.stellar.yaml");
    let model = fixture();
    let source_hash = text_source::save_new(&path, &model).unwrap();
    let source_bytes = std::fs::read(&path).unwrap();
    assert!(!dir.path.join(".stellar").exists());
    let (_, status) = text_source::open_with_cache(&path, &dir.path).unwrap();
    assert!(matches!(status, CacheStatus::Rebuilt));
    let cache = dir
        .path
        .join(".stellar/cache/text-source-v1")
        .join(source_hash.digest_hex());
    let chunks: Vec<_> = std::fs::read_dir(&cache)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(chunks.len(), 3);
    let untracked = common::git::git_ok(
        &runner,
        &dir.path,
        [
            "ls-files",
            "--others",
            "--exclude-standard",
            "--",
            ".stellar",
        ],
    );
    assert!(
        untracked.stdout.is_empty(),
        "cache must stay outside Git even in existing projects"
    );
    let (_, status) = text_source::open_with_cache(&path, &dir.path).unwrap();
    assert!(matches!(status, CacheStatus::Verified));
    std::fs::write(&chunks[0], b"corrupt cache").unwrap();
    let (opened, status) = text_source::open_with_cache(&path, &dir.path).unwrap();
    assert!(matches!(status, CacheStatus::Rebuilt));
    assert_eq!(
        opened.model.to_records().unwrap(),
        model.to_records().unwrap()
    );
    std::fs::remove_dir_all(dir.path.join(".stellar/cache")).unwrap();
    assert_eq!(
        text_source::open(&path)
            .unwrap()
            .model
            .to_records()
            .unwrap(),
        model.to_records().unwrap()
    );
    let (_, status) = text_source::open_with_cache(&path, &dir.path).unwrap();
    assert!(matches!(status, CacheStatus::Rebuilt));
    std::fs::remove_dir_all(dir.path.join(".stellar/cache")).unwrap();
    std::fs::write(dir.path.join(".stellar/cache"), b"not a directory").unwrap();
    let (opened, status) = text_source::open_with_cache(&path, &dir.path).unwrap();
    assert!(matches!(status, CacheStatus::Unavailable(_)));
    assert_eq!(
        opened.model.to_records().unwrap(),
        model.to_records().unwrap()
    );
    assert_eq!(std::fs::read(&path).unwrap(), source_bytes);
}

#[test]
fn external_property_edit_reloads_and_invalidates_cache_by_source_bytes() {
    let dir = TempDir::new("stellar-text-source-external");
    let path = dir.path.join("view.stellar.yaml");
    let model = fixture();
    let before = text_source::save_new(&path, &model).unwrap();
    text_source::open_with_cache(&path, &dir.path).unwrap();
    let edited = std::fs::read_to_string(&path)
        .unwrap()
        .replace("320.25", "480.5");
    std::fs::write(&path, &edited).unwrap();
    let (opened, status) = text_source::open_with_cache(&path, &dir.path).unwrap();
    assert!(matches!(status, CacheStatus::Rebuilt));
    assert_ne!(opened.source_hash, before);
    let records = opened.model.to_records().unwrap();
    let root = records
        .nodes
        .iter()
        .find(|node| node.id == id(1, 4))
        .unwrap();
    let NodePayload::Frame(frame) = &root.payload else {
        panic!("root frame")
    };
    assert_eq!(frame.size.width(), 480.5);
    assert!(matches!(
        text_source::save(&path, &model, before),
        Err(TextSourceError::Changed)
    ));
    assert_eq!(std::fs::read_to_string(path).unwrap(), edited);
}

#[test]
fn unsupported_input_or_external_fields_never_overwrite_source() {
    let dir = TempDir::new("stellar-text-source-refusal");
    let path = dir.path.join("view.stellar.yaml");
    let model = fixture();
    let hash = text_source::save_new(&path, &model).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let mut records = model.to_records().unwrap();
    records.nodes.push(node(
        1,
        7,
        Some(4),
        80,
        "Text",
        NodePayload::Text(TextPayload::empty(0)),
    ));
    let unsupported = DocumentModel::from_records(records).unwrap().0;
    assert!(matches!(
        text_source::save(&path, &unsupported, hash),
        Err(TextSourceError::Unsupported(_))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(text_source::save_new(&path, &model).is_err());
    let future = format!(
        "{}future_records: preserved\n",
        String::from_utf8(bytes).unwrap()
    );
    std::fs::write(&path, &future).unwrap();
    assert!(text_source::open(&path).is_err());
    assert!(text_source::save(&path, &model, hash).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), future);
}

#[cfg(unix)]
#[test]
fn symlink_source_is_readable_but_writes_and_cache_links_are_rejected() {
    use std::os::unix::fs::symlink;
    let dir = TempDir::new("stellar-text-source-link");
    let path = dir.path.join("view.stellar.yaml");
    let model = fixture();
    text_source::save_new(&path, &model).unwrap();
    let link = dir.path.join("link.stellar.yaml");
    symlink(&path, &link).unwrap();
    assert_eq!(
        text_source::open(&link)
            .unwrap()
            .model
            .to_records()
            .unwrap(),
        model.to_records().unwrap()
    );
    assert!(text_source::save_new(&link, &model).is_err());
    let mut opened = text_source::open(&link).unwrap();
    let original = std::fs::read(&path).unwrap();
    opened
        .model
        .set_display_name(id(1, 6), "Edited".into())
        .unwrap();
    assert!(text_source::save(&link, &opened.model, opened.source_hash).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::read_link(&link).unwrap(), path);
    let outside = dir.path.join("outside");
    std::fs::create_dir(&outside).unwrap();
    symlink(&outside, dir.path.join(".stellar")).unwrap();
    let (_, status) = text_source::open_with_cache(&path, &dir.path).unwrap();
    assert!(matches!(status, CacheStatus::Unavailable(_)));
    assert_eq!(std::fs::read_dir(outside).unwrap().count(), 0);
}

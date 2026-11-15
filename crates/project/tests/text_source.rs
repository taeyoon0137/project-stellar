//! ADR-0059 첫 원본 경로: 모델 편집→저장→재열기→실제 Git diff와 파생 cache 복구.
mod common;

use common::{id, node, TempDir};
use stellar_core_types::color::{Color, ColorSpace};
use stellar_core_types::reference::NodeRef;
use stellar_core_types::text::UserText;
use stellar_core_types::value::{Size2, Transform2D};
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::fragment::FragmentPayload;
use stellar_document::frame::{Border, Clipping, CornerRadius, FramePayload};
use stellar_document::model::DocumentModel;
use stellar_document::records::{DocumentRecords, NodePayload};
use stellar_document::shape::ShapeGeometry;
use stellar_document::text::{Paragraph, StyleRun, TextIndex, TextPayload, TextRange};
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
            node(
                1,
                8,
                Some(6),
                50,
                "Label",
                NodePayload::Text(
                    TextPayload::new(
                        UserText::new("확인 e\u{301} 👩‍💻\r\n다음\0\t\"줄\""),
                        vec![Paragraph { style: 7 }, Paragraph { style: u32::MAX }],
                        vec![
                            StyleRun {
                                range: TextRange::new(3, 5).unwrap(),
                                style: 9,
                            },
                            StyleRun {
                                range: TextRange::new(0, 8).unwrap(),
                                style: 2,
                            },
                            StyleRun {
                                range: TextRange::new(5, 5).unwrap(),
                                style: 1,
                            },
                        ],
                    )
                    .unwrap(),
                ),
            ),
        ],
        modifier_stacks: Vec::new(),
    };
    DocumentModel::from_records(records).unwrap().0
}

#[test]
fn initial_typography_recreates_identical_chunks_and_preserves_v1() {
    use stellar_project::document_io::serialize::serialize_document;

    let dir = TempDir::new("text-source-typography");
    let model = fixture();
    let legacy = dir.path.join("v1.stellar.yaml");
    text_source::save_new(&legacy, &model).unwrap();
    let before = std::fs::read_to_string(&legacy).unwrap();
    assert!(before.starts_with("schema_version: \"1\"\n"));
    let resource = id(2, 101);
    let modern = dir.path.join("v2.stellar.yaml");
    let after = before
        .replacen("schema_version: \"1\"", "schema_version: \"2\"", 1)
        .replacen(
            "\npage:",
            &format!("\n  initial_typography_id: \"{resource}\"\npage:"),
            1,
        );
    std::fs::write(&modern, &after).unwrap();
    let opened = text_source::open(&modern).unwrap();
    // 앱 기본값에 의존하지 않고, 기존 v2 원본이 복원한 전체 STTS 표를 고정합니다.
    assert_eq!(
        stellar_storage::hash::ContentHash::compute(
            &opened.model.definition().text_styles.encode()
        )
        .to_string(),
        "sha256:86dbafa6c6cfc9e3de5a4b7c0d4aeaff6876f3318f0a0e7aa032443aca841b73",
    );
    let copy = dir.path.join("copy.stellar.yaml");
    let hash = text_source::save_new(&copy, &opened.model).unwrap();
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), after);
    let copied = text_source::open(&copy).unwrap();
    assert_eq!(
        serialize_document(&opened.model).unwrap(),
        serialize_document(&copied.model).unwrap()
    );
    assert_eq!(
        text_source::save(&modern, &opened.model, hash).unwrap(),
        hash
    );
    assert_eq!(std::fs::read_to_string(&modern).unwrap(), after);

    for wrong in [
        after.replacen("schema_version: \"2\"", "schema_version: \"1\"", 1),
        before.replacen("schema_version: \"1\"", "schema_version: \"2\"", 1),
        after.replace(&resource.to_string(), "invalid-id"),
        after.replace(&resource.to_string(), &model.document_id().to_string()),
        after.replace(
            &resource.to_string(),
            &stellar_core_types::ids::StableId::NIL.to_string(),
        ),
        after.replace("initial_typography_id:", "unknown_typography_id:"),
    ] {
        assert_ne!(after, wrong);
        std::fs::write(&modern, &wrong).unwrap();
        assert!(text_source::open(&modern).is_err());
        assert!(text_source::save(&modern, &model, hash).is_err());
        assert_eq!(std::fs::read_to_string(&modern).unwrap(), wrong);
    }
}

#[test]
fn explicit_resources_and_modified_tables_never_fall_back_to_initial() {
    use stellar_core_types::font::FontFileHash;
    use stellar_core_types::typed_value::TypographyValue;
    use stellar_core_types::typography::FontSize;
    use stellar_document::text_styles::{DocumentFontAsset, DocumentTextStyles};
    let dir = TempDir::new("text-source-typography");
    let path = dir.path.join("view.stellar.yaml");
    let mut model = fixture();
    let resource = id(2, 101);
    let initial = DocumentTextStyles::initial(resource);
    model.replace_text_styles(initial.clone());
    let expected = text_source::save_new(&path, &model).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    for change in 0..7 {
        let mut changed = initial.clone();
        match change {
            0 => changed.default_fill = Some(Color::new(ColorSpace::Srgb, 1., 0., 0., 1.).unwrap()),
            1 => {
                changed.paragraphs.insert(1, changed.paragraphs[&0]);
            }
            2 => {
                changed.runs.insert(1, changed.runs[&0].clone());
            }
            3 => {
                changed
                    .typography
                    .insert(id(2, 102), changed.typography[&resource].clone());
            }
            4 => {
                changed.fonts.insert(
                    FontFileHash::from_sha256_digest([1; 32]),
                    DocumentFontAsset {
                        font_record: vec![1],
                        asset_record: vec![2],
                    },
                );
            }
            5 => {
                changed.default_typography = None;
            }
            _ => {
                let mut typography = changed.typography[&resource].record().unwrap();
                typography.size = FontSize::Subpixels(16 * 64);
                changed
                    .typography
                    .insert(resource, TypographyValue::from_record(&typography).unwrap());
            }
        }
        model.replace_text_styles(changed);
        assert!(matches!(
            text_source::save(&path, &model, expected),
            Err(TextSourceError::Unsupported(_))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn placed_text_and_registered_document_font_recreate_the_same_chunks() {
    use stellar_document::modifier::{ModifierId, ModifierKind, ModifierPayload, ModifierStack};
    use stellar_document::text_styles::DocumentTextStyles;
    use stellar_project::document_io::serialize::serialize_document;
    use stellar_project::fonts::document::{
        import_default_font, load_document_face, verify_document_font_assets, DocumentFontSelection,
    };

    let project = TempDir::new("text-source-font");
    let user_data = TempDir::new("text-source-font-user");
    let font = user_data.path.join("face.ttf");
    std::fs::write(&font, stellar_text::test_support::outline_test_font()).unwrap();
    let resource = id(2, 101);
    let selection = DocumentFontSelection {
        file: font,
        index: 0,
        size: 24 * 64 + 1,
        fill: Color::new(ColorSpace::DisplayP3, 1.125, 0., -0.25, 0.5).unwrap(),
    };
    let imported = import_default_font(
        &project.path,
        &user_data.path,
        &DocumentTextStyles::initial(resource),
        [resource, id(2, 102)],
        &selection,
    )
    .unwrap();
    let mut model = fixture();
    model.replace_text_styles(imported.styles);
    let node = id(1, 8);
    model
        .set_payload(
            node,
            NodePayload::Text(
                TextPayload::new(
                    UserText::new("Submit\n확인"),
                    vec![Paragraph { style: 0 }; 2],
                    vec![StyleRun {
                        style: 0,
                        range: TextRange::new(2, 5).unwrap(),
                    }],
                )
                .unwrap(),
            ),
        )
        .unwrap();
    let mut stack = ModifierStack::new(node);
    let modifier = ModifierId::new(id(2, 110));
    stack.push_back(modifier, ModifierKind::Transform).unwrap();
    stack
        .set_payload(
            modifier,
            ModifierPayload::Transform(Transform2D::new(1., 0., 0., 1., -12.25, 8.5).unwrap()),
        )
        .unwrap();
    model.set_modifier_stack(stack).unwrap();
    let path = project.path.join("submit.stellar.yaml");
    let hash = text_source::save_new(&path, &model).unwrap();
    let source = std::fs::read_to_string(&path).unwrap();
    assert!(source.starts_with("schema_version: \"3\""));
    assert!(source.contains("24.015625px") && source.contains("-12.25 8.5"));
    let loaded = text_source::open(&path).unwrap().model;
    assert_eq!(
        serialize_document(&model).unwrap(),
        serialize_document(&loaded).unwrap()
    );
    load_document_face(
        &project.path,
        &loaded.definition().text_styles,
        &imported.reference,
    )
    .unwrap();
    assert_eq!(text_source::save(&path, &loaded, hash).unwrap(), hash);

    let styles = &loaded.definition().text_styles;
    verify_document_font_assets(&project.path, styles).unwrap();
    let store = stellar_assets::store::AssetStore::new(&project.path);
    let record = stellar_assets::record::AssetRecord::decode(
        &styles.fonts[&imported.reference.file_hash()].asset_record,
    )
    .unwrap();
    let object = store
        .locate_content(&record.content_hash, record.size)
        .unwrap()
        .unwrap();
    let asset_path = store.content_path(&object);
    let mut bytes = std::fs::read(&asset_path).unwrap();
    bytes[0] ^= 1;
    std::fs::write(&asset_path, bytes).unwrap();
    assert!(verify_document_font_assets(&project.path, styles).is_err());
    std::fs::remove_file(asset_path).unwrap();
    assert!(verify_document_font_assets(&project.path, styles).is_err());
    assert!(text_source::open(&path).is_ok());

    for (before, after) in [
        ("24.015625px", "24.01562500000000001px"),
        ("face_index: \"0\"", "face_index: \"1\""),
        ("file_hash:", "unknown_hash:"),
        ("style: \"0\"", "style: \"9\""),
        ("font-style: normal", "font-style: italic"),
        ("placement:", "lost_placement:"),
        ("font/ttf", "image/png"),
        ("kind: external", "kind: project"),
        ("schema_version: \"3\"", "schema_version: \"2\""),
    ] {
        let bad = source.replacen(before, after, 1);
        assert_ne!(bad, source);
        std::fs::write(&path, &bad).unwrap();
        assert!(text_source::open(&path).is_err(), "{after}");
        assert!(text_source::save(&path, &loaded, hash).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), bad);
    }
}

#[test]
fn unchanged_save_keeps_collaborator_comments_and_external_edits() {
    let dir = TempDir::new("stellar-text-source-noop");
    let path = dir.path.join("sample.stellar.yaml");
    text_source::save_new(&path, &fixture()).unwrap();
    let canonical = std::fs::read_to_string(&path).unwrap();
    let annotated = format!("# 편집자 주석\n{canonical}");
    std::fs::write(&path, &annotated).unwrap();
    let opened = text_source::open(&path).unwrap();
    assert_eq!(
        text_source::save(&path, &opened.model, opened.source_hash).unwrap(),
        opened.source_hash
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), annotated);
    let external = format!("# 다른 편집\n{annotated}");
    std::fs::write(&path, &external).unwrap();
    assert!(matches!(
        text_source::save(&path, &opened.model, opened.source_hash),
        Err(TextSourceError::Changed)
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), external);
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
fn rectangle_preserves_affine_color_corners_and_modifier_identity() {
    use stellar_core_types::order_key::OrderKey;
    use stellar_document::modifier::{
        ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack,
    };
    use stellar_document::shape::RectangleParams;
    use stellar_project::document_io::serialize::serialize_document;

    let dir = TempDir::new("text-source-rectangle");
    let source = dir.path.join("view.stellar.yaml");
    let mut model = fixture();
    let shape = node(
        1,
        7,
        Some(4),
        80,
        "Card background",
        NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
            size: Size2::new(37.5, 18.25).unwrap(),
            corner_radius: CornerRadius::new(25.0, 0.0, 3.5, 0.25).unwrap(),
        })),
    );
    let id = shape.id;
    model.insert_node(shape).unwrap();
    let fill = Color::new(ColorSpace::DisplayP3, 1.2, -0.1, 0.25, 0.0).unwrap();
    let transform = Transform2D::new(0., -1., 1.25, 0., 17.5, -3.).unwrap();
    let mut stack = ModifierStack::new(id);
    for (index, payload, enabled) in [
        (1, ModifierPayload::Fill { color: fill }, false),
        (2, ModifierPayload::Transform(transform), true),
    ] {
        stack
            .insert(ModifierEntry {
                id: ModifierId::new(common::id(4, index)),
                order: OrderKey::between(stack.entries().last().map(|entry| &entry.order), None),
                kind: payload.kind(),
                enabled,
                payload,
            })
            .unwrap();
    }
    model.set_modifier_stack(stack.clone()).unwrap();
    let hash = text_source::save_new(&source, &model).unwrap();
    let yaml = std::fs::read_to_string(&source).unwrap();
    let opened = text_source::open(&source).unwrap();
    assert_eq!(
        serialize_document(&opened.model).unwrap(),
        serialize_document(&model).unwrap()
    );
    stack
        .push_back(ModifierId::new(common::id(4, 3)), ModifierKind::Opacity)
        .unwrap();
    model.set_modifier_stack(stack).unwrap();
    assert!(matches!(
        text_source::save(&source, &model, hash),
        Err(TextSourceError::Unsupported(_))
    ));
    assert_eq!(std::fs::read_to_string(&source).unwrap(), yaml);
    for malformed in [
        yaml.replace("border-top-left-radius:", "future-radius:"),
        yaml.replace("kind: fill", "kind: future-fill"),
        yaml.replace("height: \"18.25\"", "height: \"-1\""),
    ] {
        assert_ne!(malformed, yaml);
        std::fs::write(&source, malformed).unwrap();
        assert!(text_source::open(&source).is_err());
    }
}

#[test]
fn ellipse_directed_lines_and_polygon_preserve_geometry_and_modifier_chunk_bytes() {
    use stellar_core_types::value::{Angle, FiniteF32, Rect, Vec2};
    use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierPayload, ModifierStack};
    use stellar_document::shape::{EllipseParams, LineCap, LineParams, PolygonParams};
    use stellar_project::document_io::serialize::serialize_document;

    let dir = TempDir::new("text-source-shapes");
    let path = dir.path.join("view.stellar.yaml");
    let mut model = fixture();
    let point = |x, y| Vec2::new(x, y).unwrap();
    let bounds = Rect::new(point(-3.25, 7.5), Size2::new(12.75, 8.5).unwrap());
    let color = Color::new(ColorSpace::DisplayP3, 1.25, -0.125, 0.75, 0.5).unwrap();
    let transform = Transform2D::new(-1., 0.125, 0.75, 1., -23.5, 0.25).unwrap();
    let line = |start, end, cap| ShapeGeometry::Line(LineParams { start, end, cap });
    let shapes = [
        ShapeGeometry::Ellipse(EllipseParams::new(bounds).unwrap()),
        line(point(7., 8.), point(-52.25, -24.5), LineCap::Square),
        line(point(-4., 6.), point(-4., 6.), LineCap::Round),
        line(point(0., 9.), point(0., 1.), LineCap::Butt),
        ShapeGeometry::Polygon(
            PolygonParams::new(bounds, 9, Angle::new(-32.5).unwrap(), 19.75).unwrap(),
        ),
    ];
    for (index, geometry) in shapes.into_iter().enumerate() {
        let number = index as u64 + 10;
        let shape = node(
            1,
            number,
            Some(5),
            number as u8,
            "Geometry",
            NodePayload::Shape(geometry),
        );
        let mut stack = ModifierStack::new(shape.id);
        model.insert_node(shape).unwrap();
        // 순서 키와 ID의 정렬이 달라도 실제 순서, 비활성 항목 및 색상 공간을 보존합니다.
        for (order, enabled, payload) in [
            (
                132,
                true,
                ModifierPayload::Stroke {
                    color,
                    width: FiniteF32::new(2.375).unwrap(),
                },
            ),
            (33, false, ModifierPayload::Fill { color }),
            (235, true, ModifierPayload::Transform(transform)),
        ] {
            stack
                .insert(ModifierEntry {
                    id: ModifierId::new(id(number, 255 - u64::from(order))),
                    order: common::key(order),
                    enabled,
                    kind: payload.kind(),
                    payload,
                })
                .unwrap();
        }
        model.set_modifier_stack(stack).unwrap();
    }
    text_source::save_new(&path, &model).unwrap();
    let source = std::fs::read_to_string(&path).unwrap();
    let opened = text_source::open(&path).unwrap();
    assert_eq!(
        serialize_document(&opened.model).unwrap(),
        serialize_document(&model).unwrap()
    );
    let second = dir.path.join("canonical.stellar.yaml");
    text_source::save_new(&second, &opened.model).unwrap();
    assert_eq!(std::fs::read_to_string(second).unwrap(), source);
    for (old, new) in [
        ("kind: ellipse", "kind: future-shape"),
        ("origin:", "future-origin:"),
        ("end:", "future-end:"),
        ("cap: square", "cap: future"),
        ("sides: \"9\"", "sides: \"1001\""),
        ("rotation_degrees: \"-32.5\"", "rotation_degrees: NaN"),
        ("corner_radius: \"19.75\"", "corner_radius: \"-1\""),
        ("width: \"2.375\"", "width: \"-1\""),
        ("kind: stroke", "kind: future-stroke"),
        (
            "color:\n              space:",
            "future-color:\n              space:",
        ),
    ] {
        let malformed = source.replacen(old, new, 1);
        assert_ne!(malformed, source, "{old}");
        std::fs::write(&path, &malformed).unwrap();
        assert!(text_source::open(&path).is_err(), "{new}");
        assert!(text_source::save(&path, &model, opened.source_hash).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), malformed);
    }
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
        "Path",
        NodePayload::Path(
            stellar_document::path::IdentifiedPath::new(
                stellar_document::path::PathData::default(),
                vec![],
            )
            .unwrap(),
        ),
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

#[test]
fn malformed_text_payloads_and_unknown_nested_fields_preserve_source() {
    let dir = TempDir::new("stellar-text-payload-refusal");
    let path = dir.path.join("view.stellar.yaml");
    let model = fixture();
    text_source::save_new(&path, &model).unwrap();
    let source = std::fs::read_to_string(&path).unwrap();
    for malformed in [
        source.replacen("content:", "future_content:", 1),
        source.replacen("style: \"7\"", "style: \"7\"\n          font: future", 1),
        source.replacen(
            "start: \"3\"",
            "start: \"3\"\n          future_run: keep",
            1,
        ),
        source.replacen("start: \"3\"", "start: \"6\"", 1),
        source.replacen("end: \"5\"", "end: \"9999\"", 1),
        source.replacen("style: \"7\"", "style: \"4294967296\"", 1),
        source.replacen("style: \"7\"", "style: \"-1\"", 1),
        source.replacen("        -\n          style: \"7\"\n", "", 1),
        source.replacen("style_runs:", "missing_runs:", 1),
    ] {
        assert_ne!(malformed, source);
        std::fs::write(&path, &malformed).unwrap();
        let hash = stellar_storage::hash::ContentHash::compute(malformed.as_bytes());
        assert!(text_source::open(&path).is_err());
        assert!(text_source::save(&path, &model, hash).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), malformed);
    }
}

#[test]
fn empty_text_and_long_scalar_limits_round_trip_or_preserve_existing_bytes() {
    let dir = TempDir::new("stellar-text-content-limits");
    let path = dir.path.join("view.stellar.yaml");
    let mut records = fixture().to_records().unwrap();
    let text_index = records.nodes.iter().position(|n| n.id == id(1, 8)).unwrap();
    records.nodes[text_index].payload = NodePayload::Text(TextPayload::empty(u32::MAX));
    let empty = DocumentModel::from_records(records.clone()).unwrap().0;
    let hash = text_source::save_new(&path, &empty).unwrap();
    assert_eq!(
        text_source::open(&path)
            .unwrap()
            .model
            .to_records()
            .unwrap(),
        records
    );
    let bytes = std::fs::read(&path).unwrap();
    records.nodes[text_index].payload = NodePayload::Text(
        TextPayload::new(
            UserText::new(&"x".repeat(40 * 1024)),
            vec![Paragraph { style: 0 }],
            vec![],
        )
        .unwrap(),
    );
    let too_long = DocumentModel::from_records(records).unwrap().0;
    assert!(text_source::save(&path, &too_long, hash).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);

    let mut records = empty.to_records().unwrap();
    records.nodes[text_index].payload = NodePayload::Text(
        TextPayload::new(
            UserText::new("text"),
            vec![Paragraph { style: 0 }],
            vec![StyleRun {
                range: TextRange {
                    start: TextIndex(3),
                    end: TextIndex(1),
                },
                style: 0,
            }],
        )
        .unwrap(),
    );
    let reversed = DocumentModel::from_records(records).unwrap().0;
    assert!(text_source::save(&path, &reversed, hash).is_err());
    let new_path = dir.path.join("invalid.stellar.yaml");
    assert!(text_source::save_new(&new_path, &reversed).is_err());
    assert!(!new_path.exists());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
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

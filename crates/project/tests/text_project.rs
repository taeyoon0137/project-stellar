//! V.10.4 다중 파일 텍스트 원본: 전체 의미 corpus 왕복, 큰 Frame 분할, 거부 규칙, Git 병합.

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use common::golden::golden_document;
use common::{rich_model, TempDir};
use stellar_assets::media_type::MediaType;
use stellar_assets::record::{AssetRecord, SourceMetadata};
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::NodeRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_core_types::value::Length;
use stellar_document::component::definition::{
    ComponentDefinition, ComponentLibrary, DefinitionNode, DefinitionTree,
};
use stellar_document::definitions::{DocumentDefinition, PageDefinition, ViewDefinition};
use stellar_document::frame::FramePayload;
use stellar_document::model::DocumentModel;
use stellar_document::node_kind::NodeKind;
use stellar_document::record_codec::encode_payload;
use stellar_document::records::{DocumentRecords, NodePayload, NodeRecord};
use stellar_document::test_support::random_document::random_valid_document;
use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_document::text::TextPayload;
use stellar_document::text_styles::DocumentTextStyles;
use stellar_project::document_io::source_v4::project::{
    decode, encode, open_dir, read_dir, write_new, TextProject, SPLIT_CHILD_THRESHOLD,
};
use stellar_project::document_io::source_v4::token::{TokenSetSource, TokenSource};
use stellar_storage::hash::ContentHash;

fn sid(n: u64) -> StableId {
    StableId::new_v7(9_000_000 + n, 0, n)
}

fn library() -> ComponentLibrary {
    let mut library = ComponentLibrary::new();
    let component = sid(1);
    let root = sid(2);
    let mut definition = ComponentDefinition::new(component, "Card".into(), NodeRef::new(root));
    definition
        .local_tokens_mut()
        .insert(stellar_core_types::reference::TokenRef::new(sid(40)));
    let mut tree = DefinitionTree::new(definition.reference());
    let node = |id, kind, parent, payload: &NodePayload| DefinitionNode {
        id,
        kind,
        parent,
        order_key: OrderKey::between(None, None),
        visible: true,
        locked: false,
        display_name: "Part".into(),
        payload: encode_payload(payload),
    };
    tree.insert(node(
        root,
        NodeKind::Frame,
        None,
        &NodePayload::Frame(FramePayload::default()),
    ))
    .unwrap();
    tree.insert(node(
        sid(3),
        NodeKind::Text,
        Some(root),
        &NodePayload::Text(TextPayload::empty(0)),
    ))
    .unwrap();
    library.insert(definition, tree).unwrap();
    library
}

fn tokens() -> Vec<TokenSetSource> {
    vec![TokenSetSource {
        id: sid(30),
        name: "Brand Space".into(),
        tokens: vec![TokenSource {
            id: sid(40),
            name: "space/4".into(),
            expression: sid(41),
            value: TypedValue::Length(Length::new(16.0).unwrap()),
        }],
    }]
}

fn images() -> BTreeMap<StableId, AssetRecord> {
    BTreeMap::from([(
        sid(50),
        AssetRecord::new(
            ContentHash::compute(b"png"),
            MediaType::new("image/png").unwrap(),
            3,
            Some(SourceMetadata::project("assets/hero.png").unwrap()),
        ),
    )])
}

fn project(document: DocumentModel) -> TextProject {
    TextProject {
        document,
        components: library(),
        token_sets: tokens(),
        images: images(),
    }
}

fn assert_round_trip(project: &TextProject) -> BTreeMap<String, String> {
    let encoded = encode(project).unwrap();
    let decoded = decode(&encoded.files).unwrap();
    assert_eq!(
        decoded.semantic_key().unwrap(),
        project.semantic_key().unwrap()
    );
    assert_eq!(
        model_snapshot_json(&decoded.document).unwrap(),
        model_snapshot_json(&project.document).unwrap()
    );
    // 같은 의미는 같은 파일 집합·bytes입니다.
    assert_eq!(encode(&decoded).unwrap(), encoded);
    encoded.files
}

#[test]
fn full_semantic_corpus_round_trips_through_text_files() {
    // random corpus: 모든 node 종류(Frame·Fragment·도형·Path·Text·Image)와 모든 modifier 종류, 여러 페이지·View,
    // 깊이 한도 chain을 담습니다.
    for seed in 0..120 {
        let mut document = random_valid_document(seed);
        if seed % 3 == 0 {
            document.replace_text_styles(DocumentTextStyles::initial(sid(100 + seed)));
        }
        assert_round_trip(&project(document));
    }
    for i in 0..12 {
        assert_round_trip(&project(golden_document(i)));
    }
    for doc in 1..4 {
        assert_round_trip(&project(rich_model(doc, 3)));
    }
}

#[test]
fn layout_follows_document_pages_views_components_tokens_and_catalog() {
    let files = assert_round_trip(&project(golden_document(3)));
    let paths: Vec<&str> = files.keys().map(String::as_str).collect();
    assert!(paths.contains(&"document.stellar.yaml"));
    assert!(paths.contains(&"assets/catalog.stellar.yaml"));
    assert!(
        paths.contains(&"tokens/brand-space.stellar.yaml"),
        "{paths:?}"
    );
    assert!(paths.iter().any(|p| p.starts_with("components/Card-")));
    assert!(paths.iter().filter(|p| p.starts_with("pages/")).count() >= 1);
    for path in &paths {
        assert!(path.ends_with(".stellar.yaml"));
    }
    // 문서 파일은 View 파일을 참조만 하고 node를 담지 않습니다.
    assert!(!files["document.stellar.yaml"].contains("nodes:"));
}

fn wide_frame(children: usize) -> DocumentModel {
    let mut definition = DocumentDefinition::new(sid(200), "Wide".into());
    let mut page = PageDefinition::new(sid(201), "Home".into());
    page.add_view(ViewDefinition::new(sid(202), "Main".into(), sid(203)))
        .unwrap();
    definition.add_page(page).unwrap();
    let mut records = DocumentRecords::new(definition);
    let frame = |id, parent, key: &OrderKey, name: &str| NodeRecord {
        id,
        parent,
        order_key: key.clone(),
        visible: true,
        locked: false,
        display_name: name.into(),
        payload: NodePayload::Frame(FramePayload::default()),
    };
    let first = OrderKey::between(None, None);
    records.nodes.push(frame(sid(203), None, &first, "Root"));
    records
        .nodes
        .push(frame(sid(204), Some(sid(203)), &first, "Grid"));
    let mut key = OrderKey::between(None, None);
    for n in 0..children {
        records.nodes.push(NodeRecord {
            payload: NodePayload::Fragment(stellar_document::fragment::FragmentPayload),
            ..frame(
                StableId::new_v7(9_500_000 + n as u64, 0, n as u64),
                Some(sid(204)),
                &key,
                "",
            )
        });
        key = OrderKey::between(Some(&key), None);
    }
    DocumentModel::from_records(records).unwrap().0
}

#[test]
fn frames_over_the_child_threshold_move_to_ref_files() {
    let files = assert_round_trip(&project(wide_frame(SPLIT_CHILD_THRESHOLD + 1)));
    let view = &files["pages/home/main.stellar.yaml"];
    assert!(
        view.contains("\"$ref\": main/grid.stellar.yaml\n")
            || view.contains("\"$ref\": \"main/grid.stellar.yaml\"\n"),
        "{view}"
    );
    let grid = &files["pages/home/main/grid.stellar.yaml"];
    assert!(grid.starts_with("format: stellar-frame\n"));
    for text in files.values() {
        assert!(text.len() <= stellar_project::document_io::source_v4::project::SPLIT_FILE_BYTES);
    }
    // 임계값 이하는 나누지 않습니다.
    let small = assert_round_trip(&project(wide_frame(10)));
    assert!(!small.keys().any(|p| p.starts_with("pages/home/main/")));
}

fn two_grids(children: usize) -> DocumentModel {
    let mut records = wide_frame(0).to_records().unwrap();
    let mut key = OrderKey::between(None, None);
    let second = sid(205);
    records.nodes.push(NodeRecord {
        id: second,
        parent: Some(sid(203)),
        order_key: OrderKey::between(Some(&records.nodes[1].order_key), None),
        visible: true,
        locked: false,
        display_name: "Second".into(),
        payload: NodePayload::Frame(FramePayload::default()),
    });
    for n in 0..children * 2 {
        let parent = if n < children { sid(204) } else { second };
        records.nodes.push(NodeRecord {
            id: StableId::new_v7(9_600_000 + n as u64, 0, n as u64),
            parent: Some(parent),
            order_key: key.clone(),
            visible: true,
            locked: false,
            display_name: String::new(),
            payload: NodePayload::Frame(FramePayload::default()),
        });
        key = OrderKey::between(Some(&key), None);
    }
    DocumentModel::from_records(records).unwrap().0
}

#[test]
fn files_over_one_mebibyte_split_the_largest_frame_first() {
    let size = |files: &BTreeMap<String, String>| files.values().map(String::len).max().unwrap();
    // 두 Frame 하위 tree를 합친 View 파일이 1 MiB를 넘으면 큰 쪽(같으면 작은 StableId)부터 나눕니다.
    let files = assert_round_trip(&project(two_grids(600)));
    assert!(
        files.keys().any(|p| p.starts_with("pages/home/main/")),
        "{:?}",
        files.keys()
    );
    assert!(size(&files) <= stellar_project::document_io::source_v4::project::SPLIT_FILE_BYTES);
    // 더 나눌 Frame이 없는 1 MiB 초과 파일은 쓰지 않습니다.
    assert!(encode(&project(wide_frame(8_000))).is_err());
}

#[test]
fn unreferenced_duplicate_or_unknown_files_and_fields_are_rejected() {
    let files = encode(&project(golden_document(1))).unwrap().files;
    let view = files
        .keys()
        .find(|p| p.starts_with("pages/"))
        .unwrap()
        .clone();
    let mut extra = files.clone();
    extra.insert("pages/stale/old.stellar.yaml".into(), files[&view].clone());
    let mut unknown = files.clone();
    unknown.insert("notes/readme.stellar.yaml".into(), "x: y\n".into());
    let mut missing = files.clone();
    missing.remove(&view);
    let mut no_catalog = files.clone();
    no_catalog.remove("assets/catalog.stellar.yaml");
    let mut field = files.clone();
    field.insert(
        view.clone(),
        files[&view].replacen("    kind: frame\n", "    kind: frame\n    future: x\n", 1),
    );
    let mut version = files.clone();
    version.insert(
        "document.stellar.yaml".into(),
        files["document.stellar.yaml"].replacen("\"4\"", "\"5\"", 1),
    );
    for (case, broken) in [
        ("extra", extra),
        ("unknown", unknown),
        ("missing", missing),
        ("no-catalog", no_catalog),
        ("field", field),
        ("version", version),
    ] {
        assert!(decode(&broken).is_err(), "{case}");
    }
}

#[test]
fn unrepresentable_meaning_refuses_to_write() {
    // 정의 root가 Frame이 아닌 컴포넌트와 리터럴로 쓰지 않는 값 타입의 token은 텍스트로 쓰지 않습니다.
    let mut bad = ComponentLibrary::new();
    let definition = ComponentDefinition::new(sid(70), "Bad".into(), NodeRef::new(sid(71)));
    let mut tree = DefinitionTree::new(definition.reference());
    tree.insert(DefinitionNode {
        id: sid(71),
        kind: NodeKind::Text,
        parent: None,
        order_key: OrderKey::between(None, None),
        visible: true,
        locked: false,
        display_name: String::new(),
        payload: encode_payload(&NodePayload::Text(TextPayload::empty(0))),
    })
    .unwrap();
    if bad.insert(definition, tree).is_ok() {
        let project = TextProject {
            components: bad,
            ..project(golden_document(0))
        };
        assert!(encode(&project).is_err());
    }
    let mut sets = tokens();
    sets[0].tokens[0].value = TypedValue::Typography(
        stellar_core_types::typed_value::TypographyValue::from_record(
            &stellar_core_types::typography::Typography::default(),
        )
        .unwrap(),
    );
    let project = TextProject {
        token_sets: sets,
        ..project(golden_document(0))
    };
    assert!(encode(&project).is_err());
}

#[test]
fn disk_round_trip_writes_only_new_folders_and_rejects_symlinks() {
    let dir = TempDir::new("text-project-disk");
    let design = dir.path.join("design");
    let project = project(golden_document(2));
    let encoded = encode(&project).unwrap();
    write_new(&design, &encoded).unwrap();
    assert_eq!(read_dir(&design).unwrap(), encoded.files);
    let (opened, spans) = open_dir(&design).unwrap();
    assert_eq!(
        opened.semantic_key().unwrap(),
        project.semantic_key().unwrap()
    );
    assert_eq!(spans, encoded.spans);
    assert!(write_new(&design, &encoded).is_err());
    #[cfg(unix)]
    {
        let link = dir.path.join("linked");
        std::os::unix::fs::symlink(&design, &link).unwrap();
        assert!(read_dir(&link).is_err());
    }
}

fn git(repo: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap()
}

fn git_ok(repo: &Path, args: &[&str]) -> String {
    let out = git(repo, args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// 같은 Frame의 다른 속성(사이에 다른 줄이 있는 `background`와 `width`)을 두 clone에서 바꾸면 시스템 git merge가
/// 충돌 없이 끝나고, 합친 원본은 두 변경을 모두 담습니다.
#[test]
fn different_properties_of_one_frame_merge_cleanly_in_git() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let dir = TempDir::new("text-project-git");
    let base = dir.path.join("base");
    std::fs::create_dir_all(&base).unwrap();
    let original = project(golden_document(0));
    write_new(&base.join("design"), &encode(&original).unwrap()).unwrap();
    git_ok(&base, &["init", "-q", "-b", "main"]);
    git_ok(&base, &["add", "."]);
    git_ok(&base, &["commit", "-qm", "base"]);
    let records = original.document.to_records().unwrap();
    let root = records.definition.pages[0].views[0].root_frame;
    let edit = |name: &str, change: &dyn Fn(&mut FramePayload)| {
        let mut changed = original.document.to_records().unwrap();
        let node = changed.nodes.iter_mut().find(|n| n.id == root).unwrap();
        let NodePayload::Frame(frame) = &mut node.payload else {
            panic!("root Frame")
        };
        change(frame);
        let model = DocumentModel::from_records(changed).unwrap().0;
        let files = encode(&TextProject {
            document: model,
            ..project(golden_document(0))
        })
        .unwrap()
        .files;
        git_ok(&base, &["switch", "-qc", name, "main"]);
        for (path, text) in files {
            std::fs::write(base.join("design").join(path), text).unwrap();
        }
        git_ok(&base, &["commit", "-qam", name]);
    };
    edit("width", &|frame| {
        frame.size = stellar_core_types::value::Size2::new(777.0, frame.size.height()).unwrap()
    });
    edit("background", &|frame| {
        frame.background = Some(
            stellar_core_types::color::Color::new(
                stellar_core_types::color::ColorSpace::Srgb,
                0.0,
                1.0,
                0.0,
                1.0,
            )
            .unwrap(),
        )
    });
    git_ok(&base, &["switch", "-q", "width"]);
    let merged = git(&base, &["merge", "--no-edit", "background"]);
    assert!(
        merged.status.success(),
        "{}",
        String::from_utf8_lossy(&merged.stdout)
    );
    let (opened, _) = open_dir(&base.join("design")).unwrap();
    let records = opened.document.to_records().unwrap();
    let NodePayload::Frame(frame) = &records.nodes.iter().find(|n| n.id == root).unwrap().payload
    else {
        panic!("root Frame")
    };
    assert_eq!(frame.size.width(), 777.0);
    assert_eq!(frame.background.unwrap().g(), 1.0);
}

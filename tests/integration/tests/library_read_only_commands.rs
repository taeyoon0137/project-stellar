//! 사용 프로젝트에서 library resource 수정 command 거부 종단 시험 (XV.5.3, ADR-0119 결정 4).
//!
//! 실제 Git library release를 제품 경로로 lock·materialize하고 제품 열기(`open_project_libraries`)로 로드합니다. 사용
//! 프로젝트 문서에는 library Card의 instance가 있습니다. 실제 document command(`PropertyCommand`·
//! `PropertyOverrideCommand`)를 제품 사전 검사(`check_component_edits`)에 통과시켜, 정의 편집은 거부되고 instance 공개
//! override만 허용되는지 확인합니다. 허용된 override는 실제 command로 적용됩니다.

mod e2e_common;
mod library_common;

use e2e_common::{trusted_git, TempDir};
use library_common::*;
use stellar_core_types::reference::ComponentRef;
use stellar_dependencies::cache_entry::LibraryEntryStore;
use stellar_dependencies::graph::DependencyNodeKey;
use stellar_dependencies::library_snapshot::{LibraryResourceKind, SnapshotLimits};
use stellar_dependencies::read_only::{AllowedLibraryEdit, LibraryEditRejection};
use stellar_document::component::definition::{ComponentLibrary, ComponentPropertyId};
use stellar_document::component::expansion::ScenePayloads;
use stellar_document::component::instance::{ComponentInstancePayload, OverrideKey};
use stellar_document::component::override_commands::PropertyOverrideCommand;
use stellar_document::component::property::PropertyVisibility;
use stellar_document::component::property_commands::PropertyCommand;
use stellar_document::context::fixtures::{instance_of, SceneBuilder};
use stellar_document::expressions::types::{ExpressionType, TypeSpec};
use stellar_document::model::DocumentModel;
use stellar_document::record_codec::encode_payload;
use stellar_project::library_exports::read_library_resources;
use stellar_project::library_git::StorageSha256;
use stellar_project::library_guard::{check_component_edits, ComponentEdit, LibraryGuardRejection};
use stellar_project::library_open::open_project_libraries;

const SPACE: u64 = 4;
const CARD: u64 = 1;
const TITLE: u64 = 10;
const SECRET: u64 = 11;
const LOCAL: u64 = 50;
const INSTANCE: u64 = 100;
const LOCAL_INSTANCE: u64 = 101;

fn card(title_default: &str) -> ComponentLibrary {
    let mut library = ComponentLibrary::new();
    add_component(
        &mut library,
        sid(SPACE, CARD),
        sid(SPACE, 2),
        "Card",
        &[
            public_string(sid(SPACE, TITLE), "title", title_default),
            PropertySpec {
                id: sid(SPACE, SECRET),
                name: "secret",
                value_type: TypeSpec::required(ExpressionType::String),
                default: text_value("x"),
                visibility: PropertyVisibility::Private,
            },
        ],
    );
    library
}

fn scene(model: &DocumentModel) -> ScenePayloads {
    let mut payloads = ScenePayloads::new();
    for handle in model.columns().iter_live() {
        if let Some(id) = model.stable_id(handle) {
            if let Some(payload) = model.payload(id) {
                payloads.insert(id, encode_payload(payload));
            }
        }
    }
    payloads
}

fn prop(n: u64) -> ComponentPropertyId {
    ComponentPropertyId::new(sid(SPACE, n))
}

#[test]
fn editing_library_definition_is_rejected_and_public_override_succeeds() {
    let Some(gate) = trusted_git() else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };
    let dir = TempDir::new("library-read-only");
    let libraries = dir.path.join("libraries");
    let ds = library_repo(&gate, &libraries, "ds");
    let ds_manifest = manifest_text(library_id(41), "DS", &[]);
    let v1 = release(
        &gate,
        &ds,
        Release {
            manifest: &ds_manifest,
            document: None,
            components: card("제목"),
            token_sets: vec![],
        },
        "v1",
    );
    release(
        &gate,
        &ds,
        Release {
            manifest: &ds_manifest,
            document: None,
            components: card("새 제목"),
            token_sets: vec![],
        },
        "v2",
    );
    let manifest = manifest_text(
        library_id(96),
        "사용 프로젝트",
        &[("ds", library_id(41), &ds.url, "v1")],
    );
    let project = using_project(&dir.path, &manifest);
    let cache = cache_root("read-only");
    let mut store: Store = LibraryEntryStore::new(cache.clone(), StorageSha256, Sha256Keyer);
    let work = dir.path.join("neutral");
    std::fs::create_dir_all(&work).unwrap();
    let graph = resolve(&cache, &work, &declarations(&manifest));
    let (_, dirs) = materialize_and_lock(&mut store, &cache, &graph, &project);
    let opened = open_project_libraries(&project, &store, &SnapshotLimits::default(), 2);
    assert!(opened.is_complete());
    let libraries_instance = opened.instance();

    // 사용 프로젝트: 자기 component(Local) 하나와, library Card instance·자기 component instance가 있는 문서.
    let mut own = ComponentLibrary::new();
    add_component(
        &mut own,
        sid(SPACE, LOCAL),
        sid(SPACE, LOCAL + 1),
        "Local",
        &[public_string(sid(SPACE, LOCAL + 2), "caption", "c")],
    );
    let model = SceneBuilder::new(
        sid(SPACE, 900),
        sid(SPACE, 901),
        sid(SPACE, 902),
        sid(SPACE, 903),
    )
    .instance(
        sid(SPACE, INSTANCE),
        sid(SPACE, 903),
        instance_of(sid(SPACE, CARD), &[]),
    )
    .instance(
        sid(SPACE, LOCAL_INSTANCE),
        sid(SPACE, 903),
        instance_of(sid(SPACE, LOCAL), &[]),
    )
    .build();
    let columns = model.columns();
    let mut payloads = scene(&model);
    let card_ref = ComponentRef::new(sid(SPACE, CARD));
    let card_key = |id: u64| {
        stellar_dependencies::library_snapshot::LibraryResourceKey::new(
            DependencyNodeKey::new(library_id(41), v1.clone()),
            sid(SPACE, id),
        )
    };

    // library component 정의 편집 command는 모두 거부됩니다.
    let rename = PropertyCommand::Rename {
        component: card_ref,
        property: prop(TITLE),
        name: "heading".into(),
    };
    let delete = PropertyCommand::Delete {
        component: card_ref,
        property: prop(TITLE),
    };
    for command in [&rename, &delete] {
        let rejected = check_component_edits(
            &own,
            &libraries_instance,
            columns,
            &payloads,
            &[ComponentEdit::Property(command)],
        )
        .unwrap_err();
        assert!(
            matches!(
                rejected,
                (0, LibraryGuardRejection::Library(LibraryEditRejection::ReadOnlyResource { ref key, kind: LibraryResourceKind::Component }))
                    if *key == card_key(CARD)
            ),
            "{rejected:?}"
        );
    }

    // 공개 property override는 허용되고 실제 command로 적용됩니다.
    let set = PropertyOverrideCommand::Set {
        instance: sid(SPACE, INSTANCE),
        property: prop(TITLE),
        value: text_value("저장"),
    };
    let guarded = check_component_edits(
        &own,
        &libraries_instance,
        columns,
        &payloads,
        &[ComponentEdit::Override(&set)],
    )
    .expect("공개 override 허용");
    assert_eq!(
        guarded.allowed,
        [AllowedLibraryEdit::SetOverride {
            instance: sid(SPACE, INSTANCE),
            component: card_key(CARD),
            property: sid(SPACE, TITLE),
        }]
    );
    let node = DependencyNodeKey::new(library_id(41), v1.clone());
    let (definitions, _) = read_library_resources(&dirs[&node], library_id(41)).unwrap();
    let inverse = set
        .apply(&definitions, columns, &mut payloads)
        .expect("override 적용");
    let applied = ComponentInstancePayload::decode(&payloads[&sid(SPACE, INSTANCE)]).unwrap();
    assert_eq!(
        applied.property_override(&OverrideKey::direct(prop(TITLE))),
        Some(&text_value("저장"))
    );
    assert!(matches!(inverse, PropertyOverrideCommand::Reset { .. }));

    // 비공개 property·타입이 다른 값의 override는 거부됩니다.
    let private = PropertyOverrideCommand::Set {
        instance: sid(SPACE, INSTANCE),
        property: prop(SECRET),
        value: text_value("y"),
    };
    let retyped = PropertyOverrideCommand::Set {
        instance: sid(SPACE, INSTANCE),
        property: prop(TITLE),
        value: number_value(3.0),
    };
    let reject = |command: &PropertyOverrideCommand| {
        check_component_edits(
            &own,
            &libraries_instance,
            columns,
            &payloads,
            &[ComponentEdit::Override(command)],
        )
        .unwrap_err()
        .1
    };
    assert!(matches!(
        reject(&private),
        LibraryGuardRejection::Library(LibraryEditRejection::PrivateProperty { .. })
    ));
    assert!(matches!(
        reject(&retyped),
        LibraryGuardRejection::Library(LibraryEditRejection::OverrideTypeMismatch { .. })
    ));

    // override와 정의 편집이 섞인 transaction은 정의 편집 위치에서 전체 거부됩니다.
    let mixed = check_component_edits(
        &own,
        &libraries_instance,
        columns,
        &payloads,
        &[
            ComponentEdit::Override(&set),
            ComponentEdit::Property(&rename),
        ],
    );
    assert!(matches!(mixed, Err((1, LibraryGuardRejection::Library(_)))));

    // 사용 프로젝트 자기 component 편집과 그 instance override는 library 편집이 아닙니다(통과, command 자신이 검증).
    let local_rename = PropertyCommand::Rename {
        component: ComponentRef::new(sid(SPACE, LOCAL)),
        property: prop(LOCAL + 2),
        name: "label".into(),
    };
    let local_set = PropertyOverrideCommand::Set {
        instance: sid(SPACE, LOCAL_INSTANCE),
        property: prop(LOCAL + 2),
        value: text_value("d"),
    };
    let local = check_component_edits(
        &own,
        &libraries_instance,
        columns,
        &payloads,
        &[
            ComponentEdit::Property(&local_rename),
            ComponentEdit::Override(&local_set),
        ],
    )
    .expect("로컬 편집 통과");
    assert!(local.allowed.is_empty());

    // offline(library snapshot 없음): reset은 허용되고, 출처를 알 수 없는 override 설정은 library 편집으로 보지 않아
    // command 자신의 검증(정의 없음)에 맡깁니다.
    let offline_store: Store =
        LibraryEntryStore::new(cache_root("read-only-offline"), StorageSha256, Sha256Keyer);
    let offline = open_project_libraries(&project, &offline_store, &SnapshotLimits::default(), 3);
    assert!(!offline.is_complete());
    let reset = PropertyOverrideCommand::Reset {
        instance: sid(SPACE, INSTANCE),
        property: prop(TITLE),
    };
    let guarded = check_component_edits(
        &own,
        &offline.instance(),
        columns,
        &payloads,
        &[ComponentEdit::Override(&reset)],
    )
    .unwrap();
    assert!(guarded.allowed.is_empty());

    // 같은 library를 두 이름으로 선언해 두 버전이 같은 Card를 export하면 대상을 정할 수 없어 거부합니다.
    let manifest2 = manifest_text(
        library_id(96),
        "사용 프로젝트",
        &[
            ("ds", library_id(41), &ds.url, "v1"),
            ("ds-next", library_id(41), &ds.url, "v2"),
        ],
    );
    let project2 = dir.path.join("app-two");
    std::fs::create_dir_all(&project2).unwrap();
    std::fs::write(project2.join("stellar.yaml"), &manifest2).unwrap();
    let graph2 = resolve(&cache, &work, &declarations(&manifest2));
    materialize_and_lock(&mut store, &cache, &graph2, &project2);
    let two = open_project_libraries(&project2, &store, &SnapshotLimits::default(), 4);
    assert!(two.is_complete());
    let ambiguous = check_component_edits(
        &own,
        &two.instance(),
        columns,
        &payloads,
        &[ComponentEdit::Override(&set)],
    )
    .unwrap_err();
    match ambiguous {
        (0, LibraryGuardRejection::AmbiguousComponent { dependencies, .. }) => {
            let names: Vec<_> = dependencies
                .iter()
                .map(|n| n.as_str().to_string())
                .collect();
            assert_eq!(names, ["ds", "ds-next"]);
        }
        other => panic!("{other:?}"),
    }
}

//! 제품 library cache 경로 fixture (XV.5.5·XV.5.3, ADR-0127).
//!
//! 실제 Git library release 둘(ds: Card, chips: Chip)을 만들고, 사용 프로젝트가 둘을 lock합니다. 이 기기의 제품 OS 기본
//! library cache(시험 HOME 아래 `<사용자 데이터>/xyz.taeyoon.stellar/library-cache`)에는 제품 keyer
//! (`stellar_ops::library::ProductKeyer`)로 ds만 materialize합니다(chips는 받은 적 없는 offline 기기). 사용 프로젝트 문서는
//! 텍스트 원본이고 ds Card의 instance 하나가 있습니다. fixture를 만든 뒤에는 library 저장소를 지워 Git·network를 쓰지 않습니다.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use stellar_core_types::ids::{StableId, WindowSessionId};
use stellar_core_types::reference::ComponentRef;
use stellar_core_types::typed_value::TypedValue;
use stellar_dependencies::cache_entry::LibraryEntryStore;
use stellar_dependencies::declaration::RepositoryUrl;
use stellar_dependencies::mirror::MirrorCache;
use stellar_document::component::definition::{ComponentLibrary, ComponentPropertyId};
use stellar_document::component::instance::{ComponentInstancePayload, OverrideKey};
use stellar_document::component::property::PropertyVisibility;
use stellar_document::context::fixtures::SceneBuilder;
use stellar_document::expressions::types::{ExpressionType, TypeSpec};
use stellar_document::model::DocumentModel;
use stellar_document::records::NodePayload;
use stellar_ops::library::{LibraryCache, ProductKeyer};
use stellar_ops::lock::APP_DATA_DIR;
use stellar_ops::snapshot::open_snapshot;
use stellar_project::document_io::source_v4::project::{encode, write_new, TextProject};
use stellar_project::identity::ProjectIdentity;
use stellar_project::library_git::StorageSha256;
use stellar_runtime::window_session::ProjectWindowSession;

use super::*;
use crate::e2e_common::{trusted_git, TempDir};

/// fixture StableId 공간.
pub const SPACE: u64 = 6;
/// ds Card component.
pub const CARD: u64 = 1;
/// Card 공개 String property `title`.
pub const TITLE: u64 = 10;
/// Card 비공개 String property `secret`.
pub const SECRET: u64 = 11;
/// chips Chip component.
pub const CHIP: u64 = 20;
/// 사용 문서의 Card instance.
pub const INSTANCE: u64 = 100;
/// ds library id 번호.
pub const DS_ID: u64 = 61;
/// chips library id 번호.
pub const CHIPS_ID: u64 = 62;

fn card() -> ComponentLibrary {
    let mut library = ComponentLibrary::new();
    add_component(
        &mut library,
        sid(SPACE, CARD),
        sid(SPACE, 2),
        "Card",
        &[
            public_string(sid(SPACE, TITLE), "title", "제목"),
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

fn chips() -> ComponentLibrary {
    let mut library = ComponentLibrary::new();
    add_component(
        &mut library,
        sid(SPACE, CHIP),
        sid(SPACE, CHIP + 1),
        "Chip",
        &[public_string(sid(SPACE, CHIP + 2), "label", "c")],
    );
    library
}

/// Card property id.
pub fn prop(n: u64) -> ComponentPropertyId {
    ComponentPropertyId::new(sid(SPACE, n))
}

/// override 없는 Card instance payload.
pub fn card_payload() -> ComponentInstancePayload {
    ComponentInstancePayload::new(ComponentRef::new(sid(SPACE, CARD)))
}

/// Card instance payload에 직접 override 하나.
pub fn with_override(property: u64, value: TypedValue) -> ComponentInstancePayload {
    let mut payload = card_payload();
    payload.set_property_override(OverrideKey::direct(prop(property)), value);
    payload
}

fn using_document() -> DocumentModel {
    SceneBuilder::new(
        sid(SPACE, 900),
        sid(SPACE, 901),
        sid(SPACE, 902),
        sid(SPACE, 903),
    )
    .instance(sid(SPACE, INSTANCE), sid(SPACE, 903), card_payload())
    .build()
}

/// 제품 OS 기본 cache의 app data 위치(시험 HOME 아래).
pub fn product_app_data() -> PathBuf {
    stellar_project::trust_store::os_user_data_base()
        .expect("시험 HOME")
        .join(APP_DATA_DIR)
}

/// 저장된 사용 문서의 Card instance override(property, 값 Debug).
pub fn instance_overrides(root: &Path, document: StableId) -> Vec<(StableId, String)> {
    let model = open_snapshot(root)
        .unwrap()
        .document_model(document)
        .unwrap();
    match model.payload(sid(SPACE, INSTANCE)) {
        Some(NodePayload::ComponentInstance(payload)) => payload
            .property_overrides()
            .map(|(key, value)| (key.property().stable_id(), format!("{value:?}")))
            .collect(),
        other => panic!("instance가 아님: {other:?}"),
    }
}

/// 사용 프로젝트를 제품 창 session 열기 경로로 엽니다.
pub fn open_window(project: &Path) -> ProjectWindowSession {
    let snapshot = open_snapshot(project).unwrap();
    let identity = ProjectIdentity::new(snapshot.project_id(), project).unwrap();
    ProjectWindowSession::load(identity, WindowSessionId::from_stable_id(sid(SPACE, 999))).unwrap()
}

/// fixture를 만듭니다(모듈 문서). 시스템 Git이 없으면 `None`.
pub fn product_project(dir: &TempDir) -> Option<PathBuf> {
    let gate = trusted_git()?;
    let libraries = dir.path.join("libraries");
    let ds = library_repo(&gate, &libraries, "ds");
    let ds_v1 = release(
        &gate,
        &ds,
        Release {
            manifest: &manifest_text(library_id(DS_ID), "DS", &[]),
            document: None,
            components: card(),
            token_sets: vec![],
        },
        "v1",
    );
    let chips_repo = library_repo(&gate, &libraries, "chips");
    release(
        &gate,
        &chips_repo,
        Release {
            manifest: &manifest_text(library_id(CHIPS_ID), "Chips", &[]),
            document: None,
            components: chips(),
            token_sets: vec![],
        },
        "v1",
    );
    let manifest = manifest_text(
        library_id(93),
        "사용 프로젝트",
        &[
            ("ds", library_id(DS_ID), &ds.url, "v1"),
            ("chips", library_id(CHIPS_ID), &chips_repo.url, "v1"),
        ],
    );
    let project = using_project(&dir.path, &manifest);
    std::fs::create_dir_all(project.join(".git")).unwrap();
    let encoded = encode(&TextProject {
        document: using_document(),
        components: ComponentLibrary::new(),
        token_sets: vec![],
        images: BTreeMap::new(),
    })
    .unwrap();
    write_new(&project.join("design"), &encoded).unwrap();

    // 온라인 기기(시험 cache)에서 두 library를 해석·materialize하고 lock을 씁니다.
    let online = cache_root("product-paths-online");
    let mut online_store: Store =
        LibraryEntryStore::new(online.clone(), StorageSha256, Sha256Keyer);
    let work = dir.path.join("neutral");
    std::fs::create_dir_all(&work).unwrap();
    let graph = resolve(&online, &work, &declarations(&manifest));
    materialize_and_lock(&mut online_store, &online, &graph, &project);

    // 이 기기의 제품 기본 cache: 제품 keyer로 ds만 받습니다(chips는 받은 적 없음).
    let mut cache = LibraryCache::create_in_app_data(&product_app_data()).unwrap();
    let keyer = ProductKeyer::load_or_create(cache.root()).unwrap();
    let mut mirrors = MirrorCache::new(cache.root().clone(), commands(), keyer);
    cache
        .store_mut()
        .materialize(
            &mut commands(),
            &mut mirrors,
            &RepositoryUrl::new(&ds.url).unwrap(),
            &ds_v1,
            1,
        )
        .unwrap();
    std::fs::remove_dir_all(&libraries).unwrap();
    Some(project)
}

//! offline·missing cache 부분 열기 종단 시험 (XV.5.5, ADR-0119 결정 6).
//!
//! 사용 프로젝트가 library 두 개(a, b)를 lock합니다. 열 때 공용 cache에는 a만 있습니다(offline에서 b를 아직 받지 않은
//! 기기). 제품 열기(`open_project_libraries`)는 Git·network 없이 a를 로드하고 b를 library resource unavailable 진단으로
//! 남깁니다. 프로젝트 기본 데이터(텍스트 원본 문서)는 제품 원본 reader로 그대로 열립니다. lock이 손상돼도 프로젝트
//! 문서는 열리고 lock 문제만 진단으로 남습니다.

mod e2e_common;
mod library_common;

use std::collections::BTreeMap;

use e2e_common::{trusted_git, TempDir};
use library_common::*;
use stellar_core_types::reference::ComponentRef;
use stellar_dependencies::cache_entry::LibraryEntryStore;
use stellar_dependencies::declaration::{LogicalName, RepositoryUrl};
use stellar_dependencies::external_ref::ExternalComponentRef;
use stellar_dependencies::graph::DependencyNodeKey;
use stellar_dependencies::library_load::{ResourceResolution, UnavailableReason};
use stellar_dependencies::library_snapshot::SnapshotLimits;
use stellar_dependencies::mirror::MirrorCache;
use stellar_dependencies::ref_resolution::RefScope;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_project::document_io::session::{decode_source, read_source};
use stellar_project::document_io::source_v4::project::{encode, write_new, TextProject};
use stellar_project::library_git::StorageSha256;
use stellar_project::library_open::{open_project_libraries, LibraryOpenProblem};

const SPACE: u64 = 3;

fn component(n: u64, name: &str) -> ComponentLibrary {
    let mut library = ComponentLibrary::new();
    add_component(
        &mut library,
        sid(SPACE, n),
        sid(SPACE, n + 1),
        name,
        &[public_string(sid(SPACE, n + 2), "label", name)],
    );
    library
}

#[test]
fn offline_missing_library_opens_project_partially_with_unavailable_diagnostic() {
    let Some(gate) = trusted_git() else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };
    let dir = TempDir::new("library-partial-open");
    let libraries = dir.path.join("libraries");
    let a = library_repo(&gate, &libraries, "a");
    let a_v1 = release(
        &gate,
        &a,
        Release {
            manifest: &manifest_text(library_id(21), "A", &[]),
            document: None,
            components: component(10, "Badge"),
            token_sets: vec![],
        },
        "v1",
    );
    let b = library_repo(&gate, &libraries, "b");
    let b_v1 = release(
        &gate,
        &b,
        Release {
            manifest: &manifest_text(library_id(22), "B", &[]),
            document: None,
            components: component(20, "Chip"),
            token_sets: vec![],
        },
        "v1",
    );
    let manifest = manifest_text(
        library_id(98),
        "사용 프로젝트",
        &[
            ("a", library_id(21), &a.url, "v1"),
            ("b", library_id(22), &b.url, "v1"),
        ],
    );
    let project = using_project(&dir.path, &manifest);
    // 프로젝트 기본 데이터: 자기 텍스트 원본 문서.
    let own = model_snapshot_json(&empty_document(SPACE)).unwrap();
    let encoded = encode(&TextProject {
        document: empty_document(SPACE),
        components: ComponentLibrary::new(),
        token_sets: vec![],
        images: BTreeMap::new(),
    })
    .unwrap();
    write_new(&project.join("design"), &encoded).unwrap();

    // 온라인 기기에서 해석·materialize하고 lock을 씁니다.
    let online = cache_root("partial-online");
    let mut online_store: Store =
        LibraryEntryStore::new(online.clone(), StorageSha256, Sha256Keyer);
    let work = dir.path.join("neutral");
    std::fs::create_dir_all(&work).unwrap();
    let graph = resolve(&online, &work, &declarations(&manifest));
    materialize_and_lock(&mut online_store, &online, &graph, &project);

    // offline 기기: 공용 cache에 a만 있습니다(b는 받은 적 없음).
    let offline = cache_root("partial-offline");
    let mut offline_store: Store =
        LibraryEntryStore::new(offline.clone(), StorageSha256, Sha256Keyer);
    let mut mirrors = MirrorCache::new(offline.clone(), commands(), Sha256Keyer);
    offline_store
        .materialize(
            &mut commands(),
            &mut mirrors,
            &RepositoryUrl::new(&a.url).unwrap(),
            &a_v1,
            1,
        )
        .unwrap();
    // 이 뒤로는 Git을 쓰지 않습니다: 저장소를 지워 network·Git 접근이 있으면 실패하게 합니다.
    std::fs::remove_dir_all(&libraries).unwrap();

    let opened = open_project_libraries(&project, &offline_store, &SnapshotLimits::default(), 2);
    assert!(opened.problem().is_none());
    assert!(!opened.is_complete());
    let loaded = opened.loaded().expect("lock 정상");
    let b_node = DependencyNodeKey::new(library_id(22), b_v1.clone());
    let diagnostics: Vec<_> = loaded.unavailable().collect();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].node, b_node);
    assert_eq!(diagnostics[0].reason, UnavailableReason::MissingCacheEntry);
    let text = diagnostics[0].to_string();
    assert!(text.contains("offline"), "{text}");
    assert!(!text.contains(&b.url), "진단에 URL 원문 없음");

    // a의 resource는 해석되고 b의 resource는 library resource unavailable입니다.
    let badge = ExternalComponentRef::new(
        LogicalName::new("a").unwrap(),
        library_id(21),
        ComponentRef::new(sid(SPACE, 10)),
    );
    assert!(matches!(
        loaded.resolve(&RefScope::Project, &badge.into()),
        ResourceResolution::Resolved(_)
    ));
    let chip = ExternalComponentRef::new(
        LogicalName::new("b").unwrap(),
        library_id(22),
        ComponentRef::new(sid(SPACE, 20)),
    );
    match loaded.resolve(&RefScope::Project, &chip.clone().into()) {
        ResourceResolution::Unavailable(missing) => {
            assert_eq!(missing.library.node, b_node);
            assert_eq!(missing.reference, chip.into());
        }
        other => panic!("b component는 unavailable이어야 함: {other:?}"),
    }

    // 프로젝트 기본 데이터는 library와 무관하게 열립니다.
    let source = read_source(&project).unwrap().expect("텍스트 원본");
    let decoded = decode_source(&source).unwrap();
    assert_eq!(model_snapshot_json(&decoded.document).unwrap(), own);

    // lock 손상: library는 하나도 로드하지 않고 lock 문제만 남깁니다. 문서는 여전히 열립니다.
    std::fs::write(project.join("stellar.lock.yaml"), "schema_version: [\n").unwrap();
    let broken = open_project_libraries(&project, &offline_store, &SnapshotLimits::default(), 3);
    assert_eq!(broken.problem(), Some(&LibraryOpenProblem::LockInvalid));
    assert!(broken.loaded().is_none());
    assert!(read_source(&project).unwrap().is_some());

    // lock 없음: 의존성이 없는 프로젝트처럼 엽니다.
    std::fs::remove_file(project.join("stellar.lock.yaml")).unwrap();
    let none = open_project_libraries(&project, &offline_store, &SnapshotLimits::default(), 4);
    assert!(none.is_complete());
    assert_eq!(none.instance().snapshots().count(), 0);
}

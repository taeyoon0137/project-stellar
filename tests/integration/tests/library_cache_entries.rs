//! library cache entry 검증 종단 시험 (XV.3.4, ADR-0119).
//!
//! 실제 Git 저장소의 library release를 제품 mirror·cache entry로 materialize하고, 제품 검증기
//! (`ProjectLibraryValidator`: cache entry probe + storage validator)와 제품 export 공급자(`SourceExports`)를 거치는
//! 제품 cache 읽기 경로(`load_cached_snapshot`)로 판정합니다.

mod e2e_common;
mod library_common;

use e2e_common::{git, trusted_git, TempDir};
use library_common::*;
use stellar_core_types::reference::ComponentRef;
use stellar_dependencies::cache_entry::LibraryEntryStore;
use stellar_dependencies::declaration::CommitId;
use stellar_dependencies::entry_validation::{
    validate_library_entry, LibraryEntryRejection, StorageValidationFailure,
};
use stellar_dependencies::graph::DependencyNodeKey;
use stellar_dependencies::library_snapshot::{
    load_cached_snapshot, ExportVisibility, SnapshotLimits, SnapshotLoadError, SnapshotRequest,
};
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::component::property::PropertyVisibility;
use stellar_document::expressions::types::{ExpressionType, TypeSpec};
use stellar_project::library_entry::ProjectLibraryValidator;
use stellar_project::library_exports::SourceExports;
use stellar_project::library_git::StorageSha256;

const SPACE: u64 = 1;

fn card_library() -> ComponentLibrary {
    let mut library = ComponentLibrary::new();
    add_component(
        &mut library,
        sid(SPACE, 1),
        sid(SPACE, 2),
        "Card",
        &[
            public_string(sid(SPACE, 10), "title", "제목"),
            PropertySpec {
                id: sid(SPACE, 11),
                name: "tone",
                value_type: TypeSpec::optional(ExpressionType::Number),
                default: number_value(1.0),
                visibility: PropertyVisibility::Private,
            },
        ],
    );
    library
}

#[test]
fn cache_entries_are_validated_by_probe_and_storage_before_exports_are_read() {
    let Some(gate) = trusted_git() else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };
    let dir = TempDir::new("library-entries");
    let libraries = dir.path.join("libraries");
    let cache = cache_root("entries");
    let mut store: Store = LibraryEntryStore::new(cache.clone(), StorageSha256, Sha256Keyer);
    let mut mirrors =
        stellar_dependencies::mirror::MirrorCache::new(cache.clone(), commands(), Sha256Keyer);
    let mut runner = commands();

    // 정상 library: 텍스트 원본 Card component.
    let good = library_repo(&gate, &libraries, "good");
    let good_commit = release(
        &gate,
        &good,
        Release {
            manifest: &manifest_text(library_id(1), "Good", &[]),
            document: None,
            components: card_library(),
            token_sets: vec![token_set(SPACE, "ink", number_value(3.0))],
        },
        "v1",
    );
    // 다른 library id를 가진 저장소.
    let other = library_repo(&gate, &libraries, "other");
    let other_commit = release(
        &gate,
        &other,
        Release {
            manifest: &manifest_text(library_id(3), "Other", &[]),
            document: None,
            components: ComponentLibrary::new(),
            token_sets: vec![],
        },
        "v1",
    );
    // 매니페스트는 정상이지만 텍스트 원본이 손상된 library.
    let broken = library_repo(&gate, &libraries, "broken");
    std::fs::write(
        broken.dir.join("stellar.yaml"),
        manifest_text(library_id(4), "Broken", &[]),
    )
    .unwrap();
    std::fs::create_dir_all(broken.dir.join("design")).unwrap();
    std::fs::write(
        broken.dir.join("design/document.stellar.yaml"),
        "format: stellar-document\nschema_version: \"999\"\n",
    )
    .unwrap();
    git(&gate, &broken.dir, &["add", "-A"]);
    git(&gate, &broken.dir, &["commit", "-q", "-m", "broken"]);
    let broken_commit: CommitId = git(&gate, &broken.dir, &["rev-parse", "HEAD"])
        .trim()
        .parse()
        .unwrap();
    // 일반 Git 저장소(stellar.yaml 없음).
    let plain = library_repo(&gate, &libraries, "plain");
    std::fs::write(plain.dir.join("README.md"), "plain\n").unwrap();
    git(&gate, &plain.dir, &["add", "-A"]);
    git(&gate, &plain.dir, &["commit", "-q", "-m", "plain"]);
    let plain_commit: CommitId = git(&gate, &plain.dir, &["rev-parse", "HEAD"])
        .trim()
        .parse()
        .unwrap();

    let url = |repo: &LibraryRepo| {
        stellar_dependencies::declaration::RepositoryUrl::new(&repo.url).unwrap()
    };
    let mut materialize = |store: &mut Store, repo: &LibraryRepo, commit: &CommitId| {
        store.materialize(&mut runner, &mut mirrors, &url(repo), commit, 1)
    };

    // 일반 Git 저장소는 materialize 단계에서도, 작업 폴더 probe에서도 Stellar 프로젝트가 아닙니다.
    assert!(materialize(&mut store, &plain, &plain_commit).is_err());
    for mut validator in [
        ProjectLibraryValidator::cache_entry(),
        ProjectLibraryValidator::checkout(),
    ] {
        assert_eq!(
            validate_library_entry(&plain.dir, library_id(5), &mut validator),
            Err(LibraryEntryRejection::NotAStellarProject)
        );
    }

    let good_entry = materialize(&mut store, &good, &good_commit).unwrap();
    let other_entry = materialize(&mut store, &other, &other_commit).unwrap();
    let broken_entry = materialize(&mut store, &broken, &broken_commit).unwrap();
    // cache entry에는 `.git`이 없습니다(ProjectProbe의 `.git` 요구를 뺀 cache entry probe가 필요한 이유).
    assert!(!good_entry.dir.join(".git").exists());

    let mut validator = ProjectLibraryValidator::cache_entry();
    let validated = validate_library_entry(&good_entry.dir, library_id(1), &mut validator).unwrap();
    assert_eq!(validated.schema_version(), 4);
    assert_eq!(
        validate_library_entry(&other_entry.dir, library_id(2), &mut validator),
        Err(LibraryEntryRejection::LibraryIdMismatch {
            expected: library_id(2),
            found: library_id(3),
        })
    );
    assert_eq!(
        validate_library_entry(&broken_entry.dir, library_id(4), &mut validator),
        Err(LibraryEntryRejection::CorruptStorage(
            StorageValidationFailure::TextSource
        ))
    );

    // 제품 cache 읽기 경로: 검증을 통과한 entry에서만 텍스트 원본 export를 읽습니다.
    let limits = SnapshotLimits::default();
    let good_node = DependencyNodeKey::new(library_id(1), good_commit.clone());
    let snapshot = load_cached_snapshot(
        &store,
        SnapshotRequest {
            repository: &url(&good),
            node: &good_node,
            expected_content_hash: Some(&good_entry.content_hash),
        },
        &mut validator,
        &mut SourceExports,
        &limits,
        2,
    )
    .expect("정상 entry snapshot");
    let card = snapshot
        .component(ComponentRef::new(sid(SPACE, 1)))
        .expect("Card export");
    assert_eq!(card.export().name, "Card");
    let title = card.export().property(sid(SPACE, 10)).unwrap();
    assert_eq!(
        (
            title.name.as_str(),
            title.type_signature.as_str(),
            title.visibility
        ),
        ("title", "String", ExportVisibility::Public)
    );
    let tone = card.export().property(sid(SPACE, 11)).unwrap();
    assert_eq!(
        (tone.type_signature.as_str(), tone.visibility),
        ("Optional<Number>", ExportVisibility::Private)
    );
    assert_eq!(snapshot.tokens().count(), 1);

    let broken_node = DependencyNodeKey::new(library_id(4), broken_commit.clone());
    assert_eq!(
        load_cached_snapshot(
            &store,
            SnapshotRequest {
                repository: &url(&broken),
                node: &broken_node,
                expected_content_hash: Some(&broken_entry.content_hash),
            },
            &mut validator,
            &mut SourceExports,
            &limits,
            3,
        ),
        Err(SnapshotLoadError::Rejected(
            LibraryEntryRejection::CorruptStorage(StorageValidationFailure::TextSource)
        ))
    );
}

/// generation component library와 텍스트 컴포넌트 원본이 함께 정의를 담으면 storage validator와 export 공급자가 같은
/// reader(`read_library_root`)로 손상 판정합니다.
#[test]
fn generation_and_text_component_definitions_together_are_corrupt() {
    let Some(gate) = trusted_git() else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };
    let dir = TempDir::new("library-entries-conflict");
    let repo = library_repo(&gate, &dir.path.join("libraries"), "both");
    let id = library_id(6);
    std::fs::write(
        repo.dir.join("stellar.yaml"),
        manifest_text(id, "Both", &[]),
    )
    .unwrap();
    // 텍스트 원본에 property 없는 컴포넌트 하나(텍스트로 쓸 수 있는 정의).
    let mut text_components = ComponentLibrary::new();
    stellar_document::context::fixtures::ComponentBuilder::new(
        sid(SPACE, 50),
        "Badge",
        sid(SPACE, 51),
    )
    .register(&mut text_components);
    let project = stellar_project::document_io::source_v4::project::TextProject {
        document: empty_document(SPACE),
        components: text_components,
        token_sets: vec![],
        images: Default::default(),
    };
    let encoded =
        stellar_project::document_io::source_v4::project::encode(&project).expect("encode");
    stellar_project::document_io::source_v4::project::write_new(&repo.dir.join("design"), &encoded)
        .expect("텍스트 원본 쓰기");
    let mut validator = ProjectLibraryValidator::cache_entry();
    assert_eq!(
        validate_library_entry(&repo.dir, id, &mut validator).map(|_| ()),
        Ok(()),
        "텍스트 원본만 있으면 정상"
    );
    // 같은 root에 generation component library도 씁니다.
    write_component_generation(&repo.dir, id, &card_library());
    assert_eq!(
        validate_library_entry(&repo.dir, id, &mut validator).map(|_| ()),
        Err(LibraryEntryRejection::CorruptStorage(
            StorageValidationFailure::TextSource
        ))
    );
    assert_eq!(
        stellar_project::library_exports::read_library_resources(&repo.dir, id).map(|_| ()),
        Err(stellar_dependencies::library_snapshot::ExportReadFailure::Corrupt)
    );
}

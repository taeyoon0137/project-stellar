//! lock 기록과 cache 손상 판정 종단 시험 (XV.4.2, XV.4.4, ADR-0119).
//!
//! 제품 resolver·materialize로 만든 lock을 제품 lock 기록(`write_resolved_lockfile`)·제품 lock parser(`read_lock`)로
//! 확인하고, cache entry bytes를 변조한 뒤 제품 무결성 검사(`verify_lock_against_cache`)와 제품 열기
//! (`open_project_libraries`)가 손상으로 판정하며 같은 library의 다른 commit을 고르지 않는지 봅니다.

mod e2e_common;
mod library_common;

use e2e_common::{trusted_git, TempDir};
use library_common::*;
use stellar_dependencies::cache_entry::LibraryEntryStore;
use stellar_dependencies::graph::DependencyNodeKey;
use stellar_dependencies::library_load::UnavailableReason;
use stellar_dependencies::library_snapshot::SnapshotLimits;
use stellar_dependencies::lock_integrity::{verify_lock_against_cache, LockIntegrityError};
use stellar_document::component::definition::ComponentLibrary;
use stellar_project::library_git::StorageSha256;
use stellar_project::library_open::{open_project_libraries, read_lock, LibraryOpenProblem};

const SPACE: u64 = 2;

fn card_library() -> ComponentLibrary {
    let mut library = ComponentLibrary::new();
    add_component(
        &mut library,
        sid(SPACE, 1),
        sid(SPACE, 2),
        "Card",
        &[public_string(sid(SPACE, 10), "title", "제목")],
    );
    library
}

#[test]
fn lock_records_entry_hash_and_tampered_cache_is_corruption_without_other_commit() {
    let Some(gate) = trusted_git() else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };
    let dir = TempDir::new("library-lock-integrity");
    let libraries = dir.path.join("libraries");
    let cache = cache_root("lock-integrity");
    let mut store: Store = LibraryEntryStore::new(cache.clone(), StorageSha256, Sha256Keyer);

    // C v1·v2, A v1은 C v1에 의존(전이 의존성).
    let c = library_repo(&gate, &libraries, "c");
    let c_manifest = manifest_text(library_id(13), "C", &[]);
    let c_v1 = release(
        &gate,
        &c,
        Release {
            manifest: &c_manifest,
            document: None,
            components: card_library(),
            token_sets: vec![],
        },
        "v1",
    );
    let c_v2 = release(
        &gate,
        &c,
        Release {
            manifest: &c_manifest,
            document: None,
            components: ComponentLibrary::new(),
            token_sets: vec![],
        },
        "v2",
    );
    let a = library_repo(&gate, &libraries, "a");
    let a_v1 = release(
        &gate,
        &a,
        Release {
            manifest: &manifest_text(library_id(11), "A", &[("c", library_id(13), &c.url, "v1")]),
            document: None,
            components: ComponentLibrary::new(),
            token_sets: vec![],
        },
        "v1",
    );
    let project_manifest = manifest_text(
        library_id(99),
        "사용 프로젝트",
        &[("a", library_id(11), &a.url, "v1")],
    );
    let project = using_project(&dir.path, &project_manifest);
    let work = dir.path.join("neutral");
    std::fs::create_dir_all(&work).unwrap();
    let graph = resolve(&cache, &work, &declarations(&project_manifest));
    let (lock, dirs) = materialize_and_lock(&mut store, &cache, &graph, &project);

    // XV.4.2: lock entry는 exact commit·cache entry content hash·library schema version·전이 의존성을 기록합니다.
    let reread = read_lock(&project).unwrap().expect("lock 파일");
    assert_eq!(reread, lock, "제품 lock parser round-trip");
    let a_node = DependencyNodeKey::new(library_id(11), a_v1.clone());
    let c_node = DependencyNodeKey::new(library_id(13), c_v1.clone());
    let a_entry = lock.entries().iter().find(|e| e.key() == a_node).unwrap();
    assert_eq!(a_entry.exact_commit, a_v1);
    assert_eq!(a_entry.schema_version, 4);
    assert_eq!(a_entry.dependencies, vec![c_node.clone()]);
    let recomputed =
        stellar_dependencies::cache_entry::recompute_entry_hash(&dirs[&a_node], &StorageSha256)
            .unwrap()
            .0;
    assert_eq!(
        a_entry.content_hash, recomputed,
        "content hash = entry bytes SHA-256"
    );
    // 필드가 빠진 lock은 제품 lock parser가 거부합니다.
    let text = std::fs::read_to_string(project.join("stellar.lock.yaml")).unwrap();
    let missing: String = text
        .lines()
        .filter(|l| !l.trim_start().starts_with("content_hash:"))
        .map(|l| format!("{l}\n"))
        .collect();
    let probe_dir = dir.path.join("missing-field");
    std::fs::create_dir_all(&probe_dir).unwrap();
    std::fs::write(probe_dir.join("stellar.lock.yaml"), missing).unwrap();
    assert_eq!(read_lock(&probe_dir), Err(LibraryOpenProblem::LockInvalid));

    // 정상 cache: 모든 library를 엽니다.
    let limits = SnapshotLimits::default();
    let opened = open_project_libraries(&project, &store, &limits, 5);
    assert!(
        opened.is_complete(),
        "{:?}",
        opened.loaded().map(|l| l.unavailable().collect::<Vec<_>>())
    );

    // C v2도 cache에 둡니다(같은 library의 다른 commit). 변조된 v1 대신 고르면 안 됩니다.
    let mut mirrors =
        stellar_dependencies::mirror::MirrorCache::new(cache.clone(), commands(), Sha256Keyer);
    let c_url = stellar_dependencies::declaration::RepositoryUrl::new(&c.url).unwrap();
    store
        .materialize(&mut commands(), &mut mirrors, &c_url, &c_v2, 6)
        .expect("v2 materialize");

    // XV.4.4: C v1 entry 파일을 변조합니다.
    let c_dir = &dirs[&c_node];
    let victim = walk_files(c_dir)
        .into_iter()
        .find(|p| p.extension().is_some_and(|e| e == "yaml") && p.starts_with(c_dir.join("design")))
        .expect("design 원본 파일");
    make_writable(&victim);
    let mut bytes = std::fs::read(&victim).unwrap();
    bytes.extend_from_slice(b"# tampered\n");
    std::fs::write(&victim, bytes).unwrap();

    let integrity = verify_lock_against_cache(&lock, &mut store);
    assert!(
        matches!(integrity, Err(LockIntegrityError::Corrupted { ref node, .. }) if *node == c_node),
        "{integrity:?}"
    );
    let opened = open_project_libraries(&project, &store, &limits, 7);
    let loaded = opened.loaded().expect("lock은 정상");
    let reasons: Vec<_> = loaded
        .unavailable()
        .map(|d| (d.node.clone(), d.reason.clone()))
        .collect();
    assert_eq!(
        reasons,
        [(c_node.clone(), UnavailableReason::ContentHashMismatch)]
    );
    // 다른 commit(C v2)을 자동으로 고르지 않습니다.
    let c_v2_node = DependencyNodeKey::new(library_id(13), c_v2.clone());
    assert!(loaded.instance().snapshot(&c_v2_node).is_none());
    assert!(loaded.instance().snapshot(&c_node).is_none());
    // A는 그대로 열립니다.
    assert!(loaded.instance().snapshot(&a_node).is_some());
}

fn walk_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for item in std::fs::read_dir(&d).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

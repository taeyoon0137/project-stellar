//! 같은 원본 digest의 snapshot cache 재사용 시험과 2만 node 측정(ADR-0138 결정 1).
//!
//! 측정은 기본 시험에서 빼고(`#[ignore]`) release로 실행합니다.
//!
//! ```text
//! cargo test -p stellar-ops --release --lib git_cache::snapshot_tests::measure -- --ignored --nocapture
//! ```

use std::path::Path;
use std::time::{Duration, SystemTime};

use stellar_project::cache::{
    prepare_snapshot_with, Rebuild, SnapshotCache, SnapshotKey, SnapshotStore, MAX_ENTRIES,
};
use stellar_project::document_io::session::{observe, OpenPlan};
use stellar_project::saved_project::SavedProjectLoader;

use crate::candidate::saved_semantic_snapshot;
use crate::test_support::{
    large_document, project_id, save_text_document, scaffold_project, text_document, TempDir,
};

/// Git tree를 materialize한 것처럼 `stellar.yaml`과 텍스트 원본만 `to`에 복사합니다(cache 없음).
fn materialize(from: &Path, to: &Path) {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), &target).unwrap();
            }
        }
    }
    std::fs::create_dir_all(to).unwrap();
    std::fs::copy(from.join("stellar.yaml"), to.join("stellar.yaml")).unwrap();
    copy(&from.join("design"), &to.join("design"));
}

fn text_project(dir: &TempDir, model: &crate::test_support::FixtureDocument) -> std::path::PathBuf {
    let root = dir.path.join("project");
    scaffold_project(&root);
    save_text_document(&root, model);
    root
}

fn semantic(root: &Path) -> stellar_semantic_diff::snapshot::SemanticSnapshot {
    let project = SavedProjectLoader::for_materialized_snapshot()
        .open(root)
        .unwrap();
    saved_semantic_snapshot(&project).unwrap()
}

#[test]
fn a_second_snapshot_of_the_same_source_reuses_the_stored_generation() {
    let dir = TempDir::new("snapshot-store-reuse");
    let root = text_project(&dir, &text_document());
    let store = SnapshotStore::at(&dir.path.join("store")).unwrap();

    let first = dir.path.join("first");
    materialize(&root, &first);
    let SnapshotCache::Built(Rebuild::Rebuilt { manifest, .. }) =
        prepare_snapshot_with(&first, Some(&store)).unwrap()
    else {
        panic!("처음은 원본에서 만들어야 함");
    };
    let second = dir.path.join("second");
    materialize(&root, &second);
    assert_eq!(
        prepare_snapshot_with(&second, Some(&store)).unwrap(),
        SnapshotCache::Reused {
            generation: 1,
            manifest
        },
        "같은 원본 digest는 보관 generation을 씀"
    );
    // 재사용한 snapshot은 stamp 짝인 cache라 원본 대신 cache로 열고, 의미는 원본에서 만든 것과 같습니다.
    assert_eq!(observe(&second).unwrap().plan, OpenPlan::Cache);
    assert_eq!(semantic(&second), semantic(&first));

    // 원본이 한 byte라도 다르면 다른 key입니다.
    let third = dir.path.join("third");
    materialize(&root, &third);
    let document = third.join("design/document.stellar.yaml");
    let mut text = std::fs::read_to_string(&document).unwrap();
    text.push('\n');
    std::fs::write(&document, text).unwrap();
    assert!(matches!(
        prepare_snapshot_with(&third, Some(&store)).unwrap(),
        SnapshotCache::Built(Rebuild::Rebuilt { .. })
    ));
    // 보관소 없이는 항상 원본에서 만듭니다.
    let fourth = dir.path.join("fourth");
    materialize(&root, &fourth);
    assert!(matches!(
        prepare_snapshot_with(&fourth, None).unwrap(),
        SnapshotCache::Built(Rebuild::Rebuilt { .. })
    ));
}

#[test]
fn a_damaged_stored_chunk_is_not_reused_and_the_snapshot_is_built_from_text() {
    let dir = TempDir::new("snapshot-store-damage");
    let root = text_project(&dir, &text_document());
    let store = SnapshotStore::at(&dir.path.join("store")).unwrap();
    let first = dir.path.join("first");
    materialize(&root, &first);
    prepare_snapshot_with(&first, Some(&store)).unwrap();
    // 보관 항목의 chunk 하나를 다른 bytes로 바꿉니다(hard link가 아니라 새 파일로 바꿔 첫 snapshot은 그대로 둠).
    let entry = std::fs::read_dir(store.root())
        .unwrap()
        .flatten()
        .find(|e| e.file_name().len() == 64)
        .unwrap()
        .path();
    let objects = entry.join(".stellar/cache/chunks/objects");
    let prefix = std::fs::read_dir(&objects)
        .unwrap()
        .flatten()
        .next()
        .unwrap();
    let chunk = std::fs::read_dir(prefix.path())
        .unwrap()
        .flatten()
        .next()
        .unwrap()
        .path();
    std::fs::remove_file(&chunk).unwrap();
    std::fs::write(&chunk, b"damaged").unwrap();

    let second = dir.path.join("second");
    materialize(&root, &second);
    assert!(matches!(
        prepare_snapshot_with(&second, Some(&store)).unwrap(),
        SnapshotCache::Built(Rebuild::Rebuilt { .. })
    ));
    assert_eq!(semantic(&second), semantic(&first));
}

#[test]
fn the_key_separates_projects_builds_and_sources() {
    let source = stellar_storage::hash::ContentHash::compute(b"source");
    let other = stellar_storage::hash::ContentHash::compute(b"other");
    let project = project_id();
    let other_project =
        stellar_project::identity::parse_project_id("0195c530-3acd-7bc1-8414-c213f61cb966")
            .unwrap();
    let key = SnapshotKey::with_build(project, source, [1; 32]);
    assert_eq!(key, SnapshotKey::with_build(project, source, [1; 32]));
    assert_ne!(key, SnapshotKey::with_build(project, other, [1; 32]));
    assert_ne!(key, SnapshotKey::with_build(other_project, source, [1; 32]));
    assert_ne!(key, SnapshotKey::with_build(project, source, [2; 32]));
    assert!(
        SnapshotKey::new(project, source).is_some(),
        "시험 실행 파일의 build 식별자"
    );
}

#[test]
fn prune_keeps_the_most_recent_entries_and_drops_idle_ones() {
    let dir = TempDir::new("snapshot-store-prune");
    let store = SnapshotStore::at(&dir.path.join("store")).unwrap();
    let now = SystemTime::now();
    // 가짜 항목: 이름이 key 형식이고 current pointer의 수정 시각이 마지막 사용 시각입니다.
    let total = MAX_ENTRIES + 3;
    for n in 0..total {
        let entry = store.root().join(format!("{n:064x}"));
        let pointer = entry.join(".stellar/cache/chunks/current");
        std::fs::create_dir_all(pointer.parent().unwrap()).unwrap();
        std::fs::write(&pointer, b"x").unwrap();
        let file = std::fs::File::options().write(true).open(&pointer).unwrap();
        file.set_modified(now - Duration::from_secs(n as u64 * 10))
            .unwrap();
    }
    // key 형식이 아닌 항목은 건드리지 않습니다.
    std::fs::create_dir_all(store.root().join("unrelated")).unwrap();
    let report = store.prune(now, 3_600);
    assert_eq!(report.kept, MAX_ENTRIES);
    assert_eq!(report.removed, 3);
    for n in 0..total {
        let exists = store.root().join(format!("{n:064x}")).exists();
        assert_eq!(exists, n < MAX_ENTRIES, "가장 최근에 쓴 항목만 남음: {n}");
    }
    assert!(store.root().join("unrelated").exists());
    // 유예 시간보다 오래 쓰지 않은 항목은 수가 적어도 지웁니다.
    let report = store.prune(now + Duration::from_secs(7_200), 3_600);
    assert_eq!(report.kept, 0);
    assert_eq!(report.removed, MAX_ENTRIES);
}

#[cfg(unix)]
#[test]
fn a_shared_store_folder_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("snapshot-store-shared");
    let shared = dir.path.join("store");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(SnapshotStore::at(&shared).is_err(), "group·other 권한");
    let link = dir.path.join("link");
    std::os::unix::fs::symlink(&shared, &link).unwrap();
    assert!(SnapshotStore::at(&link).is_err(), "symlink");
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn time<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let started = std::time::Instant::now();
    let value = f();
    (value, ms(started.elapsed()))
}

#[test]
#[ignore = "실측(release로 실행)"]
fn measure_snapshot_preparation_on_a_20k_node_document() {
    let dir = TempDir::new("snapshot-store-measure");
    let model = large_document(20_000);
    let nodes = model.node_count();
    let root = text_project(&dir, &model);
    let store = SnapshotStore::at(&dir.path.join("store")).unwrap();
    let mut rows = Vec::new();
    for round in 0..3 {
        let cold = dir.path.join(format!("cold-{round}"));
        materialize(&root, &cold);
        let (built, build_ms) = time(|| prepare_snapshot_with(&cold, None).unwrap());
        assert!(matches!(
            built,
            SnapshotCache::Built(Rebuild::Rebuilt { .. })
        ));
        let (_, open_cold_ms) = time(|| semantic(&cold));

        let warm = dir.path.join(format!("warm-{round}"));
        materialize(&root, &warm);
        if round == 0 {
            // 첫 round는 보관소를 채웁니다(원본에서 만들고 공개).
            let (first, publish_ms) = time(|| prepare_snapshot_with(&warm, Some(&store)).unwrap());
            assert!(matches!(first, SnapshotCache::Built(_)));
            println!("publish(원본에서 만들고 보관) {publish_ms:.1} ms");
            std::fs::remove_dir_all(&warm).unwrap();
            materialize(&root, &warm);
        }
        let (reused, reuse_ms) = time(|| prepare_snapshot_with(&warm, Some(&store)).unwrap());
        assert!(matches!(reused, SnapshotCache::Reused { .. }));
        let (_, open_warm_ms) = time(|| semantic(&warm));
        rows.push((build_ms, open_cold_ms, reuse_ms, open_warm_ms));
        std::fs::remove_dir_all(&cold).unwrap();
        std::fs::remove_dir_all(&warm).unwrap();
    }
    println!("node {nodes}");
    println!("round | 원본에서 만들기 | 열기·semantic | 재사용 | 열기·semantic");
    for (round, (build, open_cold, reuse, open_warm)) in rows.iter().enumerate() {
        println!("{round} | {build:.1} ms | {open_cold:.1} ms | {reuse:.1} ms | {open_warm:.1} ms");
    }
}

#[test]
#[ignore = "실측(release로 실행)"]
fn measure_cache_gc_on_a_20k_node_document() {
    // 앱 유휴 GC는 document thread job 안에서 collect_garbage를 부릅니다(ADR-0138 결정 2). 그 시간이 document
    // thread를 막는 시간입니다.
    use crate::test_support::save_documents;
    use stellar_project::cache::collect_garbage;
    use stellar_storage::gc::retention::RetentionPolicy;
    let dir = TempDir::new("snapshot-store-gc-measure");
    let root = dir.path.join("project");
    scaffold_project(&root);
    let base = large_document(20_000);
    let records = base.to_records().unwrap();
    let mut model = base;
    let generations = 6;
    for n in 0..generations {
        let id = records.nodes[n * 1_000].id;
        model.set_display_name(id, format!("g{n}")).unwrap();
        save_documents(&root, &[&model]);
    }
    let policy = RetentionPolicy {
        keep_recent_generations: 1,
        grace_period_seconds: 0,
    };
    let later = || SystemTime::now() + Duration::from_secs(1);
    let (report, gc_ms) = time(|| collect_garbage(&root, &policy, later()).unwrap());
    let (again, idle_ms) = time(|| collect_garbage(&root, &policy, later()).unwrap());
    println!(
        "세대 {generations}: 정리 {gc_ms:.1} ms(지운 파일 {}), 지울 것 없는 정리 {idle_ms:.1} ms(지운 파일 {})",
        report.deleted, again.deleted
    );
}

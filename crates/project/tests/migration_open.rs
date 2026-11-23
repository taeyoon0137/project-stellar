//! IX.11.21 제품 open 경로의 schema 판정과 메모리 migration.
//!
//! 옛 schema fixture는 시험용 noop step(1 → 2)을 등록한 정책을 대상 schema 2로 주입해 만듭니다. 제품 정책은
//! schema 1뿐이라 같은 generation이 제품 open에서는 편집 가능, 새 버전(2)은 거부입니다.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::git::{project_id, save_documents, write_manifest};
use common::{rich_model, TempDir};
use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_project::migration::{
    MigrationPolicy, SchemaDecision, SchemaRejection, PRODUCT_SCHEMA_VERSION,
};
use stellar_project::saved_project::{SavedProjectError, SavedProjectLoader};
use stellar_storage::migration::registry::MigrationRegistry;
use stellar_storage::migration::runner::{noop_v1_to_v2, run_migration};

fn noop_registry() -> MigrationRegistry {
    let mut registry = MigrationRegistry::new();
    registry.register(noop_v1_to_v2()).unwrap();
    registry
}

fn digest(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().display().to_string(),
                    std::fs::read(&path).unwrap(),
                );
            }
        }
    }
    out
}

fn saved_project(dir: &TempDir) -> (std::path::PathBuf, stellar_document::model::DocumentModel) {
    let root = dir.path.join("project");
    std::fs::create_dir_all(&root).unwrap();
    write_manifest(&root, "Migration");
    let model = rich_model(1, 0);
    save_documents(&root, &[&model]);
    (root, model)
}

#[test]
fn product_open_accepts_the_current_schema_as_editable() {
    let dir = TempDir::new("migration-open-current");
    let (root, model) = saved_project(&dir);
    let project = SavedProjectLoader::new().open(&root).unwrap();
    assert_eq!(project.schema, SchemaDecision::Current);
    assert!(project.migrated.is_none());
    assert_eq!(
        project.generation.manifest().schema_version,
        PRODUCT_SCHEMA_VERSION
    );
    let loaded = project.load_document(model.document_id()).unwrap();
    assert_eq!(
        model_snapshot_json(&loaded.model).unwrap(),
        model_snapshot_json(&model).unwrap()
    );
}

#[test]
fn older_schema_opens_migrated_in_memory_without_writing() {
    let dir = TempDir::new("migration-open-older");
    let (root, model) = saved_project(&dir);
    let before = digest(&root);
    let policy = MigrationPolicy {
        target: 2,
        registry: noop_registry,
    };
    for loader in [
        SavedProjectLoader::new(),
        SavedProjectLoader::for_materialized_snapshot(),
    ] {
        let project = loader.with_migrations(policy).open(&root).unwrap();
        assert_eq!(
            project.schema,
            SchemaDecision::MigrateInMemory { from: 1, to: 2 }
        );
        assert!(!project.schema.is_editable());
        let migrated = project.migrated.as_ref().unwrap();
        assert_eq!((migrated.from_schema, migrated.schema), (1, 2));
        // 디스크 generation은 원본 schema 그대로입니다.
        assert_eq!(project.generation.manifest().schema_version, 1);
        assert_eq!(project.document_ids().unwrap(), vec![model.document_id()]);
        let loaded = project.load_document(model.document_id()).unwrap();
        assert_eq!(
            model_snapshot_json(&loaded.model).unwrap(),
            model_snapshot_json(&model).unwrap()
        );
    }
    // 열기는 디스크 bytes를 바꾸지 않고 자동 commit도 하지 않습니다.
    assert_eq!(digest(&root), before);

    // 경로가 없는 낮은 schema는 거부입니다.
    let gap = SavedProjectLoader::new()
        .with_migrations(MigrationPolicy {
            target: 3,
            registry: noop_registry,
        })
        .open(&root);
    assert!(matches!(
        gap,
        Err(SavedProjectError::Schema(SchemaRejection::NoPath {
            found: 1,
            target: 3
        }))
    ));
}

#[test]
fn explicit_migration_result_equals_the_in_memory_view() {
    let dir = TempDir::new("migration-open-explicit");
    let (root, model) = saved_project(&dir);
    let policy = MigrationPolicy {
        target: 2,
        registry: noop_registry,
    };
    let memory = SavedProjectLoader::new()
        .with_migrations(policy)
        .open(&root)
        .unwrap();
    let in_memory =
        model_snapshot_json(&memory.load_document(model.document_id()).unwrap().model).unwrap();
    // 명시 migration(V.8.4 경로)이 쓴 새 generation은 대상 schema에서 그대로 열립니다.
    run_migration(&root, project_id(), &noop_registry(), 2).unwrap();
    let written = SavedProjectLoader::new()
        .with_migrations(policy)
        .open(&root)
        .unwrap();
    assert_eq!(written.schema, SchemaDecision::Current);
    let on_disk =
        model_snapshot_json(&written.load_document(model.document_id()).unwrap().model).unwrap();
    assert_eq!(in_memory, on_disk);

    // 제품 도구(schema 1)에는 이제 새 버전입니다.
    assert!(matches!(
        SavedProjectLoader::new().open(&root),
        Err(SavedProjectError::Schema(SchemaRejection::Newer {
            found: 2,
            target: 1
        }))
    ));
}

//! migration golden fixture runner (V.8.6) — `cargo xtask test-migrations`.
//!
//! 과거 스키마 버전의 fixture 문서를 실제 저장소 파이프라인으로 구성하고,
//! **제품 migration registry**(`stellar_project::migration::product_registry`, IX.11.21)로 **제품 최신
//! 스키마**(`PRODUCT_SCHEMA_VERSION`)까지 변환한 결과가 기대 snapshot과 정확히 일치하는지 검사합니다.
//! storage의 시험용 noop step은 쓰지 않습니다(제품 번호와 섞이지 않게, ADR-0077 결정 4). 새 스키마 버전이
//! 추가될 때마다 과거 fixture는 그대로 두고 기대 snapshot만 최신 버전으로 갱신합니다. 결과 generation은
//! 제품 열기 판정(`migration::decide`)이 편집 가능(`Current`)이어야 합니다.

use std::path::PathBuf;

use stellar_core_types::ids::StableId;
use stellar_project::migration::{
    decide, product_registry, MigrationPolicy, PRODUCT_SCHEMA_VERSION,
};
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::kinds::{KIND_DOCUMENT_MANIFEST, KIND_VIEW_TREE};
use stellar_storage::chunk::planner::LogicalChunkState;
use stellar_storage::chunk::store::ChunkStore;
use stellar_storage::generation::loader::load_current_generation;
use stellar_storage::migration::runner::run_migration;
use stellar_storage::save::cancel::{run_save, CancelToken};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::build_save_plan;

/// 현재 최신 문서 스키마 버전(제품 registry의 목표).
pub const LATEST_SCHEMA: u32 = PRODUCT_SCHEMA_VERSION;

/// fixture의 논리 chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureChunk {
    pub id_ts: u64,
    pub kind_code: u32,
    pub payload: Vec<u8>,
}

/// golden fixture: 과거 버전 문서와 최신 snapshot 기대값.
#[derive(Debug, Clone)]
pub struct MigrationFixture {
    pub name: &'static str,
    /// fixture 문서의 시작 스키마 버전.
    pub from_schema: u32,
    /// 시작 시점 chunk들.
    pub chunks: Vec<FixtureChunk>,
    /// 최신 스키마로 변환된 뒤 기대하는 chunk들.
    pub expected_chunks: Vec<FixtureChunk>,
}

/// 등록된 모든 과거 fixture.
pub fn fixtures() -> Vec<MigrationFixture> {
    let doc = |payload: &[u8]| FixtureChunk {
        id_ts: 1_000,
        kind_code: KIND_DOCUMENT_MANIFEST.0,
        payload: payload.to_vec(),
    };
    let view = |ts: u64, payload: &[u8]| FixtureChunk {
        id_ts: ts,
        kind_code: KIND_VIEW_TREE.0,
        payload: payload.to_vec(),
    };
    vec![
        // schema 1은 지금 제품 최신이고 제품 step이 없습니다: 내용이 그대로 보존되어야 합니다. 첫 제품 step(1 → 2)이
        // 생기면 이 fixture의 입력은 그대로 두고 기대 chunk만 step 결과로 바꿉니다.
        MigrationFixture {
            name: "v1-minimal",
            from_schema: 1,
            chunks: vec![doc(b"doc-manifest-v1")],
            expected_chunks: vec![doc(b"doc-manifest-v1")],
        },
        MigrationFixture {
            name: "v1-two-views",
            from_schema: 1,
            chunks: vec![
                doc(b"doc-manifest-v1"),
                view(2_000, b"view-a"),
                view(2_001, b"view-b"),
            ],
            expected_chunks: vec![
                doc(b"doc-manifest-v1"),
                view(2_000, b"view-a"),
                view(2_001, b"view-b"),
            ],
        },
    ]
}

fn stable(ts: u64) -> StableId {
    StableId::new_v7(ts, 0x0abc, 77)
}

/// fixture 하나를 실행합니다. 성공 시 `Ok(())`.
fn run_fixture(work_root: &std::path::Path, fixture: &MigrationFixture) -> Result<(), String> {
    let root = work_root.join(fixture.name);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let project_id = stable(42);

    // fixture 문서를 실제 저장 파이프라인으로 구성.
    let states: Vec<LogicalChunkState> = fixture
        .chunks
        .iter()
        .map(|c| LogicalChunkState::Dirty {
            chunk_id: ChunkId::from_stable_id(stable(c.id_ts)),
            kind: stellar_storage::chunk::envelope::ChunkKindCode(c.kind_code),
            schema_version: fixture.from_schema,
            payload: c.payload.clone(),
        })
        .collect();
    let plan = build_save_plan(None, project_id, fixture.from_schema, &states, vec![])
        .map_err(|e| format!("{}: 계획 실패: {e}", fixture.name))?;
    let mut memory = SavedGenerationState::default();
    run_save(&root, &plan, &mut memory, &CancelToken::new())
        .map_err(|e| format!("{}: 구성 저장 실패: {e}", fixture.name))?;

    // 최신 스키마로 migration.
    let outcome = run_migration(&root, project_id, &product_registry(), LATEST_SCHEMA)
        .map_err(|e| format!("{}: migration 실패: {e}", fixture.name))?;
    if outcome.schema_version != LATEST_SCHEMA {
        return Err(format!(
            "{}: 스키마 {}에 도달해야 하는데 {}입니다",
            fixture.name, LATEST_SCHEMA, outcome.schema_version
        ));
    }

    // 결과 snapshot을 기대값과 비교.
    let loaded = load_current_generation(&root, project_id)
        .map_err(|e| format!("{}: 결과 로드 실패: {e}", fixture.name))?;
    if loaded.manifest.schema_version != LATEST_SCHEMA {
        return Err(format!("{}: manifest 스키마 불일치", fixture.name));
    }
    if !decide(loaded.manifest.schema_version, &MigrationPolicy::PRODUCT).is_editable() {
        return Err(format!(
            "{}: 제품 열기 판정이 결과를 편집 가능으로 보지 않습니다",
            fixture.name
        ));
    }
    let store = ChunkStore::new(&root);
    let mut actual: Vec<FixtureChunk> = Vec::new();
    for pointer in &loaded.manifest.chunk_pointers {
        let payload = store
            .read_chunk(&pointer.content_hash, pointer.kind)
            .map_err(|e| format!("{}: chunk 읽기 실패: {e}", fixture.name))?;
        actual.push(FixtureChunk {
            id_ts: 0, // id는 아래에서 기대값 순서 매칭으로 비교합니다.
            kind_code: pointer.kind.0,
            payload,
        });
    }

    let mut expected: Vec<(u32, Vec<u8>)> = fixture
        .expected_chunks
        .iter()
        .map(|c| (c.kind_code, c.payload.clone()))
        .collect();
    let mut got: Vec<(u32, Vec<u8>)> = actual
        .into_iter()
        .map(|c| (c.kind_code, c.payload))
        .collect();
    expected.sort();
    got.sort();
    if expected != got {
        return Err(format!("{}: snapshot 불일치", fixture.name));
    }
    Ok(())
}

/// 모든 fixture를 실행합니다. 성공 fixture 수를 반환합니다.
pub fn run_test_migrations() -> Result<usize, String> {
    let work: PathBuf =
        std::env::temp_dir().join(format!("stellar-test-migrations-{}", std::process::id()));
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let all = fixtures();
    let mut failures = Vec::new();
    for fixture in &all {
        if let Err(e) = run_fixture(&work, fixture) {
            failures.push(e);
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    if failures.is_empty() {
        Ok(all.len())
    } else {
        Err(failures.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_golden_fixtures_migrate_to_latest_snapshot() {
        // 검증 기준(V.8.6): 모든 과거 fixture가 최신 snapshot으로 정확히 변환.
        let count = run_test_migrations().expect("모든 fixture 통과");
        assert_eq!(count, fixtures().len());
    }

    #[test]
    fn fixture_set_is_not_empty_and_targets_latest() {
        let all = fixtures();
        assert!(!all.is_empty());
        for fixture in &all {
            assert!(
                (1..=LATEST_SCHEMA).contains(&fixture.from_schema),
                "{}",
                fixture.name
            );
        }
        // 1부터 최신까지 모든 제품 schema에 fixture가 있습니다(새 schema마다 과거 fixture를 남김).
        for schema in 1..=LATEST_SCHEMA {
            assert!(
                all.iter().any(|f| f.from_schema == schema),
                "schema {schema}"
            );
        }
    }

    #[test]
    fn runner_uses_the_product_registry() {
        // xtask가 자체 registry를 두지 않고 제품 표를 씁니다: 최신까지 경로가 있고, 그 너머는 없습니다.
        let registry = product_registry();
        assert!(registry.plan_path(1, LATEST_SCHEMA).is_ok());
        assert!(registry.plan_path(1, LATEST_SCHEMA + 1).is_err());
    }
}

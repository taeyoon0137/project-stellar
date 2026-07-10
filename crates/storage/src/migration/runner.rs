//! migration 실행기 (V.8.3, V.8.4).
//!
//! migration은 **원본 세대를 수정하지 않습니다**:
//!
//! 1. current generation을 로드하고 모든 chunk payload를 읽어 in-memory
//!    [`MigrationDocument`]를 만듭니다.
//! 2. registry(V.8.2)로 목표 스키마 버전까지 변환합니다.
//! 3. 결과를 **새 generation**으로 저장합니다 — 저장 파이프라인(V.5)을 그대로
//!    사용하므로 chunk·manifest는 새 파일 추가뿐이고(temp generation), current
//!    pointer 전환은 모든 검증(기록 chunk 재읽기 검증 포함)이 끝난 마지막
//!    단계입니다. 중간 실패 시 원본 current가 유지됩니다.
//!
//! Git commit은 만들지 않습니다 — 결과는 working tree 변경으로 남습니다(V.8.5).

use std::fmt;
use std::path::Path;

use stellar_core_types::ids::StableId;

use crate::atomic_file::WriteObserver;
use crate::chunk::planner::LogicalChunkState;
use crate::chunk::store::{ChunkStore, ChunkStoreError};
use crate::generation::loader::{load_current_generation, LoadError};
use crate::hash::ContentHash;
use crate::save::cancel::{run_save_observed, CancelToken, SaveRunError};
use crate::save::commit::SavedGenerationState;
use crate::save::executor::SaveObserver;
use crate::save::plan::{build_save_plan, SavePlanError};

use super::registry::{MigrationDocument, MigrationRegistry, MigrationRegistryError};

/// migration 실행 오류.
#[derive(Debug)]
pub enum MigrationRunError {
    /// 원본 세대 로드 실패.
    Load(LoadError),
    /// 원본 chunk 읽기 실패.
    Chunk(ChunkStoreError),
    /// 변환 실패(경로 없음·step 실패 등).
    Registry(MigrationRegistryError),
    /// 결과 저장 계획 실패.
    Plan(SavePlanError),
    /// 결과 저장 실패 — 원본 current는 유지됩니다.
    Save(SaveRunError),
}

impl fmt::Display for MigrationRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrationRunError::Load(e) => write!(f, "원본 세대 로드 실패: {e}"),
            MigrationRunError::Chunk(e) => write!(f, "원본 chunk 읽기 실패: {e}"),
            MigrationRunError::Registry(e) => write!(f, "migration 변환 실패: {e}"),
            MigrationRunError::Plan(e) => write!(f, "결과 저장 계획 실패: {e}"),
            MigrationRunError::Save(e) => write!(f, "결과 저장 실패: {e}"),
        }
    }
}

impl std::error::Error for MigrationRunError {}

/// migration 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationOutcome {
    /// 원본 generation number.
    pub from_generation: u64,
    /// 새 generation number.
    pub to_generation: u64,
    /// 새 manifest hash(현재 current pointer 값).
    pub new_manifest_hash: ContentHash,
    /// 도달한 스키마 버전.
    pub schema_version: u32,
}

/// current generation을 목표 스키마 버전으로 migration합니다.
pub fn run_migration(
    root: &Path,
    project_id: StableId,
    registry: &MigrationRegistry,
    target_schema: u32,
) -> Result<MigrationOutcome, MigrationRunError> {
    let mut noop_prepare = crate::save::executor::NoopSaveObserver;
    let mut noop_pointer = crate::atomic_file::NoopObserver;
    run_migration_observed(
        root,
        project_id,
        registry,
        target_schema,
        &mut noop_prepare,
        &mut noop_pointer,
    )
}

/// [`run_migration`]에 저장 단계 관찰자를 주입한 버전(fault injection 테스트용,
/// V.8.4).
pub fn run_migration_observed(
    root: &Path,
    project_id: StableId,
    registry: &MigrationRegistry,
    target_schema: u32,
    prepare_observer: &mut dyn SaveObserver,
    pointer_observer: &mut dyn WriteObserver,
) -> Result<MigrationOutcome, MigrationRunError> {
    // 1. 원본 로드(읽기 전용).
    let base = load_current_generation(root, project_id).map_err(MigrationRunError::Load)?;
    let store = ChunkStore::new(root);
    let mut document = MigrationDocument {
        schema_version: base.manifest.schema_version,
        chunks: Vec::with_capacity(base.manifest.chunk_pointers.len()),
        root_document_ids: base.manifest.root_document_ids.clone(),
    };
    for pointer in &base.manifest.chunk_pointers {
        let payload = store
            .read_chunk(&pointer.content_hash, pointer.kind)
            .map_err(MigrationRunError::Chunk)?;
        document
            .chunks
            .push((pointer.chunk_id, pointer.kind, payload));
    }

    // 2. in-memory 변환.
    registry
        .migrate_to(&mut document, target_schema)
        .map_err(MigrationRunError::Registry)?;

    // 3. 새 generation으로 저장(변경 없는 payload는 planner가 재사용).
    let states: Vec<LogicalChunkState> = document
        .chunks
        .iter()
        .map(|(chunk_id, kind, payload)| LogicalChunkState::Dirty {
            chunk_id: *chunk_id,
            kind: *kind,
            schema_version: document.schema_version,
            payload: payload.clone(),
        })
        .collect();
    let plan = build_save_plan(
        Some(&base),
        project_id,
        document.schema_version,
        &states,
        document.root_document_ids.clone(),
    )
    .map_err(MigrationRunError::Plan)?;

    let mut memory =
        SavedGenerationState::new(Some((base.manifest.generation, base.manifest_hash)));
    run_save_observed(
        root,
        &plan,
        &mut memory,
        &CancelToken::new(),
        prepare_observer,
        pointer_observer,
    )
    .map_err(MigrationRunError::Save)?;

    let (to_generation, new_manifest_hash) = memory.saved().expect("저장 성공");
    Ok(MigrationOutcome {
        from_generation: base.manifest.generation,
        to_generation,
        new_manifest_hash,
        schema_version: document.schema_version,
    })
}

/// noop v1→v2 test migration step (V.8.3).
///
/// payload를 바꾸지 않고 스키마 버전만 올립니다. registry에 등록해 사용합니다.
pub fn noop_v1_to_v2() -> super::registry::MigrationStep {
    super::registry::MigrationStep {
        from: 1,
        to: 2,
        migrate: Box::new(|_| Ok(())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::envelope::ChunkKindCode;
    use crate::chunk::id::ChunkId;
    use crate::chunk::path::ChunkObjectPath;
    use crate::chunk::pointer::ChunkPointer;
    use crate::chunk::store::WriteOutcome;
    use crate::generation::current::{read_current_pointer, write_current_pointer};
    use crate::generation::loader::{manifest_path, read_manifest_file, write_manifest_file};
    use crate::generation::manifest::GenerationManifest;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("stellar-migrate-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    const KIND: ChunkKindCode = ChunkKindCode(0x8000_0001);

    fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    /// schema v1, chunk 2개짜리 current generation fixture.
    fn v1_project(dir: &TempDir, project_id: StableId) -> (ContentHash, Vec<ContentHash>) {
        let store = ChunkStore::new(&dir.path);
        let mut pointers = Vec::new();
        let mut chunk_hashes = Vec::new();
        for (i, payload) in [b"chunk-a".as_slice(), b"chunk-b".as_slice()]
            .iter()
            .enumerate()
        {
            let WriteOutcome::Written { hash, .. } = store.write_chunk(KIND, payload).unwrap()
            else {
                panic!("Written이어야 함");
            };
            chunk_hashes.push(hash);
            pointers.push(ChunkPointer {
                chunk_id: ChunkId::from_stable_id(stable(1_000 + i as u64)),
                content_hash: hash,
                kind: KIND,
                schema_version: 1,
            });
        }
        let manifest = GenerationManifest {
            generation: 1,
            parent_generation: None,
            project_id,
            schema_version: 1,
            chunk_pointers: pointers,
            root_document_ids: vec![stable(7_000)],
        };
        let manifest_hash = write_manifest_file(&dir.path, &manifest).unwrap();
        write_current_pointer(&dir.path, &manifest_hash).unwrap();
        (manifest_hash, chunk_hashes)
    }

    fn registry_v1_to_v2() -> MigrationRegistry {
        let mut registry = MigrationRegistry::new();
        registry.register(noop_v1_to_v2()).unwrap();
        registry
    }

    #[test]
    fn noop_migration_preserves_original_and_creates_new_generation() {
        // 검증 기준(V.8.3): 원본 hash 유지, 새 generation 생성.
        let dir = TempDir::new();
        let project_id = stable(42);
        let (original_manifest_hash, chunk_hashes) = v1_project(&dir, project_id);
        let original_manifest_bytes =
            std::fs::read(manifest_path(&dir.path, &original_manifest_hash)).unwrap();

        let outcome = run_migration(&dir.path, project_id, &registry_v1_to_v2(), 2).unwrap();
        assert_eq!(outcome.from_generation, 1);
        assert_eq!(outcome.to_generation, 2);
        assert_eq!(outcome.schema_version, 2);

        // 원본 manifest 파일이 byte 그대로 남아 있음(원본 hash 유지).
        assert_eq!(
            std::fs::read(manifest_path(&dir.path, &original_manifest_hash)).unwrap(),
            original_manifest_bytes
        );
        // 원본 chunk 파일도 그대로(내용 무변경 → 재사용).
        for hash in &chunk_hashes {
            let path = dir
                .path
                .join(ChunkObjectPath::for_hash(hash).to_relative_path());
            assert!(path.is_file());
        }

        // 새 generation manifest가 생성되고 current가 그것을 가리킴.
        assert_ne!(outcome.new_manifest_hash, original_manifest_hash);
        assert_eq!(
            read_current_pointer(&dir.path).unwrap(),
            outcome.new_manifest_hash
        );
        let new_manifest = read_manifest_file(&dir.path, &outcome.new_manifest_hash).unwrap();
        assert_eq!(new_manifest.generation, 2);
        assert_eq!(new_manifest.parent_generation, Some(1));
        assert_eq!(new_manifest.schema_version, 2);
        // noop이므로 chunk pointer hash는 원본과 동일(재사용).
        let new_hashes: Vec<ContentHash> = new_manifest
            .chunk_pointers
            .iter()
            .map(|p| p.content_hash)
            .collect();
        assert_eq!(new_hashes, chunk_hashes);
    }

    #[test]
    fn missing_migration_path_fails_without_changes() {
        let dir = TempDir::new();
        let project_id = stable(42);
        let (original_hash, _) = v1_project(&dir, project_id);
        // 빈 registry: v1→v3 경로 없음.
        let err = run_migration(&dir.path, project_id, &MigrationRegistry::new(), 3).unwrap_err();
        assert!(matches!(err, MigrationRunError::Registry(_)));
        assert_eq!(read_current_pointer(&dir.path).unwrap(), original_hash);
    }
}

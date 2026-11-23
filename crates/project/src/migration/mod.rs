//! 제품 문서 migration registry (IX.11.21, ADR-0077 결정 4).
//!
//! storage crate는 document에 의존하지 않아 node record를 해석하지 못하므로, 제품 schema step과
//! schema version별 고정 decoder는 document와 storage에 모두 의존하는 이 crate에 둡니다.
//!
//! - **제품 schema version**: generation manifest의 `schema_version`입니다. 이 도구가 쓰고 읽는 값은
//!   [`PRODUCT_SCHEMA_VERSION`] 하나입니다.
//! - **제품 step 표**: [`PRODUCT_STEPS`]만 제품 registry에 들어갑니다. storage의 시험용 noop step
//!   (`noop_v1_to_v2`)과 xtask fixture의 step은 이 표를 지나지 않으므로 제품 번호와 섞이지 않습니다.
//!   표는 1부터 [`PRODUCT_SCHEMA_VERSION`]까지 빈틈 없이 이어져야 합니다(테스트가 검사).
//! - **열기 판정**([`decide`]): V.8.1 판정 matrix를 generation 하나에 적용합니다. 같은 schema는
//!   편집 가능, 낮은 schema는 경로가 있으면 메모리에서만 migration한 읽기 전용, 경로가 없거나 높은
//!   schema는 거부입니다. 열기는 디스크를 쓰지 않으며 migration 결과를 자동 commit하지 않습니다
//!   (아키텍처 §36, §40.4). 디스크에 새 generation을 쓰는 일은 명시 명령(`stellar migrate`)만 합니다.
//! - **고정 decoder**([`decoders`]): schema version별 payload decoder 사본입니다. migration step은 옛
//!   bytes를 현재 decoder가 아니라 그 version의 사본으로 읽습니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_storage::chunk::id::ChunkId;
use stellar_storage::chunk::kinds::KIND_DOCUMENT_MANIFEST;
use stellar_storage::generation::lazy::{LazyGeneration, LazyLoadError, NoopRecorder};
use stellar_storage::migration::registry::{
    MigrationChunk, MigrationDocument, MigrationRegistry, MigrationRegistryError, MigrationStep,
};

use crate::document_io::load::{load_document, LoadDocumentError, LoadedDocument, MemoryChunks};
use crate::document_io::manifest::DocumentManifestChunk;

/// schema version별 고정 payload decoder.
pub mod decoders;

/// 제품 schema 1→2 묶음 step의 레코드 변환(아직 등록하지 않음, ADR-0086).
pub mod steps;

/// 이 도구가 쓰고 읽는 제품 generation schema version.
pub const PRODUCT_SCHEMA_VERSION: u32 = 1;

/// 제품 step 표. 원소 `i`는 `i + 1 → i + 2` step을 만듭니다. 지금 제품 schema는 1뿐이라 비어 있습니다.
///
/// IX.11.6·IX.11.9·IX.11.10·IX.11.11의 schema 변경은 공개 포맷(Phase XXVIII) 전 version 증가 한 번
/// (1 → 2)으로 묶습니다(ADR-0077 결정 4). 그 step이 생기면 여기에 한 항목을 더하고
/// [`PRODUCT_SCHEMA_VERSION`]을 2로 올립니다.
pub const PRODUCT_STEPS: &[fn() -> MigrationStep] = &[];

/// 제품 migration registry. [`PRODUCT_STEPS`]만 등록합니다.
pub fn product_registry() -> MigrationRegistry {
    let mut registry = MigrationRegistry::new();
    for step in PRODUCT_STEPS {
        registry
            .register(step())
            .expect("제품 step 표는 테스트가 검증한 연속 전진 step입니다");
    }
    registry
}

/// 열기에 쓰는 migration 정책: 대상 schema와 registry 생성 함수.
///
/// [`SavedProjectLoader`](crate::saved_project::SavedProjectLoader)가 `Copy`이므로 registry 자체 대신
/// 생성 함수를 보관합니다.
#[derive(Clone, Copy)]
pub struct MigrationPolicy {
    /// 열린 결과가 따를 schema version.
    pub target: u32,
    /// step을 등록한 registry를 만듭니다.
    pub registry: fn() -> MigrationRegistry,
}

impl MigrationPolicy {
    /// 제품 정책: [`PRODUCT_SCHEMA_VERSION`]과 [`product_registry`].
    pub const PRODUCT: MigrationPolicy = MigrationPolicy {
        target: PRODUCT_SCHEMA_VERSION,
        registry: product_registry,
    };
}

impl fmt::Debug for MigrationPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MigrationPolicy")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

impl PartialEq for MigrationPolicy {
    fn eq(&self, other: &Self) -> bool {
        self.target == other.target && std::ptr::fn_addr_eq(self.registry, other.registry)
    }
}

impl Eq for MigrationPolicy {}

/// 거부 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaRejection {
    /// 이 도구보다 새 schema입니다(내려 바꾸지 않음).
    Newer {
        /// generation schema.
        found: u32,
        /// 대상 schema.
        target: u32,
    },
    /// 낮은 schema지만 대상까지 가는 step이 없습니다.
    NoPath {
        /// generation schema.
        found: u32,
        /// 대상 schema.
        target: u32,
    },
}

impl fmt::Display for SchemaRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchemaRejection::Newer { found, target } => write!(
                f,
                "generation schema {found}가 이 도구의 schema {target}보다 새 버전입니다"
            ),
            SchemaRejection::NoPath { found, target } => write!(
                f,
                "generation schema {found}에서 {target}로 가는 migration step이 없습니다"
            ),
        }
    }
}

/// V.8.1 판정 matrix를 generation 하나에 적용한 열기 결정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaDecision {
    /// 대상과 같은 schema: 그대로 열고 편집할 수 있습니다.
    Current,
    /// 낮은 schema: 메모리에서만 migration해 읽기 전용으로 엽니다. 디스크는 바꾸지 않습니다.
    MigrateInMemory {
        /// generation schema.
        from: u32,
        /// 대상 schema.
        to: u32,
    },
    /// 열 수 없습니다.
    Reject(SchemaRejection),
}

impl SchemaDecision {
    /// 편집(수정 저장) 가능한 결정인지. 메모리 migration 결과는 명시 migration 전까지 읽기 전용입니다.
    pub fn is_editable(&self) -> bool {
        matches!(self, SchemaDecision::Current)
    }
}

/// `found` schema generation을 `policy`로 열 때의 결정.
pub fn decide(found: u32, policy: &MigrationPolicy) -> SchemaDecision {
    let target = policy.target;
    if found == target {
        return SchemaDecision::Current;
    }
    if found > target {
        return SchemaDecision::Reject(SchemaRejection::Newer { found, target });
    }
    match (policy.registry)().plan_path(found, target) {
        Ok(_) => SchemaDecision::MigrateInMemory {
            from: found,
            to: target,
        },
        Err(_) => SchemaDecision::Reject(SchemaRejection::NoPath { found, target }),
    }
}

/// 메모리 migration 실패.
#[derive(Debug)]
pub enum InMemoryMigrationError {
    /// 원본 chunk 읽기 실패.
    Read(LazyLoadError),
    /// step 실행 실패.
    Registry(MigrationRegistryError),
}

impl fmt::Display for InMemoryMigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InMemoryMigrationError::Read(error) => write!(f, "원본 chunk 읽기 실패: {error}"),
            InMemoryMigrationError::Registry(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for InMemoryMigrationError {}

/// 메모리에서만 migration한 generation. 디스크에는 원본 generation이 그대로 남습니다.
#[derive(Debug, Clone)]
pub struct MigratedGeneration {
    /// 원본 schema.
    pub from_schema: u32,
    /// 도달한 schema.
    pub schema: u32,
    /// root 문서 id.
    pub root_document_ids: Vec<StableId>,
    chunks: Vec<MigrationChunk>,
}

impl MigratedGeneration {
    /// 원본 generation의 모든 chunk를 읽어 `policy` 대상까지 메모리에서 migration합니다.
    pub fn from_lazy(
        lazy: &LazyGeneration,
        policy: &MigrationPolicy,
    ) -> Result<MigratedGeneration, InMemoryMigrationError> {
        let manifest = lazy.manifest();
        let mut document = MigrationDocument {
            schema_version: manifest.schema_version,
            chunks: Vec::with_capacity(manifest.chunk_pointers.len()),
            root_document_ids: manifest.root_document_ids.clone(),
        };
        for pointer in &manifest.chunk_pointers {
            let payload = lazy
                .read_chunk_payload(pointer.chunk_id, &mut NoopRecorder)
                .map_err(InMemoryMigrationError::Read)?;
            document.chunks.push(MigrationChunk {
                chunk_id: pointer.chunk_id,
                kind: pointer.kind,
                schema_version: pointer.schema_version,
                payload,
            });
        }
        Self::migrate(document, policy)
    }

    /// 이미 읽은 원본 chunk 집합을 메모리에서 migration합니다.
    pub fn migrate(
        mut document: MigrationDocument,
        policy: &MigrationPolicy,
    ) -> Result<MigratedGeneration, InMemoryMigrationError> {
        let from_schema = document.schema_version;
        (policy.registry)()
            .migrate_to(&mut document, policy.target)
            .map_err(InMemoryMigrationError::Registry)?;
        Ok(MigratedGeneration {
            from_schema,
            schema: document.schema_version,
            root_document_ids: document.root_document_ids,
            chunks: document.chunks,
        })
    }

    /// migration한 chunk(원본 pointer 순서).
    pub fn chunks(&self) -> &[MigrationChunk] {
        &self.chunks
    }

    fn manifests(&self) -> impl Iterator<Item = &MigrationChunk> {
        self.chunks
            .iter()
            .filter(|chunk| chunk.kind == KIND_DOCUMENT_MANIFEST)
    }

    /// 저장된 문서 id(DocumentManifest chunk 순서).
    pub fn document_ids(&self) -> Result<Vec<StableId>, LoadDocumentError> {
        self.manifests()
            .map(|chunk| {
                DocumentManifestChunk::decode_for_schema(chunk.schema_version, &chunk.payload)
                    .map(|manifest| manifest.definition.id)
                    .map_err(LoadDocumentError::Manifest)
            })
            .collect()
    }

    /// migration한 chunk에서 문서 하나를 로드합니다.
    pub fn load_document(&self, document: StableId) -> Result<LoadedDocument, LoadDocumentError> {
        let by_id: BTreeMap<ChunkId, &MigrationChunk> = self
            .chunks
            .iter()
            .map(|chunk| (chunk.chunk_id, chunk))
            .collect();
        for chunk in self.manifests() {
            let manifest =
                DocumentManifestChunk::decode_for_schema(chunk.schema_version, &chunk.payload)
                    .map_err(LoadDocumentError::Manifest)?;
            if manifest.definition.id == document {
                let mut source = MemoryChunks::new();
                for (id, chunk) in &by_id {
                    source.insert(*id, chunk.payload.clone());
                }
                return load_document(&chunk.payload, &mut source);
            }
        }
        Err(LoadDocumentError::UnknownDocument { document })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_storage::migration::runner::noop_v1_to_v2;

    fn noop_registry() -> MigrationRegistry {
        let mut registry = MigrationRegistry::new();
        registry.register(noop_v1_to_v2()).unwrap();
        registry
    }

    #[test]
    fn product_steps_are_contiguous_and_exclude_test_steps() {
        // 제품 표의 step `i`는 정확히 `i + 1 → i + 2`이고, 끝은 제품 schema입니다.
        for (index, step) in PRODUCT_STEPS.iter().enumerate() {
            let step = step();
            assert_eq!(step.from, index as u32 + 1);
            assert_eq!(step.to, index as u32 + 2);
        }
        assert_eq!(PRODUCT_STEPS.len() as u32 + 1, PRODUCT_SCHEMA_VERSION);
        let registry = product_registry();
        assert!(registry
            .plan_path(1, PRODUCT_SCHEMA_VERSION)
            .is_ok_and(|path| path.len() == PRODUCT_STEPS.len()));
        // 시험용 noop step(1 → 2)은 제품 registry에 없습니다.
        assert!(registry.plan_path(1, 2).is_err());
    }

    #[test]
    fn decision_matrix_follows_v81() {
        let product = MigrationPolicy::PRODUCT;
        assert_eq!(
            decide(PRODUCT_SCHEMA_VERSION, &product),
            SchemaDecision::Current
        );
        assert!(decide(PRODUCT_SCHEMA_VERSION, &product).is_editable());
        assert_eq!(
            decide(PRODUCT_SCHEMA_VERSION + 1, &product),
            SchemaDecision::Reject(SchemaRejection::Newer {
                found: PRODUCT_SCHEMA_VERSION + 1,
                target: PRODUCT_SCHEMA_VERSION
            })
        );
        let test = MigrationPolicy {
            target: 3,
            registry: noop_registry,
        };
        // 경로가 있는 낮은 schema는 메모리 migration(읽기 전용), 경로가 없으면 거부입니다.
        let two = MigrationPolicy {
            target: 2,
            registry: noop_registry,
        };
        let migrated = decide(1, &two);
        assert_eq!(migrated, SchemaDecision::MigrateInMemory { from: 1, to: 2 });
        assert!(!migrated.is_editable());
        assert_eq!(
            decide(1, &test),
            SchemaDecision::Reject(SchemaRejection::NoPath {
                found: 1,
                target: 3
            })
        );
        assert!(!decide(4, &test).is_editable());
    }

    #[test]
    fn in_memory_migration_keeps_payloads_and_raises_schema() {
        let chunk = MigrationChunk {
            chunk_id: ChunkId::from_stable_id(StableId::new_v7(1, 0, 1)),
            kind: KIND_DOCUMENT_MANIFEST,
            schema_version: 1,
            payload: vec![1, 2, 3],
        };
        let document = MigrationDocument {
            schema_version: 1,
            chunks: vec![chunk.clone()],
            root_document_ids: vec![],
        };
        let policy = MigrationPolicy {
            target: 2,
            registry: noop_registry,
        };
        let migrated = MigratedGeneration::migrate(document.clone(), &policy).unwrap();
        assert_eq!((migrated.from_schema, migrated.schema), (1, 2));
        assert_eq!(migrated.chunks(), &[chunk]);
        assert!(matches!(
            MigratedGeneration::migrate(
                document,
                &MigrationPolicy {
                    target: 3,
                    ..policy
                }
            ),
            Err(InMemoryMigrationError::Registry(_))
        ));
    }
}

//! 저장 스키마 migration (XXIV.1.3).
//!
//! current generation의 스키마 버전을 이 도구가 쓰는 버전([`CURRENT_SCHEMA_VERSION`])과 비교하고,
//! 낮으면 등록된 migration 경로(V.8.2)로 새 generation을 만듭니다(V.8.3 runner: 원본 세대 불변,
//! 중간 실패 시 current 유지). 쓰기는 저장 잠금(XXIV.4.5)을 잡은 뒤 상태를 다시 판정하고 실행합니다.
//!
//! 제품 스키마 버전과 제품 registry는 `stellar_project::migration`(IX.11.21)이 정합니다. 이 모듈은
//! 그 값을 그대로 씁니다. 현재 제품 스키마는 1뿐이라 제품 registry에는 step이 없고, 새 스키마의 step은
//! 제품 step 표(`stellar_project::migration::PRODUCT_STEPS`)에 등록합니다. 판정:
//!
//! | 상태 | 조건 |
//! |---|---|
//! | `unsaved` | 저장된 generation 없음 |
//! | `up-to-date` | 스키마 = 대상 |
//! | `migration-required` | 스키마 < 대상 |
//! | `unsupported-newer` | 스키마 > 대상(이 도구가 읽을 수 없는 새 버전) |

use std::fmt;
use std::path::Path;

use stellar_project::identity::parse_project_id;
use stellar_storage::migration::registry::MigrationRegistry;
use stellar_storage::migration::runner::run_migration;

use crate::error::OpenError;
use crate::lock::{LockError, LockPolicy, SaveLock};
use crate::revision::ProjectRevision;
use crate::snapshot::{open_snapshot, probe};

/// 이 도구가 쓰고 읽는 generation 스키마 버전(제품 migration registry와 같은 값).
pub const CURRENT_SCHEMA_VERSION: u32 = stellar_project::migration::PRODUCT_SCHEMA_VERSION;

/// 제품 migration registry(`stellar_project::migration::product_registry`).
pub fn product_registry() -> MigrationRegistry {
    stellar_project::migration::product_registry()
}

/// 스키마 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaState {
    /// 저장된 generation 없음.
    Unsaved,
    /// 대상 버전과 같음.
    UpToDate,
    /// 대상보다 낮아 migration이 필요함.
    MigrationRequired,
    /// 대상보다 높음(읽을 수 없는 새 버전).
    UnsupportedNewer,
}

impl SchemaState {
    /// 고정 이름.
    pub fn name(self) -> &'static str {
        match self {
            SchemaState::Unsaved => "unsaved",
            SchemaState::UpToDate => "up-to-date",
            SchemaState::MigrationRequired => "migration-required",
            SchemaState::UnsupportedNewer => "unsupported-newer",
        }
    }
}

/// 스키마 판정 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaStatus {
    /// 판정한 revision.
    pub revision: ProjectRevision,
    /// generation 스키마 버전(저장 전이면 `None`).
    pub schema_version: Option<u32>,
    /// 대상 스키마 버전.
    pub target_schema: u32,
    /// 상태.
    pub state: SchemaState,
}

/// migration 실패.
#[derive(Debug)]
pub enum MigrateError {
    /// 프로젝트를 열 수 없습니다.
    Open(OpenError),
    /// 저장 잠금을 잡지 못했습니다.
    Lock(LockError),
    /// 이 도구보다 새 스키마입니다(내려 바꾸지 않음).
    UnsupportedNewer {
        /// generation 스키마.
        found: u32,
        /// 대상 스키마.
        target: u32,
    },
    /// migration 실행 실패(원본 current 유지, V.8.4).
    Run(String),
}

impl fmt::Display for MigrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrateError::Open(e) => write!(f, "{e}"),
            MigrateError::Lock(e) => write!(f, "{e}"),
            MigrateError::UnsupportedNewer { found, target } => write!(
                f,
                "generation 스키마 {found}가 이 도구의 스키마 {target}보다 새 버전입니다"
            ),
            MigrateError::Run(e) => write!(f, "migration 실패: {e}"),
        }
    }
}

impl std::error::Error for MigrateError {}

/// migration 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrateOutcome {
    /// 실행 전 상태.
    pub before: SchemaStatus,
    /// 실행 후 revision(아무것도 하지 않았으면 이전과 같음).
    pub revision: ProjectRevision,
    /// 새 generation을 기록했는지.
    pub migrated: bool,
}

/// 스키마 상태를 판정합니다(읽기 전용).
pub fn schema_status(path: &Path, target_schema: u32) -> Result<SchemaStatus, OpenError> {
    let snapshot = open_snapshot(path)?;
    let schema_version = snapshot.generation().map(|g| g.manifest().schema_version);
    let state = match schema_version {
        None => SchemaState::Unsaved,
        Some(v) if v == target_schema => SchemaState::UpToDate,
        Some(v) if v < target_schema => SchemaState::MigrationRequired,
        Some(_) => SchemaState::UnsupportedNewer,
    };
    Ok(SchemaStatus {
        revision: snapshot.revision(),
        schema_version,
        target_schema,
        state,
    })
}

/// 필요하면 `target_schema`까지 migration합니다(`registry`의 경로 사용).
///
/// 이미 최신이거나 저장 전이면 아무것도 쓰지 않습니다. 새 버전이면 [`MigrateError::UnsupportedNewer`].
pub fn migrate(
    path: &Path,
    registry: &MigrationRegistry,
    target_schema: u32,
    lock: &LockPolicy,
) -> Result<MigrateOutcome, MigrateError> {
    let _guard = match lock {
        LockPolicy::Required { dir } => {
            let probe = probe(path).map_err(MigrateError::Open)?;
            let project_id = parse_project_id(&probe.manifest.project.id).ok_or_else(|| {
                MigrateError::Open(OpenError::InvalidProjectId {
                    id: probe.manifest.project.id.clone(),
                })
            })?;
            Some(SaveLock::try_acquire(dir, project_id, &probe.root).map_err(MigrateError::Lock)?)
        }
        LockPolicy::None => None,
    };
    let before = schema_status(path, target_schema).map_err(MigrateError::Open)?;
    match before.state {
        SchemaState::Unsaved | SchemaState::UpToDate => Ok(MigrateOutcome {
            before,
            revision: before.revision,
            migrated: false,
        }),
        SchemaState::UnsupportedNewer => Err(MigrateError::UnsupportedNewer {
            found: before.schema_version.unwrap_or_default(),
            target: target_schema,
        }),
        SchemaState::MigrationRequired => {
            let snapshot = open_snapshot(path).map_err(MigrateError::Open)?;
            let outcome = run_migration(
                snapshot.root(),
                snapshot.project_id(),
                registry,
                target_schema,
            )
            .map_err(|e| MigrateError::Run(e.to_string()))?;
            Ok(MigrateOutcome {
                before,
                // 프로젝트 revision은 새 generation에 stellar.yaml 세트 선택을 더한 값입니다(ADR-0128 결정 2).
                revision: ProjectRevision::Saved {
                    generation: outcome.to_generation,
                    hash: outcome.new_manifest_hash,
                }
                .with_style_defaults(snapshot.style_defaults_name()),
                migrated: true,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        file_digest, random_valid_document, save_documents, scaffold_project, TempDir,
    };
    use stellar_storage::migration::runner::noop_v1_to_v2;

    fn saved(dir: &TempDir) -> std::path::PathBuf {
        let root = dir.path.join("project");
        scaffold_project(&root);
        save_documents(&root, &[&random_valid_document(400)]);
        root
    }

    #[test]
    fn current_schema_is_up_to_date_and_writes_nothing() {
        let dir = TempDir::new("migrate-current");
        let root = saved(&dir);
        let locks = LockPolicy::Required {
            dir: dir.path.join("locks"),
        };
        let before = file_digest(&root);
        let outcome = migrate(&root, &product_registry(), CURRENT_SCHEMA_VERSION, &locks).unwrap();
        assert!(!outcome.migrated);
        assert_eq!(outcome.before.state, SchemaState::UpToDate);
        assert_eq!(outcome.revision, outcome.before.revision);
        assert_eq!(file_digest(&root), before);
    }

    #[test]
    fn older_schema_migrates_to_a_new_generation_and_reopens() {
        // 시험용 v1→v2 noop step(V.8.3)으로 쓰기 경로를 확인합니다.
        let dir = TempDir::new("migrate-old");
        let root = saved(&dir);
        let mut registry = MigrationRegistry::new();
        registry.register(noop_v1_to_v2()).unwrap();
        let status = schema_status(&root, 2).unwrap();
        assert_eq!(status.state, SchemaState::MigrationRequired);
        let outcome = migrate(&root, &registry, 2, &LockPolicy::None).unwrap();
        assert!(outcome.migrated);
        assert_eq!(outcome.revision.generation(), 2);
        let after = schema_status(&root, 2).unwrap();
        assert_eq!(after.state, SchemaState::UpToDate);
        assert_eq!(after.revision, outcome.revision);
        // 이제 이 도구(스키마 1)에는 새 버전입니다.
        assert_eq!(
            schema_status(&root, CURRENT_SCHEMA_VERSION).unwrap().state,
            SchemaState::UnsupportedNewer
        );
        let before = file_digest(&root);
        assert!(matches!(
            migrate(
                &root,
                &product_registry(),
                CURRENT_SCHEMA_VERSION,
                &LockPolicy::None
            ),
            Err(MigrateError::UnsupportedNewer {
                found: 2,
                target: 1
            })
        ));
        assert_eq!(file_digest(&root), before);
    }

    #[test]
    fn migrated_revision_includes_the_style_defaults_selection() {
        // ADR-0128 결정 2: 세트가 `css`가 아닌 프로젝트에서도 migrate 결과 revision은 다시 연 revision과 같아 바로
        // 다음 쓰기의 base로 쓸 수 있습니다.
        let dir = TempDir::new("migrate-selection");
        let root = saved(&dir);
        stellar_project::manifest_style_defaults::set_active(&root, "react-native").unwrap();
        let mut registry = MigrationRegistry::new();
        registry.register(noop_v1_to_v2()).unwrap();
        let outcome = migrate(&root, &registry, 2, &LockPolicy::None).unwrap();
        assert!(outcome.migrated);
        assert_eq!(outcome.revision.generation(), 2);
        let reopened = crate::snapshot::open_snapshot(&root).unwrap();
        assert_ne!(reopened.revision(), reopened.storage_revision());
        assert_eq!(outcome.revision, reopened.revision());
        assert_eq!(schema_status(&root, 2).unwrap().revision, outcome.revision);
    }

    #[test]
    fn missing_migration_path_keeps_the_original_generation() {
        let dir = TempDir::new("migrate-gap");
        let root = saved(&dir);
        let before = file_digest(&root);
        let revision = schema_status(&root, 3).unwrap().revision;
        assert!(matches!(
            migrate(&root, &product_registry(), 3, &LockPolicy::None),
            Err(MigrateError::Run(_))
        ));
        assert_eq!(schema_status(&root, 3).unwrap().revision, revision);
        assert_eq!(file_digest(&root), before);
    }

    #[test]
    fn unsaved_project_needs_nothing() {
        let dir = TempDir::new("migrate-unsaved");
        let root = dir.path.join("project");
        scaffold_project(&root);
        let outcome = migrate(
            &root,
            &product_registry(),
            CURRENT_SCHEMA_VERSION,
            &LockPolicy::None,
        )
        .unwrap();
        assert_eq!(outcome.before.state, SchemaState::Unsaved);
        assert!(!outcome.migrated);
    }
}

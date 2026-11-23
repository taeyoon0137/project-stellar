//! Git 밖 chunk cache 관리 명령(`stellar cache status|rebuild|gc|migrate`, V.10.7, ADR-0122).
//!
//! chunk cache는 Git이 추적하지 않는 `.stellar/cache/chunks/`에 있는 로컬 데이터입니다(위치 함수는
//! `stellar_storage::location` 하나). 이 모듈은 앱 없이 cache를 점검·재생성·정리하는 CLI·MCP 공용 진입점이고, 실제
//! 동작은 `stellar_project::cache`의 같은 함수를 부릅니다(앱 열기·semantic Git과 같은 경로).
//!
//! | 명령 | 동작 | 쓰기 |
//! |---|---|---|
//! | `status` | cache 위치·열기 계획·current·세대 수·옛 배치 여부 | 원본이 있으면 옛 배치를 옮김(옛 pointer는 버림). 원본 없는 옛 배치는 거부 |
//! | `rebuild` | 원본과 짝이 아닌 cache를 원본에서 다시 만듦(clone 직후·Git pull 뒤) | cache·stamp(저장 잠금 안) |
//! | `gc` | 보존 정책(`config/storage-gc.yaml`) 밖 cache 파일을 지움 | cache만(저장 잠금 안) |
//! | `migrate` | 옛 배치 이동 → Git index에서 옛 경로 빼기(원본이 있을 때) → 다시 만들기 → GC | cache·index |
//! | `migrate --convert` | 원본 없는 옛 chunk 프로젝트를 텍스트 원본으로 바꾼 뒤(`stellar_project::cache::convert_legacy`, 앱 열기와 같은 함수) `migrate`와 같은 절차. Git index에는 새 원본을 넣고 옛 경로를 뺌 | 원본·cache·index |
//!
//! 원본 파일(Git 원본)은 `migrate --convert`만 씁니다(ADR-0131 결정 3). 다른 명령은 원본 없는 옛 배치를 거부합니다.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use stellar_project::cache::{
    collect_garbage, default_retention, migrate_working_tree_with, rebuild_from_text, CacheError,
    LegacyLayout,
};
pub use stellar_project::cache::{GcReport, LayoutMigration, LegacyConversion, Rebuild};
use stellar_project::document_io::session::observe;
use stellar_storage::hash::ContentHash;

use crate::lock::{LockError, LockPolicy, SaveLock};

/// cache 명령 실패.
#[derive(Debug)]
pub enum CacheCommandError {
    /// 프로젝트 경로를 확인할 수 없습니다.
    Path(std::io::ErrorKind),
    /// cache를 관찰·쓰기·정리하지 못했습니다.
    Cache(CacheError),
    /// 원본·cache를 관찰하지 못했습니다.
    Observe(String),
    /// 저장 잠금을 잡지 못했습니다(다른 writer가 저장 중).
    Lock(LockError),
}

impl fmt::Display for CacheCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheCommandError::Path(kind) => write!(f, "프로젝트 경로 확인 실패: {kind:?}"),
            CacheCommandError::Cache(error) => write!(f, "{error}"),
            CacheCommandError::Observe(error) => write!(f, "{error}"),
            CacheCommandError::Lock(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CacheCommandError {}

/// `status` 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheStatus {
    /// cache 위치(프로젝트 root 기준 `/` 경로, 항상 `.stellar/cache/chunks`).
    pub location: &'static str,
    /// 열기 계획(`cache`·`forward`·`from-text`·`migrate`·`empty`).
    pub plan: &'static str,
    /// 텍스트 원본이 있는지.
    pub text_source: bool,
    /// current manifest hash.
    pub current: Option<ContentHash>,
    /// cache에 있는 manifest 파일 수.
    pub generations: usize,
    /// 관찰 때 옛 배치를 옮겼는지.
    pub migrated: LayoutMigration,
}

fn canonical(root: &Path) -> Result<PathBuf, CacheCommandError> {
    std::fs::canonicalize(root).map_err(|e| CacheCommandError::Path(e.kind()))
}

fn locked(root: &Path, lock: &LockPolicy) -> Result<Option<SaveLock>, CacheCommandError> {
    match lock {
        LockPolicy::Required { dir } => {
            let project =
                stellar_project::cache::project_id(root).map_err(CacheCommandError::Cache)?;
            SaveLock::try_acquire(dir, project, root)
                .map(Some)
                .map_err(CacheCommandError::Lock)
        }
        LockPolicy::None => Ok(None),
    }
}

fn manifest_count(root: &Path) -> usize {
    let dir =
        stellar_project::cache::cache_dir(root).join(stellar_storage::location::GENERATIONS_DIR);
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".starmanifest"))
            .count()
    })
}

/// cache 상태를 읽습니다. 원본이 있으면 옛 배치를 cache 위치로 옮깁니다(옛 pointer는 버림). 원본 없는 옛 배치는
/// 바꾸지 않고 [`CacheError::LegacyWithoutSource`]입니다(변환은 [`migrate_with`]의 `convert`).
pub fn status(root: &Path) -> Result<CacheStatus, CacheCommandError> {
    let root = canonical(root)?;
    let migrated =
        migrate_working_tree_with(&root, LegacyLayout::Refuse).map_err(CacheCommandError::Cache)?;
    let disk = observe(&root).map_err(|e| CacheCommandError::Observe(e.to_string()))?;
    Ok(CacheStatus {
        location: stellar_project::cache::CACHE_RELATIVE,
        plan: disk.plan.code(),
        text_source: disk.text.is_some(),
        current: disk.current,
        generations: manifest_count(&root),
        migrated,
    })
}

/// 원본과 짝이 아닌 cache를 원본에서 다시 만듭니다(저장 잠금 안). 원본 파일은 쓰지 않으므로 원본 없는 옛 배치는
/// 바꾸지 않고 [`CacheError::LegacyWithoutSource`]입니다(`rebuild_from_text`의 열기 관찰이 변환하기 전에 막음).
pub fn rebuild(root: &Path, lock: &LockPolicy) -> Result<Rebuild, CacheCommandError> {
    let root = canonical(root)?;
    let _guard = locked(&root, lock)?;
    migrate_working_tree_with(&root, LegacyLayout::Refuse).map_err(CacheCommandError::Cache)?;
    rebuild_from_text(&root).map_err(CacheCommandError::Cache)
}

/// 보존 정책 밖 cache 파일을 지웁니다(저장 잠금 안).
pub fn gc(root: &Path, lock: &LockPolicy, now: SystemTime) -> Result<GcReport, CacheCommandError> {
    let root = canonical(root)?;
    let _guard = locked(&root, lock)?;
    collect_garbage(&root, &default_retention(), now).map_err(CacheCommandError::Cache)
}

/// `migrate` 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheMigration {
    /// 옛 배치 이동.
    pub layout: LayoutMigration,
    /// Git index에서 뺀 옛 경로. Git을 쓰지 못했으면 `Err(이유)`입니다(작업 트리 이동·다시 만들기는 계속).
    pub untracked: Result<Vec<String>, String>,
    /// 원본에서 다시 만들기.
    pub rebuilt: Rebuild,
    /// GC.
    pub gc: GcReport,
}

/// [`migrate_with`]`(root, lock, now, false)`: 원본 없는 옛 배치는 바꾸지 않고 거부합니다.
pub fn migrate(
    root: &Path,
    lock: &LockPolicy,
    now: SystemTime,
) -> Result<CacheMigration, CacheCommandError> {
    migrate_with(root, lock, now, false)
}

/// 옛 배치를 Git 밖 cache로 옮기는 전체 절차(저장 잠금 안): 작업 트리 이동, (신뢰한 Git 저장소이고 원본이 있으면)
/// index에서 옛 경로 빼기, 원본에서 cache 다시 만들기, GC. commit은 하지 않습니다(다음 commit이 옛 cache 삭제를 담음).
///
/// `convert`면 원본 없는 옛 chunk 프로젝트를 먼저 텍스트 원본으로 바꿉니다(앱 열기 관찰과 같은
/// `stellar_project::cache::convert_legacy`). 바꾼 원본은 index 정리 단계가 Git index에 넣고, 그때 작업 트리의 옛 배치를
/// cache로 옮깁니다. Git을 쓰지 못하면 옛 배치는 작업 트리에 남습니다([`CacheMigration::untracked`]가 이유).
/// 바꿀 수 없으면 아무것도 바꾸지 않고 오류입니다.
pub fn migrate_with(
    root: &Path,
    lock: &LockPolicy,
    now: SystemTime,
    convert: bool,
) -> Result<CacheMigration, CacheCommandError> {
    let root = canonical(root)?;
    let _guard = locked(&root, lock)?;
    let mode = if convert {
        LegacyLayout::Convert
    } else {
        LegacyLayout::Refuse
    };
    let layout = migrate_working_tree_with(&root, mode).map_err(CacheCommandError::Cache)?;
    let untracked = crate::git::GitSession::open_for_os_user(&root)
        .map_err(|e| e.to_string())
        .and_then(|session| session.migrate_cache().map_err(|e| e.to_string()));
    let rebuilt = rebuild_from_text(&root).map_err(CacheCommandError::Cache)?;
    let gc = collect_garbage(&root, &default_retention(), now).map_err(CacheCommandError::Cache)?;
    Ok(CacheMigration {
        layout,
        untracked,
        rebuilt,
        gc,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        save_documents, save_text_document, scaffold_project, text_document, TempDir,
    };

    #[test]
    fn status_rebuild_and_gc_work_on_a_text_project_without_cache() {
        let dir = TempDir::new("git-cache-commands");
        let root = dir.path.join("project");
        scaffold_project(&root);
        save_text_document(&root, &text_document());
        // clone 직후처럼 로컬 cache를 지웁니다.
        std::fs::remove_dir_all(root.join(".stellar")).unwrap();
        let before = status(&root).unwrap();
        assert_eq!(before.location, ".stellar/cache/chunks");
        assert_eq!(before.plan, "from-text");
        assert!(before.text_source);
        assert_eq!(before.current, None);
        let Rebuild::Rebuilt { generation, .. } = rebuild(&root, &LockPolicy::None).unwrap() else {
            panic!("원본에서 다시 만들어야 함");
        };
        assert_eq!(generation, 1);
        assert_eq!(status(&root).unwrap().plan, "cache");
        assert_eq!(rebuild(&root, &LockPolicy::None).unwrap(), Rebuild::Fresh);
        let report = gc(&root, &LockPolicy::None, SystemTime::now()).unwrap();
        assert_eq!(report.deleted, 0);
        assert_eq!(report.retained.len(), 1);
    }

    /// 원본 없는 옛 chunk 프로젝트: `status`·`rebuild`·`migrate`는 바꾸지 않고 거부하고, `migrate --convert`는 텍스트 원본으로
    /// 바꿉니다(앱 열기와 같은 함수). Git을 쓸 수 없으면 옛 배치는 작업 트리에 남고 그 이유를 돌려줍니다.
    #[test]
    fn migrate_convert_turns_a_legacy_project_into_a_text_source() {
        let dir = TempDir::new("git-cache-convert");
        let root = dir.path.join("project");
        scaffold_project(&root);
        save_documents(&root, &[&text_document()]);
        let cache = stellar_project::cache::cache_dir(&root);
        std::fs::create_dir_all(root.join("design")).unwrap();
        for name in ["current", "generations", "objects"] {
            std::fs::rename(cache.join(name), root.join("design").join(name)).unwrap();
        }
        std::fs::remove_dir_all(root.join(".stellar")).unwrap();
        let refused = |result: Result<_, CacheCommandError>| {
            matches!(
                result,
                Err(CacheCommandError::Cache(CacheError::LegacyWithoutSource))
            )
        };
        assert!(refused(status(&root).map(|_| ())));
        assert!(refused(
            migrate(&root, &LockPolicy::None, SystemTime::now()).map(|_| ())
        ));
        assert!(refused(rebuild(&root, &LockPolicy::None).map(|_| ())));
        assert!(stellar_project::document_io::session::read_source(&root)
            .unwrap()
            .is_none());

        let report = migrate_with(&root, &LockPolicy::None, SystemTime::now(), true).unwrap();
        let converted = report.layout.converted.expect("변환함");
        assert_eq!(converted.documents, 1);
        assert!(converted.written > 0);
        assert!(
            report.untracked.is_err(),
            "신뢰하지 않은 fixture는 Git을 쓰지 않음"
        );
        assert!(
            root.join("design/current").is_file(),
            "옛 배치는 Git에 넣기 전까지 남음"
        );
        assert_eq!(report.rebuilt, Rebuild::Fresh, "변환이 cache 짝을 맞춤");
        let after = status(&root).unwrap();
        assert_eq!(after.plan, "cache");
        assert!(after.text_source);
    }
}

#[cfg(test)]
mod snapshot_tests;

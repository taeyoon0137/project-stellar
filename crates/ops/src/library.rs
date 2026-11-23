//! 외부 library 제품 경로: 공용 cache 열기·진단 (XV.5.5·XXV.2.1, ADR-0127).
//!
//! 앱 창 session(`stellar-runtime`)과 headless 도구(CLI·MCP)가 같은 함수로 외부 library를 다룹니다.
//!
//! | 진입점 | 하는 일 | 호출자 |
//! |---|---|---|
//! | [`LibraryCache::os_default`] | OS 사용자 데이터 `<base>/xyz.taeyoon.stellar/library-cache`를 제품 keyer로 엽니다 | 아래 두 함수 |
//! | [`open_libraries`] | 문서를 연 뒤 lock 기준으로 library를 로드합니다(`open_project_libraries`, 실패하지 않음) | 창 session 열기, [`crate::integrity::diagnose_snapshot`] |
//! | [`ProductLibraries::contribution`] | 부분 열기 진단(missing library)과 lock·cache 문제를 진단 index 기여 하나로 바꿉니다 | `stellar diagnose`·MCP `document.diagnostics`, 창 session |
//!
//! ## 제품 keyer
//!
//! mirror 디렉터리와 privacy index key는 URL 원문이 아니라 keyed identifier입니다(XV.1.4·XV.3.5). [`ProductKeyer`]는
//! cache root의 비밀 key 파일([`KEY_FILE`], 32 bytes, 소유자 전용 `0600`)로 repository 비교 키의 HMAC-SHA256(저장 content
//! hash와 같은 SHA-256)을 계산하고 앞 32 hex를 씁니다. key는 materialize 쪽([`LibraryCache::create_in_app_data`])만 만듭니다.
//! 읽기 쪽은 key가 없으면 저장하지 않는 일회용 key로 엽니다: 그 cache에는 이 keyer로 기록된 entry가 없으므로 모든 node가
//! `MissingCacheEntry`가 됩니다(파일을 만들지 않음).

use std::fmt;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use stellar_dependencies::cache_entry::LibraryEntryStore;
use stellar_dependencies::cache_root::{CacheRootError, LibraryCacheRoot};
use stellar_dependencies::declaration::RepositoryUrl;
use stellar_dependencies::library_snapshot::SnapshotLimits;
use stellar_dependencies::mirror::{RepositoryKey, RepositoryKeyer};
use stellar_integrity::diagnostic::{Diagnostic, DiagnosticKey, DiagnosticKind, Severity};
use stellar_integrity::incremental::{origins, Contribution, Footprint, IntegrityEngine};
use stellar_integrity::remediation::actions_for;
use stellar_integrity::sources;
use stellar_project::library_open::{open_project_libraries, read_lock};
use stellar_storage::hash::ContentHash;

use crate::lock::APP_DATA_DIR;

pub use stellar_dependencies::ref_resolution::LibraryGraphInstance;
pub use stellar_project::library_git::StorageSha256;
pub use stellar_project::library_open::{LibraryOpenProblem, ProjectLibraries};

/// cache root 안 repository keyer 비밀 key 파일.
pub const KEY_FILE: &str = "repository-key";
/// 비밀 key 길이(bytes).
const KEY_LEN: usize = 32;
/// HMAC-SHA256 block 크기.
const BLOCK: usize = 64;

/// cache root의 비밀 key로 repository 비교 키를 keyed hash합니다(모듈 문서 "제품 keyer").
#[derive(Clone)]
pub struct ProductKeyer {
    key: [u8; KEY_LEN],
}

impl fmt::Debug for ProductKeyer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 비밀 key는 출력하지 않습니다.
        f.write_str("ProductKeyer(..)")
    }
}

impl ProductKeyer {
    /// key 파일을 읽습니다. 없으면 `Ok(None)`, 길이가 다르거나 정규 파일이 아니면 `InvalidData`입니다.
    pub fn load(root: &LibraryCacheRoot) -> io::Result<Option<ProductKeyer>> {
        let path = root.path().join(KEY_FILE);
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        if !meta.is_file() || meta.len() != KEY_LEN as u64 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let bytes = std::fs::read(&path)?;
        let key: [u8; KEY_LEN] = bytes
            .try_into()
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        Ok(Some(ProductKeyer { key }))
    }

    /// key 파일을 읽거나, 없으면 OS 난수로 만들어 소유자 전용으로 씁니다(materialize 쪽).
    pub fn load_or_create(root: &LibraryCacheRoot) -> io::Result<ProductKeyer> {
        if let Some(keyer) = ProductKeyer::load(root)? {
            return Ok(keyer);
        }
        std::fs::create_dir_all(root.path())?;
        let keyer = ProductKeyer { key: random_key()? };
        let path = root.path().join(KEY_FILE);
        match create_private(&path) {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(&keyer.key)?;
                file.sync_all()?;
                Ok(keyer)
            }
            // 다른 프로세스가 먼저 만들었으면 그 key를 씁니다.
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                ProductKeyer::load(root)?.ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))
            }
            Err(e) => Err(e),
        }
    }

    /// 저장하지 않는 일회용 key(읽기 쪽에서 key 파일이 없을 때).
    fn ephemeral() -> io::Result<ProductKeyer> {
        Ok(ProductKeyer { key: random_key()? })
    }
}

impl RepositoryKeyer for ProductKeyer {
    fn repository_key(&self, repository: &RepositoryUrl) -> RepositoryKey {
        let digest = hmac_sha256(&self.key, repository.comparison_key().as_bytes());
        let hex: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
        RepositoryKey::from_hex(&hex).expect("32 hex")
    }
}

/// HMAC-SHA256(RFC 2104). key는 block 크기 이하만 받습니다(제품 key는 32 bytes).
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    debug_assert!(key.len() <= BLOCK);
    let mut block = [0u8; BLOCK];
    block[..key.len()].copy_from_slice(key);
    let mut inner = Vec::with_capacity(BLOCK + message.len());
    inner.extend(block.iter().map(|b| b ^ 0x36));
    inner.extend_from_slice(message);
    let inner = ContentHash::compute(&inner);
    let mut outer = Vec::with_capacity(BLOCK + 32);
    outer.extend(block.iter().map(|b| b ^ 0x5c));
    outer.extend_from_slice(inner.digest());
    *ContentHash::compute(&outer).digest()
}

#[cfg(unix)]
fn random_key() -> io::Result<[u8; KEY_LEN]> {
    use std::io::Read;
    let mut key = [0u8; KEY_LEN];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut key)?;
    Ok(key)
}

#[cfg(not(unix))]
fn random_key() -> io::Result<[u8; KEY_LEN]> {
    // Windows 지원 재개 때 OS 난수 adapter를 더합니다(ADR-0006).
    Err(io::ErrorKind::Unsupported.into())
}

#[cfg(unix)]
fn create_private(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private(path: &Path) -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// 제품 SHA-256·keyer로 연 cache entry store.
pub type ProductEntryStore = LibraryEntryStore<StorageSha256, ProductKeyer>;

/// 공용 library cache를 열지 못한 이유. 경로 원문은 담지 않습니다(XV.1.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryCacheError {
    /// OS 사용자 데이터 위치를 정하는 환경 변수가 없습니다.
    NoUserDataDir,
    /// cache root 경로가 올바르지 않습니다.
    Root(CacheRootError),
    /// keyer 비밀 key를 읽거나 만들지 못했습니다.
    Key {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
}

impl fmt::Display for LibraryCacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LibraryCacheError::NoUserDataDir => {
                f.write_str("OS 사용자 데이터 위치를 정할 수 없어 library cache를 열지 못했습니다")
            }
            LibraryCacheError::Root(e) => write!(f, "library cache 위치 오류: {e}"),
            LibraryCacheError::Key { kind } => {
                write!(f, "library cache key를 읽지 못했습니다: {kind:?}")
            }
        }
    }
}

impl std::error::Error for LibraryCacheError {}

/// 공용 library cache(제품 keyer로 연 entry store).
pub struct LibraryCache {
    store: ProductEntryStore,
}

impl fmt::Debug for LibraryCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // cache 경로는 민감값이라 출력하지 않습니다(XV.1.4).
        f.write_str("LibraryCache(..)")
    }
}

impl LibraryCache {
    /// OS 사용자 데이터 `<base>/xyz.taeyoon.stellar` 아래 cache를 읽기용으로 엽니다(파일을 만들지 않음).
    pub fn os_default() -> Result<LibraryCache, LibraryCacheError> {
        let base = stellar_project::trust_store::os_user_data_base()
            .ok_or(LibraryCacheError::NoUserDataDir)?;
        LibraryCache::in_app_data(&base.join(APP_DATA_DIR))
    }

    /// `app_data_dir` 아래 cache를 읽기용으로 엽니다. key 파일이 없으면 일회용 key입니다(모듈 문서).
    pub fn in_app_data(app_data_dir: &Path) -> Result<LibraryCache, LibraryCacheError> {
        let root = LibraryCacheRoot::in_app_data(app_data_dir).map_err(LibraryCacheError::Root)?;
        let keyer = match ProductKeyer::load(&root) {
            Ok(Some(keyer)) => Ok(keyer),
            Ok(None) => ProductKeyer::ephemeral(),
            Err(e) => Err(e),
        }
        .map_err(|e| LibraryCacheError::Key { kind: e.kind() })?;
        Ok(LibraryCache {
            store: LibraryEntryStore::new(root, StorageSha256, keyer),
        })
    }

    /// `app_data_dir` 아래 cache를 materialize용으로 엽니다: 디렉터리와 key 파일을 만듭니다.
    pub fn create_in_app_data(app_data_dir: &Path) -> Result<LibraryCache, LibraryCacheError> {
        let root = LibraryCacheRoot::in_app_data(app_data_dir).map_err(LibraryCacheError::Root)?;
        root.create_dirs().map_err(LibraryCacheError::Root)?;
        let keyer = ProductKeyer::load_or_create(&root)
            .map_err(|e| LibraryCacheError::Key { kind: e.kind() })?;
        Ok(LibraryCache {
            store: LibraryEntryStore::new(root, StorageSha256, keyer),
        })
    }

    /// entry store.
    pub fn store(&self) -> &ProductEntryStore {
        &self.store
    }

    /// entry store(materialize).
    pub fn store_mut(&mut self) -> &mut ProductEntryStore {
        &mut self.store
    }

    /// cache root.
    pub fn root(&self) -> &LibraryCacheRoot {
        self.store.root()
    }
}

/// 프로젝트를 연 뒤의 외부 library 상태.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProductLibraries {
    opened: Option<ProjectLibraries>,
    lock_problem: Option<LibraryOpenProblem>,
    cache_problem: Option<LibraryCacheError>,
}

impl ProductLibraries {
    /// library가 없는 상태(lock 없는 프로젝트).
    pub fn none() -> ProductLibraries {
        ProductLibraries::default()
    }

    /// `open_project_libraries` 결과(cache를 열지 못했거나 lock이 없으면 `None`).
    pub fn libraries(&self) -> Option<&ProjectLibraries> {
        self.opened.as_ref()
    }

    /// 로드한 graph instance(없으면 빈 instance).
    pub fn instance(&self) -> LibraryGraphInstance {
        self.opened
            .as_ref()
            .map(ProjectLibraries::instance)
            .unwrap_or_default()
    }

    /// cache를 열지 못한 이유(lock에 의존성이 있을 때만).
    pub fn cache_problem(&self) -> Option<&LibraryCacheError> {
        self.cache_problem.as_ref()
    }

    /// lock 자체의 문제.
    pub fn lock_problem(&self) -> Option<&LibraryOpenProblem> {
        self.opened
            .as_ref()
            .and_then(ProjectLibraries::problem)
            .or(self.lock_problem.as_ref())
    }

    /// 모든 의존성을 사용할 수 있는지.
    pub fn is_complete(&self) -> bool {
        self.cache_problem.is_none()
            && self.lock_problem.is_none()
            && self
                .opened
                .as_ref()
                .is_none_or(ProjectLibraries::is_complete)
    }

    /// 진단 index 기여(프로젝트 단위, origin [`origins::LIBRARIES`]): 사용할 수 없는 library node(`sources::libraries`)와
    /// lock·cache 문제입니다. 문서 변경과 무관합니다.
    pub fn contribution(&self) -> Contribution {
        let unavailable: Vec<_> = self
            .opened
            .as_ref()
            .and_then(ProjectLibraries::loaded)
            .map(|loaded| loaded.unavailable().cloned().collect())
            .unwrap_or_default();
        let mut contribution = sources::libraries(&unavailable);
        if let Some(problem) = self.lock_problem() {
            let code = match problem {
                LibraryOpenProblem::LockUnreadable { .. } => "library-lock/unreadable",
                LibraryOpenProblem::LockInvalid => "library-lock/invalid",
                LibraryOpenProblem::LockStructure => "library-lock/structure",
            };
            contribution.diagnostics.push(project_diagnostic(
                code,
                "stellar.lock.yaml",
                Severity::Error,
                problem.to_string(),
            ));
        }
        if let Some(problem) = &self.cache_problem {
            contribution.diagnostics.push(project_diagnostic(
                "library-cache/unavailable",
                "library-cache",
                Severity::Warning,
                problem.to_string(),
            ));
        }
        contribution.footprint = Footprint::independent();
        contribution
    }

    /// [`ProductLibraries::contribution`]을 엔진에 프로젝트 단위로 넣습니다.
    pub fn submit_to(&self, engine: &mut IntegrityEngine) -> bool {
        engine.submit(None, origins::LIBRARIES, self.contribution())
    }
}

fn project_diagnostic(
    code: &str,
    subject: &str,
    severity: Severity,
    message: String,
) -> Diagnostic {
    let kind = DiagnosticKind::MissingLibrary;
    Diagnostic {
        remediation: actions_for(kind, None, None, None),
        key: DiagnosticKey {
            document: None,
            kind,
            source: None,
            property: None,
            code: code.to_string(),
            subject: subject.to_string(),
            origin: origins::LIBRARIES.to_string(),
        },
        severity,
        message,
    }
}

/// 현재 시각(Unix ms). cache index 접근 시각 기록에 씁니다.
pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// `project_root`의 lock 기준으로 외부 library를 로드합니다. 실패하지 않습니다.
///
/// lock이 없으면 cache를 건드리지 않습니다. cache를 열지 못했으면 lock에 의존성이 있을 때만 그 문제를 남깁니다.
pub fn open_libraries(
    project_root: &Path,
    cache: Result<&LibraryCache, &LibraryCacheError>,
    now_unix_ms: u64,
) -> ProductLibraries {
    match read_lock(project_root) {
        Ok(None) => return ProductLibraries::none(),
        Ok(Some(lock)) => {
            if let Err(problem) = cache {
                return ProductLibraries {
                    opened: None,
                    lock_problem: None,
                    cache_problem: (!lock.entries().is_empty()).then(|| problem.clone()),
                };
            }
        }
        Err(problem) => {
            return ProductLibraries {
                opened: None,
                lock_problem: Some(problem),
                cache_problem: None,
            }
        }
    }
    let Ok(cache) = cache else {
        unreachable!("cache 오류는 위에서 처리");
    };
    ProductLibraries {
        opened: Some(open_project_libraries(
            project_root,
            cache.store(),
            &SnapshotLimits::default(),
            now_unix_ms,
        )),
        lock_problem: None,
        cache_problem: None,
    }
}

/// OS 기본 cache([`LibraryCache::os_default`])로 [`open_libraries`]를 부릅니다. lock이 없으면 cache를 열지 않습니다.
pub fn open_libraries_default(project_root: &Path) -> ProductLibraries {
    if matches!(read_lock(project_root), Ok(None)) {
        return ProductLibraries::none();
    }
    let cache = LibraryCache::os_default();
    open_libraries(project_root, cache.as_ref(), now_unix_ms())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn hmac_matches_rfc_4231_test_vectors() {
        // RFC 4231 test case 2.
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // RFC 4231 test case 1(key 0x0b × 20).
        assert_eq!(
            hex(&hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    fn temp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "stellar-ops-library-{tag}-{}-{}",
            std::process::id(),
            now_unix_ms()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn keyer_key_is_created_once_private_and_reused() {
        let dir = temp("keyer");
        let root = LibraryCacheRoot::in_app_data(&dir).unwrap();
        assert!(ProductKeyer::load(&root).unwrap().is_none());
        let a = ProductKeyer::load_or_create(&root).unwrap();
        let b = ProductKeyer::load_or_create(&root).unwrap();
        assert_eq!(a.key, b.key);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(root.path().join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let url = RepositoryUrl::new("https://example.invalid/DS.git").unwrap();
        let same = RepositoryUrl::new("HTTPS://EXAMPLE.invalid/DS/").unwrap();
        let key = a.repository_key(&url);
        assert_eq!(key, b.repository_key(&same), "같은 비교 키는 같은 key");
        assert_eq!(key.as_str().len(), 32);
        assert!(!format!("{a:?}").contains(&hex(&a.key)), "Debug에 key 없음");
        // 다른 key는 다른 identifier(URL만으로 이름을 추측할 수 없음).
        let other = ProductKeyer { key: [7; KEY_LEN] };
        assert_ne!(other.repository_key(&url), key);
        // 길이가 다른 key 파일은 손상입니다.
        std::fs::write(root.path().join(KEY_FILE), b"short").unwrap();
        assert_eq!(
            ProductKeyer::load(&root).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reading_cache_does_not_create_files() {
        let dir = temp("read");
        let cache = LibraryCache::in_app_data(&dir).unwrap();
        assert!(
            !cache.root().path().exists(),
            "읽기 열기는 cache를 만들지 않음"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn project_without_lock_has_no_libraries_and_no_diagnostics() {
        let dir = temp("no-lock");
        let opened = open_libraries(&dir, Err(&LibraryCacheError::NoUserDataDir), 1);
        assert!(opened.is_complete());
        assert!(opened.contribution().diagnostics.is_empty());
        assert_eq!(opened.instance().snapshots().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_lock_becomes_project_error_diagnostic() {
        let dir = temp("broken-lock");
        std::fs::write(dir.join("stellar.lock.yaml"), "schema_version: [\n").unwrap();
        let opened = open_libraries(&dir, Err(&LibraryCacheError::NoUserDataDir), 1);
        assert_eq!(
            opened.lock_problem(),
            Some(&LibraryOpenProblem::LockInvalid)
        );
        let diagnostics = opened.contribution().diagnostics;
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code(), "library-lock/invalid");
        assert_eq!(diagnostics[0].severity, Severity::Error);
        assert_eq!(diagnostics[0].kind(), DiagnosticKind::MissingLibrary);
        let mut engine = IntegrityEngine::new();
        assert!(opened.submit_to(&mut engine));
        assert_eq!(engine.index().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

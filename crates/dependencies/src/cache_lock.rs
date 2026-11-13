//! library cache read lock과 fetch/write lock 분리 (XV.3.6).
//!
//! 여러 Stellar 프로젝트(프로세스)가 같은 공용 cache를 씁니다. lock은 역할별로 나눠, 읽는 쪽이
//! 오래 걸리는 fetch·materialize를 기다리지 않게 합니다. 모두 OS advisory file lock(`flock`)이며
//! lock 파일은 `<cache root>/locks/` 아래에 있고 지우지 않습니다(지우면 다른 프로세스가 옛 inode를
//! 잠그는 경쟁이 생김).
//!
//! | lock | 파일 | 모드 | 잡는 쪽 | 보유 시간 |
//! |---|---|---|---|---|
//! | entry read | `sha256-<hex>.entry.lock` | 공유 | entry를 읽는 프로젝트 | 사용하는 동안 |
//! | entry delete | 같은 파일 | 배타(try) | cache 정리(XV.8) | 삭제 동안 |
//! | fetch | `fetch-<repository key>.lock` | 배타 | mirror `init`·`fetch`(XV.3.2) | fetch 동안 |
//! | index | `index.lock` | 배타 | index·source config read-modify-write | 짧음 |
//!
//! - **read는 fetch/write와 독립**: entry를 여는 쪽은 entry read lock(공유)과 짧은 index lock만
//!   잡습니다. 같은 repository를 다른 프로세스가 fetch하거나 새 entry를 materialize하는 동안에도
//!   이미 공개된 entry를 읽을 수 있습니다(공개된 entry는 immutable, XV.3.3).
//! - **materialize는 entry 단위 배타 lock이 필요 없음**: staging은 프로세스마다 고유하고 공개는
//!   `rename` 한 번이며, 같은 내용이 먼저 공개되면 `ContentHit`으로 처리합니다.
//! - **중첩 금지**: 한 프로세스는 fetch lock을 잡은 채 index lock을 기다리지 않습니다(같은 프로세스의
//!   다른 fd끼리도 `flock`은 충돌하므로 중첩하면 스스로 막힙니다).

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use crate::cache_root::LibraryCacheRoot;
use crate::content_hash::{ContentHashText, SHA256_PREFIX};
use crate::mirror::RepositoryKey;

/// cache root 아래 lock 디렉터리 이름.
pub const LOCKS_DIR: &str = "locks";
/// index lock 파일 이름.
pub const INDEX_LOCK_FILE: &str = "index.lock";

/// lock 실패.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheLockError {
    /// `try_*`에서 다른 보유자가 있어 즉시 얻지 못함.
    Busy,
    /// lock 파일 열기·잠금 실패.
    Io {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
}

impl fmt::Display for CacheLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheLockError::Busy => f.write_str("cache lock을 다른 프로세스가 보유 중입니다"),
            CacheLockError::Io { kind } => write!(f, "cache lock 실패: {kind:?}"),
        }
    }
}

impl std::error::Error for CacheLockError {}

/// lock 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheLockKind {
    /// entry read(공유).
    EntryRead,
    /// entry delete(배타).
    EntryDelete,
    /// repository fetch(배타).
    Fetch,
    /// index read-modify-write(배타).
    Index,
}

/// 보유 중인 lock. drop하면 풀립니다(파일을 닫으면 `flock`이 해제됨).
#[derive(Debug)]
pub struct CacheLockGuard {
    file: File,
    kind: CacheLockKind,
}

impl CacheLockGuard {
    /// lock 종류.
    pub fn kind(&self) -> CacheLockKind {
        self.kind
    }
}

impl Drop for CacheLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// cache lock 디렉터리.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheLocks {
    dir: PathBuf,
}

/// 잠금 방식.
#[derive(Clone, Copy)]
enum Mode {
    Shared,
    Exclusive,
}

impl CacheLocks {
    /// cache root의 lock 디렉터리(`<root>/locks`). 디스크는 잠글 때 만듭니다.
    pub fn new(root: &LibraryCacheRoot) -> CacheLocks {
        CacheLocks {
            dir: root.path().join(LOCKS_DIR),
        }
    }

    /// lock 디렉터리.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// entry lock 파일 경로.
    pub fn entry_lock_path(&self, hash: &ContentHashText) -> PathBuf {
        let hex = hash
            .as_str()
            .strip_prefix(SHA256_PREFIX)
            .unwrap_or(hash.as_str());
        self.dir.join(format!("sha256-{hex}.entry.lock"))
    }

    fn fetch_lock_path(&self, key: &RepositoryKey) -> PathBuf {
        self.dir.join(format!("fetch-{key}.lock"))
    }

    fn open(&self, path: &Path) -> Result<File, CacheLockError> {
        let io = |e: std::io::Error| CacheLockError::Io { kind: e.kind() };
        std::fs::create_dir_all(&self.dir).map_err(io)?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(io)
    }

    fn acquire(
        &self,
        path: &Path,
        mode: Mode,
        blocking: bool,
        kind: CacheLockKind,
    ) -> Result<CacheLockGuard, CacheLockError> {
        let file = self.open(path)?;
        let result = match (mode, blocking) {
            (Mode::Shared, true) => file.lock_shared().map_err(TryLockError::Error),
            (Mode::Exclusive, true) => file.lock().map_err(TryLockError::Error),
            (Mode::Shared, false) => file.try_lock_shared(),
            (Mode::Exclusive, false) => file.try_lock(),
        };
        match result {
            Ok(()) => Ok(CacheLockGuard { file, kind }),
            Err(TryLockError::WouldBlock) => Err(CacheLockError::Busy),
            Err(TryLockError::Error(e)) => Err(CacheLockError::Io { kind: e.kind() }),
        }
    }

    /// entry read lock(공유, 삭제 중이면 기다림). 여러 프로세스가 동시에 보유할 수 있습니다.
    pub fn read_entry(&self, hash: &ContentHashText) -> Result<CacheLockGuard, CacheLockError> {
        self.acquire(
            &self.entry_lock_path(hash),
            Mode::Shared,
            true,
            CacheLockKind::EntryRead,
        )
    }

    /// entry read lock을 기다리지 않고 시도합니다(삭제 중이면 `Busy`).
    pub fn try_read_entry(&self, hash: &ContentHashText) -> Result<CacheLockGuard, CacheLockError> {
        self.acquire(
            &self.entry_lock_path(hash),
            Mode::Shared,
            false,
            CacheLockKind::EntryRead,
        )
    }

    /// entry delete lock(배타)을 시도합니다. 읽는 프로세스가 하나라도 있으면 `Busy`입니다.
    pub fn try_delete_entry(
        &self,
        hash: &ContentHashText,
    ) -> Result<CacheLockGuard, CacheLockError> {
        self.acquire(
            &self.entry_lock_path(hash),
            Mode::Exclusive,
            false,
            CacheLockKind::EntryDelete,
        )
    }

    /// repository fetch lock(배타, 기다림).
    pub fn fetch(&self, key: &RepositoryKey) -> Result<CacheLockGuard, CacheLockError> {
        self.acquire(
            &self.fetch_lock_path(key),
            Mode::Exclusive,
            true,
            CacheLockKind::Fetch,
        )
    }

    /// repository fetch lock을 기다리지 않고 시도합니다.
    pub fn try_fetch(&self, key: &RepositoryKey) -> Result<CacheLockGuard, CacheLockError> {
        self.acquire(
            &self.fetch_lock_path(key),
            Mode::Exclusive,
            false,
            CacheLockKind::Fetch,
        )
    }

    /// index lock(배타, 기다림). index·source config read-modify-write 동안만 보유합니다.
    pub fn index(&self) -> Result<CacheLockGuard, CacheLockError> {
        self.acquire(
            &self.dir.join(INDEX_LOCK_FILE),
            Mode::Exclusive,
            true,
            CacheLockKind::Index,
        )
    }

    /// index lock을 기다리지 않고 시도합니다.
    pub fn try_index(&self) -> Result<CacheLockGuard, CacheLockError> {
        self.acquire(
            &self.dir.join(INDEX_LOCK_FILE),
            Mode::Exclusive,
            false,
            CacheLockKind::Index,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::cache_entry::LibraryEntryStore;
    use crate::declaration::{CommitId, RepositoryUrl};
    use crate::mirror::{MirrorCache, RepositoryKeyer};
    use crate::test_support::{FakeSha256, FnvKeyer, SystemGit, TempDir};

    /// 자식 프로세스 역할을 고르는 환경 변수(부모가 `Command::env`로만 설정).
    const ROLE_ENV: &str = "STELLAR_CACHE_LOCK_ROLE";
    const DIR_ENV: &str = "STELLAR_CACHE_LOCK_DIR";
    const NAME_ENV: &str = "STELLAR_CACHE_LOCK_NAME";
    const TEST_NAME: &str =
        "cache_lock::tests::two_process_readers_share_entries_while_fetch_is_held";
    const WAIT: Duration = Duration::from_secs(30);

    fn wait_for(path: &Path) {
        let start = Instant::now();
        while !path.exists() {
            assert!(start.elapsed() < WAIT, "대기 시간 초과: {}", path.display());
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn store(root: &LibraryCacheRoot) -> LibraryEntryStore<FakeSha256, FnvKeyer> {
        LibraryEntryStore::new(root.clone(), FakeSha256, FnvKeyer::new(3))
    }

    /// 자식 역할 실행. `work`는 cache root가 있는 작업 디렉터리입니다.
    fn run_child(role: &str, work: &Path, name: &str) {
        let root = LibraryCacheRoot::in_app_data(&work.join("app")).unwrap();
        let locks = CacheLocks::new(&root);
        let url = RepositoryUrl::new(work.join("lib").to_str().unwrap()).unwrap();
        let commit: CommitId = std::fs::read_to_string(work.join("commit"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let ready = work.join(format!("{name}.ready"));
        let release = work.join(format!("{name}.release"));
        match role {
            // 공유 read lock을 잡고 entry를 연 채로 대기.
            "read" => {
                let opened = store(&root)
                    .open_entry(&url, &commit, 10)
                    .unwrap()
                    .expect("materialize된 entry");
                let bytes = std::fs::read(opened.dir().join("stellar.yaml")).unwrap();
                assert_eq!(bytes, b"schema_version: 1\n");
                std::fs::write(&ready, opened.content_hash().as_str()).unwrap();
                wait_for(&release);
                drop(opened);
            }
            // repository fetch lock을 잡고 대기(오래 걸리는 fetch 흉내).
            "fetch" => {
                let key = FnvKeyer::new(3).repository_key(&url);
                let guard = locks.fetch(&key).unwrap();
                std::fs::write(&ready, "fetch").unwrap();
                wait_for(&release);
                drop(guard);
            }
            other => panic!("알 수 없는 역할 {other}"),
        }
    }

    /// 부모 테스트가 중간에 실패해도 자식 프로세스를 남기지 않는 guard.
    struct ChildProcess(Child);

    impl ChildProcess {
        fn wait_success(mut self) -> bool {
            self.0.wait().map(|s| s.success()).unwrap_or(false)
        }
    }

    impl Drop for ChildProcess {
        fn drop(&mut self) {
            if let Ok(None) = self.0.try_wait() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }

    /// 이 테스트 바이너리를 자식 역할로 다시 실행합니다(역할은 `Command::env`로만 전달).
    fn spawn(role: &str, work: &Path, name: &str) -> ChildProcess {
        let child = Command::new(std::env::current_exe().unwrap())
            .args([TEST_NAME, "--exact", "--test-threads=1", "--quiet"])
            .env(ROLE_ENV, role)
            .env(DIR_ENV, work)
            .env(NAME_ENV, name)
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("자식 테스트 프로세스");
        ChildProcess(child)
    }

    #[test]
    fn two_process_readers_share_entries_while_fetch_is_held() {
        // 자식 프로세스로 실행된 경우: 역할만 수행하고 끝냅니다.
        if let Some(role) = std::env::var_os(ROLE_ENV) {
            let work = std::env::var_os(DIR_ENV).expect("작업 디렉터리");
            let name = std::env::var(NAME_ENV).expect("이름");
            run_child(role.to_str().unwrap(), Path::new(&work), &name);
            return;
        }

        let tmp = TempDir::new("cache-lock");
        let Some(mut git) = SystemGit::find(tmp.path()) else {
            return;
        };
        let work = tmp.path();
        let lib = work.join("lib");
        std::fs::create_dir_all(lib.join("design")).unwrap();
        git.ok(&lib, ["init", "-q", "-b", "main"]);
        git.ok(&lib, ["config", "user.name", "Fixture"]);
        git.ok(&lib, ["config", "user.email", "fixture@example.com"]);
        git.ok(&lib, ["config", "commit.gpgsign", "false"]);
        std::fs::write(lib.join("stellar.yaml"), "schema_version: 1\n").unwrap();
        std::fs::write(lib.join("design/chunk"), [0u8, 1, 2, 3]).unwrap();
        git.ok(&lib, ["add", "-A"]);
        git.ok(&lib, ["commit", "-q", "-m", "v1"]);
        let commit: CommitId = git.ok(&lib, ["rev-parse", "HEAD"]).trim().parse().unwrap();
        std::fs::write(work.join("commit"), commit.as_str()).unwrap();
        let url = RepositoryUrl::new(lib.to_str().unwrap()).unwrap();
        let root = LibraryCacheRoot::in_app_data(&work.join("app")).unwrap();
        let locks = CacheLocks::new(&root);

        let mut runner = SystemGit::find(work).unwrap();
        let mut mirrors = MirrorCache::new(
            root.clone(),
            SystemGit::find(work).unwrap(),
            FnvKeyer::new(3),
        );
        let entry = store(&root)
            .materialize(&mut runner, &mut mirrors, &url, &commit, 1)
            .unwrap();

        // 1) 두 자식 프로세스가 같은 entry의 공유 read lock을 동시에 보유하고, 부모도 함께 읽음.
        let reader_a = spawn("read", work, "a");
        let reader_b = spawn("read", work, "b");
        wait_for(&work.join("a.ready"));
        wait_for(&work.join("b.ready"));
        for name in ["a", "b"] {
            assert_eq!(
                std::fs::read_to_string(work.join(format!("{name}.ready"))).unwrap(),
                entry.content_hash.as_str()
            );
        }
        let parent_read = locks.try_read_entry(&entry.content_hash).unwrap();
        // 읽는 프로세스가 있는 동안 삭제(배타) 시도는 실패.
        assert_eq!(
            locks.try_delete_entry(&entry.content_hash).unwrap_err(),
            CacheLockError::Busy
        );

        // 2) 다른 프로세스가 같은 repository의 fetch lock을 잡고 있어도 entry 열기는 기다리지 않음.
        let fetcher = spawn("fetch", work, "f");
        wait_for(&work.join("f.ready"));
        let key = FnvKeyer::new(3).repository_key(&url);
        assert_eq!(locks.try_fetch(&key).unwrap_err(), CacheLockError::Busy);
        let start = Instant::now();
        let opened = store(&root).open_entry(&url, &commit, 20).unwrap().unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
        assert_eq!(opened.content_hash(), &entry.content_hash);
        // fetch lock은 read와 무관: index lock도 짧게만 쓰이므로 즉시 얻을 수 있음.
        drop(locks.try_index().unwrap());

        // 3) 모두 해제하면 삭제 lock을 얻을 수 있음.
        for name in ["a", "b", "f"] {
            std::fs::write(work.join(format!("{name}.release")), "").unwrap();
        }
        for child in [reader_a, reader_b, fetcher] {
            assert!(child.wait_success(), "자식 역할 실패");
        }
        assert_eq!(
            locks.try_delete_entry(&entry.content_hash).unwrap_err(),
            CacheLockError::Busy,
            "부모의 read lock이 아직 남아 있음"
        );
        drop(parent_read);
        drop(opened);
        let delete = locks.try_delete_entry(&entry.content_hash).unwrap();
        assert_eq!(delete.kind(), CacheLockKind::EntryDelete);
        // 삭제 lock을 보유하는 동안 새 reader는 기다리지 않고 Busy.
        assert_eq!(
            locks.try_read_entry(&entry.content_hash).unwrap_err(),
            CacheLockError::Busy
        );
    }

    #[test]
    fn lock_files_live_under_cache_root_and_kinds_conflict_as_documented() {
        let tmp = TempDir::new("cache-lock-kinds");
        let root = LibraryCacheRoot::in_app_data(tmp.path()).unwrap();
        let locks = CacheLocks::new(&root);
        let hash = ContentHashText::from_sha256_digest([7; 32]);
        let first = locks.try_read_entry(&hash).unwrap();
        let second = locks.try_read_entry(&hash).unwrap();
        assert_eq!(first.kind(), CacheLockKind::EntryRead);
        assert_eq!(
            locks.try_delete_entry(&hash).unwrap_err(),
            CacheLockError::Busy
        );
        drop((first, second));
        assert!(locks.try_delete_entry(&hash).is_ok());
        let key = RepositoryKey::from_hex("0123456789abcdef").unwrap();
        let fetch = locks.try_fetch(&key).unwrap();
        // fetch lock은 index·entry lock과 독립.
        assert!(locks.try_index().is_ok());
        assert!(locks.try_read_entry(&hash).is_ok());
        assert_eq!(locks.try_fetch(&key).unwrap_err(), CacheLockError::Busy);
        drop(fetch);
        assert!(locks.try_fetch(&key).is_ok());
        assert!(locks.entry_lock_path(&hash).starts_with(root.path()));
        assert!(locks.dir().starts_with(root.path()));
    }
}

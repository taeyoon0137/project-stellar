//! 열린 창의 외부 library 재로드 판정: lock·매니페스트·cache 지문 (XV.7.4, ADR-0137 결정 5).
//!
//! 창 session은 문서를 연 뒤 library를 한 번 로드합니다([`crate::library::open_libraries`]). 그 뒤 다른 프로세스(CLI
//! `stellar library add|update|fetch|import`, 다른 창)가 lock·`stellar.yaml`·공용 cache를 바꾸면 [`LibraryStamp`]가 달라지고,
//! 창은 library를 다시 로드합니다. 지문은 파일 내용을 읽지 않고 metadata(크기·수정 시각·inode)만 봅니다.
//!
//! | 대상 | 바뀌는 때 |
//! |---|---|
//! | `stellar.lock.yaml` | lock 갱신(add·update) |
//! | `stellar.yaml` | 의존성 선언·저장된 외부 component 참조 변경(add·update·import) |
//! | cache `repository-key` | cache key 생성(첫 materialize) |
//! | cache `entries/` 디렉터리 | entry 공개(rename)·정리 |
//! | cache index(`index.stlcache`) | 새 `(repository, commit)` 기록. **로드가 완전하지 않을 때만** 봅니다 |
//!
//! cache index는 entry를 열 때마다 최근 사용 시각을 기록하므로(XV.3.3 `touch`) 모든 library를 로드한 상태에서 index를 보면
//! 다른 창이 library를 열 때마다 다시 로드합니다. 그래서 index는 받지 못한 node가 있을 때만(같은 내용의 entry가 이미 있어
//! entry 디렉터리가 바뀌지 않는 `ContentHit` 경우를 잡기 위해) 지문에 넣습니다.

use std::path::{Path, PathBuf};

use stellar_dependencies::cache_root::LibraryCacheRoot;
use stellar_project::lockfile_write::LOCK_FILE;
use stellar_project::root::MANIFEST_FILE;

use crate::library::KEY_FILE;
use crate::lock::APP_DATA_DIR;

/// 파일 하나의 metadata 지문.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStamp {
    /// 없음.
    Missing,
    /// 있음(크기, 수정 시각 ns, inode).
    Present {
        /// 크기.
        len: u64,
        /// 수정 시각(Unix ns, 알 수 없으면 0).
        modified_ns: u128,
        /// inode(알 수 없으면 0).
        inode: u64,
    },
    /// metadata를 읽지 못함(권한 등). 매번 같은 값이라 재로드를 일으키지 않습니다.
    Unreadable,
}

impl FileStamp {
    /// `path`의 지문(symlink는 따르지 않음).
    pub fn of(path: &Path) -> FileStamp {
        match std::fs::symlink_metadata(path) {
            Ok(meta) => FileStamp::Present {
                len: meta.len(),
                modified_ns: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos()),
                inode: inode(&meta),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileStamp::Missing,
            Err(_) => FileStamp::Unreadable,
        }
    }
}

#[cfg(unix)]
fn inode(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.ino()
}

#[cfg(not(unix))]
fn inode(_: &std::fs::Metadata) -> u64 {
    0
}

/// 외부 library 로드 결과가 기대는 파일들의 지문(모듈 문서 표).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryStamp {
    lock: FileStamp,
    manifest: FileStamp,
    key: FileStamp,
    entries: FileStamp,
    index: Option<FileStamp>,
}

/// 창이 library를 읽는 cache 위치.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheLocation {
    /// OS 사용자 데이터 기본 위치(호출할 때마다 환경에서 정함).
    OsDefault,
    /// `app_data_dir` 아래(시험·cache 위치를 아는 호출자).
    AppData(PathBuf),
}

impl CacheLocation {
    /// app data 디렉터리. OS 사용자 데이터 위치를 정할 수 없으면 `None`.
    pub fn app_data_dir(&self) -> Option<PathBuf> {
        match self {
            CacheLocation::OsDefault => {
                stellar_project::trust_store::os_user_data_base().map(|b| b.join(APP_DATA_DIR))
            }
            CacheLocation::AppData(dir) => Some(dir.clone()),
        }
    }

    /// cache root 경로.
    pub fn cache_root(&self) -> Option<PathBuf> {
        self.app_data_dir()
            .and_then(|dir| LibraryCacheRoot::in_app_data(&dir).ok())
            .map(|root| root.path().to_path_buf())
    }
}

impl LibraryStamp {
    /// 지금 지문. `complete`는 직전 로드가 모든 node를 로드했는지입니다(아니면 cache index도 봄).
    pub fn now(project_root: &Path, cache: &CacheLocation, complete: bool) -> LibraryStamp {
        let root = cache.cache_root();
        let at = |name: &str| {
            root.as_ref()
                .map_or(FileStamp::Missing, |r| FileStamp::of(&r.join(name)))
        };
        LibraryStamp {
            lock: FileStamp::of(&project_root.join(LOCK_FILE)),
            manifest: FileStamp::of(&project_root.join(MANIFEST_FILE)),
            key: at(KEY_FILE),
            entries: at(stellar_dependencies::cache_root::ENTRIES_DIR),
            index: (!complete).then(|| at(stellar_dependencies::cache_root::CACHE_INDEX_FILE)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_changes_with_lock_manifest_and_entries_but_index_only_when_incomplete() {
        let dir = crate::test_support::TempDir::new("library-watch");
        let project = dir.path.join("project");
        let app = dir.path.join("app");
        std::fs::create_dir_all(&project).unwrap();
        let cache = CacheLocation::AppData(app.clone());
        let root = cache.cache_root().unwrap();
        let base = LibraryStamp::now(&project, &cache, true);
        assert_eq!(
            base,
            LibraryStamp::now(&project, &cache, true),
            "바뀐 것 없음"
        );

        std::fs::write(project.join(LOCK_FILE), "x").unwrap();
        let locked = LibraryStamp::now(&project, &cache, true);
        assert_ne!(locked, base, "lock 생성");

        std::fs::write(project.join(MANIFEST_FILE), "y").unwrap();
        let declared = LibraryStamp::now(&project, &cache, true);
        assert_ne!(declared, locked, "stellar.yaml 변경");

        std::fs::create_dir_all(root.join("entries/sha256-a")).unwrap();
        let entries = LibraryStamp::now(&project, &cache, true);
        assert_ne!(entries, declared, "entry 공개");

        // index는 완전한 로드에서는 보지 않습니다.
        std::fs::write(root.join("index.stlcache"), "i").unwrap();
        assert_eq!(LibraryStamp::now(&project, &cache, true), entries);
        let incomplete = LibraryStamp::now(&project, &cache, false);
        std::fs::write(root.join("index.stlcache"), "ii").unwrap();
        assert_ne!(LibraryStamp::now(&project, &cache, false), incomplete);
    }
}

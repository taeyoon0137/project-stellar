//! 사용자별 library cache root (XV.3.1).
//!
//! 외부 library는 사용자별 외부 공용 캐시에 내려받고 같은 버전을 여러 프로젝트가 공유합니다
//! (아키텍처 30.1). cache root는 플랫폼 셸이 주입하는 OS 앱 데이터 경로(macOS: `~/Library/
//! Application Support/<app>`) 아래 `library-cache/`입니다. **프로젝트 폴더 안에는 의존성
//! checkout·mirror·index를 만들지 않습니다** — 사용 프로젝트의 Git status를 바꾸지 않기 위함입니다.
//!
//! ```text
//! <app-data>/library-cache/
//! ├── index.stlcache          # cache index(XV.3.1, 형식은 cache_index)
//! ├── mirrors/<key>.git       # repository별 bare mirror(XV.3.2)
//! └── entries/                # content-addressed immutable entry(XV.3.3)
//! ```

use std::fmt;
use std::path::{Component, Path, PathBuf};

/// app data 아래 cache 디렉터리 이름.
pub const LIBRARY_CACHE_DIR: &str = "library-cache";
/// cache index 파일 이름.
pub const CACHE_INDEX_FILE: &str = "index.stlcache";
/// mirror 디렉터리 이름.
pub const MIRRORS_DIR: &str = "mirrors";
/// cache entry 디렉터리 이름.
pub const ENTRIES_DIR: &str = "entries";

/// cache root 오류. 경로 원문은 담지 않습니다(XV.1.4 경로 민감값).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheRootError {
    /// app data 경로가 절대 경로가 아니거나 `.`·`..` component를 포함함.
    NotNormalizedAbsolute,
    /// cache root가 사용 프로젝트 root와 같거나 그 안에 있음.
    InsideProject,
    /// 사용 프로젝트 root가 cache root 안에 있음(cache 정리 대상과 겹침).
    ProjectInsideCache,
    /// 경로 확인 I/O 실패.
    Io {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
}

impl fmt::Display for CacheRootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheRootError::NotNormalizedAbsolute => {
                f.write_str("app data 경로는 정규화된 절대 경로여야 합니다")
            }
            CacheRootError::InsideProject => {
                f.write_str("library cache는 프로젝트 폴더 밖에 있어야 합니다")
            }
            CacheRootError::ProjectInsideCache => {
                f.write_str("프로젝트 폴더가 library cache 안에 있습니다")
            }
            CacheRootError::Io { kind } => write!(f, "cache 경로 확인 실패: {kind:?}"),
        }
    }
}

impl std::error::Error for CacheRootError {}

/// 사용자별 library cache root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryCacheRoot {
    root: PathBuf,
}

impl LibraryCacheRoot {
    /// 앱 데이터 경로 아래 cache root를 정합니다. 디스크는 건드리지 않습니다.
    pub fn in_app_data(app_data_dir: &Path) -> Result<LibraryCacheRoot, CacheRootError> {
        let normalized = app_data_dir.is_absolute()
            && app_data_dir
                .components()
                .all(|c| !matches!(c, Component::CurDir | Component::ParentDir));
        if !normalized {
            return Err(CacheRootError::NotNormalizedAbsolute);
        }
        Ok(LibraryCacheRoot {
            root: app_data_dir.join(LIBRARY_CACHE_DIR),
        })
    }

    /// cache root 경로.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// cache index 파일 경로.
    pub fn index_path(&self) -> PathBuf {
        self.root.join(CACHE_INDEX_FILE)
    }

    /// mirror 디렉터리.
    pub fn mirrors_dir(&self) -> PathBuf {
        self.root.join(MIRRORS_DIR)
    }

    /// cache entry 디렉터리.
    pub fn entries_dir(&self) -> PathBuf {
        self.root.join(ENTRIES_DIR)
    }

    /// cache root와 사용 프로젝트 root가 겹치지 않는지 확인합니다(symlink 해석 후 비교).
    pub fn ensure_outside_project(&self, project_root: &Path) -> Result<(), CacheRootError> {
        let project = std::fs::canonicalize(project_root)
            .map_err(|e| CacheRootError::Io { kind: e.kind() })?;
        let cache = resolve_lenient(&self.root)?;
        if cache.starts_with(&project) {
            return Err(CacheRootError::InsideProject);
        }
        if project.starts_with(&cache) {
            return Err(CacheRootError::ProjectInsideCache);
        }
        Ok(())
    }

    /// cache 디렉터리 구조를 만듭니다.
    pub fn create_dirs(&self) -> Result<(), CacheRootError> {
        for dir in [self.mirrors_dir(), self.entries_dir()] {
            std::fs::create_dir_all(&dir).map_err(|e| CacheRootError::Io { kind: e.kind() })?;
        }
        Ok(())
    }
}

/// 아직 없을 수 있는 경로를 존재하는 가장 가까운 조상까지 canonicalize한 뒤 나머지를 붙입니다.
fn resolve_lenient(path: &Path) -> Result<PathBuf, CacheRootError> {
    let mut existing = path;
    let mut rest = Vec::new();
    loop {
        match std::fs::canonicalize(existing) {
            Ok(base) => {
                let mut out = base;
                for part in rest.iter().rev() {
                    out.push(part);
                }
                return Ok(out);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
                    return Err(CacheRootError::Io { kind: e.kind() });
                };
                rest.push(name.to_os_string());
                existing = parent;
            }
            Err(e) => return Err(CacheRootError::Io { kind: e.kind() }),
        }
    }
}

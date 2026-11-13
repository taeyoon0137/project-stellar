//! 감시 root 목록과 경로 변환 (XIII.2.2).
//!
//! 프로젝트 root(id 0)와 외부 symlink target allowlist(id 1부터)를 실제(canonical) 경로로 보관하고,
//! OS가 보고한 절대 경로를 [`WatchPath`](root 기준 상대 경로)로 바꿉니다. 플랫폼 adapter가 함께
//! 씁니다(플랫폼 독립).
//!
//! - 디렉터리 root: 그 아래의 모든 경로를 받습니다.
//! - 파일 root(파일을 가리키는 외부 symlink target): 그 파일 경로 하나만 받습니다(adapter는 부모
//!   디렉터리를 감시하고 이 규칙으로 걸러 냅니다).
//! - 경로가 여러 root에 속하면 가장 긴(가장 구체적인) root를 고릅니다.

use std::io;
use std::path::{Component, Path, PathBuf};

use super::event::{WatchPath, WatchRootId};

/// 감시 root 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchRootKind {
    /// 디렉터리(하위 전체).
    Directory,
    /// 파일 하나.
    File,
}

/// 감시 root 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchRoot {
    /// 식별자.
    pub id: WatchRootId,
    /// 실제(canonical) 경로.
    pub path: PathBuf,
    /// 종류.
    pub kind: WatchRootKind,
}

/// 감시 root 목록 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchRootError {
    /// 경로 해석·조회 실패.
    Io {
        /// 경로.
        path: PathBuf,
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
    /// 외부 target이 프로젝트 root 안에 있음(이미 프로젝트 root 감시에 포함).
    InsideProject {
        /// 실제 경로.
        path: PathBuf,
    },
    /// 프로젝트 root는 제거할 수 없음.
    ProjectRootRemoval,
}

impl std::fmt::Display for WatchRootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WatchRootError::Io { path, kind } => {
                write!(f, "감시 root 조회 실패 {path:?}: {kind:?}")
            }
            WatchRootError::InsideProject { path } => {
                write!(
                    f,
                    "프로젝트 root 안의 경로는 외부 감시 대상이 아닙니다: {path:?}"
                )
            }
            WatchRootError::ProjectRootRemoval => {
                f.write_str("프로젝트 root 감시는 제거할 수 없습니다")
            }
        }
    }
}

impl std::error::Error for WatchRootError {}

/// 감시 root 목록.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchRoots {
    roots: Vec<WatchRoot>,
    next_id: u32,
}

impl WatchRoots {
    /// 프로젝트 root(id 0) 하나로 시작합니다.
    pub fn new(project_root: &Path) -> Result<WatchRoots, WatchRootError> {
        let path = canonical(project_root)?;
        Ok(WatchRoots {
            roots: vec![WatchRoot {
                id: WatchRootId::PROJECT,
                path,
                kind: WatchRootKind::Directory,
            }],
            next_id: 1,
        })
    }

    /// 외부 symlink target을 추가합니다. 이미 있으면 기존 id를 돌려줍니다.
    pub fn add_external(&mut self, target: &Path) -> Result<WatchRootId, WatchRootError> {
        let path = canonical(target)?;
        if path.starts_with(&self.project().path) {
            return Err(WatchRootError::InsideProject { path });
        }
        if let Some(existing) = self.roots.iter().find(|r| r.path == path) {
            return Ok(existing.id);
        }
        let meta = std::fs::metadata(&path).map_err(|e| io_error(&path, &e))?;
        let kind = if meta.is_dir() {
            WatchRootKind::Directory
        } else {
            WatchRootKind::File
        };
        let id = WatchRootId(self.next_id);
        self.next_id += 1;
        self.roots.push(WatchRoot { id, path, kind });
        Ok(id)
    }

    /// 외부 root를 제거합니다(id는 재사용하지 않음).
    pub fn remove(&mut self, id: WatchRootId) -> Result<Option<WatchRoot>, WatchRootError> {
        if id.is_project() {
            return Err(WatchRootError::ProjectRootRemoval);
        }
        Ok(self
            .roots
            .iter()
            .position(|r| r.id == id)
            .map(|index| self.roots.remove(index)))
    }

    /// 프로젝트 root.
    pub fn project(&self) -> &WatchRoot {
        &self.roots[0]
    }

    /// id로 root를 찾습니다.
    pub fn get(&self, id: WatchRootId) -> Option<&WatchRoot> {
        self.roots.iter().find(|r| r.id == id)
    }

    /// 모든 root(프로젝트 root 먼저, 이후 추가 순).
    pub fn iter(&self) -> impl Iterator<Item = &WatchRoot> {
        self.roots.iter()
    }

    /// OS가 보고한 절대 경로를 [`WatchPath`]로 바꿉니다. 어느 root에도 속하지 않거나 프로젝트 경로
    /// 규칙으로 표현할 수 없으면(UTF-8 아님 등) `None`.
    pub fn relativize(&self, absolute: &Path) -> Option<WatchPath> {
        let root = self
            .roots
            .iter()
            .filter(|r| match r.kind {
                WatchRootKind::Directory => absolute.starts_with(&r.path),
                WatchRootKind::File => absolute == r.path,
            })
            .max_by_key(|r| r.path.components().count())?;
        let rest = absolute.strip_prefix(&root.path).ok()?;
        let mut parts = Vec::new();
        for component in rest.components() {
            match component {
                Component::Normal(name) => parts.push(name.to_str()?),
                _ => return None,
            }
        }
        WatchPath::new(root.id, parts.join("/")).ok()
    }

    /// 파일 root를 감시하려고 함께 보게 되는 부모 디렉터리 아래의 다른 경로인지(무시 대상).
    ///
    /// adapter는 파일 root의 부모 디렉터리를 감시하므로 형제 항목 알림도 받습니다. 이 알림은 감시
    /// 대상이 아니므로 넘침으로 보고하지 않고 버립니다.
    pub fn is_file_root_neighbor(&self, absolute: &Path) -> bool {
        self.roots.iter().any(|r| {
            r.kind == WatchRootKind::File
                && absolute != r.path
                && r.path
                    .parent()
                    .is_some_and(|parent| absolute.starts_with(parent))
        })
    }

    /// [`WatchPath`]의 절대 경로.
    pub fn absolute(&self, path: &WatchPath) -> Option<PathBuf> {
        let root = self.get(path.root())?;
        if path.is_root() {
            return Some(root.path.clone());
        }
        let mut absolute = root.path.clone();
        for part in path.relative().split('/') {
            absolute.push(part);
        }
        Some(absolute)
    }
}

fn canonical(path: &Path) -> Result<PathBuf, WatchRootError> {
    std::fs::canonicalize(path).map_err(|e| io_error(path, &e))
}

fn io_error(path: &Path, error: &io::Error) -> WatchRootError {
    WatchRootError::Io {
        path: path.to_path_buf(),
        kind: error.kind(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "stellar-watch-roots-test-{}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn paths_are_relativized_against_the_most_specific_root() {
        let dir = TempDir::new();
        let project = dir.path.join("project");
        let shared = dir.path.join("shared");
        let nested = shared.join("nested");
        std::fs::create_dir_all(project.join("design")).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(dir.path.join("font.otf"), b"f").unwrap();

        let mut roots = WatchRoots::new(&project).unwrap();
        let shared_id = roots.add_external(&shared).unwrap();
        let nested_id = roots.add_external(&nested).unwrap();
        let file_id = roots.add_external(&dir.path.join("font.otf")).unwrap();
        assert_eq!(roots.add_external(&shared).unwrap(), shared_id, "중복 등록");
        assert_eq!(roots.get(file_id).unwrap().kind, WatchRootKind::File);

        let project_root = roots.project().path.clone();
        let shared_root = roots.get(shared_id).unwrap().path.clone();
        assert_eq!(
            roots.relativize(&project_root.join("design/current")),
            Some(WatchPath::project("design/current").unwrap())
        );
        assert_eq!(
            roots.relativize(&project_root),
            Some(WatchPath::project("").unwrap())
        );
        assert_eq!(
            roots.relativize(&shared_root.join("a.png")),
            Some(WatchPath::new(shared_id, "a.png").unwrap())
        );
        assert_eq!(
            roots.relativize(&shared_root.join("nested/b.png")),
            Some(WatchPath::new(nested_id, "b.png").unwrap())
        );
        let font = roots.get(file_id).unwrap().path.clone();
        assert_eq!(
            roots.relativize(&font),
            Some(WatchPath::new(file_id, "").unwrap())
        );
        // 파일 root의 형제, 어느 root에도 없는 경로는 표현 불가.
        assert_eq!(roots.relativize(&font.with_file_name("other.otf")), None);
        assert_eq!(roots.relativize(Path::new("/elsewhere/x")), None);

        // 왕복.
        let p = WatchPath::new(nested_id, "deep/c.png").unwrap();
        assert_eq!(roots.relativize(&roots.absolute(&p).unwrap()), Some(p));
    }

    #[test]
    fn external_roots_exclude_project_and_can_be_removed() {
        let dir = TempDir::new();
        let project = dir.path.join("project");
        let outside = dir.path.join("outside");
        std::fs::create_dir_all(project.join("assets")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let mut roots = WatchRoots::new(&project).unwrap();
        assert!(matches!(
            roots.add_external(&project.join("assets")),
            Err(WatchRootError::InsideProject { .. })
        ));
        assert!(matches!(
            roots.add_external(&dir.path.join("missing")),
            Err(WatchRootError::Io { .. })
        ));
        let id = roots.add_external(&outside).unwrap();
        assert_eq!(
            roots.remove(WatchRootId::PROJECT),
            Err(WatchRootError::ProjectRootRemoval)
        );
        assert_eq!(roots.remove(id).unwrap().map(|r| r.id), Some(id));
        assert_eq!(roots.remove(id).unwrap(), None);
        // id는 재사용하지 않음.
        assert_ne!(roots.add_external(&outside).unwrap(), id);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_names_are_unrepresentable() {
        use std::os::unix::ffi::OsStrExt;
        let dir = TempDir::new();
        let roots = WatchRoots::new(&dir.path).unwrap();
        let bad = roots
            .project()
            .path
            .join(std::ffi::OsStr::from_bytes(b"bad\xff"));
        assert_eq!(roots.relativize(&bad), None);
        assert_eq!(
            roots.relativize(&roots.project().path.join("back\\slash")),
            None
        );
    }
}

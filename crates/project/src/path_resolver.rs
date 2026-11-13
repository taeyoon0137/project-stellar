//! 프로젝트 경로 resolver (XIII.4.1).
//!
//! 프로젝트 상대 경로(`/` 구분)를 실제 파일 시스템 경로로 해석합니다. 파일·디렉터리 symlink를
//! **그대로 허용**하며, target이 프로젝트 **내부든 외부든** 모두 읽을 수 있습니다(기술 아키텍처
//! 19.3). resolver는 링크 자체를 수정·삭제하지 않고 외부 대상을 프로젝트 안으로 복사하지도
//! 않습니다(링크는 Git 항목으로 그대로 유지).
//!
//! 해석 결과 [`ResolvedPath`]는 다음을 담습니다.
//! - **lexical 경로**: root + 상대 경로(symlink 보존). 링크 자체를 가리키는 경로입니다.
//! - **target**: symlink를 모두 해석한 실제(canonical) 경로.
//! - **symlink 목록**: 요청 경로의 lexical 컴포넌트 중 링크인 것(프로젝트 상대)과 원문 target.
//! - **hop 체인**: 해석 중 따라간 **모든** symlink hop(링크가 다른 링크를 가리키는 체인, 링크
//!   target 안의 중간 링크, 외부 위치의 링크 포함)을 따라간 순서대로 기록한 [`ChainHop`] 목록.
//! - **target 위치**: canonical root 기준 [`TargetLocation::Internal`]/[`TargetLocation::External`].
//!
//! cycle·최대 hop 검사(XIII.4.2), broken link diagnostic(XIII.4.3), 외부 target watcher 등록
//! (XIII.4.4), 비신뢰 프로젝트의 외부 target 읽기 차단(XIII.4.5)은 후속 작업입니다. 현재는 OS
//! 경로 해석(`canonicalize`)이 반환하는 오류를 [`ResolveError::Io`]로 그대로 돌려줍니다.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::identity::ProjectIdentity;

/// 해석된 target이 프로젝트 root 기준 어디에 있는지.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetLocation {
    /// canonical root 안(root 자신 포함).
    Internal,
    /// canonical root 밖(외부 target).
    External,
}

/// 해석된 target의 종류(symlink를 따라간 뒤의 실제 항목).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// 일반 파일.
    File,
    /// 디렉터리.
    Directory,
    /// 그 밖의 항목(장치·소켓 등).
    Other,
}

/// 요청 경로의 lexical 컴포넌트 중 symlink 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymlinkHop {
    /// 링크의 프로젝트 상대 경로(`/` 구분).
    pub link: String,
    /// 링크에 기록된 원문 target(`read_link` 결과, 상대·절대 그대로).
    pub raw_target: PathBuf,
}

/// 해석 중 따라간 symlink hop 하나(체인의 모든 링크).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainHop {
    /// 링크 파일의 실제 위치(부모 디렉터리까지 symlink를 해석한 절대 경로 + 링크 이름).
    pub link: PathBuf,
    /// 링크에 기록된 원문 target(`read_link` 결과).
    pub raw_target: PathBuf,
    /// 링크 파일 자체가 canonical root 안(`Internal`)에 있는지 밖(`External`)에 있는지.
    pub link_location: TargetLocation,
}

/// 경로 해석 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPath {
    /// 요청한 프로젝트 상대 경로(`/` 구분).
    pub project_path: String,
    /// root + 상대 경로(symlink 보존).
    pub lexical: PathBuf,
    /// symlink를 모두 해석한 실제 경로.
    pub target: PathBuf,
    /// 실제 항목 종류.
    pub kind: TargetKind,
    /// target 위치(내부/외부).
    pub location: TargetLocation,
    /// 요청 경로의 lexical 컴포넌트 중 symlink(root에 가까운 순서).
    pub symlinks: Vec<SymlinkHop>,
    /// 해석 중 따라간 모든 symlink hop(따라간 순서).
    pub chain: Vec<ChainHop>,
}

impl ResolvedPath {
    /// 경로 체인에 symlink가 하나라도 있는지.
    pub fn via_symlink(&self) -> bool {
        !self.chain.is_empty()
    }
}

/// 프로젝트 상대 경로 형식 오류 사유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidPathReason {
    /// 빈 경로.
    Empty,
    /// 절대 경로(`/`로 시작).
    Absolute,
    /// `\` 구분자 사용(내부 경로 구분자는 `/`).
    Backslash,
    /// 빈 컴포넌트(`a//b`, 끝의 `/`).
    EmptyComponent,
    /// `.` 또는 `..` 컴포넌트.
    DotComponent,
}

impl std::fmt::Display for InvalidPathReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InvalidPathReason::Empty => f.write_str("빈 경로"),
            InvalidPathReason::Absolute => f.write_str("절대 경로"),
            InvalidPathReason::Backslash => f.write_str("'\\' 구분자"),
            InvalidPathReason::EmptyComponent => f.write_str("빈 경로 컴포넌트"),
            InvalidPathReason::DotComponent => f.write_str("'.'·'..' 컴포넌트"),
        }
    }
}

/// 경로 해석·읽기 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// 프로젝트 상대 경로 형식이 잘못됨.
    InvalidPath {
        /// 요청 경로.
        path: String,
        /// 사유.
        reason: InvalidPathReason,
    },
    /// 파일 시스템 오류(없음·권한·OS 경로 해석 실패 등).
    Io {
        /// 실패한 경로.
        path: PathBuf,
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
    /// 파일 읽기를 요청했으나 target이 파일이 아님.
    NotAFile {
        /// 요청 경로.
        path: String,
    },
    /// 디렉터리 목록을 요청했으나 target이 디렉터리가 아님.
    NotADirectory {
        /// 요청 경로.
        path: String,
    },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::InvalidPath { path, reason } => {
                write!(f, "잘못된 프로젝트 경로 {path:?}: {reason}")
            }
            ResolveError::Io { path, kind } => {
                write!(f, "경로 해석 실패 {path:?}: {kind:?}")
            }
            ResolveError::NotAFile { path } => write!(f, "파일이 아닙니다: {path:?}"),
            ResolveError::NotADirectory { path } => write!(f, "디렉터리가 아닙니다: {path:?}"),
        }
    }
}

impl std::error::Error for ResolveError {}

/// 프로젝트 상대 경로를 검증하고 컴포넌트로 나눕니다.
///
/// 내부 경로 구분자는 `/`만 허용하며, 절대 경로·빈 컴포넌트·`.`/`..`를 거부합니다. `..`로
/// 프로젝트 밖을 가리키는 것은 금지하지만, **symlink target이 밖을 가리키는 것은 허용**합니다.
pub fn split_project_path(path: &str) -> Result<Vec<&str>, InvalidPathReason> {
    if path.is_empty() {
        return Err(InvalidPathReason::Empty);
    }
    if path.starts_with('/') {
        return Err(InvalidPathReason::Absolute);
    }
    if path.contains('\\') {
        return Err(InvalidPathReason::Backslash);
    }
    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" => return Err(InvalidPathReason::EmptyComponent),
            "." | ".." => return Err(InvalidPathReason::DotComponent),
            other => components.push(other),
        }
    }
    Ok(components)
}

/// symlink를 그대로 허용하는 프로젝트 경로 resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathResolver {
    /// 절대·lexical root(symlink 보존).
    root: PathBuf,
    /// symlink를 해석한 실제 root(내부/외부 판정 기준).
    canonical_root: PathBuf,
}

impl PathResolver {
    /// 열린 root 경로로 resolver를 만듭니다. root가 없으면 실패합니다.
    pub fn new(root: &Path) -> io::Result<PathResolver> {
        Ok(PathResolver {
            root: std::path::absolute(root)?,
            canonical_root: std::fs::canonicalize(root)?,
        })
    }

    /// [`ProjectIdentity`]의 root path와 canonical target으로 resolver를 만듭니다.
    pub fn from_identity(identity: &ProjectIdentity) -> PathResolver {
        PathResolver {
            root: identity.root_path().to_path_buf(),
            canonical_root: identity.canonical_target().to_path_buf(),
        }
    }

    /// lexical root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// canonical root.
    pub fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }

    /// 프로젝트 상대 경로를 해석합니다. 링크는 수정하지 않습니다.
    pub fn resolve(&self, project_path: &str) -> Result<ResolvedPath, ResolveError> {
        let components =
            split_project_path(project_path).map_err(|reason| ResolveError::InvalidPath {
                path: project_path.to_string(),
                reason,
            })?;

        // OS 해석이 실패하면(없음·권한·순환 등) 그 오류를 그대로 돌려줍니다. 성공했다면 경로 체인은
        // 유한하므로 아래 컴포넌트 단위 해석도 끝납니다.
        let lexical = components
            .iter()
            .fold(self.root.clone(), |path, component| path.join(component));
        let target = std::fs::canonicalize(&lexical).map_err(|e| io_error(&lexical, &e))?;
        let walk = self.walk(&components)?;
        let meta = std::fs::metadata(&target).map_err(|e| io_error(&target, &e))?;
        let kind = if meta.is_file() {
            TargetKind::File
        } else if meta.is_dir() {
            TargetKind::Directory
        } else {
            TargetKind::Other
        };
        let location = if target.starts_with(&self.canonical_root) {
            TargetLocation::Internal
        } else {
            TargetLocation::External
        };

        Ok(ResolvedPath {
            project_path: project_path.to_string(),
            lexical,
            target,
            kind,
            location,
            symlinks: walk.symlinks,
            chain: walk.chain,
        })
    }

    /// 경로 위치가 canonical root 기준 내부인지 외부인지.
    fn location_of(&self, path: &Path) -> TargetLocation {
        if path.starts_with(&self.canonical_root) {
            TargetLocation::Internal
        } else {
            TargetLocation::External
        }
    }

    /// canonical root에서 시작해 컴포넌트 단위로 symlink를 직접 따라가며 모든 hop을 기록합니다.
    ///
    /// 링크를 만나면 원문 target의 컴포넌트를 남은 컴포넌트 앞에 넣어 이어서 해석합니다(절대 target은
    /// `/`에서 다시 시작). 현재 위치는 항상 symlink가 없는 실제 경로이므로 `..`는 마지막 컴포넌트를
    /// 제거하는 것과 같습니다. 호출 전에 OS 해석이 성공했으므로 체인은 유한하며, 그 사이 링크가
    /// 바뀌는 경쟁에 대비해 [`WALK_STEP_GUARD`]로 반복을 제한합니다.
    fn walk(&self, components: &[&str]) -> Result<Walk, ResolveError> {
        let mut current = self.canonical_root.clone();
        // (컴포넌트, 요청 경로 컴포넌트 index — 링크 target에서 온 컴포넌트는 None)
        let mut pending: VecDeque<(OsString, Option<usize>)> = components
            .iter()
            .enumerate()
            .map(|(index, c)| (OsString::from(c), Some(index)))
            .collect();
        let mut symlinks = Vec::new();
        let mut chain = Vec::new();
        let mut steps = 0usize;
        while let Some((component, request_index)) = pending.pop_front() {
            steps += 1;
            if steps > WALK_STEP_GUARD {
                return Err(ResolveError::Io {
                    path: current,
                    kind: io::ErrorKind::Other,
                });
            }
            if component == "." {
                continue;
            }
            if component == ".." {
                current.pop();
                continue;
            }
            let candidate = current.join(&component);
            let meta =
                std::fs::symlink_metadata(&candidate).map_err(|e| io_error(&candidate, &e))?;
            if !meta.file_type().is_symlink() {
                current = candidate;
                continue;
            }
            let raw_target =
                std::fs::read_link(&candidate).map_err(|e| io_error(&candidate, &e))?;
            if let Some(index) = request_index {
                symlinks.push(SymlinkHop {
                    link: components[..=index].join("/"),
                    raw_target: raw_target.clone(),
                });
            }
            chain.push(ChainHop {
                link: candidate,
                raw_target: raw_target.clone(),
                link_location: self.location_of(&current),
            });
            // 원문 target 컴포넌트를 남은 컴포넌트 앞에 넣습니다.
            let mut expanded = Vec::new();
            for part in raw_target.components() {
                match part {
                    Component::RootDir | Component::Prefix(_) => {
                        current = PathBuf::from(part.as_os_str());
                        expanded.clear();
                    }
                    Component::CurDir => {}
                    Component::ParentDir => expanded.push(OsString::from("..")),
                    Component::Normal(name) => expanded.push(name.to_os_string()),
                }
            }
            for part in expanded.into_iter().rev() {
                pending.push_front((part, None));
            }
        }
        Ok(Walk { symlinks, chain })
    }

    /// 프로젝트 상대 경로의 파일 내용을 읽습니다(내부·외부 symlink target 모두 허용).
    pub fn read(&self, project_path: &str) -> Result<Vec<u8>, ResolveError> {
        let resolved = self.resolve(project_path)?;
        if resolved.kind != TargetKind::File {
            return Err(ResolveError::NotAFile {
                path: project_path.to_string(),
            });
        }
        std::fs::read(&resolved.target).map_err(|e| io_error(&resolved.target, &e))
    }

    /// 프로젝트 상대 경로의 디렉터리 항목 이름을 정렬해 반환합니다(디렉터리 symlink 허용).
    pub fn read_dir(&self, project_path: &str) -> Result<Vec<OsString>, ResolveError> {
        let resolved = self.resolve(project_path)?;
        if resolved.kind != TargetKind::Directory {
            return Err(ResolveError::NotADirectory {
                path: project_path.to_string(),
            });
        }
        let entries =
            std::fs::read_dir(&resolved.target).map_err(|e| io_error(&resolved.target, &e))?;
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| io_error(&resolved.target, &e))?;
            names.push(entry.file_name());
        }
        names.sort();
        Ok(names)
    }
}

/// OS 해석 성공 뒤 컴포넌트 단위 해석의 반복 상한(경쟁으로 링크가 바뀌는 경우의 방어).
const WALK_STEP_GUARD: usize = 1 << 16;

/// 컴포넌트 단위 해석 결과.
struct Walk {
    symlinks: Vec<SymlinkHop>,
    chain: Vec<ChainHop>,
}

/// io 오류를 경로와 함께 [`ResolveError::Io`]로 변환합니다.
fn io_error(path: &Path, error: &io::Error) -> ResolveError {
    ResolveError::Io {
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
                "stellar-path-resolver-test-{}-{n}",
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

    /// 프로젝트(`project/`)와 프로젝트 밖 폴더(`outside/`)를 가진 fixture.
    struct Fixture {
        _dir: TempDir,
        project: PathBuf,
        outside: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = TempDir::new();
        let project = dir.path.join("project");
        let outside = dir.path.join("outside");
        std::fs::create_dir_all(project.join("data/sub")).unwrap();
        std::fs::create_dir_all(outside.join("shared")).unwrap();
        std::fs::write(project.join("data/real.txt"), b"internal-bytes").unwrap();
        std::fs::write(outside.join("secret.txt"), b"external-bytes").unwrap();
        std::fs::write(outside.join("shared/asset.bin"), [0u8, 1, 2, 3]).unwrap();
        std::fs::write(outside.join("shared/other.bin"), [9u8]).unwrap();
        Fixture {
            _dir: dir,
            project,
            outside,
        }
    }

    #[test]
    fn regular_file_resolves_internal_without_symlinks() {
        let fx = fixture();
        let resolver = PathResolver::new(&fx.project).unwrap();
        let resolved = resolver.resolve("data/real.txt").unwrap();
        assert_eq!(resolved.kind, TargetKind::File);
        assert_eq!(resolved.location, TargetLocation::Internal);
        assert!(!resolved.via_symlink());
        assert_eq!(resolver.read("data/real.txt").unwrap(), b"internal-bytes");
    }

    #[cfg(unix)]
    #[test]
    fn internal_file_symlink_is_read() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        // 상대 target의 내부 파일 링크.
        symlink("data/real.txt", fx.project.join("link.txt")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();

        let resolved = resolver.resolve("link.txt").unwrap();
        assert_eq!(resolved.kind, TargetKind::File);
        assert_eq!(resolved.location, TargetLocation::Internal);
        assert_eq!(
            resolved.symlinks,
            vec![SymlinkHop {
                link: "link.txt".to_string(),
                raw_target: PathBuf::from("data/real.txt"),
            }]
        );
        // lexical 경로는 링크 자체, target은 실제 파일.
        assert!(resolved.lexical.ends_with("link.txt"));
        assert_eq!(
            resolved.target,
            std::fs::canonicalize(fx.project.join("data/real.txt")).unwrap()
        );
        assert_eq!(resolver.read("link.txt").unwrap(), b"internal-bytes");
    }

    #[cfg(unix)]
    #[test]
    fn internal_relative_parent_symlink_stays_internal() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        // `..`를 쓰지만 프로젝트 안에 머무는 링크.
        symlink("../real.txt", fx.project.join("data/sub/up.txt")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();
        let resolved = resolver.resolve("data/sub/up.txt").unwrap();
        assert_eq!(resolved.location, TargetLocation::Internal);
        assert_eq!(resolver.read("data/sub/up.txt").unwrap(), b"internal-bytes");
    }

    #[cfg(unix)]
    #[test]
    fn external_file_symlink_is_read() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        // 절대 target의 외부 파일 링크.
        let external = fx.outside.join("secret.txt");
        symlink(&external, fx.project.join("ext.txt")).unwrap();
        // 상대 target(`..`)으로 프로젝트 밖을 가리키는 링크.
        symlink("../outside/secret.txt", fx.project.join("ext-rel.txt")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();

        for path in ["ext.txt", "ext-rel.txt"] {
            let resolved = resolver.resolve(path).unwrap();
            assert_eq!(resolved.kind, TargetKind::File, "{path}");
            assert_eq!(resolved.location, TargetLocation::External, "{path}");
            assert_eq!(resolved.target, std::fs::canonicalize(&external).unwrap());
            assert_eq!(resolver.read(path).unwrap(), b"external-bytes", "{path}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn internal_directory_symlink_is_read() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        symlink("data", fx.project.join("alias-dir")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();

        let dir = resolver.resolve("alias-dir").unwrap();
        assert_eq!(dir.kind, TargetKind::Directory);
        assert_eq!(dir.location, TargetLocation::Internal);

        // 디렉터리 링크를 거친 파일: 체인의 링크가 기록됨.
        let file = resolver.resolve("alias-dir/real.txt").unwrap();
        assert_eq!(file.location, TargetLocation::Internal);
        assert_eq!(file.symlinks.len(), 1);
        assert_eq!(file.symlinks[0].link, "alias-dir");
        assert_eq!(
            resolver.read("alias-dir/real.txt").unwrap(),
            b"internal-bytes"
        );
        assert_eq!(
            resolver.read_dir("alias-dir").unwrap(),
            vec![OsString::from("real.txt"), OsString::from("sub")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn external_directory_symlink_is_read() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        symlink(fx.outside.join("shared"), fx.project.join("shared")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();

        let dir = resolver.resolve("shared").unwrap();
        assert_eq!(dir.kind, TargetKind::Directory);
        assert_eq!(dir.location, TargetLocation::External);

        let file = resolver.resolve("shared/asset.bin").unwrap();
        assert_eq!(file.location, TargetLocation::External);
        assert_eq!(file.symlinks[0].link, "shared");
        assert_eq!(resolver.read("shared/asset.bin").unwrap(), [0u8, 1, 2, 3]);
        assert_eq!(
            resolver.read_dir("shared").unwrap(),
            vec![OsString::from("asset.bin"), OsString::from("other.bin")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn reading_does_not_modify_or_copy_links() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        let external = fx.outside.join("secret.txt");
        symlink(&external, fx.project.join("ext.txt")).unwrap();
        symlink(fx.outside.join("shared"), fx.project.join("shared")).unwrap();
        let before: Vec<OsString> = list(&fx.project);

        let resolver = PathResolver::new(&fx.project).unwrap();
        resolver.read("ext.txt").unwrap();
        resolver.read("shared/asset.bin").unwrap();
        resolver.read_dir("shared").unwrap();

        // 링크는 링크 그대로(원문 target 동일), 프로젝트에 복사본 없음, 외부 원본 불변.
        for (name, raw) in [
            ("ext.txt", external.clone()),
            ("shared", fx.outside.join("shared")),
        ] {
            let link = fx.project.join(name);
            assert!(std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(std::fs::read_link(&link).unwrap(), raw);
        }
        assert_eq!(list(&fx.project), before);
        assert_eq!(std::fs::read(&external).unwrap(), b"external-bytes");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_root_classifies_by_canonical_root() {
        use std::os::unix::fs::symlink;
        use stellar_core_types::ids::StableId;
        let fx = fixture();
        symlink("data/real.txt", fx.project.join("link.txt")).unwrap();
        // root 자체를 alias 경로로 엶.
        let alias_root = fx.project.parent().unwrap().join("project-alias");
        symlink(&fx.project, &alias_root).unwrap();
        let identity = ProjectIdentity::new(StableId::new_v7(1, 0, 1), &alias_root).unwrap();
        let resolver = PathResolver::from_identity(&identity);

        assert_eq!(resolver.root(), identity.root_path());
        assert_eq!(resolver.canonical_root(), identity.canonical_target());
        let resolved = resolver.resolve("link.txt").unwrap();
        assert_eq!(resolved.location, TargetLocation::Internal);
        assert!(resolved.lexical.starts_with(&alias_root));
        assert_eq!(resolver.read("link.txt").unwrap(), b"internal-bytes");
    }

    #[test]
    fn invalid_project_paths_are_rejected() {
        let fx = fixture();
        let resolver = PathResolver::new(&fx.project).unwrap();
        for (path, reason) in [
            ("", InvalidPathReason::Empty),
            ("/etc/passwd", InvalidPathReason::Absolute),
            ("data\\real.txt", InvalidPathReason::Backslash),
            ("data//real.txt", InvalidPathReason::EmptyComponent),
            ("data/", InvalidPathReason::EmptyComponent),
            ("./data", InvalidPathReason::DotComponent),
            (
                "data/../../outside/secret.txt",
                InvalidPathReason::DotComponent,
            ),
            ("..", InvalidPathReason::DotComponent),
        ] {
            assert_eq!(
                resolver.resolve(path),
                Err(ResolveError::InvalidPath {
                    path: path.to_string(),
                    reason,
                }),
                "입력: {path:?}"
            );
        }
    }

    #[test]
    fn missing_path_is_io_not_found() {
        let fx = fixture();
        let resolver = PathResolver::new(&fx.project).unwrap();
        assert!(matches!(
            resolver.resolve("data/missing.txt"),
            Err(ResolveError::Io {
                kind: io::ErrorKind::NotFound,
                ..
            })
        ));
    }

    #[test]
    fn kind_mismatch_is_reported() {
        let fx = fixture();
        let resolver = PathResolver::new(&fx.project).unwrap();
        assert_eq!(
            resolver.read("data"),
            Err(ResolveError::NotAFile {
                path: "data".to_string()
            })
        );
        assert_eq!(
            resolver.read_dir("data/real.txt"),
            Err(ResolveError::NotADirectory {
                path: "data/real.txt".to_string()
            })
        );
    }

    fn list(dir: &Path) -> Vec<OsString> {
        let mut names: Vec<OsString> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        names
    }

    #[cfg(unix)]
    #[test]
    fn every_hop_of_a_link_chain_is_recorded() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        // 시스템 경로 링크(예: macOS `/var` → `private/var`)가 hop에 섞이지 않도록 실경로를 씁니다.
        let outside = std::fs::canonicalize(&fx.outside).unwrap();
        // a -> b -> outside/hop -> outside/secret.txt (링크가 링크를 가리키는 체인, 외부 링크 포함).
        symlink(outside.join("secret.txt"), outside.join("hop")).unwrap();
        symlink(outside.join("hop"), fx.project.join("b")).unwrap();
        symlink("b", fx.project.join("a")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();

        let resolved = resolver.resolve("a").unwrap();
        assert_eq!(resolved.location, TargetLocation::External);
        assert_eq!(resolver.read("a").unwrap(), b"external-bytes");
        // lexical 링크는 요청 컴포넌트 `a` 하나뿐이지만, 체인에는 따라간 모든 hop이 기록됨.
        assert_eq!(resolved.symlinks.len(), 1);
        let root = resolver.canonical_root();
        assert_eq!(
            resolved.chain,
            vec![
                ChainHop {
                    link: root.join("a"),
                    raw_target: PathBuf::from("b"),
                    link_location: TargetLocation::Internal,
                },
                ChainHop {
                    link: root.join("b"),
                    raw_target: outside.join("hop"),
                    link_location: TargetLocation::Internal,
                },
                ChainHop {
                    link: outside.join("hop"),
                    raw_target: outside.join("secret.txt"),
                    link_location: TargetLocation::External,
                },
            ]
        );
        let last = resolved.chain.last().unwrap();
        assert!(last.link.ends_with("outside/hop"));
        assert_eq!(last.link_location, TargetLocation::External);
    }

    #[cfg(unix)]
    #[test]
    fn hops_inside_link_targets_are_recorded() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        // 디렉터리 링크 `d` -> `data`, target 안의 링크 `data/inner` -> `real.txt`.
        symlink("data", fx.project.join("d")).unwrap();
        symlink("real.txt", fx.project.join("data/inner")).unwrap();
        // 링크 target 경로 중간에 링크(`d`)가 있는 링크.
        symlink("d/inner", fx.project.join("via")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();

        let resolved = resolver.resolve("via").unwrap();
        assert_eq!(resolved.location, TargetLocation::Internal);
        assert_eq!(resolver.read("via").unwrap(), b"internal-bytes");
        let links: Vec<PathBuf> = resolved.chain.iter().map(|h| h.link.clone()).collect();
        let root = resolver.canonical_root();
        assert_eq!(
            links,
            vec![root.join("via"), root.join("d"), root.join("data/inner")]
        );
        assert!(resolved
            .chain
            .iter()
            .all(|h| h.link_location == TargetLocation::Internal));

        // 요청 경로의 lexical 링크(`d`, `d/inner`)는 symlinks에, 모든 hop은 chain에.
        let resolved = resolver.resolve("d/inner").unwrap();
        let lexical: Vec<&str> = resolved.symlinks.iter().map(|h| h.link.as_str()).collect();
        assert_eq!(lexical, vec!["d", "d/inner"]);
        assert_eq!(resolved.chain.len(), 2);
    }

    #[test]
    fn regular_paths_have_empty_chain() {
        let fx = fixture();
        let resolver = PathResolver::new(&fx.project).unwrap();
        let resolved = resolver.resolve("data/real.txt").unwrap();
        assert!(resolved.chain.is_empty());
        assert!(resolved.symlinks.is_empty());
    }
}

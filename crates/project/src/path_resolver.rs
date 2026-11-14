//! 프로젝트 경로 resolver (XIII.4.1, XIII.4.2, XIII.4.3, XIII.4.5).
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
//! 무한 순회 차단(XIII.4.2, 기술 아키텍처 19.3 "순환 참조 표시"·37.2 "순환 참조 탐지"):
//! - **해석**: symlink를 직접 따라가며 (링크 위치, 남은 경로) 상태가 반복되면
//!   [`ResolveError::SymlinkCycle`](self·two-node·deep cycle)로, hop이 [`MAX_SYMLINK_HOPS`]를
//!   넘으면 [`ResolveError::TooManySymlinkHops`]로 거부합니다(OS 오류에 의존하지 않음).
//! - **트리 순회**: [`PathResolver::walk_tree`]는 디렉터리 symlink를 따라 내려가되, 현재 경로의
//!   조상 디렉터리를 다시 가리키는 디렉터리는 들어가지 않고 [`WalkIssue::DirectoryCycle`]로
//!   표시합니다. 같은 실제 디렉터리는 한 번만 읽고 다른 경로는 [`WalkIssue::AlreadyTraversed`]로
//!   표시하므로 symlink DAG에서도 작업량이 지수적으로 늘지 않습니다. 해석할 수 없는 항목은
//!   [`WalkIssue::Unresolvable`]로 표시하고 순회를 계속합니다.
//!
//! broken link(XIII.4.3, 기술 아키텍처 19.3 "broken link 표시"): 링크 target(체인의 어느 hop이든)이
//! 없으면 [`ResolveError::BrokenSymlink`]로 **어느 링크가 깨졌는지** 보고합니다. 링크가 없는 일반
//! 경로의 부재는 지금처럼 [`ResolveError::Io`](`NotFound`)입니다. resolver는 깨진 링크를 **삭제·수정하지
//! 않으며**, [`PathResolver::symlink_diagnostics`]가 트리 전체의 깨진 링크·순환을
//! [`SymlinkDiagnostic`]으로 모아 표시용으로 돌려줍니다. 깨진 링크가 있어도 다른 경로의 해석·읽기와
//! 프로젝트 열기는 영향을 받지 않습니다.
//!
//! 비신뢰 프로젝트(XIII.4.5, 기술 아키텍처 19.3·37.1): [`ExternalAccess::MetadataOnly`]인 resolver는
//! 최종 target이 프로젝트 **밖**인 경로의 **내용**(파일 bytes, 디렉터리 항목)을 읽지 않고
//! [`ResolveError::ExternalTargetNotTrusted`]로 거부합니다. 경로 해석에 필요한 metadata(`lstat`,
//! `readlink`, `stat`)만 사용하며, [`PathResolver::external_target_info`]로 target 경로·종류만
//! 제한적으로 보여 줍니다. 내용 접근은 [`PathResolver::read`]·[`PathResolver::read_dir`]·
//! [`PathResolver::walk_tree`]의 디렉터리 목록 한 곳(`open_content`·`list_content`)으로만 일어나며
//! 그 직전에 정책을 검사합니다. 프로젝트 열기 경로는 [`PathResolver::for_project`]로 신뢰 상태를
//! 넘겨 만듭니다.
//!
//! 외부 target watcher 등록(XIII.4.4)은 후속 작업입니다.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::capability::{capability_level, CapabilityLevel, ProjectCapability};
use crate::identity::ProjectIdentity;
use crate::trust::TrustStatus;

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

/// 경로 하나를 해석할 때 따라갈 수 있는 symlink hop 상한.
///
/// macOS 커널의 `MAXSYMLINKS`(32)와 같은 값입니다. OS가 거부하는 체인을 resolver가 받아들이지
/// 않도록 맞춥니다.
pub const MAX_SYMLINK_HOPS: usize = 32;

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
    /// symlink 순환(같은 링크에서 같은 남은 경로로 다시 돌아옴).
    SymlinkCycle {
        /// 요청 경로.
        path: String,
        /// 순환을 이루는 링크 위치(처음 반복된 링크부터 다시 만난 같은 링크까지, 양 끝 포함).
        cycle: Vec<PathBuf>,
    },
    /// 따라간 symlink hop이 상한을 넘음(긴 체인 또는 남은 경로가 계속 늘어나는 순환).
    TooManySymlinkHops {
        /// 요청 경로.
        path: String,
        /// 적용한 hop 상한.
        limit: usize,
    },
    /// 비신뢰 프로젝트에서 프로젝트 밖 target의 내용 접근을 거부함(내용을 열지 않음).
    ExternalTargetNotTrusted {
        /// 요청 경로.
        path: String,
        /// 해석된 외부 target 경로(metadata).
        target: PathBuf,
    },
    /// 깨진 symlink: 링크의 원문 target이 가리키는 경로가 없음(링크 자체는 그대로 둠).
    BrokenSymlink {
        /// 요청 경로.
        path: String,
        /// 깨진 링크의 실제 위치.
        link: PathBuf,
        /// 링크에 기록된 원문 target.
        raw_target: PathBuf,
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
            ResolveError::SymlinkCycle { path, cycle } => {
                write!(f, "symlink 순환 {path:?}: {cycle:?}")
            }
            ResolveError::TooManySymlinkHops { path, limit } => {
                write!(f, "symlink hop이 상한 {limit}을 넘습니다: {path:?}")
            }
            ResolveError::ExternalTargetNotTrusted { path, target } => write!(
                f,
                "신뢰되지 않은 프로젝트에서는 외부 target 내용을 읽지 않습니다: {path:?} -> {target:?}"
            ),
            ResolveError::BrokenSymlink {
                path,
                link,
                raw_target,
            } => write!(
                f,
                "깨진 symlink {path:?}: {link:?} -> {raw_target:?} 대상이 없습니다"
            ),
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

/// 프로젝트 밖 target의 내용 접근 정책.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalAccess {
    /// 외부 target 내용을 읽을 수 있음(신뢰된 프로젝트).
    Allowed,
    /// 외부 target은 경로 metadata만 해석하고 내용은 읽지 않음(신뢰되지 않은 프로젝트).
    MetadataOnly,
}

impl ExternalAccess {
    /// 프로젝트 신뢰 상태에 맞는 정책. 신뢰되지 않으면 [`ExternalAccess::MetadataOnly`]입니다.
    ///
    /// 값은 capability matrix(XVI.7.1)의 외부 symlink target 읽기 행에서 얻습니다.
    pub fn from_trust(trust: TrustStatus) -> ExternalAccess {
        match capability_level(ProjectCapability::ReadExternalSymlinkTarget, trust) {
            CapabilityLevel::Allowed => ExternalAccess::Allowed,
            CapabilityLevel::Restricted | CapabilityLevel::Denied => ExternalAccess::MetadataOnly,
        }
    }
}

/// 외부 target의 제한된 metadata(내용·크기·시간 정보 없음).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalTargetInfo {
    /// 요청 경로(프로젝트 상대).
    pub path: String,
    /// target 위치.
    pub location: TargetLocation,
    /// target 종류.
    pub kind: TargetKind,
    /// 해석된 target 경로.
    pub target: PathBuf,
    /// 요청 경로에서 처음 만난 링크의 원문 target(링크가 없으면 `None`).
    pub raw_target: Option<PathBuf>,
}

/// symlink를 그대로 허용하는 프로젝트 경로 resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathResolver {
    /// 절대·lexical root(symlink 보존).
    root: PathBuf,
    /// symlink를 해석한 실제 root(내부/외부 판정 기준).
    canonical_root: PathBuf,
    /// 외부 target 내용 접근 정책.
    external_access: ExternalAccess,
}

impl PathResolver {
    /// 열린 root 경로로 resolver를 만듭니다. root가 없으면 실패합니다.
    ///
    /// 외부 target 정책은 [`ExternalAccess::Allowed`]입니다. 열린 프로젝트에는 신뢰 상태를 반영하는
    /// [`PathResolver::for_project`] 또는 [`PathResolver::with_trust`]를 씁니다.
    pub fn new(root: &Path) -> io::Result<PathResolver> {
        Ok(PathResolver {
            root: std::path::absolute(root)?,
            canonical_root: std::fs::canonicalize(root)?,
            external_access: ExternalAccess::Allowed,
        })
    }

    /// [`ProjectIdentity`]의 root path와 canonical target으로 resolver를 만듭니다(정책은
    /// [`PathResolver::new`]와 같음).
    pub fn from_identity(identity: &ProjectIdentity) -> PathResolver {
        PathResolver {
            root: identity.root_path().to_path_buf(),
            canonical_root: identity.canonical_target().to_path_buf(),
            external_access: ExternalAccess::Allowed,
        }
    }

    /// 열린 프로젝트의 resolver. 신뢰되지 않은 프로젝트는 외부 target 내용을 읽지 않습니다.
    pub fn for_project(identity: &ProjectIdentity, trust: TrustStatus) -> PathResolver {
        PathResolver::from_identity(identity).with_trust(trust)
    }

    /// 신뢰 상태에 맞게 외부 target 정책을 바꾼 resolver.
    pub fn with_trust(mut self, trust: TrustStatus) -> PathResolver {
        self.external_access = ExternalAccess::from_trust(trust);
        self
    }

    /// 외부 target 내용 접근 정책.
    pub fn external_access(&self) -> ExternalAccess {
        self.external_access
    }

    /// 경로를 해석해 target의 제한된 metadata를 반환합니다. 내용은 열지 않으므로 신뢰 상태와
    /// 무관하게 사용할 수 있습니다.
    pub fn external_target_info(
        &self,
        project_path: &str,
    ) -> Result<ExternalTargetInfo, ResolveError> {
        let resolved = self.resolve(project_path)?;
        Ok(ExternalTargetInfo {
            path: resolved.project_path,
            location: resolved.location,
            kind: resolved.kind,
            target: resolved.target,
            raw_target: resolved.chain.first().map(|hop| hop.raw_target.clone()),
        })
    }

    /// 내용 접근 직전 정책 검사: 비신뢰 프로젝트에서 외부 target이면 거부합니다.
    fn check_content_access(&self, resolved: &ResolvedPath) -> Result<(), ResolveError> {
        if self.external_access == ExternalAccess::MetadataOnly
            && resolved.location == TargetLocation::External
        {
            return Err(ResolveError::ExternalTargetNotTrusted {
                path: resolved.project_path.clone(),
                target: resolved.target.clone(),
            });
        }
        Ok(())
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

        let lexical = components
            .iter()
            .fold(self.root.clone(), |path, component| path.join(component));
        // symlink를 직접 따라가며 순환·hop 상한을 검사합니다(무한 순회 없음).
        let walk = self.walk(project_path, &components)?;
        // walk 결과는 symlink가 없는 실제 경로입니다. canonicalize는 대소문자 등 표기만 OS 기준으로
        // 맞춥니다(링크를 따라가지 않음).
        let target =
            std::fs::canonicalize(&walk.resolved).map_err(|e| io_error(&walk.resolved, &e))?;
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
    /// 제거하는 것과 같습니다.
    ///
    /// 종료 보장: 링크를 만날 때마다 (링크 위치, 남은 컴포넌트) 상태를 기록해 같은 상태가 다시 나오면
    /// [`ResolveError::SymlinkCycle`], 따라간 hop이 [`MAX_SYMLINK_HOPS`]에 이른 뒤 링크를 또 만나면
    /// [`ResolveError::TooManySymlinkHops`]로 멈춥니다. 링크가 아닌 컴포넌트는 남은 목록을 줄이기만
    /// 하고, 링크 확장은 hop 상한만큼만 일어나므로 반복 횟수는 유한합니다.
    fn walk(&self, project_path: &str, components: &[&str]) -> Result<Walk, ResolveError> {
        let mut current = self.canonical_root.clone();
        // (컴포넌트, 출처 — 요청 경로 컴포넌트 index 또는 이 컴포넌트를 만든 hop index)
        let mut pending: VecDeque<(OsString, Origin)> = components
            .iter()
            .enumerate()
            .map(|(index, c)| (OsString::from(c), Origin::Request(index)))
            .collect();
        let mut symlinks = Vec::new();
        let mut chain: Vec<ChainHop> = Vec::new();
        // (링크 위치, 남은 컴포넌트) → 그 상태에서 기록한 hop index.
        let mut seen: HashMap<(PathBuf, Vec<OsString>), usize> = HashMap::new();
        while let Some((component, origin)) = pending.pop_front() {
            if component == "." {
                continue;
            }
            if component == ".." {
                current.pop();
                continue;
            }
            let candidate = current.join(&component);
            let meta = match std::fs::symlink_metadata(&candidate) {
                Ok(meta) => meta,
                Err(e) => {
                    // 링크 target에서 온 컴포넌트가 없으면 그 링크가 깨진 것입니다.
                    let missing = matches!(
                        e.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    );
                    if let (true, Origin::Hop(hop)) = (missing, origin) {
                        let broken = &chain[hop];
                        return Err(ResolveError::BrokenSymlink {
                            path: project_path.to_string(),
                            link: broken.link.clone(),
                            raw_target: broken.raw_target.clone(),
                        });
                    }
                    return Err(io_error(&candidate, &e));
                }
            };
            if !meta.file_type().is_symlink() {
                current = candidate;
                continue;
            }
            let state = (
                candidate.clone(),
                pending.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>(),
            );
            if let Some(&start) = seen.get(&state) {
                let mut cycle: Vec<PathBuf> =
                    chain[start..].iter().map(|h| h.link.clone()).collect();
                cycle.push(candidate);
                return Err(ResolveError::SymlinkCycle {
                    path: project_path.to_string(),
                    cycle,
                });
            }
            if chain.len() >= MAX_SYMLINK_HOPS {
                return Err(ResolveError::TooManySymlinkHops {
                    path: project_path.to_string(),
                    limit: MAX_SYMLINK_HOPS,
                });
            }
            seen.insert(state, chain.len());
            let raw_target =
                std::fs::read_link(&candidate).map_err(|e| io_error(&candidate, &e))?;
            if let Origin::Request(index) = origin {
                symlinks.push(SymlinkHop {
                    link: components[..=index].join("/"),
                    raw_target: raw_target.clone(),
                });
            }
            let hop = chain.len();
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
                pending.push_front((part, Origin::Hop(hop)));
            }
        }
        Ok(Walk {
            resolved: current,
            symlinks,
            chain,
        })
    }

    /// 프로젝트 root부터 트리를 순회합니다. 디렉터리 symlink(내부·외부)를 따라 내려가지만 무한
    /// 순회하지 않습니다([`ExternalDescent::Follow`]로 [`PathResolver::walk_tree_with`]를 부름).
    pub fn walk_tree(&self, include: impl Fn(&str) -> bool) -> Result<TreeWalk, ResolveError> {
        self.walk_tree_with(ExternalDescent::Follow, include)
    }

    /// 프로젝트 root부터 트리를 순회합니다. 디렉터리 symlink(내부·외부)를 따라 내려가지만 무한
    /// 순회하지 않고, 같은 실제 디렉터리의 목록은 **한 번만** 읽습니다.
    ///
    /// - `include(name)`은 root 바로 아래 항목의 이름(최상위 이름)으로**만** 불립니다. `false`면 그
    ///   항목과 그 아래 전체를 목록에 넣지 않고 내려가지도 않습니다. 더 깊은 경로는 `include`에 넘기지
    ///   않습니다(코드로 강제). 공유 디렉터리의 하위 항목은 처음 도달한 경로 하나로만 나오므로(아래
    ///   [`WalkIssue::AlreadyTraversed`]), 더 깊은 경로로 거를 수 있으면 어느 경로가 먼저 도달했는지에
    ///   따라 다른 별칭 경로의 하위 항목까지 함께 사라집니다. 최상위 이름으로만 거르면 포함된 최상위
    ///   아래의 하위 항목은 모두 포함되므로 공유 디렉터리의 하위 항목 집합이 도달 순서와 무관합니다.
    /// - 현재 순회 경로의 조상 디렉터리를 다시 가리키는 디렉터리는 [`WalkIssue::DirectoryCycle`]로
    ///   표시하고 들어가지 않습니다.
    /// - 이미 다른 경로로 목록을 읽은 실제 디렉터리(symlink DAG의 공유 디렉터리)는 항목으로 넣되
    ///   다시 내려가지 않고 [`WalkIssue::AlreadyTraversed`]로 표시합니다. 따라서 작업량은 서로 다른
    ///   실제 디렉터리의 항목 수에 비례합니다(링크 조합 수에 지수적으로 늘지 않음). symlink를 거치지
    ///   않는 경로를 먼저, 그다음 symlink를 거친 경로를 너비 우선으로 읽으므로 공유 디렉터리는 실제
    ///   위치(없으면 가장 짧은 링크 경로)로 한 번 나옵니다.
    /// - 해석할 수 없는 항목(symlink 순환·hop 초과·없는 target 등)은 [`WalkIssue::Unresolvable`]로
    ///   표시하고 나머지 순회를 계속합니다. 링크는 수정·삭제하지 않습니다.
    /// - 목록을 읽지 못한 하위 디렉터리(권한 없음 등)는 [`WalkIssue::DirectoryUnreadable`]로 표시하고
    ///   나머지 순회를 계속합니다. root 목록을 읽지 못할 때만 `Err`입니다.
    /// - 비신뢰 프로젝트([`ExternalAccess::MetadataOnly`])에서는 외부 디렉터리를 항목(metadata)으로만
    ///   넣고 목록을 읽지 않으며 [`WalkIssue::ExternalNotTraversed`]로 표시합니다.
    /// - [`ExternalDescent::Stop`]이면 신뢰 상태와 무관하게 외부 디렉터리를 항목으로만 넣고 목록을
    ///   읽지 않습니다(호출자가 고른 범위라 문제로 표시하지 않음).
    /// - 결과 항목·문제는 프로젝트 상대 경로 순으로 정렬되어 결정론적입니다.
    pub fn walk_tree_with(
        &self,
        external: ExternalDescent,
        include: impl Fn(&str) -> bool,
    ) -> Result<TreeWalk, ResolveError> {
        let mut walk = TreeWalk::default();
        // 실제 디렉터리 → 그 목록을 읽은 프로젝트 상대 경로(root는 빈 문자열).
        let mut traversed: HashMap<PathBuf, String> = HashMap::new();
        // symlink를 거치지 않는 디렉터리를 먼저 읽고, symlink를 거친 디렉터리는 그 뒤에 읽습니다.
        let mut direct: VecDeque<PendingDir> = VecDeque::new();
        let mut linked: VecDeque<PendingDir> = VecDeque::new();
        direct.push_back(PendingDir {
            path: String::new(),
            target: self.canonical_root.clone(),
            ancestors: vec![self.canonical_root.clone()],
        });
        while let Some(dir) = direct.pop_front().or_else(|| linked.pop_front()) {
            if let Some(first) = traversed.get(&dir.target) {
                walk.issues.push(WalkIssue::AlreadyTraversed {
                    path: dir.path,
                    target: dir.target,
                    traversed_as: first.clone(),
                });
                continue;
            }
            traversed.insert(dir.target.clone(), dir.path.clone());
            let names = match list_content(&dir.target) {
                Ok(names) => names,
                // root 목록을 못 읽으면 순회 자체가 실패입니다.
                Err(error) if dir.path.is_empty() => return Err(error),
                // 하위 디렉터리 하나(권한 없음 등) 때문에 전체 순회를 멈추지 않습니다.
                Err(error) => {
                    walk.issues.push(WalkIssue::DirectoryUnreadable {
                        path: dir.path,
                        error,
                    });
                    continue;
                }
            };
            for name in names {
                let Some(name) = name.to_str() else {
                    walk.issues.push(WalkIssue::NonUtf8Name {
                        path: dir.target.join(&name),
                    });
                    continue;
                };
                // include는 최상위 이름에만 적용합니다(문서: 도달 순서와 무관한 공유 하위 항목).
                let path = if dir.path.is_empty() {
                    if !include(name) {
                        continue;
                    }
                    name.to_string()
                } else {
                    format!("{}/{name}", dir.path)
                };
                let resolved = match self.resolve(&path) {
                    Ok(resolved) => resolved,
                    Err(error) => {
                        walk.issues.push(WalkIssue::Unresolvable { path, error });
                        continue;
                    }
                };
                let entry = WalkEntry {
                    path: path.clone(),
                    kind: resolved.kind,
                    location: resolved.location,
                    via_symlink: resolved.via_symlink(),
                };
                if resolved.kind != TargetKind::Directory {
                    walk.entries.push(entry);
                    continue;
                }
                if self.check_content_access(&resolved).is_err() {
                    walk.issues.push(WalkIssue::ExternalNotTraversed {
                        path,
                        target: resolved.target,
                    });
                    walk.entries.push(entry);
                    continue;
                }
                if dir.ancestors.contains(&resolved.target) {
                    walk.issues.push(WalkIssue::DirectoryCycle {
                        path,
                        target: resolved.target,
                    });
                    continue;
                }
                walk.entries.push(entry);
                if external == ExternalDescent::Stop
                    && resolved.location == TargetLocation::External
                {
                    continue;
                }
                let mut ancestors = dir.ancestors.clone();
                ancestors.push(resolved.target.clone());
                let pending = PendingDir {
                    path,
                    target: resolved.target,
                    ancestors,
                };
                if resolved.chain.is_empty() {
                    direct.push_back(pending);
                } else {
                    linked.push_back(pending);
                }
            }
        }
        walk.entries.sort_by(|a, b| a.path.cmp(&b.path));
        walk.issues.sort_by_key(WalkIssue::sort_key);
        Ok(walk)
    }

    /// 트리를 순회해 깨진 링크·순환 링크를 표시용 진단으로 모읍니다(경로 순).
    ///
    /// 링크는 삭제·수정하지 않습니다. `include`는 [`PathResolver::walk_tree_with`]와 같습니다(최상위
    /// 이름만 받음).
    pub fn symlink_diagnostics(
        &self,
        include: impl Fn(&str) -> bool,
    ) -> Result<Vec<SymlinkDiagnostic>, ResolveError> {
        Ok(self
            .walk_tree(include)?
            .issues
            .iter()
            .filter_map(SymlinkDiagnostic::from_issue)
            .collect())
    }

    /// 프로젝트 상대 경로의 파일 내용을 읽습니다(내부·외부 symlink target 모두 허용).
    ///
    /// 비신뢰 프로젝트에서 외부 target이면 파일을 열지 않고
    /// [`ResolveError::ExternalTargetNotTrusted`]를 반환합니다.
    pub fn read(&self, project_path: &str) -> Result<Vec<u8>, ResolveError> {
        let resolved = self.resolve(project_path)?;
        if resolved.kind != TargetKind::File {
            return Err(ResolveError::NotAFile {
                path: project_path.to_string(),
            });
        }
        self.check_content_access(&resolved)?;
        open_content(&resolved.target)
    }

    /// 프로젝트 상대 경로의 디렉터리 항목 이름을 정렬해 반환합니다(디렉터리 symlink 허용).
    ///
    /// 비신뢰 프로젝트에서 외부 target이면 목록을 읽지 않고
    /// [`ResolveError::ExternalTargetNotTrusted`]를 반환합니다.
    pub fn read_dir(&self, project_path: &str) -> Result<Vec<OsString>, ResolveError> {
        let resolved = self.resolve(project_path)?;
        if resolved.kind != TargetKind::Directory {
            return Err(ResolveError::NotADirectory {
                path: project_path.to_string(),
            });
        }
        self.check_content_access(&resolved)?;
        list_content(&resolved.target)
    }
}

/// 파일 내용 접근(유일한 지점). 호출 전에 정책 검사를 마쳐야 합니다.
fn open_content(path: &Path) -> Result<Vec<u8>, ResolveError> {
    content_access::record(path);
    std::fs::read(path).map_err(|e| io_error(path, &e))
}

/// 디렉터리 내용(항목 이름) 접근(유일한 지점). 이름 순으로 정렬해 반환합니다.
fn list_content(dir: &Path) -> Result<Vec<OsString>, ResolveError> {
    content_access::record(dir);
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| io_error(dir, &e))? {
        names.push(entry.map_err(|e| io_error(dir, &e))?.file_name());
    }
    names.sort();
    Ok(names)
}

/// 내용 접근 기록(테스트 빌드에서만 기록, 그 외에는 빈 함수).
mod content_access {
    use std::path::Path;

    #[cfg(test)]
    thread_local! {
        static LOG: std::cell::RefCell<Vec<std::path::PathBuf>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    /// 내용 접근 직전에 호출됩니다.
    #[cfg(test)]
    pub(super) fn record(path: &Path) {
        LOG.with(|log| log.borrow_mut().push(path.to_path_buf()));
    }

    #[cfg(not(test))]
    pub(super) fn record(_path: &Path) {}

    /// 현재 스레드의 기록을 꺼내고 비웁니다.
    #[cfg(test)]
    pub(super) fn take() -> Vec<std::path::PathBuf> {
        LOG.with(|log| std::mem::take(&mut *log.borrow_mut()))
    }
}

/// 해석 대기 컴포넌트의 출처.
#[derive(Debug, Clone, Copy)]
enum Origin {
    /// 요청 경로의 컴포넌트(index).
    Request(usize),
    /// 체인의 hop(index)이 원문 target에서 만든 컴포넌트.
    Hop(usize),
}

/// 트리 순회에서 외부 디렉터리를 다루는 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalDescent {
    /// 외부 디렉터리 안으로도 내려갑니다(비신뢰 프로젝트 정책은 그대로 적용).
    Follow,
    /// 외부 디렉터리는 항목으로만 넣고 목록을 읽지 않습니다(외부 target root만 필요할 때).
    Stop,
}

/// 목록을 읽을 차례를 기다리는 디렉터리.
struct PendingDir {
    /// 프로젝트 상대 경로(root는 빈 문자열).
    path: String,
    /// 실제 디렉터리.
    target: PathBuf,
    /// 이 경로의 조상 실제 디렉터리들(자신 포함).
    ancestors: Vec<PathBuf>,
}

/// 컴포넌트 단위 해석 결과.
struct Walk {
    /// symlink가 없는 실제 경로.
    resolved: PathBuf,
    symlinks: Vec<SymlinkHop>,
    chain: Vec<ChainHop>,
}

/// 트리 순회 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkEntry {
    /// 프로젝트 상대 경로(`/` 구분).
    pub path: String,
    /// 실제 항목 종류.
    pub kind: TargetKind,
    /// 실제 위치(내부/외부).
    pub location: TargetLocation,
    /// 경로 체인에 symlink가 있는지.
    pub via_symlink: bool,
}

/// 트리 순회에서 들어가지 않거나 해석하지 못한 항목.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkIssue {
    /// 해석할 수 없는 항목(symlink 순환·hop 초과·없는 target 등). 순회는 계속합니다.
    Unresolvable {
        /// 프로젝트 상대 경로.
        path: String,
        /// 해석 오류.
        error: ResolveError,
    },
    /// 현재 순회 경로의 조상 디렉터리를 다시 가리키는 디렉터리(들어가면 무한 순회).
    DirectoryCycle {
        /// 프로젝트 상대 경로.
        path: String,
        /// 가리키는 조상 디렉터리의 실제 경로.
        target: PathBuf,
    },
    /// 이미 다른 경로로 목록을 읽은 실제 디렉터리(항목으로는 포함, 다시 내려가지 않음).
    AlreadyTraversed {
        /// 프로젝트 상대 경로.
        path: String,
        /// 실제 디렉터리.
        target: PathBuf,
        /// 그 디렉터리를 읽은 프로젝트 상대 경로(root는 빈 문자열).
        traversed_as: String,
    },
    /// 목록을 읽지 못한 하위 디렉터리(권한 없음 등, 항목으로는 포함). 순회는 계속합니다.
    DirectoryUnreadable {
        /// 프로젝트 상대 경로.
        path: String,
        /// 목록 읽기 오류.
        error: ResolveError,
    },
    /// 비신뢰 프로젝트라 목록을 읽지 않은 외부 디렉터리(항목으로는 포함).
    ExternalNotTraversed {
        /// 프로젝트 상대 경로.
        path: String,
        /// 외부 target 경로(metadata).
        target: PathBuf,
    },
    /// UTF-8이 아닌 이름(프로젝트 상대 경로로 표현할 수 없음).
    NonUtf8Name {
        /// 실제 경로.
        path: PathBuf,
    },
}

impl WalkIssue {
    /// 정렬 키(프로젝트 상대 경로, UTF-8이 아닌 이름은 실제 경로의 손실 변환).
    fn sort_key(&self) -> String {
        match self {
            WalkIssue::Unresolvable { path, .. }
            | WalkIssue::DirectoryCycle { path, .. }
            | WalkIssue::AlreadyTraversed { path, .. }
            | WalkIssue::DirectoryUnreadable { path, .. }
            | WalkIssue::ExternalNotTraversed { path, .. } => path.clone(),
            WalkIssue::NonUtf8Name { path } => path.to_string_lossy().into_owned(),
        }
    }
}

/// symlink 진단 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymlinkDiagnosticKind {
    /// 깨진 링크(target 없음).
    Broken,
    /// 링크 순환.
    Cycle,
    /// hop 상한 초과.
    TooManyHops,
    /// 조상 디렉터리를 다시 가리키는 디렉터리 링크(순회하지 않음).
    DirectoryCycle,
}

impl SymlinkDiagnosticKind {
    /// 고정 진단 코드.
    pub fn code(self) -> &'static str {
        match self {
            SymlinkDiagnosticKind::Broken => "symlink-broken",
            SymlinkDiagnosticKind::Cycle => "symlink-cycle",
            SymlinkDiagnosticKind::TooManyHops => "symlink-too-many-hops",
            SymlinkDiagnosticKind::DirectoryCycle => "symlink-directory-cycle",
        }
    }
}

/// 표시용 symlink 진단 하나. 링크를 고치거나 지우는 동작은 포함하지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymlinkDiagnostic {
    /// 진단 종류.
    pub kind: SymlinkDiagnosticKind,
    /// 문제가 드러난 항목의 프로젝트 상대 경로.
    pub path: String,
    /// 문제 링크의 실제 위치(깨진 링크, 순환을 다시 만난 링크). 없으면 `None`.
    pub link: Option<PathBuf>,
    /// 문제 링크의 원문 target 또는 순환 디렉터리의 실제 target. 없으면 `None`.
    pub target: Option<PathBuf>,
}

impl SymlinkDiagnostic {
    /// 트리 순회 문제를 진단으로 바꿉니다. symlink와 무관한 문제(일반 I/O 등)는 `None`.
    pub fn from_issue(issue: &WalkIssue) -> Option<SymlinkDiagnostic> {
        let (kind, path, link, target) = match issue {
            WalkIssue::Unresolvable { path, error } => match error {
                ResolveError::BrokenSymlink {
                    link, raw_target, ..
                } => (
                    SymlinkDiagnosticKind::Broken,
                    path,
                    Some(link.clone()),
                    Some(raw_target.clone()),
                ),
                ResolveError::SymlinkCycle { cycle, .. } => (
                    SymlinkDiagnosticKind::Cycle,
                    path,
                    cycle.last().cloned(),
                    None,
                ),
                ResolveError::TooManySymlinkHops { .. } => {
                    (SymlinkDiagnosticKind::TooManyHops, path, None, None)
                }
                _ => return None,
            },
            WalkIssue::DirectoryCycle { path, target } => (
                SymlinkDiagnosticKind::DirectoryCycle,
                path,
                None,
                Some(target.clone()),
            ),
            WalkIssue::AlreadyTraversed { .. }
            | WalkIssue::DirectoryUnreadable { .. }
            | WalkIssue::ExternalNotTraversed { .. }
            | WalkIssue::NonUtf8Name { .. } => return None,
        };
        Some(SymlinkDiagnostic {
            kind,
            path: path.clone(),
            link,
            target,
        })
    }
}

/// 트리 순회 결과.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TreeWalk {
    /// 방문한 항목(프로젝트 상대 경로 순).
    pub entries: Vec<WalkEntry>,
    /// 들어가지 않거나 해석하지 못한 항목(경로 순).
    pub issues: Vec<WalkIssue>,
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

    /// 프로젝트 root 기준 링크 위치.
    fn link_at(resolver: &PathResolver, rel: &str) -> PathBuf {
        resolver.canonical_root().join(rel)
    }

    #[cfg(unix)]
    #[test]
    fn self_cycle_is_an_error() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        symlink("self", fx.project.join("self")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();
        assert_eq!(
            resolver.resolve("self"),
            Err(ResolveError::SymlinkCycle {
                path: "self".to_string(),
                cycle: vec![link_at(&resolver, "self"), link_at(&resolver, "self")],
            })
        );
        // 읽기·목록도 같은 오류로 멈춤(무한 순회 없음).
        assert!(matches!(
            resolver.read("self"),
            Err(ResolveError::SymlinkCycle { .. })
        ));
        assert!(matches!(
            resolver.read_dir("self"),
            Err(ResolveError::SymlinkCycle { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn two_node_cycle_is_an_error() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        symlink("b", fx.project.join("a")).unwrap();
        symlink("a", fx.project.join("b")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();
        assert_eq!(
            resolver.resolve("a"),
            Err(ResolveError::SymlinkCycle {
                path: "a".to_string(),
                cycle: vec![
                    link_at(&resolver, "a"),
                    link_at(&resolver, "b"),
                    link_at(&resolver, "a"),
                ],
            })
        );
        // 순환 링크를 거치는 하위 경로도 오류.
        assert!(matches!(
            resolver.resolve("b/x"),
            Err(ResolveError::SymlinkCycle { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn deep_cycle_behind_a_chain_is_an_error() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        let outside = std::fs::canonicalize(&fx.outside).unwrap();
        // 진입 체인 e0 -> e1 -> ... -> e7 -> c0, 순환 c0 -> data/sub/c1 -> (외부) x -> c2 -> c0.
        for i in 0..7 {
            symlink(format!("e{}", i + 1), fx.project.join(format!("e{i}"))).unwrap();
        }
        symlink("c0", fx.project.join("e7")).unwrap();
        symlink("data/sub/c1", fx.project.join("c0")).unwrap();
        symlink(outside.join("x"), fx.project.join("data/sub/c1")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();
        symlink(resolver.canonical_root().join("c2"), outside.join("x")).unwrap();
        symlink("c0", fx.project.join("c2")).unwrap();

        match resolver.resolve("e0") {
            Err(ResolveError::SymlinkCycle { path, cycle }) => {
                assert_eq!(path, "e0");
                assert_eq!(
                    cycle,
                    vec![
                        link_at(&resolver, "c0"),
                        link_at(&resolver, "data/sub/c1"),
                        outside.join("x"),
                        link_at(&resolver, "c2"),
                        link_at(&resolver, "c0"),
                    ]
                );
            }
            other => panic!("SymlinkCycle이어야 함: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn hop_limit_bounds_long_chains_and_long_cycles() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        // 정확히 MAX_SYMLINK_HOPS hop인 비순환 체인은 허용: h0 -> h1 -> ... -> h31 -> data/real.txt.
        for i in 0..MAX_SYMLINK_HOPS - 1 {
            symlink(format!("h{}", i + 1), fx.project.join(format!("h{i}"))).unwrap();
        }
        symlink(
            "data/real.txt",
            fx.project.join(format!("h{}", MAX_SYMLINK_HOPS - 1)),
        )
        .unwrap();
        // 한 hop 더 긴 체인은 거부.
        symlink("h0", fx.project.join("over")).unwrap();
        // 상한보다 긴 순환(40개): 반복 전에 hop 상한에서 멈춤.
        let n = MAX_SYMLINK_HOPS + 8;
        for i in 0..n {
            symlink(
                format!("r{}", (i + 1) % n),
                fx.project.join(format!("r{i}")),
            )
            .unwrap();
        }
        // 남은 경로가 계속 늘어나 상태가 반복되지 않는 순환.
        symlink("grow/x", fx.project.join("grow")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();

        let resolved = resolver.resolve("h0").unwrap();
        assert_eq!(resolved.chain.len(), MAX_SYMLINK_HOPS);
        assert_eq!(resolver.read("h0").unwrap(), b"internal-bytes");
        for path in ["over", "r0", "grow"] {
            assert_eq!(
                resolver.resolve(path),
                Err(ResolveError::TooManySymlinkHops {
                    path: path.to_string(),
                    limit: MAX_SYMLINK_HOPS,
                }),
                "{path}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn revisiting_a_link_with_a_different_suffix_is_not_a_cycle() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        // L -> data, data/x -> ../L/real.txt: L을 두 번 지나지만 남은 경로가 달라 유한합니다.
        symlink("data", fx.project.join("L")).unwrap();
        symlink("../L/real.txt", fx.project.join("data/x")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();
        let resolved = resolver.resolve("L/x").unwrap();
        assert_eq!(resolver.read("L/x").unwrap(), b"internal-bytes");
        let links: Vec<PathBuf> = resolved.chain.iter().map(|h| h.link.clone()).collect();
        assert_eq!(
            links,
            vec![
                link_at(&resolver, "L"),
                link_at(&resolver, "data/x"),
                link_at(&resolver, "L"),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn tree_walk_terminates_on_directory_and_link_cycles() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        // 조상(root)을 가리키는 디렉터리 링크, 자기 순환 링크, 외부 디렉터리 링크.
        symlink("../..", fx.project.join("data/sub/up")).unwrap();
        symlink("loop", fx.project.join("loop")).unwrap();
        symlink(fx.outside.join("shared"), fx.project.join("shared")).unwrap();
        // 외부 디렉터리 안에서 프로젝트 root로 되돌아오는 링크.
        symlink(&fx.project, fx.outside.join("shared/back")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();

        let walk = resolver.walk_tree(|_| true).unwrap();
        let paths: Vec<&str> = walk.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "data",
                "data/real.txt",
                "data/sub",
                "shared",
                "shared/asset.bin",
                "shared/other.bin",
            ]
        );
        assert_eq!(
            walk.issues,
            vec![
                WalkIssue::DirectoryCycle {
                    path: "data/sub/up".to_string(),
                    target: resolver.canonical_root().to_path_buf(),
                },
                WalkIssue::Unresolvable {
                    path: "loop".to_string(),
                    error: ResolveError::SymlinkCycle {
                        path: "loop".to_string(),
                        cycle: vec![link_at(&resolver, "loop"), link_at(&resolver, "loop")],
                    },
                },
                WalkIssue::DirectoryCycle {
                    path: "shared/back".to_string(),
                    target: resolver.canonical_root().to_path_buf(),
                },
            ]
        );
        let shared = walk.entries.iter().find(|e| e.path == "shared").unwrap();
        assert_eq!(shared.location, TargetLocation::External);
        assert!(shared.via_symlink);
        // 조상 링크를 거친 유한 경로 해석은 그대로 허용.
        assert_eq!(
            resolver.read("data/sub/up/data/real.txt").unwrap(),
            b"internal-bytes"
        );
        // 링크는 그대로 남음.
        assert!(std::fs::symlink_metadata(fx.project.join("loop"))
            .unwrap()
            .file_type()
            .is_symlink());
    }

    #[test]
    fn tree_walk_respects_include_filter() {
        let fx = fixture();
        std::fs::create_dir_all(fx.project.join(".git/objects")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();
        let walk = resolver.walk_tree(|p| p != ".git").unwrap();
        assert!(walk.entries.iter().all(|e| !e.path.starts_with(".git")));
        assert!(walk.issues.is_empty());
        assert_eq!(walk.entries.len(), 3);
    }

    #[cfg(unix)]
    #[test]
    fn include_sees_only_top_level_names_so_shared_children_do_not_depend_on_arrival_order() {
        use std::cell::RefCell;
        use std::os::unix::fs::symlink;
        let fx = fixture();
        // 외부 `shared`를 두 별칭(mm이 먼저 도달)으로 가리킴.
        symlink(fx.outside.join("shared"), fx.project.join("zz")).unwrap();
        symlink(fx.outside.join("shared"), fx.project.join("mm")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();
        let seen = RefCell::new(Vec::new());
        let all = resolver
            .walk_tree(|path| {
                seen.borrow_mut().push(path.to_string());
                true
            })
            .unwrap();
        let seen = seen.into_inner();
        assert!(!seen.is_empty());
        // 계약은 코드로 강제됩니다: include는 최상위 이름만 받습니다(더 깊은 경로로 거를 수 없음).
        assert!(
            seen.iter().all(|path| !path.contains('/')),
            "include가 하위 경로를 받음: {seen:?}"
        );
        // 그래서 하위 경로를 거르려는 include도 먼저 도달한 별칭(mm)의 공유 하위 항목을 지우지 못합니다.
        let deeper = resolver.walk_tree(|path| path != "mm/asset.bin").unwrap();
        assert_eq!(deeper.entries, all.entries);
        assert!(all.entries.iter().any(|e| e.path == "mm/asset.bin"));
    }

    /// 순회가 목록을 읽은 디렉터리(내용 접근 기록 중 디렉터리만).
    fn listed_dirs() -> Vec<PathBuf> {
        content_access::take()
            .into_iter()
            .filter(|path| path.is_dir())
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn tree_walk_lists_each_shared_directory_once_in_a_symlink_dag() {
        use std::os::unix::fs::symlink;
        // 회귀(검토 지적): D0..D(n-1)의 각 D_k에 `a`·`b` 두 링크가 모두 D_(k+1)을 가리키면 경로 수가
        // 2^n으로 늘어납니다. 같은 실제 디렉터리는 한 번만 읽어야 합니다.
        const LEVELS: usize = 14;
        let fx = fixture();
        for k in 0..LEVELS {
            std::fs::create_dir_all(fx.project.join(format!("D{k:02}"))).unwrap();
        }
        for k in 0..LEVELS - 1 {
            for name in ["a", "b"] {
                symlink(
                    format!("../D{:02}", k + 1),
                    fx.project.join(format!("D{k:02}/{name}")),
                )
                .unwrap();
            }
        }
        std::fs::write(
            fx.project.join(format!("D{:02}/leaf.txt", LEVELS - 1)),
            b"x",
        )
        .unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();
        let _ = content_access::take();

        let walk = resolver.walk_tree(|_| true).unwrap();
        let listed = listed_dirs();
        let mut unique = listed.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(listed.len(), unique.len(), "같은 디렉터리를 두 번 읽음");
        // root, data, data/sub, D00..D13.
        assert_eq!(listed.len(), 3 + LEVELS);
        // 항목: data 3개 + D 디렉터리 + 링크 디렉터리 2개씩 + leaf. 지수적으로 늘지 않음.
        assert_eq!(walk.entries.len(), 3 + LEVELS + 2 * (LEVELS - 1) + 1);
        let leaf = format!("D{:02}/leaf.txt", LEVELS - 1);
        assert!(walk.entries.iter().any(|e| e.path == leaf));
        // 링크는 실제 위치로 이미 읽은 디렉터리를 가리킴(실제 경로 우선).
        let canonical = resolver.canonical_root().to_path_buf();
        let expected: Vec<WalkIssue> = (0..LEVELS - 1)
            .flat_map(|k| {
                let canonical = canonical.clone();
                ["a", "b"].map(move |name| WalkIssue::AlreadyTraversed {
                    path: format!("D{k:02}/{name}"),
                    target: canonical.join(format!("D{:02}", k + 1)),
                    traversed_as: format!("D{:02}", k + 1),
                })
            })
            .collect();
        assert_eq!(walk.issues, expected);
        // 공유 디렉터리는 진단(문제)이 아님.
        assert!(resolver.symlink_diagnostics(|_| true).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn shared_directory_reached_only_through_links_is_read_once_by_the_shortest_path() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        // 외부 `shared`를 가리키는 링크 셋(깊이·이름이 다름).
        std::fs::create_dir_all(fx.project.join("deep/er")).unwrap();
        symlink(fx.outside.join("shared"), fx.project.join("zz")).unwrap();
        symlink(fx.outside.join("shared"), fx.project.join("mm")).unwrap();
        symlink(fx.outside.join("shared"), fx.project.join("deep/er/link")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();
        let _ = content_access::take();
        let walk = resolver.walk_tree(|_| true).unwrap();
        let shared = std::fs::canonicalize(fx.outside.join("shared")).unwrap();
        let listed = listed_dirs();
        assert_eq!(listed.iter().filter(|p| **p == shared).count(), 1);
        let paths: Vec<&str> = walk.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "data",
                "data/real.txt",
                "data/sub",
                "deep",
                "deep/er",
                "deep/er/link",
                "mm",
                "mm/asset.bin",
                "mm/other.bin",
                "zz",
            ]
        );
        let already = |path: &str| WalkIssue::AlreadyTraversed {
            path: path.to_string(),
            target: shared.clone(),
            traversed_as: "mm".to_string(),
        };
        assert_eq!(walk.issues, vec![already("deep/er/link"), already("zz")]);
    }

    /// 디렉터리 권한을 `mode`로 바꿉니다. 이 프로세스가 그래도 목록을 읽을 수 있으면(root 실행 등)
    /// `false`입니다.
    #[cfg(unix)]
    fn make_unlistable(dir: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_dir(dir).is_ok() {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
            return false;
        }
        true
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_subdirectory_is_an_issue_and_the_walk_continues() {
        use std::os::unix::fs::PermissionsExt;
        let fx = fixture();
        std::fs::create_dir_all(fx.project.join("locked")).unwrap();
        std::fs::write(fx.project.join("locked/hidden.txt"), b"h").unwrap();
        std::fs::write(fx.project.join("zz.txt"), b"z").unwrap();
        let locked = fx.project.join("locked");
        if !make_unlistable(&locked) {
            return; // 권한 검사가 없는 환경(root)에서는 건너뜀.
        }
        let resolver = PathResolver::new(&fx.project).unwrap();
        let walk = resolver.walk_tree(|_| true);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let walk = walk.unwrap();
        let paths: Vec<&str> = walk.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["data", "data/real.txt", "data/sub", "locked", "zz.txt"]
        );
        assert!(matches!(
            walk.issues.as_slice(),
            [WalkIssue::DirectoryUnreadable {
                path,
                error: ResolveError::Io {
                    kind: io::ErrorKind::PermissionDenied,
                    ..
                },
            }] if path == "locked"
        ));
        // root를 읽지 못하면 순회 실패.
        let gone = fx.project.join("gone");
        std::fs::create_dir_all(&gone).unwrap();
        let broken = PathResolver::new(&gone).unwrap();
        std::fs::remove_dir_all(&gone).unwrap();
        assert!(broken.walk_tree(|_| true).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn stop_descent_does_not_list_external_directories() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        symlink(fx.outside.join("shared"), fx.project.join("shared")).unwrap();
        symlink(&fx.outside, fx.project.join("parent")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();
        let _ = content_access::take();
        let walk = resolver
            .walk_tree_with(ExternalDescent::Stop, |_| true)
            .unwrap();
        let outside = std::fs::canonicalize(&fx.outside).unwrap();
        assert!(
            listed_dirs().iter().all(|p| !p.starts_with(&outside)),
            "외부 디렉터리 목록을 읽음"
        );
        let external: Vec<&str> = walk
            .entries
            .iter()
            .filter(|e| e.location == TargetLocation::External)
            .map(|e| e.path.as_str())
            .collect();
        assert_eq!(external, vec!["parent", "shared"]);
        assert!(walk.issues.is_empty(), "{:?}", walk.issues);
    }

    #[cfg(unix)]
    #[test]
    fn broken_links_are_reported_with_the_broken_hop() {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        symlink("data/missing.txt", fx.project.join("broken")).unwrap();
        symlink("nowhere", fx.project.join("broken-dir")).unwrap();
        // 파일을 디렉터리처럼 가리키는 링크.
        symlink("data/real.txt/x", fx.project.join("through-file")).unwrap();
        // 체인 중간이 깨진 경우: head -> mid -> (없음). 깨진 hop은 mid.
        symlink("gone", fx.project.join("mid")).unwrap();
        symlink("mid", fx.project.join("head")).unwrap();
        let resolver = PathResolver::new(&fx.project).unwrap();
        let root = resolver.canonical_root().to_path_buf();

        for (path, link, raw) in [
            ("broken", "broken", "data/missing.txt"),
            ("broken-dir/inner.txt", "broken-dir", "nowhere"),
            ("through-file", "through-file", "data/real.txt/x"),
            ("head", "mid", "gone"),
        ] {
            assert_eq!(
                resolver.resolve(path),
                Err(ResolveError::BrokenSymlink {
                    path: path.to_string(),
                    link: root.join(link),
                    raw_target: PathBuf::from(raw),
                }),
                "{path}"
            );
        }
        // 링크가 없는 일반 경로의 부재, 정상 링크 아래 없는 항목은 broken link가 아님.
        symlink("data", fx.project.join("ok-dir")).unwrap();
        for path in ["data/missing.txt", "ok-dir/missing.txt"] {
            assert!(
                matches!(
                    resolver.resolve(path),
                    Err(ResolveError::Io {
                        kind: io::ErrorKind::NotFound,
                        ..
                    })
                ),
                "{path}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn project_with_broken_links_opens_and_other_data_is_intact() {
        use crate::probe::{probe_project, safe_probe};
        use crate::root::MANIFEST_FILE;
        use crate::trust::TrustStatus;
        use std::os::unix::fs::symlink;
        let fx = fixture();
        std::fs::create_dir_all(fx.project.join(".git")).unwrap();
        std::fs::write(
            fx.project.join(MANIFEST_FILE),
            "schema_version: 1\nproject:\n  id: 0195c530-3acd-7bc1-8414-c213f61cb965\n  name: Demo\ntarget_profiles: []\ndependencies: {}\n",
        )
        .unwrap();
        symlink("data/missing.txt", fx.project.join("broken")).unwrap();
        symlink(
            fx.outside.join("deleted-dir"),
            fx.project.join("data/ext-broken"),
        )
        .unwrap();
        symlink("loop", fx.project.join("loop")).unwrap();
        let before = list(&fx.project);
        let raw_before = std::fs::read_link(fx.project.join("broken")).unwrap();

        // 프로젝트 열기(probe)는 깨진 링크와 무관하게 성공.
        probe_project(&fx.project).unwrap();
        safe_probe(&fx.project, TrustStatus::NotTrusted).unwrap();

        let resolver = PathResolver::new(&fx.project).unwrap();
        // 다른 데이터는 정상적으로 읽힘.
        assert_eq!(resolver.read("data/real.txt").unwrap(), b"internal-bytes");
        let walk = resolver.walk_tree(|p| p != ".git").unwrap();
        let paths: Vec<&str> = walk.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["data", "data/real.txt", "data/sub", MANIFEST_FILE]
        );

        // 깨진 링크·순환은 진단으로 표시.
        let diagnostics = resolver.symlink_diagnostics(|p| p != ".git").unwrap();
        let root = resolver.canonical_root().to_path_buf();
        assert_eq!(
            diagnostics,
            vec![
                SymlinkDiagnostic {
                    kind: SymlinkDiagnosticKind::Broken,
                    path: "broken".to_string(),
                    link: Some(root.join("broken")),
                    target: Some(PathBuf::from("data/missing.txt")),
                },
                SymlinkDiagnostic {
                    kind: SymlinkDiagnosticKind::Broken,
                    path: "data/ext-broken".to_string(),
                    link: Some(root.join("data/ext-broken")),
                    target: Some(fx.outside.join("deleted-dir")),
                },
                SymlinkDiagnostic {
                    kind: SymlinkDiagnosticKind::Cycle,
                    path: "loop".to_string(),
                    link: Some(root.join("loop")),
                    target: None,
                },
            ]
        );
        assert_eq!(diagnostics[0].kind.code(), "symlink-broken");

        // 깨진 링크를 삭제·수정하지 않음.
        assert_eq!(list(&fx.project), before);
        assert_eq!(
            std::fs::read_link(fx.project.join("broken")).unwrap(),
            raw_before
        );
        assert!(
            std::fs::symlink_metadata(fx.project.join("data/ext-broken"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn diagnostic_kinds_have_stable_codes() {
        assert_eq!(SymlinkDiagnosticKind::Broken.code(), "symlink-broken");
        assert_eq!(SymlinkDiagnosticKind::Cycle.code(), "symlink-cycle");
        assert_eq!(
            SymlinkDiagnosticKind::TooManyHops.code(),
            "symlink-too-many-hops"
        );
        assert_eq!(
            SymlinkDiagnosticKind::DirectoryCycle.code(),
            "symlink-directory-cycle"
        );
        // symlink와 무관한 문제는 진단이 아님.
        let io_issue = WalkIssue::Unresolvable {
            path: "x".to_string(),
            error: ResolveError::Io {
                path: PathBuf::from("/x"),
                kind: io::ErrorKind::PermissionDenied,
            },
        };
        assert_eq!(SymlinkDiagnostic::from_issue(&io_issue), None);
    }

    /// 외부 파일·디렉터리 링크와, 외부를 거쳐 내부로 돌아오는 링크를 가진 fixture.
    #[cfg(unix)]
    fn untrusted_fixture() -> Fixture {
        use std::os::unix::fs::symlink;
        let fx = fixture();
        symlink(fx.outside.join("secret.txt"), fx.project.join("ext.txt")).unwrap();
        symlink(fx.outside.join("shared"), fx.project.join("shared")).unwrap();
        symlink(fx.project.join("data/real.txt"), fx.outside.join("back-in")).unwrap();
        symlink(fx.outside.join("back-in"), fx.project.join("via-ext")).unwrap();
        fx
    }

    /// 기록된 내용 접근 중 `outside` 아래 경로.
    fn external_accesses(fx: &Fixture) -> Vec<PathBuf> {
        let outside = std::fs::canonicalize(&fx.outside).unwrap();
        content_access::take()
            .into_iter()
            .filter(|p| p.starts_with(&outside) || p.starts_with(&fx.outside))
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn untrusted_project_never_reads_external_target_content() {
        let fx = untrusted_fixture();
        let identity = ProjectIdentity::new(
            stellar_core_types::ids::StableId::new_v7(1, 0, 1),
            &fx.project,
        )
        .unwrap();
        let untrusted = PathResolver::for_project(&identity, TrustStatus::NotTrusted);
        assert_eq!(untrusted.external_access(), ExternalAccess::MetadataOnly);
        let _ = content_access::take();

        let secret = std::fs::canonicalize(fx.outside.join("secret.txt")).unwrap();
        assert_eq!(
            untrusted.read("ext.txt"),
            Err(ResolveError::ExternalTargetNotTrusted {
                path: "ext.txt".to_string(),
                target: secret.clone(),
            })
        );
        for path in ["shared", "shared/asset.bin"] {
            assert!(matches!(
                if path == "shared" {
                    untrusted.read_dir(path).map(|_| ())
                } else {
                    untrusted.read(path).map(|_| ())
                },
                Err(ResolveError::ExternalTargetNotTrusted { .. })
            ));
        }
        let walk = untrusted.walk_tree(|_| true).unwrap();
        // 외부 target 내용에 대한 접근 기록 0.
        assert_eq!(external_accesses(&fx), Vec::<PathBuf>::new());

        // 내부 데이터, 외부 링크를 거쳐 내부로 돌아오는 경로는 그대로 읽힘.
        assert_eq!(untrusted.read("data/real.txt").unwrap(), b"internal-bytes");
        assert_eq!(untrusted.read("via-ext").unwrap(), b"internal-bytes");
        // 외부 디렉터리는 항목(metadata)으로만 나오고 하위 항목은 없음.
        let paths: Vec<&str> = walk.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"shared") && paths.contains(&"ext.txt"));
        assert!(!paths.iter().any(|p| p.starts_with("shared/")));
        assert!(walk.issues.contains(&WalkIssue::ExternalNotTraversed {
            path: "shared".to_string(),
            target: std::fs::canonicalize(fx.outside.join("shared")).unwrap(),
        }));

        // 같은 동작을 신뢰된 resolver로 하면 외부 내용 접근이 기록됨(기록 장치 자체 검증).
        let trusted = PathResolver::for_project(&identity, TrustStatus::Trusted);
        assert_eq!(trusted.read("ext.txt").unwrap(), b"external-bytes");
        let trusted_walk = trusted.walk_tree(|_| true).unwrap();
        assert!(trusted_walk
            .entries
            .iter()
            .any(|e| e.path == "shared/asset.bin"));
        assert!(!external_accesses(&fx).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn untrusted_project_shows_limited_target_metadata() {
        let fx = untrusted_fixture();
        let untrusted = PathResolver::new(&fx.project)
            .unwrap()
            .with_trust(TrustStatus::NotTrusted);
        let _ = content_access::take();
        let info = untrusted.external_target_info("ext.txt").unwrap();
        assert_eq!(
            info,
            ExternalTargetInfo {
                path: "ext.txt".to_string(),
                location: TargetLocation::External,
                kind: TargetKind::File,
                target: std::fs::canonicalize(fx.outside.join("secret.txt")).unwrap(),
                raw_target: Some(fx.outside.join("secret.txt")),
            }
        );
        let dir = untrusted.external_target_info("shared").unwrap();
        assert_eq!(dir.kind, TargetKind::Directory);
        assert_eq!(dir.location, TargetLocation::External);
        let internal = untrusted.external_target_info("data/real.txt").unwrap();
        assert_eq!(internal.location, TargetLocation::Internal);
        assert_eq!(internal.raw_target, None);
        // metadata 조회는 내용 접근이 아님.
        assert!(content_access::take().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn untrusted_reads_do_not_open_fifo_or_unreadable_targets() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        use std::sync::mpsc;
        use std::time::Duration;
        let fx = fixture();
        // 열면(open) 쓰는 쪽이 올 때까지 막히는 FIFO, 권한 0의 파일.
        let pipe = fx.outside.join("pipe");
        let status = std::process::Command::new("mkfifo")
            .arg(&pipe)
            .status()
            .unwrap();
        assert!(status.success());
        let locked = fx.outside.join("locked.bin");
        std::fs::write(&locked, b"x").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        symlink(&pipe, fx.project.join("pipe")).unwrap();
        symlink(&locked, fx.project.join("locked")).unwrap();
        let untrusted = PathResolver::new(&fx.project)
            .unwrap()
            .with_trust(TrustStatus::NotTrusted);

        let (tx, rx) = mpsc::channel();
        let worker = untrusted.clone();
        std::thread::spawn(move || {
            let info = worker.external_target_info("pipe");
            let read = worker.read("pipe");
            let locked = worker.read("locked");
            let _ = tx.send((info, read, locked));
        });
        // FIFO를 열었다면 여기서 시간 초과로 실패합니다.
        let (info, read, locked_read) = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("FIFO target을 열어 막힘");
        let info = info.unwrap();
        assert_eq!(info.kind, TargetKind::Other);
        assert_eq!(info.location, TargetLocation::External);
        // FIFO는 파일이 아니므로 NotAFile(열지 않음), 권한 0 파일은 권한 오류가 아닌 정책 거부.
        assert_eq!(
            read,
            Err(ResolveError::NotAFile {
                path: "pipe".to_string()
            })
        );
        assert!(matches!(
            locked_read,
            Err(ResolveError::ExternalTargetNotTrusted { .. })
        ));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[test]
    fn trust_maps_to_external_access_policy() {
        assert_eq!(
            ExternalAccess::from_trust(TrustStatus::Trusted),
            ExternalAccess::Allowed
        );
        assert_eq!(
            ExternalAccess::from_trust(TrustStatus::NotTrusted),
            ExternalAccess::MetadataOnly
        );
        let fx = fixture();
        assert_eq!(
            PathResolver::new(&fx.project).unwrap().external_access(),
            ExternalAccess::Allowed
        );
    }
}

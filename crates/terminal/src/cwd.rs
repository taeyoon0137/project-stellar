//! 터미널 작업 디렉터리 선택·preset·검증 (XVIII.1.1, XVIII.1.3).
//!
//! 세션의 cwd는 프로젝트 루트, 사용자가 고른 worktree root, 사용자가 명시 입력한 임의 경로 중
//! 하나입니다(기술 아키텍처 32 "프로젝트 루트 또는 특정 worktree에 귀속").
//!
//! - **preset**: [`CwdContext::presets`]는 프로젝트 루트와 worktree root 목록을 UI 선택지로 줍니다.
//!   worktree root 목록은 호출자가 시스템 Git으로 얻어 공급합니다(XIV.1.2의 `git rev-parse`
//!   결과 등). 이 crate는 Git을 실행하거나 `.git` 구조로 경로를 추측하지 않습니다.
//! - **worktree는 시스템 Git 기능**(XVIII.5.3): worktree 생성·삭제·전환은 시스템 Git(외부 터미널이나
//!   Git UI)이 하고, 터미널은 그 결과 root를 cwd preset으로만 제공합니다. 외부 CLI 에이전트 세션도
//!   같은 preset을 쓰며, agent 전용 worktree 타입을 두지 않습니다(`tests/worktree_type_inventory.rs`).
//! - **임의 cwd**: [`ExplicitCwd::from_user_input`]으로 사용자가 명시 입력한 경로만 허용합니다.
//! - **검증**([`CwdContext::resolve`]): 실행 직전에 경로 문법(비어 있음·NUL·길이·절대 경로)과
//!   실제 디렉터리 여부를 확인하고 symlink를 해석한 canonical 경로를 돌려줍니다. worktree 선택은
//!   공급된 worktree root 중 하나여야 합니다.

use std::ffi::OsStr;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// cwd 경로 길이 상한(byte, macOS `PATH_MAX`).
pub const MAX_CWD_BYTES: usize = 1024;
/// [`CwdContext`]가 받는 worktree root 수 상한.
pub const MAX_WORKTREE_ROOTS: usize = 256;

/// cwd 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwdError {
    /// 입력이 비었습니다.
    Empty,
    /// 절대 경로가 아닙니다.
    NotAbsolute {
        /// 입력 경로.
        path: PathBuf,
    },
    /// NUL byte가 있습니다.
    ContainsNul,
    /// [`MAX_CWD_BYTES`]를 넘습니다.
    TooLong {
        /// 길이(byte).
        len: usize,
    },
    /// 경로가 없습니다.
    NotFound {
        /// 경로.
        path: PathBuf,
    },
    /// 디렉터리가 아닙니다.
    NotDirectory {
        /// 경로.
        path: PathBuf,
    },
    /// 경로에 접근할 수 없습니다(권한 등).
    Inaccessible {
        /// 경로.
        path: PathBuf,
        /// I/O 오류 종류.
        kind: io::ErrorKind,
    },
    /// 공급된 worktree root가 아닙니다.
    UnknownWorktree {
        /// 선택한 경로.
        path: PathBuf,
    },
    /// worktree root가 [`MAX_WORKTREE_ROOTS`]개를 넘습니다.
    TooManyWorktrees {
        /// 상한.
        limit: usize,
    },
}

impl fmt::Display for CwdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CwdError::Empty => f.write_str("작업 디렉터리가 비었습니다"),
            CwdError::NotAbsolute { path } => {
                write!(
                    f,
                    "작업 디렉터리는 절대 경로여야 합니다: {}",
                    path.display()
                )
            }
            CwdError::ContainsNul => f.write_str("작업 디렉터리 경로에 NUL 문자가 있습니다"),
            CwdError::TooLong { len } => write!(
                f,
                "작업 디렉터리 경로가 너무 깁니다({len} byte, 상한 {MAX_CWD_BYTES})"
            ),
            CwdError::NotFound { path } => {
                write!(f, "작업 디렉터리가 없습니다: {}", path.display())
            }
            CwdError::NotDirectory { path } => {
                write!(f, "작업 디렉터리가 디렉터리가 아닙니다: {}", path.display())
            }
            CwdError::Inaccessible { path, kind } => write!(
                f,
                "작업 디렉터리에 접근할 수 없습니다({kind}): {}",
                path.display()
            ),
            CwdError::UnknownWorktree { path } => {
                write!(f, "프로젝트의 worktree root가 아닙니다: {}", path.display())
            }
            CwdError::TooManyWorktrees { limit } => {
                write!(f, "worktree root가 너무 많습니다(상한 {limit})")
            }
        }
    }
}

impl std::error::Error for CwdError {}

/// 경로 문법 검사: 비어 있지 않음, NUL 없음, 길이 상한, 절대 경로.
pub(crate) fn check_path_syntax(path: &Path) -> Result<(), CwdError> {
    let bytes = path.as_os_str().as_encoded_bytes();
    if bytes.is_empty() {
        return Err(CwdError::Empty);
    }
    if bytes.contains(&0) {
        return Err(CwdError::ContainsNul);
    }
    if bytes.len() > MAX_CWD_BYTES {
        return Err(CwdError::TooLong { len: bytes.len() });
    }
    if !path.is_absolute() {
        return Err(CwdError::NotAbsolute {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

/// 문법 검사 후 실제 디렉터리인지 확인하고 canonical 경로를 돌려줍니다.
fn canonical_dir(path: &Path) -> Result<PathBuf, CwdError> {
    check_path_syntax(path)?;
    let canonical = std::fs::canonicalize(path).map_err(|err| match err.kind() {
        io::ErrorKind::NotFound => CwdError::NotFound {
            path: path.to_path_buf(),
        },
        kind => CwdError::Inaccessible {
            path: path.to_path_buf(),
            kind,
        },
    })?;
    // symlink 해석 뒤 경로도 상한 안이어야 합니다.
    check_path_syntax(&canonical)?;
    let metadata = std::fs::metadata(&canonical).map_err(|err| CwdError::Inaccessible {
        path: path.to_path_buf(),
        kind: err.kind(),
    })?;
    if !metadata.is_dir() {
        return Err(CwdError::NotDirectory {
            path: path.to_path_buf(),
        });
    }
    Ok(canonical)
}

/// 사용자가 명시 입력한 cwd. [`ExplicitCwd::from_user_input`]으로만 만듭니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitCwd(PathBuf);

impl ExplicitCwd {
    /// 사용자 입력을 문법 검사합니다(존재·디렉터리 검사는 실행 시점에 합니다).
    pub fn from_user_input(input: impl AsRef<OsStr>) -> Result<ExplicitCwd, CwdError> {
        let path = PathBuf::from(input.as_ref());
        check_path_syntax(&path)?;
        Ok(ExplicitCwd(path))
    }

    /// 입력 경로.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

/// 세션 cwd 선택.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwdSelection {
    /// 프로젝트 루트(기본).
    ProjectRoot,
    /// 사용자가 고른 worktree root.
    WorktreeRoot(PathBuf),
    /// 사용자가 명시 입력한 임의 경로.
    Explicit(ExplicitCwd),
}

/// UI에 보여 줄 cwd preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwdPreset {
    /// 프로젝트 루트.
    ProjectRoot {
        /// canonical 경로.
        path: PathBuf,
    },
    /// worktree root.
    WorktreeRoot {
        /// canonical 경로.
        path: PathBuf,
    },
}

impl CwdPreset {
    /// preset 경로.
    pub fn path(&self) -> &Path {
        match self {
            CwdPreset::ProjectRoot { path } | CwdPreset::WorktreeRoot { path } => path,
        }
    }

    /// 이 preset을 고른 cwd 선택.
    pub fn selection(&self) -> CwdSelection {
        match self {
            CwdPreset::ProjectRoot { .. } => CwdSelection::ProjectRoot,
            CwdPreset::WorktreeRoot { path } => CwdSelection::WorktreeRoot(path.clone()),
        }
    }
}

/// 해석된 cwd가 무엇에 귀속되는지.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwdBinding {
    /// 프로젝트 루트(canonical).
    ProjectRoot(PathBuf),
    /// 특정 worktree root(canonical).
    Worktree(PathBuf),
    /// 명시 입력한 임의 경로(귀속 없음).
    Unbound,
}

/// 검증된 cwd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCwd {
    path: PathBuf,
    binding: CwdBinding,
}

impl ResolvedCwd {
    /// canonical 디렉터리 경로.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 귀속.
    pub fn binding(&self) -> &CwdBinding {
        &self.binding
    }
}

/// 프로젝트 창의 cwd 기준: 프로젝트 루트와 호출자가 공급한 worktree root 목록.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CwdContext {
    project_root: PathBuf,
    worktree_roots: Vec<PathBuf>,
}

impl CwdContext {
    /// 프로젝트 루트와 worktree root 목록(시스템 Git이 보고한 절대 경로)을 검증합니다. 경로는
    /// canonical로 바꾸고, worktree root는 정렬·중복 제거합니다.
    pub fn new(project_root: &Path, worktree_roots: &[PathBuf]) -> Result<CwdContext, CwdError> {
        if worktree_roots.len() > MAX_WORKTREE_ROOTS {
            return Err(CwdError::TooManyWorktrees {
                limit: MAX_WORKTREE_ROOTS,
            });
        }
        let project_root = canonical_dir(project_root)?;
        let mut roots = Vec::with_capacity(worktree_roots.len());
        for root in worktree_roots {
            roots.push(canonical_dir(root)?);
        }
        roots.sort();
        roots.dedup();
        Ok(CwdContext {
            project_root,
            worktree_roots: roots,
        })
    }

    /// 프로젝트 루트(canonical).
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// worktree root 목록(canonical, 정렬).
    pub fn worktree_roots(&self) -> &[PathBuf] {
        &self.worktree_roots
    }

    /// cwd preset: 프로젝트 루트가 먼저, 그다음 worktree root(프로젝트 루트와 같은 경로 제외).
    pub fn presets(&self) -> Vec<CwdPreset> {
        let mut presets = vec![CwdPreset::ProjectRoot {
            path: self.project_root.clone(),
        }];
        presets.extend(
            self.worktree_roots
                .iter()
                .filter(|root| **root != self.project_root)
                .map(|root| CwdPreset::WorktreeRoot { path: root.clone() }),
        );
        presets
    }

    /// 선택을 실행 직전 기준으로 검증합니다.
    pub fn resolve(&self, selection: &CwdSelection) -> Result<ResolvedCwd, CwdError> {
        match selection {
            CwdSelection::ProjectRoot => {
                let path = canonical_dir(&self.project_root)?;
                Ok(ResolvedCwd {
                    binding: CwdBinding::ProjectRoot(path.clone()),
                    path,
                })
            }
            CwdSelection::WorktreeRoot(root) => {
                let path = canonical_dir(root)?;
                if self.worktree_roots.binary_search(&path).is_err() {
                    return Err(CwdError::UnknownWorktree { path: root.clone() });
                }
                Ok(ResolvedCwd {
                    binding: CwdBinding::Worktree(path.clone()),
                    path,
                })
            }
            CwdSelection::Explicit(explicit) => Ok(ResolvedCwd {
                path: canonical_dir(explicit.path())?,
                binding: CwdBinding::Unbound,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn explicit_cwd_syntax_is_checked() {
        assert_eq!(
            ExplicitCwd::from_user_input("/tmp").unwrap().path(),
            Path::new("/tmp")
        );
        assert_eq!(ExplicitCwd::from_user_input(""), Err(CwdError::Empty));
        assert!(matches!(
            ExplicitCwd::from_user_input("relative/dir"),
            Err(CwdError::NotAbsolute { .. })
        ));
        assert_eq!(
            ExplicitCwd::from_user_input("/tmp/\0x"),
            Err(CwdError::ContainsNul)
        );
        let long = format!("/{}", "a".repeat(MAX_CWD_BYTES));
        assert_eq!(
            ExplicitCwd::from_user_input(&long),
            Err(CwdError::TooLong {
                len: MAX_CWD_BYTES + 1
            })
        );
    }

    /// project/ + worktrees/feature-a, worktrees/feature-b fixture.
    fn fixture() -> (TempDir, PathBuf, PathBuf, PathBuf) {
        let dir = TempDir::new();
        let project = dir.mkdir("project");
        let a = dir.mkdir("worktrees/feature-a");
        let b = dir.mkdir("worktrees/feature-b");
        (dir, project, a, b)
    }

    #[test]
    fn presets_list_project_root_then_worktree_roots() {
        let (_dir, project, a, b) = fixture();
        // 순서가 섞이고 중복·프로젝트 루트가 들어 있어도 preset은 정렬·중복 제거됩니다.
        let context = CwdContext::new(
            &project,
            &[b.clone(), a.clone(), b.clone(), project.clone()],
        )
        .unwrap();
        let presets = context.presets();
        assert_eq!(
            presets,
            vec![
                CwdPreset::ProjectRoot {
                    path: project.clone()
                },
                CwdPreset::WorktreeRoot { path: a.clone() },
                CwdPreset::WorktreeRoot { path: b.clone() },
            ]
        );
        // preset 선택은 그대로 해석됩니다.
        for preset in &presets {
            let resolved = context.resolve(&preset.selection()).unwrap();
            assert_eq!(resolved.path(), preset.path());
        }
        let resolved = context.resolve(&CwdSelection::ProjectRoot).unwrap();
        assert_eq!(resolved.binding(), &CwdBinding::ProjectRoot(project));
        let resolved = context
            .resolve(&CwdSelection::WorktreeRoot(a.clone()))
            .unwrap();
        assert_eq!(resolved.binding(), &CwdBinding::Worktree(a));
    }

    #[test]
    fn worktree_selection_must_be_a_supplied_root() {
        let (dir, project, a, _b) = fixture();
        let context = CwdContext::new(&project, std::slice::from_ref(&a)).unwrap();
        let other = dir.mkdir("not-a-worktree");
        assert_eq!(
            context.resolve(&CwdSelection::WorktreeRoot(other.clone())),
            Err(CwdError::UnknownWorktree { path: other })
        );
        // worktree 내부 하위 디렉터리도 worktree root가 아닙니다(명시 입력으로만 허용).
        let nested = dir.mkdir("worktrees/feature-a/src");
        assert!(matches!(
            context.resolve(&CwdSelection::WorktreeRoot(nested.clone())),
            Err(CwdError::UnknownWorktree { .. })
        ));
        let explicit = ExplicitCwd::from_user_input(&nested).unwrap();
        let resolved = context.resolve(&CwdSelection::Explicit(explicit)).unwrap();
        assert_eq!(resolved.path(), nested);
        assert_eq!(resolved.binding(), &CwdBinding::Unbound);
    }

    #[test]
    fn explicit_cwd_must_be_an_existing_directory() {
        let (dir, project, _a, _b) = fixture();
        let context = CwdContext::new(&project, &[]).unwrap();
        let missing = dir.path().join("missing");
        assert_eq!(
            context.resolve(&CwdSelection::Explicit(
                ExplicitCwd::from_user_input(&missing).unwrap()
            )),
            Err(CwdError::NotFound { path: missing })
        );
        let file = dir.path().join("file.txt");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(
            context.resolve(&CwdSelection::Explicit(
                ExplicitCwd::from_user_input(&file).unwrap()
            )),
            Err(CwdError::NotDirectory { path: file })
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_directories_resolve_to_canonical_path() {
        let (dir, project, a, _b) = fixture();
        let link = dir.path().join("link-to-a");
        std::os::unix::fs::symlink(&a, &link).unwrap();
        // symlink로 공급한 worktree root와 실제 경로 선택은 같은 root로 취급됩니다.
        let context = CwdContext::new(&project, std::slice::from_ref(&link)).unwrap();
        assert_eq!(context.worktree_roots(), std::slice::from_ref(&a));
        let resolved = context
            .resolve(&CwdSelection::WorktreeRoot(a.clone()))
            .unwrap();
        assert_eq!(resolved.path(), a);
        let resolved = context
            .resolve(&CwdSelection::Explicit(
                ExplicitCwd::from_user_input(&link).unwrap(),
            ))
            .unwrap();
        assert_eq!(resolved.path(), a);
    }

    #[test]
    fn context_rejects_invalid_roots() {
        let (dir, project, _a, _b) = fixture();
        assert!(matches!(
            CwdContext::new(Path::new("relative"), &[]),
            Err(CwdError::NotAbsolute { .. })
        ));
        assert!(matches!(
            CwdContext::new(&project, &[dir.path().join("gone")]),
            Err(CwdError::NotFound { .. })
        ));
        let too_many = vec![project.clone(); MAX_WORKTREE_ROOTS + 1];
        assert_eq!(
            CwdContext::new(&project, &too_many),
            Err(CwdError::TooManyWorktrees {
                limit: MAX_WORKTREE_ROOTS
            })
        );
    }

    #[test]
    fn removed_project_root_is_detected_at_resolve_time() {
        let (dir, _project, _a, _b) = fixture();
        let doomed = dir.mkdir("doomed");
        let context = CwdContext::new(&doomed, &[]).unwrap();
        std::fs::remove_dir(&doomed).unwrap();
        assert_eq!(
            context.resolve(&CwdSelection::ProjectRoot),
            Err(CwdError::NotFound { path: doomed })
        );
    }
}

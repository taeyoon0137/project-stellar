//! 터미널 header metadata (XVIII.4.5).
//!
//! UI header가 **일반 터미널**과 **특정 worktree 터미널**을 나눠 보여 줄 수 있게, 세션의 귀속(시작
//! cwd 선택, XVIII.1.3)과 현재 cwd를 묶은 data model입니다(기술 아키텍처 32 "프로젝트 루트 또는 특정
//! worktree에 귀속", "일반 세션과 외부 에이전트 세션 구분 가능"). 화면 표시는 앱 셸이 맡습니다.
//!
//! - **범위**([`HeaderScope`]): `Project`(프로젝트 루트에서 시작한 일반 터미널), `Worktree`(사용자가
//!   고른 worktree에 귀속), `Custom`(명시 입력 cwd, 귀속 없음).
//! - **현재 cwd**: 호출자가 관찰한 값(셸이 보고한 작업 디렉터리 등)을 받고, 없으면 시작 cwd로 봅니다.
//!   이 모듈은 셸 출력을 해석하지 않습니다. 관찰값은 절대 경로여야 합니다. 상대 경로·빈 값·NUL·길이
//!   초과는 앱 프로세스의 cwd로 해석하지 않고 [`CwdWarning::UnusableObservedCwd`]로 알립니다.
//! - **파일 시스템 I/O**: [`TerminalHeader::build`]는 symlink 해석(`canonicalize`)을 하므로 파일
//!   시스템을 읽습니다. UI는 매 frame 부르지 말고, cwd 관찰값이 바뀔 때 main thread 밖에서 만들어
//!   결과를 보관해 씁니다(기술 아키텍처 6 main-thread 예산).
//! - **잘못된 cwd 경고**([`CwdWarning`]): 귀속 root 밖으로 나감, 다른 worktree 안에 있음, 귀속
//!   worktree가 프로젝트 worktree 목록에서 사라짐. 사용자가 다른 worktree를 고친다고 착각하지 않게
//!   합니다. worktree 판정은 [`CwdContext`]의 root 목록(시스템 Git이 보고한 값)으로만 합니다.

use std::path::{Path, PathBuf};

use crate::cwd::{check_path_syntax, CwdBinding, CwdContext, CwdSelection, ResolvedCwd};
use crate::profile::ProfileCommand;

/// header가 보여 줄 세션 범위.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderScope {
    /// 프로젝트 루트에서 시작한 일반 터미널.
    Project {
        /// 프로젝트 루트.
        root: PathBuf,
    },
    /// 특정 worktree에 귀속된 터미널.
    Worktree {
        /// worktree root.
        root: PathBuf,
    },
    /// 명시 입력 cwd의 일반 터미널(귀속 없음).
    Custom,
}

impl HeaderScope {
    /// 특정 worktree 터미널인지.
    pub fn is_worktree(&self) -> bool {
        matches!(self, HeaderScope::Worktree { .. })
    }

    /// 고정 코드 문자열.
    pub fn code(&self) -> &'static str {
        match self {
            HeaderScope::Project { .. } => "project",
            HeaderScope::Worktree { .. } => "worktree",
            HeaderScope::Custom => "custom",
        }
    }

    fn binding_root(&self) -> Option<&Path> {
        match self {
            HeaderScope::Project { root } | HeaderScope::Worktree { root } => Some(root),
            HeaderScope::Custom => None,
        }
    }
}

/// 잘못된 cwd 경고.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwdWarning {
    /// 현재 cwd가 귀속 root 밖입니다(셸에서 다른 곳으로 이동).
    OutsideBinding {
        /// 귀속 root.
        root: PathBuf,
        /// 현재 cwd.
        cwd: PathBuf,
    },
    /// 현재 cwd가 귀속과 다른 worktree 안입니다.
    InsideOtherWorktree {
        /// 현재 cwd를 담은 worktree root.
        worktree: PathBuf,
        /// 현재 cwd.
        cwd: PathBuf,
    },
    /// 귀속 worktree가 프로젝트 worktree 목록에 더 이상 없습니다(삭제·이동).
    BindingMissing {
        /// 귀속 worktree root.
        root: PathBuf,
    },
    /// 관찰한 현재 cwd를 쓸 수 없습니다(절대 경로가 아님 등). 현재 cwd는 알 수 없음으로 둡니다.
    UnusableObservedCwd {
        /// 관찰값 그대로.
        observed: PathBuf,
    },
}

impl CwdWarning {
    /// 고정 코드 문자열.
    pub fn code(&self) -> &'static str {
        match self {
            CwdWarning::OutsideBinding { .. } => "outside-binding",
            CwdWarning::InsideOtherWorktree { .. } => "inside-other-worktree",
            CwdWarning::BindingMissing { .. } => "binding-missing",
            CwdWarning::UnusableObservedCwd { .. } => "unusable-observed-cwd",
        }
    }
}

/// header가 보여 줄 실행 명령(일반 셸 세션과 명령 세션 구분, 기술 아키텍처 32 "일반 세션과 외부
/// 에이전트 세션 구분 가능"). 명령 이름만 보여 주며 명령 종류를 해석하지 않습니다(XVIII.5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderCommand {
    /// 설정의 사용자 셸.
    Shell,
    /// argv 명령(실행 파일 이름).
    Program {
        /// 실행 파일 이름(경로의 마지막 요소).
        name: String,
    },
}

impl HeaderCommand {
    /// profile 명령에서 만듭니다.
    pub fn of(command: &ProfileCommand) -> HeaderCommand {
        match command {
            ProfileCommand::DefaultShell => HeaderCommand::Shell,
            ProfileCommand::Argv(argv) => HeaderCommand::Program {
                name: argv
                    .program()
                    .file_name()
                    .unwrap_or(argv.program().as_os_str())
                    .to_string_lossy()
                    .into_owned(),
            },
        }
    }
}

/// 터미널 header metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalHeader {
    /// 표시 이름(profile 이름).
    pub title: String,
    /// 실행 명령.
    pub command: HeaderCommand,
    /// 범위.
    pub scope: HeaderScope,
    /// 시작 시 검증한 cwd(시작 전이면 `None`).
    pub start_cwd: Option<PathBuf>,
    /// 현재 cwd(관찰값, 없으면 시작 cwd). 관찰값을 쓸 수 없으면 `None`.
    pub current_cwd: Option<PathBuf>,
    /// 현재 cwd를 담은 가장 깊은 worktree root(없으면 `None`).
    pub containing_worktree: Option<PathBuf>,
    /// 경고(없으면 비어 있음).
    pub warnings: Vec<CwdWarning>,
}

impl TerminalHeader {
    /// header를 만듭니다.
    ///
    /// - `selection`: 세션 cwd 선택(시작 전 범위 판단용).
    /// - `resolved`: 시작할 때 검증한 cwd(시작 전이면 `None`).
    /// - `context`: 프로젝트 창의 **현재** cwd 기준(worktree 목록이 바뀌면 다시 만든 값).
    /// - `observed_cwd`: 호출자가 관찰한 현재 cwd(없으면 시작 cwd). 절대 경로여야 합니다.
    ///
    /// symlink 해석 때문에 파일 시스템을 읽습니다(모듈 문서).
    pub fn build(
        title: &str,
        command: HeaderCommand,
        selection: &CwdSelection,
        resolved: Option<&ResolvedCwd>,
        context: &CwdContext,
        observed_cwd: Option<&Path>,
    ) -> TerminalHeader {
        let scope = scope_of(selection, resolved, context);
        let start_cwd = resolved.map(|cwd| cwd.path().to_path_buf());
        let mut unusable = None;
        let current_cwd = match observed_cwd {
            Some(observed) if check_path_syntax(observed).is_ok() => Some(normalize(observed)),
            Some(observed) => {
                unusable = Some(CwdWarning::UnusableObservedCwd {
                    observed: observed.to_path_buf(),
                });
                None
            }
            None => start_cwd.clone(),
        };
        let containing_worktree = current_cwd
            .as_deref()
            .and_then(|cwd| deepest_worktree(context, cwd))
            .map(Path::to_path_buf);
        let mut warnings = warnings_for(&scope, current_cwd.as_deref(), context);
        warnings.extend(unusable);
        TerminalHeader {
            title: title.to_owned(),
            command,
            scope,
            start_cwd,
            current_cwd,
            containing_worktree,
            warnings,
        }
    }

    /// 경고가 있는지.
    pub fn has_warning(&self) -> bool {
        !self.warnings.is_empty()
    }
}

fn scope_of(
    selection: &CwdSelection,
    resolved: Option<&ResolvedCwd>,
    context: &CwdContext,
) -> HeaderScope {
    match resolved.map(ResolvedCwd::binding) {
        Some(CwdBinding::ProjectRoot(root)) => HeaderScope::Project { root: root.clone() },
        Some(CwdBinding::Worktree(root)) => HeaderScope::Worktree { root: root.clone() },
        Some(CwdBinding::Unbound) => HeaderScope::Custom,
        None => match selection {
            CwdSelection::ProjectRoot => HeaderScope::Project {
                root: context.project_root().to_path_buf(),
            },
            CwdSelection::WorktreeRoot(root) => HeaderScope::Worktree {
                root: normalize(root),
            },
            CwdSelection::Explicit(_) => HeaderScope::Custom,
        },
    }
}

/// symlink를 해석할 수 있으면 canonical, 아니면(사라진 경로 등) 입력 그대로.
fn normalize(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// `cwd`를 담은 가장 깊은 worktree root.
fn deepest_worktree<'a>(context: &'a CwdContext, cwd: &Path) -> Option<&'a Path> {
    context
        .worktree_roots()
        .iter()
        .filter(|root| cwd.starts_with(root))
        .max_by_key(|root| root.components().count())
        .map(PathBuf::as_path)
}

fn warnings_for(scope: &HeaderScope, cwd: Option<&Path>, context: &CwdContext) -> Vec<CwdWarning> {
    let mut warnings = Vec::new();
    if let HeaderScope::Worktree { root } = scope {
        if !context.worktree_roots().iter().any(|known| known == root) {
            warnings.push(CwdWarning::BindingMissing { root: root.clone() });
        }
    }
    let (Some(binding), Some(cwd)) = (scope.binding_root(), cwd) else {
        return warnings;
    };
    // 귀속 root가 속한 worktree(프로젝트 루트는 자기 repository의 worktree, worktree는 자기 자신).
    let own_worktree = match scope {
        HeaderScope::Worktree { root } => Some(root.as_path()),
        _ => deepest_worktree(context, binding),
    };
    match deepest_worktree(context, cwd) {
        Some(worktree) if Some(worktree) != own_worktree => {
            warnings.push(CwdWarning::InsideOtherWorktree {
                worktree: worktree.to_path_buf(),
                cwd: cwd.to_path_buf(),
            });
        }
        _ if !cwd.starts_with(binding) => {
            warnings.push(CwdWarning::OutsideBinding {
                root: binding.to_path_buf(),
                cwd: cwd.to_path_buf(),
            });
        }
        _ => {}
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cwd::ExplicitCwd;
    use crate::test_support::TempDir;

    /// repo/(main worktree = 프로젝트 루트) 안에 linked worktree `repo/.worktrees/a`, 바깥에
    /// sibling worktree `wt-b`, 아무 worktree도 아닌 `elsewhere`.
    struct Fixture {
        _dir: TempDir,
        repo: PathBuf,
        nested: PathBuf,
        sibling: PathBuf,
        elsewhere: PathBuf,
        context: CwdContext,
    }

    fn fixture() -> Fixture {
        let dir = TempDir::new();
        let repo = dir.mkdir("repo");
        let nested = dir.mkdir("repo/.worktrees/a");
        let sibling = dir.mkdir("wt-b");
        let elsewhere = dir.mkdir("elsewhere");
        dir.mkdir("repo/src");
        dir.mkdir("repo/.worktrees/a/src");
        dir.mkdir("wt-b/src");
        let context =
            CwdContext::new(&repo, &[repo.clone(), nested.clone(), sibling.clone()]).unwrap();
        Fixture {
            _dir: dir,
            repo,
            nested,
            sibling,
            elsewhere,
            context,
        }
    }

    fn header(f: &Fixture, selection: CwdSelection, observed: Option<&Path>) -> TerminalHeader {
        let resolved = f.context.resolve(&selection).unwrap();
        TerminalHeader::build(
            "zsh",
            HeaderCommand::Shell,
            &selection,
            Some(&resolved),
            &f.context,
            observed,
        )
    }

    #[test]
    fn header_command_names_the_program_without_interpreting_it() {
        use crate::profile::ArgvCommand;
        assert_eq!(
            HeaderCommand::of(&ProfileCommand::DefaultShell),
            HeaderCommand::Shell
        );
        let argv = ArgvCommand::new("/opt/tools/bin/agent-cli", ["--flag"]).unwrap();
        assert_eq!(
            HeaderCommand::of(&ProfileCommand::Argv(argv)),
            HeaderCommand::Program {
                name: "agent-cli".to_owned()
            }
        );
    }

    #[test]
    fn relative_or_malformed_observed_cwd_is_not_resolved_against_app_cwd() {
        let f = fixture();
        for observed in ["src", "", "./repo", "a\0b"] {
            let observed = Path::new(observed);
            let bound = header(
                &f,
                CwdSelection::WorktreeRoot(f.sibling.clone()),
                Some(observed),
            );
            assert_eq!(bound.current_cwd, None, "{observed:?}");
            assert_eq!(bound.containing_worktree, None);
            assert_eq!(
                bound.warnings,
                [CwdWarning::UnusableObservedCwd {
                    observed: observed.to_path_buf()
                }]
            );
            assert_eq!(bound.warnings[0].code(), "unusable-observed-cwd");
        }
        // 절대 경로 관찰값은 그대로 판단합니다.
        let inside = f.sibling.join("src");
        let ok = header(
            &f,
            CwdSelection::WorktreeRoot(f.sibling.clone()),
            Some(&inside),
        );
        assert_eq!(ok.current_cwd.as_deref(), Some(inside.as_path()));
        assert!(!ok.has_warning());
    }

    #[test]
    fn general_and_worktree_terminals_have_distinct_scopes() {
        let f = fixture();
        let project = header(&f, CwdSelection::ProjectRoot, None);
        assert_eq!(
            project.scope,
            HeaderScope::Project {
                root: f.repo.clone()
            }
        );
        assert!(!project.scope.is_worktree());
        assert_eq!(project.start_cwd.as_deref(), Some(f.repo.as_path()));
        assert_eq!(
            project.containing_worktree.as_deref(),
            Some(f.repo.as_path())
        );
        assert!(!project.has_warning());

        let worktree = header(&f, CwdSelection::WorktreeRoot(f.sibling.clone()), None);
        assert_eq!(
            worktree.scope,
            HeaderScope::Worktree {
                root: f.sibling.clone()
            }
        );
        assert!(worktree.scope.is_worktree());
        assert!(!worktree.has_warning());

        let custom = header(
            &f,
            CwdSelection::Explicit(ExplicitCwd::from_user_input(&f.elsewhere).unwrap()),
            None,
        );
        assert_eq!(custom.scope, HeaderScope::Custom);
        assert_eq!(custom.containing_worktree, None);
        assert!(!custom.has_warning());

        let codes: Vec<&str> = [&project, &worktree, &custom]
            .iter()
            .map(|header| header.scope.code())
            .collect();
        assert_eq!(codes, ["project", "worktree", "custom"]);
        assert_eq!(project.title, "zsh");
    }

    #[test]
    fn wrong_cwd_sessions_are_warned() {
        let f = fixture();
        // worktree 터미널이 다른 worktree(프로젝트 main worktree)로 이동.
        let moved = header(
            &f,
            CwdSelection::WorktreeRoot(f.sibling.clone()),
            Some(&f.repo.join("src")),
        );
        assert_eq!(
            moved.warnings,
            [CwdWarning::InsideOtherWorktree {
                worktree: f.repo.clone(),
                cwd: f.repo.join("src"),
            }]
        );
        // worktree 터미널이 어떤 worktree도 아닌 곳으로 이동.
        let outside = header(
            &f,
            CwdSelection::WorktreeRoot(f.sibling.clone()),
            Some(&f.elsewhere),
        );
        assert_eq!(
            outside.warnings,
            [CwdWarning::OutsideBinding {
                root: f.sibling.clone(),
                cwd: f.elsewhere.clone(),
            }]
        );
        // 프로젝트 터미널이 프로젝트 폴더 안에 있는 linked worktree로 이동.
        let nested = header(&f, CwdSelection::ProjectRoot, Some(&f.nested.join("src")));
        assert_eq!(
            nested.warnings,
            [CwdWarning::InsideOtherWorktree {
                worktree: f.nested.clone(),
                cwd: f.nested.join("src"),
            }]
        );
        assert_eq!(
            nested.containing_worktree.as_deref(),
            Some(f.nested.as_path())
        );
        // 프로젝트 터미널이 프로젝트 밖으로 이동.
        let away = header(&f, CwdSelection::ProjectRoot, Some(&f.elsewhere));
        assert_eq!(away.warnings[0].code(), "outside-binding");
        // 같은 worktree 안의 하위 디렉터리 이동은 경고가 없습니다.
        let fine = header(
            &f,
            CwdSelection::WorktreeRoot(f.sibling.clone()),
            Some(&f.sibling.join("src")),
        );
        assert!(!fine.has_warning());
        let fine = header(&f, CwdSelection::ProjectRoot, Some(&f.repo.join("src")));
        assert!(!fine.has_warning());
        // 명시 cwd 터미널은 귀속이 없어 이동 경고가 없습니다.
        let custom = header(
            &f,
            CwdSelection::Explicit(ExplicitCwd::from_user_input(&f.elsewhere).unwrap()),
            Some(&f.sibling.join("src")),
        );
        assert!(!custom.has_warning());
        assert_eq!(
            custom.containing_worktree.as_deref(),
            Some(f.sibling.as_path())
        );
    }

    #[test]
    fn removed_worktree_binding_is_warned() {
        let f = fixture();
        let selection = CwdSelection::WorktreeRoot(f.sibling.clone());
        let resolved = f.context.resolve(&selection).unwrap();
        // worktree가 목록에서 빠진 뒤 다시 만든 context.
        let updated = CwdContext::new(&f.repo, &[f.repo.clone(), f.nested.clone()]).unwrap();
        let header = TerminalHeader::build(
            "zsh",
            HeaderCommand::Shell,
            &selection,
            Some(&resolved),
            &updated,
            None,
        );
        assert_eq!(
            header.warnings,
            [CwdWarning::BindingMissing {
                root: f.sibling.clone()
            },]
        );
        assert_eq!(header.warnings[0].code(), "binding-missing");
    }

    #[test]
    fn header_before_start_uses_selection() {
        let f = fixture();
        let selection = CwdSelection::WorktreeRoot(f.nested.clone());
        let header = TerminalHeader::build(
            "agent",
            HeaderCommand::Shell,
            &selection,
            None,
            &f.context,
            None,
        );
        assert_eq!(
            header.scope,
            HeaderScope::Worktree {
                root: f.nested.clone()
            }
        );
        assert_eq!(header.start_cwd, None);
        assert_eq!(header.current_cwd, None);
        assert!(!header.has_warning());
        let project = TerminalHeader::build(
            "agent",
            HeaderCommand::Shell,
            &CwdSelection::ProjectRoot,
            None,
            &f.context,
            None,
        );
        assert_eq!(project.scope, HeaderScope::Project { root: f.repo });
    }
}

//! Merge Composer source 정의와 해석 (XXIII.1.1).
//!
//! Merge Composer(아키텍처 27.4)는 임의의 Source A·B를 불러와 독립 Candidate C를 만듭니다. source는
//! [`SourceRef`] 일곱 종류이며 [`resolve_source`]가 시스템 Git으로 해석해 **읽는 시점의 고정 값**
//! ([`ResolvedSource`])으로 만듭니다.
//!
//! | source | 해석 | snapshot |
//! |---|---|---|
//! | [`SourceRef::Branch`] | `refs/heads/<name>^{commit}` | 그 commit |
//! | [`SourceRef::Commit`] | `<oid>^{commit}`가 같은 ID | 그 commit |
//! | [`SourceRef::Tag`] | `refs/tags/<name>^{commit}`(annotated tag는 commit까지 peel) | 그 commit |
//! | [`SourceRef::Worktree`] | `git worktree list`에 있는 worktree의 HEAD·index·작업 트리 | 그 worktree |
//! | [`SourceRef::Head`] | 현재 worktree `HEAD^{commit}` | 그 commit |
//! | [`SourceRef::Index`] | 현재 worktree index | index(stage 0) |
//! | [`SourceRef::WorkingTree`] | 현재 worktree의 디스크 저장 파일 | 작업 트리 |
//!
//! - branch·tag는 완전한 ref 이름(`refs/heads/`·`refs/tags/`)으로만 찾으므로 같은 이름의 branch와
//!   tag가 섞이지 않습니다. 이름은 Git ref 규칙의 안전한 부분집합만 허용합니다.
//! - HEAD·branch·tag는 해석 시점의 commit ID로 고정됩니다. 이후 ref가 움직여도 해석 결과는 바뀌지
//!   않습니다(나중의 stale 검사 기준).
//! - Index·WorkingTree는 내용이 계속 바뀌는 원천입니다. 이력 기준(`commit`)은 그 worktree의 HEAD이며,
//!   unborn이면 `None`입니다.
//! - linked worktree는 `git worktree list`가 돌려준 경로만 받습니다. 같은 저장소의 worktree이므로 현재
//!   프로젝트의 trust gate로 실행합니다.

use std::path::{Path, PathBuf};

use crate::advanced::worktree::list_worktrees;
use crate::object::ObjectId;
use crate::ops::GitOpError;
use crate::refname::is_valid_ref_body;
use crate::snapshot::tree::verify_rev;
use crate::snapshot::SnapshotSource;
use crate::trust::GatedGitRunner;

/// worktree에서 읽을 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorktreeState {
    /// 그 worktree의 HEAD commit.
    Head,
    /// 그 worktree의 index.
    Index,
    /// 그 worktree의 디스크 저장 파일.
    WorkingTree,
}

impl WorktreeState {
    fn as_str(self) -> &'static str {
        match self {
            WorktreeState::Head => "head",
            WorktreeState::Index => "index",
            WorktreeState::WorkingTree => "worktree",
        }
    }
}

/// Merge Composer source.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceRef {
    /// local branch(`refs/heads/<name>`).
    Branch(String),
    /// commit.
    Commit(ObjectId),
    /// tag(`refs/tags/<name>`, commit으로 peel).
    Tag(String),
    /// `git worktree list`에 있는 worktree의 상태.
    Worktree {
        /// worktree root 경로.
        path: PathBuf,
        /// 읽을 상태.
        state: WorktreeState,
    },
    /// 현재 worktree HEAD.
    Head,
    /// 현재 worktree index.
    Index,
    /// 현재 worktree 디스크 저장 파일.
    WorkingTree,
}

impl std::fmt::Display for SourceRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceRef::Branch(name) => write!(f, "branch:{name}"),
            SourceRef::Commit(oid) => write!(f, "commit:{oid}"),
            SourceRef::Tag(name) => write!(f, "tag:{name}"),
            SourceRef::Worktree { path, state } => {
                write!(f, "worktree:{}:{}", path.display(), state.as_str())
            }
            SourceRef::Head => f.write_str("head"),
            SourceRef::Index => f.write_str("index"),
            SourceRef::WorkingTree => f.write_str("worktree"),
        }
    }
}

/// 해석된 source(읽는 시점의 고정 값).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSource {
    /// 요청한 source.
    pub source: SourceRef,
    /// snapshot을 읽을 worktree root(현재 또는 linked worktree).
    pub worktree: PathBuf,
    /// snapshot 원천. commit 계열은 고정 [`SnapshotSource::Commit`]입니다.
    pub snapshot: SnapshotSource,
    /// 이력 기준 commit(merge-base 계산용). Index·WorkingTree는 그 worktree HEAD, unborn이면 `None`.
    pub commit: Option<ObjectId>,
}

/// source 해석 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// branch·tag 이름이 허용 형식이 아님(Git 실행 전 거부).
    InvalidName(String),
    /// branch·tag·commit이 없음.
    NotFound(SourceRef),
    /// tag가 commit을 가리키지 않음(tree·blob tag).
    NotACommit(SourceRef),
    /// HEAD가 아직 commit을 가리키지 않음.
    UnbornHead {
        /// worktree root.
        worktree: PathBuf,
    },
    /// `git worktree list`에 없는 경로.
    NotAWorktree(PathBuf),
    /// Git 실행·해석 실패.
    Op(GitOpError),
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::InvalidName(name) => write!(f, "잘못된 ref 이름: {name:?}"),
            SourceError::NotFound(source) => write!(f, "source를 찾을 수 없음: {source}"),
            SourceError::NotACommit(source) => write!(f, "commit이 아닌 source: {source}"),
            SourceError::UnbornHead { worktree } => {
                write!(f, "HEAD에 commit이 없음: {}", worktree.display())
            }
            SourceError::NotAWorktree(path) => {
                write!(f, "이 저장소의 worktree가 아님: {}", path.display())
            }
            SourceError::Op(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SourceError {}

impl From<GitOpError> for SourceError {
    fn from(e: GitOpError) -> SourceError {
        SourceError::Op(e)
    }
}

/// `path`와 같은 worktree를 `git worktree list`에서 찾습니다(실경로 비교).
fn find_worktree(
    git: &GatedGitRunner,
    current: &Path,
    path: &Path,
) -> Result<PathBuf, SourceError> {
    let not_found = || SourceError::NotAWorktree(path.to_path_buf());
    let wanted = std::fs::canonicalize(path).map_err(|_| not_found())?;
    for entry in list_worktrees(git, current)? {
        if entry.bare || entry.prunable.is_set() {
            continue;
        }
        if std::fs::canonicalize(&entry.path).is_ok_and(|p| p == wanted) {
            return Ok(entry.path);
        }
    }
    Err(not_found())
}

/// worktree의 HEAD commit(unborn이면 `None`).
fn head_commit(git: &GatedGitRunner, worktree: &Path) -> Result<Option<ObjectId>, SourceError> {
    Ok(verify_rev(git, worktree, "HEAD^{commit}")?)
}

fn resolve_state(
    git: &GatedGitRunner,
    source: &SourceRef,
    worktree: PathBuf,
    state: WorktreeState,
) -> Result<ResolvedSource, SourceError> {
    let commit = head_commit(git, &worktree)?;
    let snapshot = match state {
        WorktreeState::Head => match &commit {
            Some(oid) => SnapshotSource::Commit(oid.clone()),
            None => return Err(SourceError::UnbornHead { worktree }),
        },
        WorktreeState::Index => SnapshotSource::Index,
        WorktreeState::WorkingTree => SnapshotSource::WorkingTree,
    };
    Ok(ResolvedSource {
        source: source.clone(),
        worktree,
        snapshot,
        commit,
    })
}

fn resolve_ref(
    git: &GatedGitRunner,
    current: &Path,
    source: &SourceRef,
    full_ref: String,
) -> Result<ResolvedSource, SourceError> {
    let commit = match verify_rev(git, current, &format!("{full_ref}^{{commit}}"))? {
        Some(commit) => commit,
        None if verify_rev(git, current, &full_ref)?.is_some() => {
            return Err(SourceError::NotACommit(source.clone()))
        }
        None => return Err(SourceError::NotFound(source.clone())),
    };
    Ok(ResolvedSource {
        source: source.clone(),
        worktree: current.to_path_buf(),
        snapshot: SnapshotSource::Commit(commit.clone()),
        commit: Some(commit),
    })
}

/// source를 해석합니다. `current_worktree`는 Merge Composer를 연 worktree root입니다.
pub fn resolve_source(
    git: &GatedGitRunner,
    current_worktree: &Path,
    source: &SourceRef,
) -> Result<ResolvedSource, SourceError> {
    let current = current_worktree.to_path_buf();
    match source {
        SourceRef::Branch(name) | SourceRef::Tag(name) => {
            if !is_valid_ref_body(name) {
                return Err(SourceError::InvalidName(name.clone()));
            }
            let prefix = match source {
                SourceRef::Branch(_) => "refs/heads",
                _ => "refs/tags",
            };
            resolve_ref(git, current_worktree, source, format!("{prefix}/{name}"))
        }
        SourceRef::Commit(oid) => {
            let peeled = verify_rev(git, current_worktree, &format!("{oid}^{{commit}}"))?;
            if peeled.as_ref() != Some(oid) {
                return Err(SourceError::NotFound(source.clone()));
            }
            Ok(ResolvedSource {
                source: source.clone(),
                worktree: current,
                snapshot: SnapshotSource::Commit(oid.clone()),
                commit: Some(oid.clone()),
            })
        }
        SourceRef::Worktree { path, state } => {
            let worktree = find_worktree(git, current_worktree, path)?;
            resolve_state(git, source, worktree, *state)
        }
        SourceRef::Head => resolve_state(git, source, current, WorktreeState::Head),
        SourceRef::Index => resolve_state(git, source, current, WorktreeState::Index),
        SourceRef::WorkingTree => resolve_state(git, source, current, WorktreeState::WorkingTree),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{build_standard_scenario, GitFixture};

    #[test]
    fn display_names_every_source_kind() {
        let oid = ObjectId::parse(&"a".repeat(40)).unwrap();
        let cases = [
            (SourceRef::Branch("main".into()), "branch:main".to_string()),
            (SourceRef::Commit(oid.clone()), format!("commit:{oid}")),
            (SourceRef::Tag("v1".into()), "tag:v1".into()),
            (
                SourceRef::Worktree {
                    path: PathBuf::from("/wt"),
                    state: WorktreeState::Index,
                },
                "worktree:/wt:index".into(),
            ),
            (SourceRef::Head, "head".into()),
            (SourceRef::Index, "index".into()),
            (SourceRef::WorkingTree, "worktree".into()),
        ];
        for (source, text) in cases {
            assert_eq!(source.to_string(), text);
        }
    }

    #[test]
    fn invalid_names_are_rejected_before_spawn() {
        use crate::runner::GitRunner;
        use crate::trust::GitTrust;
        let runner = GitRunner::new(
            PathBuf::from("/nonexistent/git"),
            Vec::new(),
            std::time::Duration::from_secs(5),
        );
        let gate = GatedGitRunner::new(runner.clone(), GitTrust::Trusted);
        for name in ["", "-f", "a..b", "a b", "x.lock", "@{1}", "a:b"] {
            for source in [SourceRef::Branch(name.into()), SourceRef::Tag(name.into())] {
                assert_eq!(
                    resolve_source(&gate, &std::env::temp_dir(), &source),
                    Err(SourceError::InvalidName(name.into())),
                    "{source}"
                );
            }
        }
        assert_eq!(runner.spawn_count(), 0);
    }

    /// source resolution fixture: 일곱 종류 source를 표준 scenario에서 해석합니다.
    #[test]
    fn source_resolution_fixture() {
        let Some(fx) = GitFixture::with_system_git("composer-src").unwrap() else {
            return; // git 미설치 환경 건너뜀.
        };
        let s = build_standard_scenario(&fx).unwrap();
        // 같은 이름의 tag가 있어도 branch는 refs/heads로만 찾습니다.
        fx.git(["tag", "feature", s.initial.as_str()]).unwrap();
        fx.git(["tag", "lw", s.initial.as_str()]).unwrap();
        let tree = fx.rev_parse("main^{tree}").unwrap();
        fx.git(["tag", "treetag", tree.as_str()]).unwrap();
        let wt = fx.add_worktree("linked", "refs/heads/feature").unwrap();
        let gate = fx.gated();
        let work = fx.work();
        let resolve = |source: SourceRef| resolve_source(&gate, work, &source);
        let commit_source = |source: SourceRef, oid: &ObjectId| ResolvedSource {
            source,
            worktree: work.to_path_buf(),
            snapshot: SnapshotSource::Commit(oid.clone()),
            commit: Some(oid.clone()),
        };

        for (source, oid) in [
            (SourceRef::Branch("main".into()), &s.main),
            (SourceRef::Branch("feature".into()), &s.feature),
            (SourceRef::Tag("v1".into()), &s.main),
            (SourceRef::Tag("lw".into()), &s.initial),
            (SourceRef::Tag("feature".into()), &s.initial),
            (SourceRef::Commit(s.initial.clone()), &s.initial),
            (SourceRef::Head, &s.main),
        ] {
            assert_eq!(resolve(source.clone()).unwrap(), commit_source(source, oid));
        }
        for (source, snapshot) in [
            (SourceRef::Index, SnapshotSource::Index),
            (SourceRef::WorkingTree, SnapshotSource::WorkingTree),
        ] {
            let r = resolve(source).unwrap();
            assert_eq!((r.snapshot, r.commit), (snapshot, Some(s.main.clone())));
            assert_eq!(r.worktree, work);
        }

        // linked worktree의 세 상태.
        let canonical = |p: &Path| std::fs::canonicalize(p).unwrap();
        for (state, snapshot) in [
            (
                WorktreeState::Head,
                SnapshotSource::Commit(s.feature.clone()),
            ),
            (WorktreeState::Index, SnapshotSource::Index),
            (WorktreeState::WorkingTree, SnapshotSource::WorkingTree),
        ] {
            let r = resolve(SourceRef::Worktree {
                path: wt.clone(),
                state,
            })
            .unwrap();
            assert_eq!(canonical(&r.worktree), canonical(&wt));
            assert_eq!((r.snapshot, r.commit), (snapshot, Some(s.feature.clone())));
        }
        // 현재 worktree도 Worktree source로 지정할 수 있습니다.
        let current = resolve(SourceRef::Worktree {
            path: work.to_path_buf(),
            state: WorktreeState::Head,
        })
        .unwrap();
        assert_eq!(current.snapshot, SnapshotSource::Commit(s.main.clone()));

        // 해석 결과는 고정 값: 이후 main이 움직여도 바뀌지 않습니다.
        let before = resolve(SourceRef::Branch("main".into())).unwrap();
        fx.write("x.txt", b"x\n").unwrap();
        let moved = fx.commit_all("move main").unwrap();
        assert_eq!(before.commit, Some(s.main.clone()));
        assert_eq!(
            resolve(SourceRef::Branch("main".into())).unwrap().commit,
            Some(moved)
        );

        // 실패 사례.
        let missing = ObjectId::parse(&"0123456789abcdef".repeat(3)[..40]).unwrap();
        for (source, expected) in [
            (
                SourceRef::Branch("no-such".into()),
                SourceError::NotFound(SourceRef::Branch("no-such".into())),
            ),
            (
                SourceRef::Tag("no-such".into()),
                SourceError::NotFound(SourceRef::Tag("no-such".into())),
            ),
            (
                SourceRef::Tag("treetag".into()),
                SourceError::NotACommit(SourceRef::Tag("treetag".into())),
            ),
            (
                SourceRef::Commit(missing.clone()),
                SourceError::NotFound(SourceRef::Commit(missing)),
            ),
            (
                SourceRef::Commit(tree.clone()),
                SourceError::NotFound(SourceRef::Commit(tree)),
            ),
            (
                SourceRef::Worktree {
                    path: fx.root().join("home"),
                    state: WorktreeState::Head,
                },
                SourceError::NotAWorktree(fx.root().join("home")),
            ),
            (
                SourceRef::Worktree {
                    path: fx.root().join("nowhere"),
                    state: WorktreeState::Index,
                },
                SourceError::NotAWorktree(fx.root().join("nowhere")),
            ),
        ] {
            assert_eq!(resolve(source.clone()), Err(expected), "{source}");
        }
    }

    #[test]
    fn unborn_head_is_an_error_for_head_but_not_for_index() {
        let Some(fx) = GitFixture::with_system_git("composer-unborn").unwrap() else {
            return;
        };
        let gate = fx.gated();
        assert_eq!(
            resolve_source(&gate, fx.work(), &SourceRef::Head),
            Err(SourceError::UnbornHead {
                worktree: fx.work().to_path_buf()
            })
        );
        for source in [SourceRef::Index, SourceRef::WorkingTree] {
            let r = resolve_source(&gate, fx.work(), &source).unwrap();
            assert_eq!(r.commit, None);
        }
    }
}

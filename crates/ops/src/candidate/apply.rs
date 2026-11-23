//! 검증한 해결 결과를 WorkingTree에 쓰고 `git add`로 resolved 처리 (XIV.12.4~XIV.12.6, ADR-0081 결정 6).
//!
//! 순서: (1) 세션의 모든 단위를 검증해 쓰기 계획을 만듭니다(XIV.12.3). (2) 남은 저장 거래를 복구하고, 세션을 연 뒤
//! unmerged 목록이나 대상 파일이 바뀌었으면(stale) 아무것도 쓰지 않습니다. (3) 계획한 파일을 다중 파일 저장
//! journal(ADR-0078) 거래 **하나**로 씁니다. (4) 관련 경로를 `git add`합니다. (5) Git 상태(진행 중 작업, 남은 unmerged
//! 경로)만 다시 읽어 돌려줍니다.
//!
//! **원자성·복구(XIV.12.6, ADR-0081 개정 3)**: 모든 대상(chunk → 컴포넌트 원본 → manifest → `design/current`)은 한 journal
//! 거래입니다. 쓰기 도중 실패하면 journal이 앞 세대로 되돌리고, 프로세스가 죽으면 다음 [`recover_interrupted`] 또는
//! 다음 journal 저장이 commit marker 유무로 앞·새 세대 어느 한쪽으로 맞춥니다. 대상마다 기대 hash(CAS)를 걸어 세션 뒤
//! 외부에서 바뀐 파일은 쓰지 않습니다. 쓰기 순서(chunk → manifest → current)는 거래 안 반영 순서로 유지합니다.
//! `git add`가 실패하면 쓴 대상을 이전 bytes로 되돌리는 역 거래 하나를 commit합니다. index는 `git add`가 한 번의
//! 원자적 갱신이라 실패하면 바뀌지 않습니다. 역 거래마저 실패하면 오류에 경로를 남겨 사용자가 확인하게 합니다.
//! commit은 어떤 경우에도 하지 않습니다.
//!
//! **continue·commit 없음(XIV.12.5)**: merge·rebase·cherry-pick을 이어 가거나 commit하지 않습니다. 포트(`MergeGit`)에
//! 그런 동작이 없어 이 경로는 그 명령을 부를 수 없습니다. 사용자가 결과를 보고 `git merge --continue`·`git commit` 등을
//! 직접 실행합니다(기술 아키텍처 §27.2).

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use stellar_storage::hash::ContentHash;
use stellar_storage::journal::{self, JournalChange, JournalError, RecoveryReport};

use super::git_port::{GitOperation, GitPortError, MergeGit};
use super::session::{ConflictSession, PlanError, WritePlan};

/// 파일 bytes(없으면 `None`).
fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// 적용 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    /// 쓰거나 지운 파일(프로젝트 기준).
    pub written: Vec<PathBuf>,
    /// resolved 처리한 경로.
    pub added: Vec<PathBuf>,
    /// add 뒤 진행 중인 Git 작업(읽지 못하면 `None`). 이어 가기는 사용자가 합니다.
    pub operation: Option<GitOperation>,
    /// add 뒤 남은 unmerged 경로(raw conflict 등, 읽지 못하면 `None`).
    pub remaining: Option<Vec<PathBuf>>,
}

/// 적용 실패.
#[derive(Debug)]
pub enum ApplyError {
    /// 쓰기 조건 미충족 또는 결과 직렬화 실패. 아무것도 쓰지 않았습니다.
    Plan(PlanError),
    /// 세션을 연 뒤 Git 상태·파일이 바뀜. 아무것도 쓰지 않았습니다.
    Stale(String),
    /// Git 읽기 실패. 아무것도 쓰지 않았습니다.
    Git(GitPortError),
    /// 남은 저장 거래 복구 실패. 아무것도 쓰지 않았습니다.
    Recover(JournalError),
    /// 파일 쓰기(journal 거래) 실패. journal이 모든 대상을 앞 세대로 되돌렸거나, 다음 복구가 되돌립니다.
    Write {
        /// 실패 대상 경로(거래 전체 실패면 첫 대상).
        path: PathBuf,
        /// 원인.
        error: JournalError,
        /// 되돌리지 못한 경로(journal 거래 실패에서는 비어 있음).
        unrestored: Vec<PathBuf>,
    },
    /// `git add` 실패. 쓴 파일은 역 거래로 되돌렸습니다(`unrestored`에 남은 경로 제외).
    Add {
        /// 원인.
        error: GitPortError,
        /// 되돌리지 못한 경로.
        unrestored: Vec<PathBuf>,
    },
}

impl ApplyError {
    /// 사용자 복구 안내(XIV.12.6). conflict는 unresolved 그대로입니다.
    pub fn guidance(&self) -> String {
        match self {
            ApplyError::Write { unrestored, .. } | ApplyError::Add { unrestored, .. }
                if !unrestored.is_empty() =>
            {
                let paths: Vec<String> =
                    unrestored.iter().map(|p| p.display().to_string()).collect();
                format!(
                    "다음 파일을 이전 상태로 되돌리지 못했습니다: {}. 다시 실행하면 남은 저장 거래를 먼저 복구합니다. 내용을 \
                     확인하고 `git checkout --merge -- <경로>`로 conflict 상태를 다시 만들 수 있습니다. conflict는 \
                     unresolved이며 commit하지 않았습니다",
                    paths.join(", ")
                )
            }
            ApplyError::Write { .. } | ApplyError::Add { .. } => {
                "작업 트리를 이전 상태로 되돌렸습니다. conflict는 unresolved이며 commit하지 않았습니다. 원인을 해결한 뒤 \
                 다시 실행하세요"
                    .into()
            }
            _ => "아무것도 쓰지 않았습니다. conflict는 unresolved입니다".into(),
        }
    }
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyError::Plan(e) => write!(f, "{e}"),
            ApplyError::Stale(e) => write!(f, "세션을 연 뒤 상태가 바뀌었습니다: {e}"),
            ApplyError::Git(e) => write!(f, "{e}"),
            ApplyError::Recover(e) => write!(f, "남은 저장 거래 복구 실패: {e}"),
            ApplyError::Write { path, error, .. } => {
                write!(f, "{} 쓰기 실패: {error}", path.display())
            }
            ApplyError::Add { error, .. } => write!(f, "git add 실패: {error}"),
        }
    }
}

impl std::error::Error for ApplyError {}

/// 중단된 저장 거래(이전 적용·저장의 프로세스 중단)를 앞·새 세대 어느 한쪽으로 맞춥니다(ADR-0078). 적용 전에
/// 부르면 세션이 복구된 작업 트리를 관찰합니다. [`apply_session`]도 쓰기 전에 부릅니다.
pub fn recover_interrupted(root: &Path) -> Result<RecoveryReport, JournalError> {
    journal::recover(root)
}

fn check_fresh(git: &dyn MergeGit, session: &ConflictSession) -> Result<(), ApplyError> {
    let root = session.root();
    if git.unmerged(root).map_err(ApplyError::Git)? != session.entries() {
        return Err(ApplyError::Stale("unmerged 목록".into()));
    }
    for unit in session.units() {
        let current = read(&root.join(unit.path()))
            .map_err(|error| ApplyError::Write {
                path: unit.path().to_path_buf(),
                error: JournalError::Io {
                    path: unit.path().to_path_buf(),
                    source: error,
                },
                unrestored: Vec::new(),
            })?
            .map(|bytes| ContentHash::compute(&bytes));
        if current != unit.observed() {
            return Err(ApplyError::Stale(unit.path().display().to_string()));
        }
    }
    Ok(())
}

/// 쓰기 대상 하나(이전·새 bytes).
struct Target {
    path: PathBuf,
    key: String,
    previous: Option<Vec<u8>>,
    next: Option<Vec<u8>>,
}

/// 프로젝트 기준 경로를 journal 경로(`/` 구분)로 바꿉니다.
fn journal_key(path: &Path) -> Result<String, ApplyError> {
    let parts: Option<Vec<&str>> = path
        .components()
        .map(|c| match c {
            std::path::Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect();
    parts
        .map(|parts| parts.join("/"))
        .ok_or_else(|| ApplyError::Write {
            path: path.to_path_buf(),
            error: JournalError::InvalidPath {
                path: path.display().to_string(),
                reason: "프로젝트 기준 UTF-8 상대 경로가 아닙니다",
            },
            unrestored: Vec::new(),
        })
}

fn targets(root: &Path, plan: &WritePlan) -> Result<Vec<Target>, ApplyError> {
    plan.writes
        .iter()
        .map(|write| {
            let previous = read(&root.join(&write.path)).map_err(|error| ApplyError::Write {
                path: write.path.clone(),
                error: JournalError::Io {
                    path: write.path.clone(),
                    source: error,
                },
                unrestored: Vec::new(),
            })?;
            Ok(Target {
                key: journal_key(&write.path)?,
                path: write.path.clone(),
                previous,
                next: write.bytes.clone(),
            })
        })
        .collect()
}

fn hash(bytes: &Option<Vec<u8>>) -> Option<ContentHash> {
    bytes.as_deref().map(ContentHash::compute)
}

/// `from` 상태에서 `to` 상태로 가는 journal 변경(계획 순서 유지). 이미 같은 대상과 없는 파일 삭제는 뺍니다.
fn changes(
    targets: &[Target],
    from: impl Fn(&Target) -> &Option<Vec<u8>>,
    to: impl Fn(&Target) -> &Option<Vec<u8>>,
) -> Vec<JournalChange> {
    targets
        .iter()
        .filter(|t| from(t) != to(t))
        .map(|t| match (from(t), to(t)) {
            (current, Some(bytes)) => JournalChange::write(&t.key, bytes.clone(), hash(current)),
            (Some(current), None) => JournalChange::delete(&t.key, ContentHash::compute(current)),
            (None, None) => unreachable!("같은 상태는 위에서 걸렀습니다"),
        })
        .collect()
}

/// 대상 하나의 현재·새 bytes 상한: 이 계획이 실제로 다루는 가장 큰 bytes.
fn limit(targets: &[Target]) -> u64 {
    targets
        .iter()
        .flat_map(|t| [&t.previous, &t.next])
        .filter_map(|b| b.as_ref().map(|b| b.len() as u64))
        .max()
        .unwrap_or(0)
        .max(1)
}

/// 지금 디스크에서 `want` 상태와 다른 대상 경로.
fn differing(
    root: &Path,
    targets: &[Target],
    want: impl Fn(&Target) -> &Option<Vec<u8>>,
) -> Vec<PathBuf> {
    targets
        .iter()
        .filter(|t| read(&root.join(&t.path)).ok().as_ref() != Some(want(t)))
        .map(|t| t.path.clone())
        .collect()
}

/// 계획을 journal 거래 하나로 씁니다.
fn write_plan(root: &Path, targets: &[Target]) -> Result<(), ApplyError> {
    let forward = changes(targets, |t| &t.previous, |t| &t.next);
    if forward.is_empty() {
        return Ok(());
    }
    match journal::commit(root, &forward, limit(targets)) {
        Ok(_) => Ok(()),
        Err(JournalError::Stale { paths }) => Err(ApplyError::Stale(paths.join(", "))),
        Err(error) => {
            // marker 기록 뒤 오류면 새 세대로 확정됐을 수 있으므로 실제 파일을 다시 봅니다(ADR-0078).
            if differing(root, targets, |t| &t.next).is_empty() {
                return Ok(());
            }
            let path = match &error {
                JournalError::Io { path, .. } => path
                    .strip_prefix(root)
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|_| path.clone()),
                _ => targets[0].path.clone(),
            };
            Err(ApplyError::Write {
                path,
                error,
                unrestored: differing(root, targets, |t| &t.previous),
            })
        }
    }
}

/// 쓴 대상을 이전 bytes로 되돌리는 역 거래를 commit합니다. 되돌리지 못한 경로를 돌려줍니다.
fn restore(root: &Path, targets: &[Target]) -> Vec<PathBuf> {
    let inverse = changes(targets, |t| &t.next, |t| &t.previous);
    if !inverse.is_empty() {
        let _ = journal::commit(root, &inverse, limit(targets));
    }
    differing(root, targets, |t| &t.previous)
}

/// 세션의 해결 결과를 작업 트리에 쓰고 resolved 처리합니다(XIV.12.4).
pub fn apply_session(
    git: &dyn MergeGit,
    session: &ConflictSession,
) -> Result<ApplyReport, ApplyError> {
    let plan = session.plan().map_err(ApplyError::Plan)?;
    let root = session.root();
    recover_interrupted(root).map_err(ApplyError::Recover)?;
    check_fresh(git, session)?;
    let targets = targets(root, &plan)?;
    write_plan(root, &targets)?;
    if let Err(error) = git.add(root, &plan.add) {
        let unrestored = restore(root, &targets);
        return Err(ApplyError::Add { error, unrestored });
    }
    // add 뒤에는 상태만 읽습니다(XIV.12.5). 읽기 실패는 결과(쓰기·add 성공)를 바꾸지 않습니다.
    let operation = git.operation(root).ok();
    let remaining = git
        .unmerged(root)
        .ok()
        .map(|entries| entries.into_iter().map(|e| e.path).collect());
    Ok(ApplyReport {
        written: plan.writes.iter().map(|w| w.path.clone()).collect(),
        added: plan.add,
        operation,
        remaining,
    })
}

#[cfg(test)]
pub(crate) mod tests;

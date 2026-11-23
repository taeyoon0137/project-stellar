//! 여러 의미 파일을 한 세대로 저장하는 journal (V.10.5, ADR-0078).
//!
//! 입력은 codec과 무관한 변경 집합입니다. 각 변경은 프로젝트 루트 기준 상대 경로, 새 bytes 또는 삭제,
//! 기대 원본 hash를 가집니다([`JournalChange`]). 이름 바꾸기는 옛 경로 삭제와 새 경로 생성 두 항목입니다
//! ([`JournalChange::rename`]).
//!
//! # 순서([`commit`])
//!
//! 1. 경로 규칙 검사, `.stellar/journal/` 잠금, 남은 거래 복구.
//! 2. 모든 대상의 기대 hash 비교(CAS). 하나라도 다르면 대상과 거래 디렉터리를 만들지 않고
//!    [`JournalError::Stale`]입니다.
//! 3. `.stellar/journal/tx-<id>/manifest`(대상 목록·기대 hash·새 hash)를 temp/fsync/rename으로 씁니다.
//! 4. 원본 사본 `orig/<번호>`와 새 내용 `new/<번호>`를 쓰고 fsync합니다. 그다음 staging 디렉터리를 sync합니다.
//! 5. 대상마다 디렉터리 identity와 기대 hash를 다시 확인하고 rename(교체)·hard link(새 파일)·삭제합니다.
//! 6. 대상 부모 디렉터리를 sync하고 commit marker `committed`를 씁니다. marker가 새 세대 확정 지점입니다.
//! 7. 거래 디렉터리를 `trash-<id>`로 옮기고 지웁니다.
//!
//! # 복구([`recover`], [`commit`] 시작)
//!
//! - manifest가 없으면 대상을 건드리기 전이므로 거래 디렉터리만 버립니다.
//! - marker가 없으면 앞 세대로 되돌립니다. marker가 있으면 남은 반영을 끝내 새 세대로 맞춥니다.
//! - 판정은 대상별 현재 hash로만 합니다. 현재 내용이 앞·새 세대 어느 쪽도 아니면(외부 수정, symlink, 정규 파일
//!   아님) 그 대상은 건드리지 않고, 거래를 `quarantine-<id>`로 옮겨 원본 사본과 새 내용을 보존합니다.
//! - 복구는 몇 번 중단되어도 다시 실행할 수 있습니다(같은 hash 판정이 같은 결과를 냄).
//!
//! # 남는 한계
//!
//! - 마지막 확인과 rename 사이에 비협조적 외부 writer가 대상을 바꾸는 짧은 창은 남습니다(`source_file`과 같음).
//! - 저장 중 만든 빈 부모 디렉터리는 되돌릴 때 지우지 않습니다.
//! - 대상 비교 key는 ASCII 대소문자만 접습니다. Unicode 정규화가 다른 같은 이름은 거부하지 못합니다.

mod manifest;
mod path;
#[cfg(test)]
mod tests;

use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::fs::{File, OpenOptions, Permissions};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::atomic_file::{
    sync_parent_dir, write_file_atomic, write_file_atomic_observed_with_permissions, NoopObserver,
};
use crate::fs_guard::{
    ensure_real_dirs, existing_real_dirs, read_regular_file, same_dir, DirGuard, FileIdentity,
    Link, Refusal,
};
use crate::hash::ContentHash;

use manifest::{Content, Entry, Manifest, Op};

/// journal 위치(프로젝트 루트 기준).
const JOURNAL_DIRS: [&str; 2] = [".stellar", "journal"];
/// journal Git 제외 규칙.
const IGNORE_RULE: &[u8] = b"*\n";
/// manifest 파일 이름.
const MANIFEST_FILE: &str = "manifest";
/// commit marker 파일 이름.
const MARKER_FILE: &str = "committed";
/// commit marker 내용.
const MARKER: &[u8] = b"stellar-journal-commit 1\n";
/// 원본 사본 디렉터리.
const ORIG_DIR: &str = "orig";
/// 새 내용 디렉터리.
const NEW_DIR: &str = "new";
/// 진행 중 거래 디렉터리 접두사.
const TX_PREFIX: &str = "tx-";
/// 끝난 거래(지울 디렉터리) 접두사.
const TRASH_PREFIX: &str = "trash-";
/// 판정하지 못한 거래 보존 접두사.
const QUARANTINE_PREFIX: &str = "quarantine-";
/// manifest 읽기 상한.
const MANIFEST_LIMIT: u64 = 16 << 20;
/// 거래 ID 충돌 시 다시 시도하는 횟수.
const TXID_ATTEMPTS: usize = 16;

/// 변경 하나의 내용.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    /// 이 bytes로 만들거나 교체합니다.
    Write(Vec<u8>),
    /// 지웁니다.
    Delete,
}

/// 저장할 대상 하나(프로젝트 루트 기준 상대 경로, `/` 구분).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalChange {
    path: String,
    kind: ChangeKind,
    expected: Option<ContentHash>,
}

impl JournalChange {
    /// `path`를 `bytes`로 씁니다. `expected`가 `None`이면 파일이 없어야 하고, `Some`이면 현재 bytes hash가
    /// 같아야 합니다.
    pub fn write(
        path: impl Into<String>,
        bytes: Vec<u8>,
        expected: Option<ContentHash>,
    ) -> JournalChange {
        JournalChange {
            path: path.into(),
            kind: ChangeKind::Write(bytes),
            expected,
        }
    }

    /// 현재 bytes hash가 `expected`인 `path`를 지웁니다.
    pub fn delete(path: impl Into<String>, expected: ContentHash) -> JournalChange {
        JournalChange {
            path: path.into(),
            kind: ChangeKind::Delete,
            expected: Some(expected),
        }
    }

    /// `from`(현재 hash `expected_from`)을 지우고 없는 `to`에 `bytes`를 쓰는 이름 바꾸기입니다. 두 항목은
    /// 같은 거래로 함께 반영되거나 함께 되돌려집니다. 대소문자만 다른 이름 바꾸기는 같은 대상으로 보아
    /// [`commit`]이 거부합니다.
    pub fn rename(
        from: impl Into<String>,
        to: impl Into<String>,
        bytes: Vec<u8>,
        expected_from: ContentHash,
    ) -> [JournalChange; 2] {
        [
            JournalChange::delete(from, expected_from),
            JournalChange::write(to, bytes, None),
        ]
    }

    /// 상대 경로.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// 변경 내용.
    pub fn kind(&self) -> &ChangeKind {
        &self.kind
    }

    /// 기대 원본 hash(`None`은 파일 없음).
    pub fn expected(&self) -> Option<ContentHash> {
        self.expected
    }
}

/// journal 오류. 어떤 오류에서도 대상은 앞 세대 그대로이거나(되돌림 완료), 다음 [`recover`]가 앞 세대로
/// 맞출 수 있는 상태입니다. 단 marker 기록 뒤 오류는 새 세대로 확정될 수 있으므로 호출자는 오류 뒤 실제
/// 파일을 다시 읽어야 합니다.
#[derive(Debug)]
pub enum JournalError {
    /// 경로 규칙 위반(절대 경로, `..`, `.git`, `.stellar/` 등).
    InvalidPath {
        /// 입력 경로.
        path: String,
        /// 거부 이유.
        reason: &'static str,
    },
    /// 같은 대상이 두 번 있습니다(ASCII 대소문자 무시).
    DuplicatePath {
        /// 입력 경로.
        path: String,
    },
    /// 변경 수가 상한을 넘습니다.
    TooManyChanges {
        /// 상한.
        limit: usize,
    },
    /// 새 bytes가 크기 상한을 넘습니다.
    TooLarge {
        /// 입력 경로.
        path: String,
        /// 상한(bytes).
        limit: u64,
    },
    /// 대상의 현재 내용이 기대 hash와 다릅니다(외부 변경). 아무 대상도 바꾸지 않았거나 이미 되돌렸습니다.
    Stale {
        /// 다른 대상 경로(입력 순서).
        paths: Vec<String>,
    },
    /// 다른 저장이 journal을 잡고 있습니다.
    Busy,
    /// 대상 디렉터리가 journal과 다른 장치에 있어 원자 rename을 할 수 없습니다.
    CrossDevice {
        /// 입력 경로.
        path: String,
    },
    /// 파일 시스템 오류 또는 저장소 guard 거부(symlink, 정규 파일 아님 등).
    Io {
        /// 관련 경로.
        path: PathBuf,
        /// 원인.
        source: io::Error,
    },
}

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JournalError::InvalidPath { path, reason } => {
                write!(f, "저장 대상 경로 {path:?}를 거부합니다: {reason}")
            }
            JournalError::DuplicatePath { path } => write!(f, "같은 대상이 두 번 있습니다: {path}"),
            JournalError::TooManyChanges { limit } => {
                write!(f, "변경 수가 상한 {limit}개를 넘습니다")
            }
            JournalError::TooLarge { path, limit } => {
                write!(f, "{path}의 새 내용이 상한 {limit} bytes를 넘습니다")
            }
            JournalError::Stale { paths } => {
                write!(f, "원본이 외부에서 변경되었습니다: {}", paths.join(", "))
            }
            JournalError::Busy => f.write_str("다른 저장이 journal을 사용 중입니다"),
            JournalError::CrossDevice { path } => {
                write!(f, "{path}가 journal과 다른 장치에 있습니다")
            }
            JournalError::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl std::error::Error for JournalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            JournalError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// 남은 거래 하나의 복구 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// manifest 전에 끝난 거래라 대상을 건드리지 않고 버렸습니다.
    Discarded,
    /// marker가 없어 앞 세대로 되돌렸습니다.
    RolledBack,
    /// marker가 있어 새 세대로 맞췄습니다.
    RolledForward,
    /// 일부 대상이 앞·새 세대 어느 쪽도 아니라 건드리지 않았습니다. 나머지 대상은 정해진 방향으로 맞췄고
    /// 거래는 `.stellar/journal/quarantine-<id>/`에 원본 사본·새 내용과 함께 남습니다.
    Quarantined {
        /// 건드리지 않은 대상.
        conflicts: Vec<String>,
    },
    /// manifest를 읽을 수 없어 대상을 건드리지 않고 `quarantine-<id>/`로 옮겼습니다.
    Unreadable,
}

/// 복구한 거래.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredTransaction {
    /// 거래 ID.
    pub id: String,
    /// 결과.
    pub outcome: RecoveryOutcome,
}

/// [`recover`] 결과(거래 ID 순서).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    /// 처리한 거래.
    pub transactions: Vec<RecoveredTransaction>,
}

/// [`commit`] 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitReport {
    /// 시작할 때 복구한 남은 거래.
    pub recovered: RecoveryReport,
    /// 쓴 대상 수(내용이 같아 건너뛴 대상 제외).
    pub written: usize,
    /// 지운 대상 수.
    pub deleted: usize,
    /// 새 세대는 확정됐지만 거래 디렉터리 정리에 실패했습니다. 다음 [`recover`]가 지웁니다.
    pub cleanup_pending: bool,
}

/// 변경 집합을 한 세대로 저장합니다. `limit`은 대상 하나의 현재·새 bytes 상한입니다.
///
/// 남은 거래를 먼저 복구한 뒤 모든 기대 hash를 확인합니다. 성공하면 모든 대상이 새 세대이고, 실패하면
/// 모든 대상이 앞 세대로 되돌려졌거나 다음 [`recover`]가 되돌릴 수 있습니다. symlink 디렉터리·대상,
/// 프로젝트 밖 경로는 거부합니다. 같은 프로젝트의 다른 저장과는 `.stellar/journal/` 잠금으로 배타적이며,
/// 이미 잡혀 있으면 기다리지 않고 [`JournalError::Busy`]입니다.
pub fn commit(
    root: &Path,
    changes: &[JournalChange],
    limit: u64,
) -> Result<CommitReport, JournalError> {
    commit_observed(root, changes, limit, &mut Uninterrupted).map_err(Failure::into_error)
}

/// 남은 거래를 모두 앞 세대 또는 새 세대로 맞춥니다. 프로젝트를 열 때 부릅니다.
pub fn recover(root: &Path) -> Result<RecoveryReport, JournalError> {
    recover_observed(root, &mut Uninterrupted).map_err(Failure::into_error)
}

// ---------------------------------------------------------------------------------------------
// 중단 주입(테스트 전용 관찰자, 프로덕션은 `Uninterrupted`)
// ---------------------------------------------------------------------------------------------

/// 거래·복구 단계.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// 거래 디렉터리 생성.
    CreateTx,
    /// manifest 기록.
    WriteManifest,
    /// 원본 사본 기록(항목 번호).
    WriteBackup(usize),
    /// 새 내용 기록(항목 번호).
    WriteNew(usize),
    /// staging 디렉터리 sync.
    SyncStaging,
    /// 대상 반영(항목 번호).
    Publish(usize),
    /// 대상 부모 디렉터리 sync.
    SyncTargets,
    /// commit marker 기록.
    WriteMarker,
    /// 거래 디렉터리 정리.
    Cleanup,
    /// 복구: 대상 판정·반영(항목 번호).
    Resolve(usize),
    /// 복구: 대상 부모 디렉터리 sync.
    SyncResolved,
    /// 복구: 거래 디렉터리 정리 또는 보존.
    Finish,
}

/// 주입한 중단. 프로덕션 관찰자는 만들지 않습니다(fault matrix 전용).
#[derive(Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum Interrupt {
    /// 그 단계가 이 오류로 실패합니다(우아한 실패 — 되돌림이 실행됨).
    Fail(io::Error),
    /// 프로세스가 그 단계 직전에 죽습니다(아무 정리도 실행하지 않음).
    Crash,
}

/// 각 단계 직전에 호출되는 관찰자.
pub(crate) trait Observer {
    /// `step` 직전 호출됩니다.
    fn before(&mut self, step: Step) -> Result<(), Interrupt>;
}

/// 중단을 주입하지 않는 관찰자.
pub(crate) struct Uninterrupted;

impl Observer for Uninterrupted {
    fn before(&mut self, _step: Step) -> Result<(), Interrupt> {
        Ok(())
    }
}

/// 내부 실패.
#[derive(Debug)]
pub(crate) enum Failure {
    /// 주입한 프로세스 중단.
    Crash,
    /// 오류.
    Error(JournalError),
}

impl Failure {
    fn into_error(self) -> JournalError {
        match self {
            Failure::Error(error) => error,
            // `Uninterrupted`는 중단을 만들지 않지만 오류로 보고해 panic을 피합니다.
            Failure::Crash => JournalError::Io {
                path: PathBuf::new(),
                source: io::Error::other("저장이 중단되었습니다"),
            },
        }
    }
}

impl From<JournalError> for Failure {
    fn from(error: JournalError) -> Failure {
        Failure::Error(error)
    }
}

fn step(observer: &mut dyn Observer, at: Step, path: &Path) -> Result<(), Failure> {
    match observer.before(at) {
        Ok(()) => Ok(()),
        Err(Interrupt::Crash) => Err(Failure::Crash),
        Err(Interrupt::Fail(source)) => Err(Failure::Error(io_error(path, source))),
    }
}

fn io_error(path: &Path, source: io::Error) -> JournalError {
    JournalError::Io {
        path: path.to_path_buf(),
        source,
    }
}

// ---------------------------------------------------------------------------------------------
// journal 디렉터리와 잠금
// ---------------------------------------------------------------------------------------------

/// 잠근 journal 디렉터리. drop하면 잠금이 풀립니다.
struct Journal {
    path: PathBuf,
    identity: FileIdentity,
    _lock: File,
}

fn open_journal(root: &Path) -> Result<Journal, JournalError> {
    let existed = existing_real_dirs(root, &JOURNAL_DIRS)
        .map_err(|e| io_error(&join(root, &JOURNAL_DIRS), e))?
        .is_some();
    let dirs = ensure_real_dirs(root, &JOURNAL_DIRS).map_err(|(path, e)| io_error(&path, e))?;
    let (path, identity) = dirs.last().cloned().expect("구성 요소 두 개");
    if !existed {
        // 새로 만든 `.stellar/`·`journal/` 항목을 내구화해야 거래 디렉터리가 재시작 뒤에도 보입니다.
        sync_ancestors(root, &JOURNAL_DIRS).map_err(|(path, e)| io_error(&path, e))?;
    }
    let lock = File::open(&path).map_err(|e| io_error(&path, e))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Err(JournalError::Busy),
        Err(std::fs::TryLockError::Error(e)) => return Err(io_error(&path, e)),
    }
    let opened = lock.metadata().map_err(|e| io_error(&path, e))?;
    if FileIdentity::of(&opened) != identity || !same_dir(&path, identity) {
        return Err(io_error(&path, Refusal::Changed.into_io()));
    }
    ensure_ignore_rule(&path, dirs)?;
    Ok(Journal {
        path,
        identity,
        _lock: lock,
    })
}

/// `.stellar/journal/.gitignore`를 `*`로 보장합니다. 다른 내용이면 덮어쓰지 않고 거부합니다.
fn ensure_ignore_rule(dir: &Path, dirs: Vec<(PathBuf, FileIdentity)>) -> Result<(), JournalError> {
    let path = dir.join(".gitignore");
    match read_regular_file(&path, IGNORE_RULE.len() as u64, Link::NoFollow) {
        Ok(bytes) if bytes == IGNORE_RULE => Ok(()),
        Ok(_) => Err(io_error(
            &path,
            io::Error::new(ErrorKind::InvalidData, "journal Git 제외 설정이 다릅니다"),
        )),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let mut noop = NoopObserver;
            let mut guard = DirGuard::new(&mut noop, dirs);
            write_file_atomic_observed_with_permissions(&path, IGNORE_RULE, &mut guard, None, false)
                .map_err(|e| io_error(&path, e))
        }
        Err(e) => Err(io_error(&path, e)),
    }
}

/// 새 거래 ID(시각·process·일련번호, 소문자 16진수).
fn new_txid() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:016x}-{:08x}-{n:08x}", std::process::id())
}

/// 새 거래 디렉터리와 staging 하위 디렉터리를 만듭니다(이미 있는 이름은 쓰지 않음).
fn create_tx_dir(journal: &Journal) -> Result<(String, PathBuf), JournalError> {
    if !same_dir(&journal.path, journal.identity) {
        return Err(io_error(&journal.path, Refusal::Changed.into_io()));
    }
    for _ in 0..TXID_ATTEMPTS {
        let id = new_txid();
        let tx = journal.path.join(format!("{TX_PREFIX}{id}"));
        match std::fs::create_dir(&tx) {
            Ok(()) => {
                for sub in [ORIG_DIR, NEW_DIR] {
                    let dir = tx.join(sub);
                    std::fs::create_dir(&dir).map_err(|e| io_error(&dir, e))?;
                }
                return Ok((id, tx));
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io_error(&tx, e)),
        }
    }
    Err(io_error(
        &journal.path,
        io::Error::new(ErrorKind::AlreadyExists, "사용 가능한 거래 ID가 없습니다"),
    ))
}

/// 끝난 거래를 `trash-<id>`로 옮긴 뒤 지웁니다. 지우기 실패는 다음 복구가 다시 지웁니다.
fn discard(journal: &Journal, tx: &Path, id: &str) -> io::Result<()> {
    let trash = journal.path.join(format!("{TRASH_PREFIX}{id}"));
    std::fs::rename(tx, &trash)?;
    let _ = std::fs::remove_dir_all(&trash);
    Ok(())
}

/// 판정하지 못한 거래를 `quarantine-<id>`로 옮겨 보존합니다.
fn quarantine(journal: &Journal, tx: &Path, id: &str) -> io::Result<()> {
    std::fs::rename(tx, journal.path.join(format!("{QUARANTINE_PREFIX}{id}")))
}

// ---------------------------------------------------------------------------------------------
// 대상 관찰
// ---------------------------------------------------------------------------------------------

fn join(root: &Path, components: &[&str]) -> PathBuf {
    let mut path = root.to_path_buf();
    path.extend(components);
    path
}

/// `root`와 `dirs`의 각 앞부분 디렉터리(마지막 디렉터리 자신은 제외)를 sync합니다. 새로 만든 디렉터리
/// 항목은 그 부모를 sync해야 내구화됩니다. 깊은 쪽부터 sync합니다.
fn sync_ancestors(root: &Path, dirs: &[&str]) -> Result<(), (PathBuf, io::Error)> {
    for depth in (0..dirs.len()).rev() {
        let dir = join(root, &dirs[..depth]);
        sync_parent_dir(&dir).map_err(|e| (dir, e))?;
    }
    Ok(())
}

/// 대상의 현재 파일.
struct Current {
    bytes: Vec<u8>,
    permissions: Permissions,
}

/// 대상 관찰 결과. 부모 디렉터리가 없으면 `dirs`가 `None`이고 대상도 없습니다.
struct Observed {
    dirs: Option<Vec<(PathBuf, FileIdentity)>>,
    current: Option<Current>,
}

/// 대상을 만들지 않고 읽습니다. symlink 부모·대상, 정규 파일 아님, 상한 초과는 guard 거부입니다.
fn observe(root: &Path, components: &[&str], limit: u64) -> io::Result<Observed> {
    let (parents, _) = components.split_at(components.len() - 1);
    let Some(dirs) = existing_real_dirs(root, parents)? else {
        return Ok(Observed {
            dirs: None,
            current: None,
        });
    };
    let target = join(root, components);
    let metadata = match std::fs::symlink_metadata(&target) {
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Ok(Observed {
                dirs: Some(dirs),
                current: None,
            })
        }
        other => other?,
    };
    if metadata.file_type().is_symlink() {
        return Err(Refusal::SymlinkFile.into_io());
    }
    if !metadata.is_file() {
        return Err(Refusal::NotRegularFile.into_io());
    }
    // root는 canonical 경로이므로 이미 해석한 경로로 열어 그 사이 생긴 symlink도 거부합니다.
    let bytes = read_regular_file(&target, limit, Link::Resolved)?;
    Ok(Observed {
        dirs: Some(dirs),
        current: Some(Current {
            bytes,
            permissions: metadata.permissions(),
        }),
    })
}

/// 복구 판정용 대상 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// 없음.
    Absent,
    /// 정규 파일.
    Present(Content),
    /// 판정 불가(symlink, 정규 파일 아님, 상한 초과, 부모가 디렉터리 아님).
    Unknown,
}

fn state(root: &Path, components: &[&str], limit: u64) -> io::Result<State> {
    match observe(root, components, limit) {
        Ok(Observed { current: None, .. }) => Ok(State::Absent),
        Ok(Observed {
            current: Some(current),
            ..
        }) => Ok(State::Present(Content::of(&current.bytes))),
        Err(e) if Refusal::of(&e).is_some() => Ok(State::Unknown),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------------------------
// commit
// ---------------------------------------------------------------------------------------------

/// 검사를 마친 변경.
struct Planned<'a> {
    change: &'a JournalChange,
    components: Vec<&'a str>,
}

fn plan(changes: &[JournalChange], limit: u64) -> Result<Vec<Planned<'_>>, JournalError> {
    if changes.len() > manifest::ENTRY_LIMIT {
        return Err(JournalError::TooManyChanges {
            limit: manifest::ENTRY_LIMIT,
        });
    }
    let mut seen = HashSet::new();
    let mut planned = Vec::with_capacity(changes.len());
    for change in changes {
        let components =
            path::components(&change.path).map_err(|reason| JournalError::InvalidPath {
                path: change.path.clone(),
                reason,
            })?;
        if !seen.insert(path::identity_key(&change.path)) {
            return Err(JournalError::DuplicatePath {
                path: change.path.clone(),
            });
        }
        if let ChangeKind::Write(bytes) = &change.kind {
            if bytes.len() as u64 > limit {
                return Err(JournalError::TooLarge {
                    path: change.path.clone(),
                    limit,
                });
            }
        }
        planned.push(Planned { change, components });
    }
    Ok(planned)
}

/// 반영할 대상 하나.
struct Staged<'a> {
    components: Vec<&'a str>,
    target: PathBuf,
    parent: PathBuf,
    dirs: Vec<(PathBuf, FileIdentity)>,
    current: Option<Current>,
    bytes: Option<&'a [u8]>,
    /// 부모 디렉터리 일부를 이 저장이 만들었습니다. marker 전에 조상 디렉터리도 sync합니다.
    created_dirs: bool,
}

pub(crate) fn commit_observed(
    root: &Path,
    changes: &[JournalChange],
    limit: u64,
    observer: &mut dyn Observer,
) -> Result<CommitReport, Failure> {
    let planned = plan(changes, limit)?;
    let root = root.canonicalize().map_err(|e| io_error(root, e))?;
    let journal = open_journal(&root)?;
    let recovered = recover_locked(&root, &journal, &mut Uninterrupted)?;

    // 1. 기대 hash(CAS). 어느 하나라도 다르면 아무것도 쓰지 않습니다.
    let mut stale = Vec::new();
    let mut observed = Vec::with_capacity(planned.len());
    for item in &planned {
        let target = join(&root, &item.components);
        let seen = observe(&root, &item.components, limit).map_err(|e| io_error(&target, e))?;
        let found = seen
            .current
            .as_ref()
            .map(|c| ContentHash::compute(&c.bytes));
        if found != item.change.expected {
            stale.push(item.change.path.clone());
        }
        observed.push((target, seen));
    }
    if !stale.is_empty() {
        return Err(JournalError::Stale { paths: stale }.into());
    }

    // 2. 같은 내용 쓰기는 빼고 부모 디렉터리를 준비합니다.
    let mut entries = Vec::new();
    let mut staged = Vec::new();
    for (item, (target, seen)) in planned.iter().zip(observed) {
        let current_content = seen.current.as_ref().map(|c| Content::of(&c.bytes));
        let (op, new, bytes) = match &item.change.kind {
            ChangeKind::Write(bytes) => {
                let new = Content::of(bytes);
                if current_content == Some(new) {
                    continue;
                }
                (Op::Write, Some(new), Some(bytes.as_slice()))
            }
            ChangeKind::Delete => (Op::Delete, None, None),
        };
        let (parents, _) = item.components.split_at(item.components.len() - 1);
        let parent = join(&root, parents);
        let created_dirs = seen.dirs.is_none();
        let dirs = match (op, seen.dirs) {
            (_, Some(dirs)) => dirs,
            _ => ensure_real_dirs(&root, parents).map_err(|(path, e)| io_error(&path, e))?,
        };
        let (_, parent_identity) =
            FileIdentity::lstat(&parent).map_err(|e| io_error(&parent, e))?;
        if parent_identity.dev() != journal.identity.dev() {
            return Err(JournalError::CrossDevice {
                path: item.change.path.clone(),
            }
            .into());
        }
        entries.push(Entry {
            op,
            path: item.change.path.clone(),
            expected: current_content,
            new,
        });
        staged.push(Staged {
            components: item.components.clone(),
            target,
            parent,
            dirs,
            current: seen.current,
            bytes,
            created_dirs,
        });
    }
    let written = entries.iter().filter(|e| e.op == Op::Write).count();
    let deleted = entries.len() - written;
    let mut report = CommitReport {
        recovered,
        written,
        deleted,
        cleanup_pending: false,
    };
    if entries.is_empty() {
        return Ok(report);
    }

    // 3. 거래 디렉터리.
    step(observer, Step::CreateTx, &journal.path)?;
    let (txid, tx) = create_tx_dir(&journal)?;
    let manifest = Manifest {
        txid: txid.clone(),
        entries,
    };

    // 4~6. staging → 반영 → marker. 실패하면 반영한 대상을 되돌립니다.
    let mut published = 0;
    let result = stage_and_publish(
        &root,
        &tx,
        &manifest,
        &staged,
        limit,
        observer,
        &mut published,
    );
    match result {
        Ok(()) => {}
        Err(Failure::Crash) => return Err(Failure::Crash),
        Err(Failure::Error(error)) => {
            if !marker_present(&tx).unwrap_or(true) {
                abort(&root, &journal, &tx, &manifest, published);
            }
            return Err(Failure::Error(error));
        }
    }

    // 7. 정리. 새 세대는 이미 확정이므로 실패해도 성공입니다.
    match observer.before(Step::Cleanup) {
        Ok(()) => report.cleanup_pending = discard(&journal, &tx, &txid).is_err(),
        Err(Interrupt::Crash) => return Err(Failure::Crash),
        Err(Interrupt::Fail(_)) => report.cleanup_pending = true,
    }
    Ok(report)
}

/// 새 파일 하나를 쓰고 fsync합니다(새로 만든 거래 디렉터리 안, `create_new`).
fn write_new_file(path: &Path, bytes: &[u8], permissions: Option<Permissions>) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    if let Some(permissions) = permissions {
        file.set_permissions(permissions)?;
    }
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()
}

fn stage_and_publish(
    root: &Path,
    tx: &Path,
    manifest: &Manifest,
    staged: &[Staged<'_>],
    limit: u64,
    observer: &mut dyn Observer,
    published: &mut usize,
) -> Result<(), Failure> {
    let manifest_path = tx.join(MANIFEST_FILE);
    step(observer, Step::WriteManifest, &manifest_path)?;
    write_file_atomic(&manifest_path, &manifest::encode(manifest))
        .map_err(|e| io_error(&manifest_path, e))?;

    for (i, item) in staged.iter().enumerate() {
        if let Some(current) = &item.current {
            let path = tx.join(ORIG_DIR).join(i.to_string());
            step(observer, Step::WriteBackup(i), &path)?;
            write_new_file(&path, &current.bytes, Some(current.permissions.clone()))
                .map_err(|e| io_error(&path, e))?;
        }
    }
    for (i, item) in staged.iter().enumerate() {
        if let Some(bytes) = item.bytes {
            let path = tx.join(NEW_DIR).join(i.to_string());
            step(observer, Step::WriteNew(i), &path)?;
            let permissions = item.current.as_ref().map(|c| c.permissions.clone());
            write_new_file(&path, bytes, permissions).map_err(|e| io_error(&path, e))?;
        }
    }
    step(observer, Step::SyncStaging, tx)?;
    // 거래 디렉터리 항목 자체는 journal 디렉터리를 sync해야 내구화됩니다. 대상을 건드리기 전에 필요합니다.
    let journal_dir = tx.parent().unwrap_or(root).to_path_buf();
    for dir in [
        tx.join(ORIG_DIR),
        tx.join(NEW_DIR),
        tx.to_path_buf(),
        journal_dir,
    ] {
        sync_parent_dir(&dir).map_err(|e| io_error(&dir, e))?;
    }

    for (i, (item, entry)) in staged.iter().zip(&manifest.entries).enumerate() {
        step(observer, Step::Publish(i), &item.target)?;
        recheck(root, item, entry, limit)?;
        // 반영을 시도하는 항목부터 되돌림 범위에 넣습니다(부분 반영도 hash로 판정).
        *published = i + 1;
        publish(tx, i, item, entry).map_err(|e| io_error(&item.target, e))?;
    }

    step(observer, Step::SyncTargets, root)?;
    let parents: BTreeSet<&PathBuf> = staged.iter().map(|s| &s.parent).collect();
    for parent in parents {
        sync_parent_dir(parent).map_err(|e| io_error(parent, e))?;
    }
    // 이 저장이 만든 부모 디렉터리 항목도 marker 전에 내구화합니다.
    for item in staged.iter().filter(|s| s.created_dirs) {
        let (dirs, _) = item.components.split_at(item.components.len() - 1);
        sync_ancestors(root, dirs).map_err(|(path, e)| io_error(&path, e))?;
    }

    let marker = tx.join(MARKER_FILE);
    step(observer, Step::WriteMarker, &marker)?;
    if let Err(e) = write_file_atomic(&marker, MARKER) {
        // rename 뒤 디렉터리 sync 실패면 marker가 남습니다. 지워야 되돌림이 marker와 어긋나지 않습니다.
        if marker_present(tx).unwrap_or(true) {
            std::fs::remove_file(&marker).map_err(|e| io_error(&marker, e))?;
            sync_parent_dir(tx).map_err(|e| io_error(tx, e))?;
        }
        return Err(io_error(&marker, e).into());
    }
    Ok(())
}

/// 반영 직전 디렉터리 identity와 기대 hash를 다시 확인합니다.
fn recheck(root: &Path, item: &Staged<'_>, entry: &Entry, limit: u64) -> Result<(), JournalError> {
    for (dir, identity) in &item.dirs {
        if !same_dir(dir, *identity) {
            return Err(io_error(dir, Refusal::Changed.into_io()));
        }
    }
    let seen = observe(root, &item.components, limit).map_err(|e| io_error(&item.target, e))?;
    let found = seen.current.as_ref().map(|c| Content::of(&c.bytes));
    if found != entry.expected {
        return Err(JournalError::Stale {
            paths: vec![entry.path.clone()],
        });
    }
    Ok(())
}

fn publish(tx: &Path, i: usize, item: &Staged<'_>, entry: &Entry) -> io::Result<()> {
    match entry.op {
        Op::Write => {
            let source = tx.join(NEW_DIR).join(i.to_string());
            if entry.expected.is_some() {
                std::fs::rename(&source, &item.target)
            } else {
                // 새 파일은 마지막 순간 생긴 항목도 교체하지 않도록 hard link로 게시합니다.
                std::fs::hard_link(&source, &item.target)?;
                std::fs::remove_file(&source)
            }
        }
        Op::Delete => std::fs::remove_file(&item.target),
    }
}

/// 우아한 실패: 반영한 항목만 앞 세대로 되돌리고 거래를 버립니다. 되돌리지 못하면 거래를 남겨 다음 복구에
/// 맡깁니다.
fn abort(root: &Path, journal: &Journal, tx: &Path, manifest: &Manifest, published: usize) {
    match resolve(
        root,
        tx,
        manifest,
        Direction::Back,
        published,
        &mut Uninterrupted,
    ) {
        Ok(conflicts) if conflicts.is_empty() => {
            let _ = discard(journal, tx, &manifest.txid);
        }
        Ok(_) => {
            let _ = quarantine(journal, tx, &manifest.txid);
        }
        Err(_) => {}
    }
}

fn marker_present(tx: &Path) -> io::Result<bool> {
    match read_regular_file(&tx.join(MARKER_FILE), MARKER.len() as u64, Link::NoFollow) {
        Ok(bytes) => Ok(bytes == MARKER),
        Err(e) if e.kind() == ErrorKind::NotFound || Refusal::of(&e).is_some() => Ok(false),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------------------------
// 복구
// ---------------------------------------------------------------------------------------------

/// 복구 방향.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    /// 앞 세대(marker 없음).
    Back,
    /// 새 세대(marker 있음).
    Forward,
}

/// 대상 하나에 할 일.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// 이미 목표 세대입니다.
    Keep,
    /// 대상을 지웁니다.
    Remove,
    /// journal 파일(`orig` 또는 `new`)을 대상으로 옮깁니다. `replace`면 rename, 아니면 hard link.
    Install { from_orig: bool, replace: bool },
    /// 앞·새 세대 어느 쪽도 아니라 건드리지 않습니다.
    Conflict,
}

fn decide(entry: &Entry, direction: Direction, state: State) -> Action {
    let is = |content: Option<Content>| match (state, content) {
        (State::Absent, None) => true,
        (State::Present(found), Some(content)) => found == content,
        _ => false,
    };
    let (from, to, from_orig) = match direction {
        Direction::Back => (entry.new, entry.expected, true),
        Direction::Forward => (entry.expected, entry.new, false),
    };
    if is(to) {
        Action::Keep
    } else if !is(from) {
        Action::Conflict
    } else if to.is_none() {
        Action::Remove
    } else {
        Action::Install {
            from_orig,
            replace: from.is_some(),
        }
    }
}

/// 거래의 앞 `count`개 항목을 `direction` 세대로 맞추고 건드리지 않은 대상을 돌려줍니다.
fn resolve(
    root: &Path,
    tx: &Path,
    manifest: &Manifest,
    direction: Direction,
    count: usize,
    observer: &mut dyn Observer,
) -> Result<Vec<String>, Failure> {
    let mut conflicts = Vec::new();
    let mut parents = BTreeSet::new();
    for (i, entry) in manifest.entries.iter().enumerate().take(count) {
        let components =
            path::components(&entry.path).map_err(|reason| JournalError::InvalidPath {
                path: entry.path.clone(),
                reason,
            })?;
        let target = join(root, &components);
        step(observer, Step::Resolve(i), &target)?;
        let limit = [entry.expected, entry.new]
            .iter()
            .flatten()
            .map(|c| c.len)
            .max()
            .unwrap_or(0);
        let found = state(root, &components, limit).map_err(|e| io_error(&target, e))?;
        let (parent_components, _) = components.split_at(components.len() - 1);
        match decide(entry, direction, found) {
            Action::Keep => {}
            Action::Conflict => conflicts.push(entry.path.clone()),
            Action::Remove => {
                std::fs::remove_file(&target).map_err(|e| io_error(&target, e))?;
                parents.insert(join(root, parent_components));
            }
            Action::Install { from_orig, replace } => {
                let (dir, content) = if from_orig {
                    (ORIG_DIR, entry.expected)
                } else {
                    (NEW_DIR, entry.new)
                };
                let source = tx.join(dir).join(i.to_string());
                let Some(content) = content else {
                    conflicts.push(entry.path.clone());
                    continue;
                };
                // journal 파일이 없거나 손상됐으면 대상을 건드리지 않습니다.
                let valid = match read_regular_file(&source, content.len, Link::NoFollow) {
                    Ok(bytes) => Content::of(&bytes) == content,
                    Err(e) if e.kind() == ErrorKind::NotFound || Refusal::of(&e).is_some() => false,
                    Err(e) => return Err(io_error(&source, e).into()),
                };
                if !valid {
                    conflicts.push(entry.path.clone());
                    continue;
                }
                let existed = existing_real_dirs(root, parent_components)
                    .map_err(|e| io_error(&target, e))?
                    .is_some();
                ensure_real_dirs(root, parent_components)
                    .map_err(|(path, e)| io_error(&path, e))?;
                if !existed {
                    sync_ancestors(root, parent_components)
                        .map_err(|(path, e)| io_error(&path, e))?;
                }
                if replace {
                    std::fs::rename(&source, &target).map_err(|e| io_error(&target, e))?;
                } else {
                    match std::fs::hard_link(&source, &target) {
                        Ok(()) => {}
                        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                            conflicts.push(entry.path.clone());
                            continue;
                        }
                        Err(e) => return Err(io_error(&target, e).into()),
                    }
                }
                parents.insert(join(root, parent_components));
            }
        }
    }
    step(observer, Step::SyncResolved, root)?;
    for parent in parents {
        sync_parent_dir(&parent).map_err(|e| io_error(&parent, e))?;
    }
    Ok(conflicts)
}

pub(crate) fn recover_observed(
    root: &Path,
    observer: &mut dyn Observer,
) -> Result<RecoveryReport, Failure> {
    let root = root.canonicalize().map_err(|e| io_error(root, e))?;
    let journal = open_journal(&root)?;
    recover_locked(&root, &journal, observer)
}

/// 잠근 journal의 남은 거래를 이름 순서로 복구합니다.
fn recover_locked(
    root: &Path,
    journal: &Journal,
    observer: &mut dyn Observer,
) -> Result<RecoveryReport, Failure> {
    let mut names = Vec::new();
    let entries = std::fs::read_dir(&journal.path).map_err(|e| io_error(&journal.path, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| io_error(&journal.path, e))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        // symlink·파일은 따라가거나 지우지 않습니다.
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            names.push(name);
        }
    }
    names.sort();

    let mut report = RecoveryReport::default();
    for name in names {
        let path = journal.path.join(&name);
        if name.starts_with(TRASH_PREFIX) {
            let _ = std::fs::remove_dir_all(&path);
            continue;
        }
        let Some(id) = name.strip_prefix(TX_PREFIX) else {
            continue;
        };
        let outcome = recover_tx(root, journal, &path, id, observer)?;
        report.transactions.push(RecoveredTransaction {
            id: id.to_string(),
            outcome,
        });
    }
    Ok(report)
}

fn recover_tx(
    root: &Path,
    journal: &Journal,
    tx: &Path,
    id: &str,
    observer: &mut dyn Observer,
) -> Result<RecoveryOutcome, Failure> {
    let manifest_path = tx.join(MANIFEST_FILE);
    let manifest = match read_regular_file(&manifest_path, MANIFEST_LIMIT, Link::NoFollow) {
        Err(e) if e.kind() == ErrorKind::NotFound => {
            // manifest는 대상 반영보다 먼저 쓰므로 대상은 앞 세대 그대로입니다.
            step(observer, Step::Finish, tx)?;
            discard(journal, tx, id).map_err(|e| io_error(tx, e))?;
            return Ok(RecoveryOutcome::Discarded);
        }
        Err(e) if Refusal::of(&e).is_none() => return Err(io_error(&manifest_path, e).into()),
        Err(_) => None,
        Ok(bytes) => manifest::decode(&bytes)
            .ok()
            .filter(|m| m.txid == id && manifest::valid_txid(id)),
    };
    let Some(manifest) = manifest else {
        step(observer, Step::Finish, tx)?;
        quarantine(journal, tx, id).map_err(|e| io_error(tx, e))?;
        return Ok(RecoveryOutcome::Unreadable);
    };
    let direction = if marker_present(tx).map_err(|e| io_error(tx, e))? {
        Direction::Forward
    } else {
        Direction::Back
    };
    let count = manifest.entries.len();
    let conflicts = resolve(root, tx, &manifest, direction, count, observer)?;
    step(observer, Step::Finish, tx)?;
    if conflicts.is_empty() {
        discard(journal, tx, id).map_err(|e| io_error(tx, e))?;
        Ok(match direction {
            Direction::Back => RecoveryOutcome::RolledBack,
            Direction::Forward => RecoveryOutcome::RolledForward,
        })
    } else {
        quarantine(journal, tx, id).map_err(|e| io_error(tx, e))?;
        Ok(RecoveryOutcome::Quarantined { conflicts })
    }
}

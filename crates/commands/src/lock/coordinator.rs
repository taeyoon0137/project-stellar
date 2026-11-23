//! 같은 프로젝트의 창·프로세스가 공유하는 lease 조정자 (XIX.4.5, XIX.4.6, XIX.4.7, ADR-0093).
//!
//! [`ProjectLockCoordinator`]는 프로젝트 하나의 활성 lease와 사용자 조작(gesture) 표시를 OS 파일 잠금으로
//! 공유합니다. 같은 프로세스의 여러 창과 다른 프로세스(다른 앱 instance, `stellar` CLI)가 같은 디렉터리를
//! 열면 같은 lease를 봅니다.
//!
//! - **위치**: 호출자가 준 `base` 아래 `<project key>/`입니다. key는 저장 잠금(XXIV.4.5)과 같은 프로젝트
//!   identity key(`stellar_ops::lock::lock_key`, project id ‖ canonical target)를 씁니다. 그래서 다른 worktree는
//!   다른 key, 다른 lease 공간입니다. 위치는 프로젝트 폴더와 Git 밖이어야 합니다(§31.4: 잠금은 프로젝트
//!   데이터나 Git에 저장하지 않음). 디렉터리는 소유자 전용(`0700`)이고 symlink는 따라가지 않습니다.
//! - **항목 파일**: lease 하나 = `lease-<id>.txt`, gesture 하나 = `gesture-<id>.txt`. 쥔 쪽은 파일을 연 채
//!   `flock(2)` 배타 잠금을 유지합니다(Rust std `File::try_lock`). macOS `flock(2)`은 open file description에
//!   붙으므로 같은 프로세스의 다른 창이 따로 연 파일과도 충돌합니다. 쥔 프로세스가 끝나면(비정상 종료 포함) OS가
//!   잠금을 풀고, 다음 조회가 잠기지 않은 항목을 오래된 것으로 보고 지웁니다.
//! - **직렬화**: 조회·획득·해제는 `coordinator.lock`의 배타 잠금 안에서만 합니다. 이 구간은 디렉터리 하나를
//!   읽고 파일 하나를 쓰는 짧은 구간이라 기다립니다(blocking `flock`).
//! - **판정**: 겹침은 [`super::lease::LeaseTable::first_conflict`]와 [`SubtreeLock::overlaps`] 하나로 봅니다.
//!   gesture와 겹치는 lease 요청은 [`Conflict::Gesture`]로 거부하고, 호출자가 gesture가 끝난 뒤 다시 요청합니다
//!   (XIX.4.7). 반대로 lease 안 node에서 gesture를 시작하는 것은 거부합니다.
//!
//! 항목 파일 형식(UTF-8 줄, 공백 구분):
//!
//! ```text
//! stellar-agent-lease 1
//! lease <id>                          # 또는 gesture <id>
//! subtree <document> <root> [<조상> ...]
//! external <token|component-definition|asset> <id>
//! ```

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use stellar_core_types::ids::StableId;

use super::lease::{
    Conflict, ExternalKind, ExternalResource, Lease, LeaseId, LeaseShapeError, LeaseTable,
    SubtreeLock,
};

/// 저장 잠금 디렉터리(`stellar_ops::lock::default_lock_dir`) 아래 lease 조정 디렉터리 이름. 제품 경로의 `base`는
/// `<저장 잠금 디렉터리>/agent-leases`입니다.
pub const LEASE_DIR: &str = "agent-leases";

/// 항목 파일 첫 줄.
const HEADER: &str = "stellar-agent-lease 1";
/// 직렬화 잠금 파일.
const COORDINATOR_FILE: &str = "coordinator.lock";
/// 항목 파일 하나의 최대 크기. 넘으면 손상으로 봅니다.
const MAX_ENTRY_BYTES: u64 = 1 << 20;

/// 사용자 상호작용 미리보기(drag 등) 하나. 대상 node와 그 subtree를 덮습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gesture {
    /// gesture id.
    pub id: StableId,
    /// 조작 중인 node(조상 포함).
    pub nodes: Vec<SubtreeLock>,
}

/// 조정자 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordinatorError {
    /// project key 형식이 틀렸습니다(소문자 16진수 1~64자).
    InvalidKey,
    /// 조정 디렉터리 자리가 디렉터리가 아니거나 symlink입니다.
    NotADirectory(PathBuf),
    /// 파일 입출력 실패.
    Io {
        /// 작업.
        op: &'static str,
        /// 대상.
        path: PathBuf,
        /// 원인 종류.
        kind: io::ErrorKind,
    },
    /// 살아 있는 항목 파일 내용이 형식에 맞지 않습니다.
    Corrupt(PathBuf),
}

impl fmt::Display for CoordinatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoordinatorError::InvalidKey => f.write_str("project key 형식이 올바르지 않습니다"),
            CoordinatorError::NotADirectory(path) => {
                write!(
                    f,
                    "lease 디렉터리 자리가 디렉터리가 아닙니다: {}",
                    path.display()
                )
            }
            CoordinatorError::Io { op, path, kind } => {
                write!(f, "lease {op} 실패({kind:?}): {}", path.display())
            }
            CoordinatorError::Corrupt(path) => {
                write!(f, "lease 항목 형식이 올바르지 않습니다: {}", path.display())
            }
        }
    }
}

impl std::error::Error for CoordinatorError {}

/// 획득 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquireError {
    /// 다른 lease·gesture와 겹칩니다.
    Conflict(Conflict),
    /// 요청 대상 목록이 틀렸습니다.
    Shape(LeaseShapeError),
    /// 같은 id가 이미 살아 있습니다.
    DuplicateId(StableId),
    /// 조정자 오류.
    Coordinator(CoordinatorError),
}

impl fmt::Display for AcquireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AcquireError::Conflict(c) => write!(f, "{c}"),
            AcquireError::Shape(e) => write!(f, "{e}"),
            AcquireError::DuplicateId(id) => write!(f, "id {id}가 이미 쓰이고 있습니다"),
            AcquireError::Coordinator(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AcquireError {}

impl From<CoordinatorError> for AcquireError {
    fn from(error: CoordinatorError) -> Self {
        AcquireError::Coordinator(error)
    }
}

/// 프로젝트 하나의 lease 조정자.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectLockCoordinator {
    dir: PathBuf,
}

/// 쥔 lease. drop하거나 [`HeldLease::release`]하면 풀립니다.
#[derive(Debug)]
pub struct HeldLease {
    lease: Lease,
    entry: Entry,
}

/// 진행 중 gesture 표시. drop하거나 [`HeldGesture::end`]하면 끝납니다.
#[derive(Debug)]
pub struct HeldGesture {
    gesture: Gesture,
    entry: Entry,
}

#[derive(Debug)]
struct Entry {
    dir: PathBuf,
    path: PathBuf,
    file: Option<File>,
}

impl Entry {
    fn release(&mut self) -> Result<(), CoordinatorError> {
        let Some(file) = self.file.take() else {
            return Ok(());
        };
        let _guard = lock_coordinator(&self.dir)?;
        let removed = std::fs::remove_file(&self.path);
        drop(file);
        match removed {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error("삭제", &self.path, &error)),
        }
    }
}

impl Drop for Entry {
    fn drop(&mut self) {
        // drop 경로에는 오류를 돌려줄 곳이 없습니다. 지우지 못한 파일은 잠금이 풀려 다음 조회가 지웁니다.
        let _ = self.release();
    }
}

impl HeldLease {
    /// 쥔 lease.
    pub fn lease(&self) -> &Lease {
        &self.lease
    }

    /// lease를 풉니다(정상 완료·세션 종료).
    pub fn release(mut self) -> Result<(), CoordinatorError> {
        self.entry.release()
    }
}

impl HeldGesture {
    /// gesture.
    pub fn gesture(&self) -> &Gesture {
        &self.gesture
    }

    /// gesture를 끝냅니다. 기다리던 lease 요청은 이 뒤에 다시 시도하면 획득합니다(XIX.4.7).
    pub fn end(mut self) -> Result<(), CoordinatorError> {
        self.entry.release()
    }
}

/// 조회한 살아 있는 항목.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveEntries {
    /// 활성 lease.
    pub leases: LeaseTable,
    /// 진행 중 gesture.
    pub gestures: Vec<Gesture>,
}

impl ProjectLockCoordinator {
    /// `base/<project_key>`를 엽니다(없으면 소유자 전용으로 만듦).
    pub fn open(
        base: &Path,
        project_key: &str,
    ) -> Result<ProjectLockCoordinator, CoordinatorError> {
        let valid = !project_key.is_empty()
            && project_key.len() <= 64
            && project_key
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !valid {
            return Err(CoordinatorError::InvalidKey);
        }
        let dir = base.join(project_key);
        ensure_private_dir(base)?;
        ensure_private_dir(&dir)?;
        Ok(ProjectLockCoordinator { dir })
    }

    /// 조정 디렉터리.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 살아 있는 lease와 gesture를 읽습니다. 쥔 프로세스가 끝난 항목은 지웁니다.
    pub fn live(&self) -> Result<LiveEntries, CoordinatorError> {
        let _guard = lock_coordinator(&self.dir)?;
        self.scan()
    }

    /// 활성 lease 목록. 사용자 편집 판정([`super::lease::execute_leased`])의 입력입니다.
    pub fn leases(&self) -> Result<LeaseTable, CoordinatorError> {
        Ok(self.live()?.leases)
    }

    /// lease를 획득합니다. 다른 lease나 진행 중 gesture와 겹치면 아무것도 쓰지 않고 거부합니다(XIX.4.6·XIX.4.7).
    pub fn try_acquire(&self, lease: Lease) -> Result<HeldLease, AcquireError> {
        lease.check_shape().map_err(AcquireError::Shape)?;
        let _guard = lock_coordinator(&self.dir)?;
        let live = self.scan()?;
        if live.leases.get(lease.id).is_some() {
            return Err(AcquireError::DuplicateId(lease.id.0));
        }
        if let Some(conflict) = live.leases.first_conflict(&lease) {
            return Err(AcquireError::Conflict(conflict));
        }
        for gesture in &live.gestures {
            for subtree in &lease.subtrees {
                if gesture.nodes.iter().any(|node| node.overlaps(subtree)) {
                    return Err(AcquireError::Conflict(Conflict::Gesture {
                        gesture: gesture.id,
                        requested: subtree.root,
                    }));
                }
            }
        }
        let text = encode("lease", lease.id.0, &lease.subtrees, &lease.externals);
        let entry = self.write_entry(&format!("lease-{}.txt", lease.id), &text, lease.id.0)?;
        Ok(HeldLease { lease, entry })
    }

    /// 사용자 조작 미리보기를 표시합니다. lease 안 node(또는 lease root의 조상)를 조작하면 거부합니다.
    pub fn begin_gesture(&self, gesture: Gesture) -> Result<HeldGesture, AcquireError> {
        let _guard = lock_coordinator(&self.dir)?;
        let live = self.scan()?;
        if live.gestures.iter().any(|g| g.id == gesture.id) {
            return Err(AcquireError::DuplicateId(gesture.id));
        }
        for lease in live.leases.iter() {
            for held in &lease.subtrees {
                if let Some(node) = gesture.nodes.iter().find(|node| node.overlaps(held)) {
                    return Err(AcquireError::Conflict(Conflict::Subtree {
                        holder: lease.id,
                        requested: node.root,
                        held: held.root,
                    }));
                }
            }
        }
        let text = encode("gesture", gesture.id, &gesture.nodes, &[]);
        let entry = self.write_entry(&format!("gesture-{}.txt", gesture.id), &text, gesture.id)?;
        Ok(HeldGesture { gesture, entry })
    }

    fn write_entry(&self, name: &str, text: &str, id: StableId) -> Result<Entry, AcquireError> {
        let path = self.dir.join(name);
        let mut file = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(AcquireError::DuplicateId(id));
            }
            Err(error) => return Err(io_error("생성", &path, &error).into()),
        };
        if let Err(error) = file.try_lock() {
            let _ = std::fs::remove_file(&path);
            return Err(match error {
                TryLockError::WouldBlock => AcquireError::DuplicateId(id),
                TryLockError::Error(error) => io_error("잠금", &path, &error).into(),
            });
        }
        if let Err(error) = file
            .write_all(text.as_bytes())
            .and_then(|()| file.sync_data())
        {
            let _ = std::fs::remove_file(&path);
            return Err(io_error("쓰기", &path, &error).into());
        }
        Ok(Entry {
            dir: self.dir.clone(),
            path,
            file: Some(file),
        })
    }

    /// coordinator 잠금을 쥔 상태에서 호출합니다.
    fn scan(&self) -> Result<LiveEntries, CoordinatorError> {
        let mut live = LiveEntries::default();
        let entries = std::fs::read_dir(&self.dir).map_err(|e| io_error("목록", &self.dir, &e))?;
        let mut paths: Vec<PathBuf> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| io_error("목록", &self.dir, &e))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if (name.starts_with("lease-") || name.starts_with("gesture-"))
                && name.ends_with(".txt")
            {
                paths.push(entry.path());
            }
        }
        paths.sort();
        for path in paths {
            let mut file = match File::open(&path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io_error("열기", &path, &error)),
            };
            match file.try_lock_shared() {
                Ok(()) => {
                    // 쥔 쪽이 없습니다(정상 해제 전에 끝난 프로세스). 지웁니다.
                    drop(file);
                    match std::fs::remove_file(&path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(io_error("삭제", &path, &error)),
                    }
                    continue;
                }
                Err(TryLockError::WouldBlock) => {}
                Err(TryLockError::Error(error)) => {
                    return Err(io_error("잠금 확인", &path, &error))
                }
            }
            let mut text = String::new();
            Read::by_ref(&mut file)
                .take(MAX_ENTRY_BYTES + 1)
                .read_to_string(&mut text)
                .map_err(|_| CoordinatorError::Corrupt(path.clone()))?;
            if text.len() as u64 > MAX_ENTRY_BYTES {
                return Err(CoordinatorError::Corrupt(path));
            }
            match decode(&text).ok_or_else(|| CoordinatorError::Corrupt(path.clone()))? {
                Decoded::Lease(lease) => {
                    // 살아 있는 항목끼리는 획득 때 겹침을 막았으므로 insert가 실패하면 손상입니다.
                    live.leases
                        .insert(lease)
                        .map_err(|_| CoordinatorError::Corrupt(path.clone()))?;
                }
                Decoded::Gesture(gesture) => live.gestures.push(gesture),
            }
        }
        Ok(live)
    }
}

fn ensure_private_dir(dir: &Path) -> Result<(), CoordinatorError> {
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(CoordinatorError::NotADirectory(dir.to_path_buf())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder
                .create(dir)
                .map_err(|e| io_error("디렉터리 생성", dir, &e))?;
            match std::fs::symlink_metadata(dir) {
                Ok(meta) if meta.is_dir() => Ok(()),
                _ => Err(CoordinatorError::NotADirectory(dir.to_path_buf())),
            }
        }
        Err(error) => Err(io_error("디렉터리 확인", dir, &error)),
    }
}

/// coordinator 배타 잠금. 반환값을 drop하면 풀립니다.
fn lock_coordinator(dir: &Path) -> Result<File, CoordinatorError> {
    let path = dir.join(COORDINATOR_FILE);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| io_error("열기", &path, &e))?;
    file.lock().map_err(|e| io_error("잠금", &path, &e))?;
    Ok(file)
}

fn io_error(op: &'static str, path: &Path, error: &io::Error) -> CoordinatorError {
    CoordinatorError::Io {
        op,
        path: path.to_path_buf(),
        kind: error.kind(),
    }
}

fn encode(
    kind: &str,
    id: StableId,
    subtrees: &[SubtreeLock],
    externals: &[ExternalResource],
) -> String {
    let mut out = format!("{HEADER}\n{kind} {id}\n");
    for subtree in subtrees {
        out.push_str(&format!("subtree {} {}", subtree.document, subtree.root));
        for ancestor in &subtree.ancestors {
            out.push_str(&format!(" {ancestor}"));
        }
        out.push('\n');
    }
    for resource in externals {
        out.push_str(&format!(
            "external {} {}\n",
            resource.kind.name(),
            resource.id
        ));
    }
    out
}

enum Decoded {
    Lease(Lease),
    Gesture(Gesture),
}

fn decode(text: &str) -> Option<Decoded> {
    let mut lines = text.lines();
    if lines.next()? != HEADER {
        return None;
    }
    let mut head = lines.next()?.split(' ');
    let kind = head.next()?;
    let id: StableId = head.next()?.parse().ok()?;
    if head.next().is_some() {
        return None;
    }
    let mut subtrees = Vec::new();
    let mut externals = Vec::new();
    for line in lines {
        let mut words = line.split(' ');
        match words.next()? {
            "subtree" => {
                let document = words.next()?.parse().ok()?;
                let root = words.next()?.parse().ok()?;
                let ancestors = words
                    .map(|w| w.parse().ok())
                    .collect::<Option<Vec<StableId>>>()?;
                subtrees.push(SubtreeLock {
                    document,
                    root,
                    ancestors,
                });
            }
            "external" => {
                let kind = ExternalKind::from_name(words.next()?)?;
                let id = words.next()?.parse().ok()?;
                if words.next().is_some() {
                    return None;
                }
                externals.push(ExternalResource { kind, id });
            }
            _ => return None,
        }
    }
    match kind {
        "lease" => Some(Decoded::Lease(Lease {
            id: LeaseId(id),
            subtrees,
            externals,
        })),
        "gesture" if externals.is_empty() => Some(Decoded::Gesture(Gesture {
            id,
            nodes: subtrees,
        })),
        _ => None,
    }
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;

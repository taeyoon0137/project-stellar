//! content-addressed immutable library cache entry (XV.3.3).
//!
//! exact commit의 **Stellar project files**(`stellar.yaml`, `stellar.lock.yaml`, `design/**`,
//! `assets/**` — 아키텍처 19장)를 fetch된 repository mirror(XV.3.2)에서 읽어 cache root의
//! `entries/sha256-<hex>/`에 한 번만 materialize합니다. `.git`과 그 밖의 파일(README, 소스 등)은
//! 만들지 않습니다.
//!
//! - **Git 객체를 직접 파싱하지 않음**: `ls-tree -r -l -z --full-tree`와 `cat-file --batch` 출력만
//!   해석합니다(XIV.2.6과 같은 방식). 원시 blob bytes를 쓰며 filter·textconv를 적용하지 않습니다.
//! - **content address**: entry 이름은 아래 canonical listing의 SHA-256입니다(ADR-0022). 같은
//!   내용이면 commit·repository가 달라도 같은 entry를 씁니다(아키텍처 30.3 "같은 콘텐츠는 공용
//!   캐시에서 중복 제거").
//!
//!   ```text
//!   stellar-library-entry 1\n
//!   <mode> SP <size> SP <sha256 hex> SP <path> \n     (path byte 오름차순, mode = 100644 | 100755)
//!   ```
//!
//! - **immutable**: staging 디렉터리에 모두 쓴 뒤 `rename` 한 번으로 공개합니다. 공개된 entry는
//!   다시 쓰지 않고 파일은 읽기 전용(0444/0555)입니다. 같은 commit을 다시 요청하면 cache index
//!   (privacy mode, XV.3.5)에서 바로 찾아 Git을 실행하지 않고 같은 bytes를 재사용합니다.
//! - **입력 검증**: symlink·submodule, `.git`·`.`·`..`·빈 component, 제어 문자·`\`가 있는 경로,
//!   대소문자만 다른 경로 충돌, 파일 수·총 크기 상한 초과를 거부합니다. 오류에는 경로·URL 원문을
//!   담지 않습니다(XV.1.4, II.3.2).
//!
//! SHA-256 구현은 storage crate에 있으므로(이 crate는 core-types에만 의존) [`Sha256Digest`]로
//! 받습니다. lock 무결성 검사(XV.4.4)를 위해 디스크의 entry에서 같은 hash를 다시 계산합니다
//! ([`recompute_entry_hash`], [`CachedContentSource`] 구현).

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::cache_index::{sync_dir, CacheIndexError};
use crate::cache_lock::{CacheLockError, CacheLockGuard, CacheLocks};
use crate::cache_privacy::{LocalSourceConfig, PrivateCacheIndex, PrivateIndexEntry};
use crate::cache_root::LibraryCacheRoot;
use crate::content_hash::ContentHashText;
use crate::declaration::{CommitId, RepositoryUrl};
use crate::git::{git_args, GitCommandRunner};
use crate::git_refs::FetchedRefs;
use crate::lock_integrity::{CacheReadFailure, CachedContentSource};
use crate::mirror::RepositoryKeyer;
use crate::selection::SelectorError;

/// canonical listing 첫 줄.
pub const ENTRY_LISTING_HEADER: &[u8] = b"stellar-library-entry 1\n";
/// 한 번의 `cat-file --batch`에 넘기는 최대 object 수.
pub const MAX_BATCH_OBJECTS: usize = 256;
/// entry 디렉터리 이름 접두사(`sha256-<hex>`).
pub const ENTRY_DIR_PREFIX: &str = "sha256-";

/// SHA-256 계산기(project가 storage 구현을 연결).
pub trait Sha256Digest {
    /// `bytes`의 SHA-256 digest.
    fn sha256(&self, bytes: &[u8]) -> [u8; 32];
}

/// materialize 상한.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterializeLimits {
    /// 최대 파일 수.
    pub max_files: usize,
    /// 최대 총 크기(byte).
    pub max_total_bytes: u64,
    /// `cat-file --batch` 한 번에 읽는 최대 bytes(이보다 큰 파일은 단독으로 읽음).
    pub max_batch_bytes: u64,
}

impl Default for MaterializeLimits {
    fn default() -> MaterializeLimits {
        MaterializeLimits {
            max_files: 100_000,
            max_total_bytes: 4 * 1024 * 1024 * 1024,
            max_batch_bytes: 64 * 1024 * 1024,
        }
    }
}

/// entry 파일 mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryFileMode {
    /// 일반 파일(`100644`).
    Regular,
    /// 실행 파일(`100755`).
    Executable,
}

impl EntryFileMode {
    /// Git mode 문자열.
    pub fn as_str(self) -> &'static str {
        match self {
            EntryFileMode::Regular => "100644",
            EntryFileMode::Executable => "100755",
        }
    }

    #[cfg(unix)]
    fn read_only_permissions(self) -> u32 {
        match self {
            EntryFileMode::Regular => 0o444,
            EntryFileMode::Executable => 0o555,
        }
    }
}

/// canonical listing의 파일 한 건.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntryFileRecord {
    /// entry root 기준 `/` 구분 경로.
    pub path: String,
    /// mode.
    pub mode: EntryFileMode,
    /// 크기(byte).
    pub size: u64,
    /// 내용 SHA-256.
    pub sha256: [u8; 32],
}

/// 파일 목록의 canonical listing bytes(경로 byte 오름차순으로 정렬해 기록).
pub fn entry_listing(files: &[EntryFileRecord]) -> Vec<u8> {
    let mut sorted: Vec<&EntryFileRecord> = files.iter().collect();
    sorted.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    let mut out = ENTRY_LISTING_HEADER.to_vec();
    for file in sorted {
        out.extend_from_slice(file.mode.as_str().as_bytes());
        out.push(b' ');
        out.extend_from_slice(file.size.to_string().as_bytes());
        out.push(b' ');
        for byte in file.sha256 {
            out.extend_from_slice(format!("{byte:02x}").as_bytes());
        }
        out.push(b' ');
        out.extend_from_slice(file.path.as_bytes());
        out.push(b'\n');
    }
    out
}

/// 파일 목록의 entry content hash(`sha256:` + listing SHA-256).
pub fn entry_content_hash(files: &[EntryFileRecord], hasher: &dyn Sha256Digest) -> ContentHashText {
    ContentHashText::from_sha256_digest(hasher.sha256(&entry_listing(files)))
}

/// 재사용 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryReuse {
    /// 같은 (repository, commit)이 index에 있어 Git을 실행하지 않고 재사용.
    CommitHit,
    /// 새 commit이지만 같은 내용의 entry가 이미 있어 재사용(새로 쓰지 않음).
    ContentHit,
    /// 새 entry를 만듦.
    Created,
}

/// materialize 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedEntry {
    /// entry content hash.
    pub content_hash: ContentHashText,
    /// entry 디렉터리.
    pub dir: PathBuf,
    /// Stellar project files 총 크기(byte).
    pub size_bytes: u64,
    /// 재사용 종류.
    pub reuse: EntryReuse,
}

/// 공유 read lock을 보유한 채 연 entry(XV.3.6). drop하면 lock이 풀립니다.
#[derive(Debug)]
pub struct OpenedEntry {
    content_hash: ContentHashText,
    dir: PathBuf,
    size_bytes: u64,
    _read_lock: CacheLockGuard,
}

impl OpenedEntry {
    /// entry content hash.
    pub fn content_hash(&self) -> &ContentHashText {
        &self.content_hash
    }

    /// entry 디렉터리(읽기 전용).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Stellar project files 총 크기(byte).
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
}

/// materialize할 수 없는 tree 항목 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedEntryKind {
    /// symlink(`120000`). 읽기 전용 공용 cache에서 외부 대상을 가리키지 않게 거부합니다.
    Symlink,
    /// submodule(gitlink `160000`).
    Submodule,
    /// 그 밖의 mode·종류.
    Other,
}

/// 거부한 경로 규칙.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedPathReason {
    /// UTF-8이 아님.
    NotUtf8,
    /// 빈 component·`.`·`..`.
    BadComponent,
    /// `.git`(대소문자 무시) component.
    DotGit,
    /// 제어 문자 또는 `\`.
    BadCharacter,
}

/// materialize 실패. 경로·URL 원문은 담지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializeError {
    /// mirror를 준비하지 못함.
    Mirror(SelectorError),
    /// Git 명령 실패.
    Git {
        /// 작업 이름.
        operation: &'static str,
        /// 종료 코드.
        exit_code: Option<i32>,
    },
    /// Git 출력 해석 실패.
    MalformedOutput {
        /// 작업 이름.
        operation: &'static str,
    },
    /// commit이 mirror에 없거나 commit이 아님.
    CommitNotFound,
    /// commit root에 `stellar.yaml`이 없음(일반 Git 저장소).
    NotAStellarProject,
    /// materialize할 수 없는 항목.
    UnsupportedEntry {
        /// 종류.
        kind: UnsupportedEntryKind,
    },
    /// 거부한 경로.
    UnsupportedPath {
        /// 이유.
        reason: UnsupportedPathReason,
    },
    /// 대소문자만 다른 경로 또는 파일/디렉터리 충돌.
    PathCollision,
    /// 파일 수 상한 초과.
    TooManyFiles {
        /// 상한.
        max: usize,
    },
    /// 총 크기 상한 초과.
    TooLarge {
        /// 상한(byte).
        max_bytes: u64,
    },
    /// `cat-file` 응답이 `ls-tree` 항목과 다름(object id·종류·크기).
    BlobMismatch,
    /// cache index 읽기·쓰기 실패.
    Index(CacheIndexError),
    /// cache lock 실패(XV.3.6).
    Lock(CacheLockError),
    /// 파일 시스템 실패.
    Io {
        /// 단계.
        stage: &'static str,
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
}

impl fmt::Display for MaterializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MaterializeError::Mirror(e) => write!(f, "mirror 준비 실패: {e}"),
            MaterializeError::Git {
                operation,
                exit_code,
            } => write!(f, "Git {operation} 실패(종료 코드 {exit_code:?})"),
            MaterializeError::MalformedOutput { operation } => {
                write!(f, "Git {operation} 출력을 해석할 수 없습니다")
            }
            MaterializeError::CommitNotFound => f.write_str("commit이 mirror에 없습니다"),
            MaterializeError::NotAStellarProject => {
                f.write_str("commit root에 stellar.yaml이 없습니다(일반 Git 저장소)")
            }
            MaterializeError::UnsupportedEntry { kind } => {
                write!(f, "materialize할 수 없는 항목: {kind:?}")
            }
            MaterializeError::UnsupportedPath { reason } => {
                write!(f, "허용하지 않는 경로: {reason:?}")
            }
            MaterializeError::PathCollision => {
                f.write_str("대소문자만 다르거나 파일/디렉터리가 겹치는 경로가 있습니다")
            }
            MaterializeError::TooManyFiles { max } => write!(f, "파일 수가 {max}개를 넘습니다"),
            MaterializeError::TooLarge { max_bytes } => {
                write!(f, "총 크기가 {max_bytes} byte를 넘습니다")
            }
            MaterializeError::BlobMismatch => f.write_str("cat-file 응답이 tree 항목과 다릅니다"),
            MaterializeError::Index(e) => write!(f, "cache index 오류: {e}"),
            MaterializeError::Lock(e) => write!(f, "{e}"),
            MaterializeError::Io { stage, kind } => write!(f, "cache entry {stage} 실패: {kind:?}"),
        }
    }
}

impl std::error::Error for MaterializeError {}

fn io_error(stage: &'static str) -> impl Fn(std::io::Error) -> MaterializeError {
    move |e| MaterializeError::Io {
        stage,
        kind: e.kind(),
    }
}

/// `ls-tree`에서 고른 materialize 대상 파일.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TreeFile {
    path: String,
    mode: EntryFileMode,
    oid: String,
    size: u64,
}

/// Stellar project files인지(아키텍처 19장 구조).
fn is_project_file(path: &[u8]) -> bool {
    path == b"stellar.yaml"
        || path == b"stellar.lock.yaml"
        || path.starts_with(b"design/")
        || path.starts_with(b"assets/")
}

fn validate_path(path: &[u8]) -> Result<String, MaterializeError> {
    let text = std::str::from_utf8(path).map_err(|_| MaterializeError::UnsupportedPath {
        reason: UnsupportedPathReason::NotUtf8,
    })?;
    for component in text.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(MaterializeError::UnsupportedPath {
                reason: UnsupportedPathReason::BadComponent,
            });
        }
        if component.eq_ignore_ascii_case(".git") {
            return Err(MaterializeError::UnsupportedPath {
                reason: UnsupportedPathReason::DotGit,
            });
        }
        if component.chars().any(|c| c.is_control() || c == '\\') {
            return Err(MaterializeError::UnsupportedPath {
                reason: UnsupportedPathReason::BadCharacter,
            });
        }
    }
    Ok(text.to_string())
}

/// `ls-tree -r -l -z --full-tree` 출력에서 Stellar project files를 고르고 검증합니다.
fn parse_tree(stdout: &[u8]) -> Result<Vec<TreeFile>, MaterializeError> {
    const OP: &str = "ls-tree";
    let mut files = Vec::new();
    for record in stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let tab = record
            .iter()
            .position(|b| *b == b'\t')
            .ok_or(MaterializeError::MalformedOutput { operation: OP })?;
        let (meta, path) = (&record[..tab], &record[tab + 1..]);
        if !is_project_file(path) {
            continue;
        }
        let meta = std::str::from_utf8(meta)
            .map_err(|_| MaterializeError::MalformedOutput { operation: OP })?;
        let mut fields = meta.split(' ').filter(|f| !f.is_empty());
        let (Some(mode), Some(kind), Some(oid), Some(size), None) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            return Err(MaterializeError::MalformedOutput { operation: OP });
        };
        let mode = match (mode, kind) {
            ("100644", "blob") => EntryFileMode::Regular,
            ("100755", "blob") => EntryFileMode::Executable,
            ("120000", _) => {
                return Err(MaterializeError::UnsupportedEntry {
                    kind: UnsupportedEntryKind::Symlink,
                })
            }
            ("160000", _) => {
                return Err(MaterializeError::UnsupportedEntry {
                    kind: UnsupportedEntryKind::Submodule,
                })
            }
            _ => {
                return Err(MaterializeError::UnsupportedEntry {
                    kind: UnsupportedEntryKind::Other,
                })
            }
        };
        if oid.parse::<CommitId>().is_err() {
            // object id 형식(40/64 hex)은 commit id와 같습니다.
            return Err(MaterializeError::MalformedOutput { operation: OP });
        }
        let size = size
            .parse::<u64>()
            .map_err(|_| MaterializeError::MalformedOutput { operation: OP })?;
        files.push(TreeFile {
            path: validate_path(path)?,
            mode,
            oid: oid.to_ascii_lowercase(),
            size,
        });
    }
    check_collisions(&files)?;
    Ok(files)
}

/// 대소문자 무시 기준으로 파일끼리, 파일과 디렉터리가 겹치지 않는지 확인합니다(아키텍처 19.2).
fn check_collisions(files: &[TreeFile]) -> Result<(), MaterializeError> {
    let mut file_keys = BTreeSet::new();
    let mut dir_keys = BTreeSet::new();
    for file in files {
        let folded = file.path.to_lowercase();
        if !file_keys.insert(folded.clone()) {
            return Err(MaterializeError::PathCollision);
        }
        let mut end = 0;
        while let Some(offset) = folded[end..].find('/') {
            end += offset;
            dir_keys.insert(folded[..end].to_string());
            end += 1;
        }
    }
    if file_keys.intersection(&dir_keys).next().is_some() {
        return Err(MaterializeError::PathCollision);
    }
    Ok(())
}

/// `cat-file --batch` 출력을 요청 순서대로 해석합니다(오버플로 없는 경계 검사).
fn parse_batch<'a>(
    stdout: &'a [u8],
    requested: &[&TreeFile],
) -> Result<Vec<&'a [u8]>, MaterializeError> {
    const OP: &str = "cat-file";
    let mut rest = stdout;
    let mut out = Vec::with_capacity(requested.len());
    for file in requested {
        let lf = rest
            .iter()
            .position(|b| *b == b'\n')
            .ok_or(MaterializeError::MalformedOutput { operation: OP })?;
        let header = std::str::from_utf8(&rest[..lf])
            .map_err(|_| MaterializeError::MalformedOutput { operation: OP })?;
        rest = &rest[lf + 1..];
        let mut fields = header.split(' ');
        let (Some(oid), Some(kind), Some(size), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            // `<oid> missing` 포함.
            return Err(MaterializeError::BlobMismatch);
        };
        let size: u64 = size
            .parse()
            .map_err(|_| MaterializeError::MalformedOutput { operation: OP })?;
        if oid != file.oid || kind != "blob" || size != file.size {
            return Err(MaterializeError::BlobMismatch);
        }
        let size = usize::try_from(size).map_err(|_| MaterializeError::BlobMismatch)?;
        let end = size
            .checked_add(1)
            .filter(|end| *end <= rest.len())
            .ok_or(MaterializeError::MalformedOutput { operation: OP })?;
        if rest[size] != b'\n' {
            return Err(MaterializeError::MalformedOutput { operation: OP });
        }
        out.push(&rest[..size]);
        rest = &rest[end..];
    }
    if !rest.is_empty() {
        return Err(MaterializeError::MalformedOutput { operation: OP });
    }
    Ok(out)
}

/// drop 시 지우는 staging 디렉터리.
struct Staging {
    path: PathBuf,
    keep: bool,
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// 디스크 entry 재계산 실패.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryScanError {
    /// 일반 파일·디렉터리가 아닌 항목(symlink 등) 또는 UTF-8이 아닌 이름.
    UnexpectedEntry,
    /// 읽기 실패.
    Io {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
}

/// 디스크의 entry 디렉터리에서 content hash를 다시 계산합니다(symlink를 따르지 않음, 반복 순회).
///
/// 반환: (content hash, 총 크기, 파일 수).
pub fn recompute_entry_hash(
    dir: &Path,
    hasher: &dyn Sha256Digest,
) -> Result<(ContentHashText, u64, usize), EntryScanError> {
    let io = |e: std::io::Error| EntryScanError::Io { kind: e.kind() };
    let mut files = Vec::new();
    let mut stack: Vec<(PathBuf, String)> = vec![(dir.to_path_buf(), String::new())];
    while let Some((path, prefix)) = stack.pop() {
        for item in std::fs::read_dir(&path).map_err(io)? {
            let item = item.map_err(io)?;
            let name = item
                .file_name()
                .into_string()
                .map_err(|_| EntryScanError::UnexpectedEntry)?;
            let rel = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let meta = std::fs::symlink_metadata(item.path()).map_err(io)?;
            if meta.is_dir() {
                stack.push((item.path(), rel));
            } else if meta.is_file() {
                let bytes = std::fs::read(item.path()).map_err(io)?;
                files.push(EntryFileRecord {
                    path: rel,
                    mode: file_mode(&meta),
                    size: bytes.len() as u64,
                    sha256: hasher.sha256(&bytes),
                });
            } else {
                return Err(EntryScanError::UnexpectedEntry);
            }
        }
    }
    let size = files.iter().map(|f| f.size).sum();
    let count = files.len();
    Ok((entry_content_hash(&files, hasher), size, count))
}

#[cfg(unix)]
fn file_mode(meta: &std::fs::Metadata) -> EntryFileMode {
    use std::os::unix::fs::PermissionsExt;
    if meta.permissions().mode() & 0o111 != 0 {
        EntryFileMode::Executable
    } else {
        EntryFileMode::Regular
    }
}

#[cfg(not(unix))]
fn file_mode(_: &std::fs::Metadata) -> EntryFileMode {
    EntryFileMode::Regular
}

/// content-addressed library entry store.
pub struct LibraryEntryStore<H, K> {
    root: LibraryCacheRoot,
    hasher: H,
    keyer: K,
    limits: MaterializeLimits,
}

impl<H: Sha256Digest, K: RepositoryKeyer> LibraryEntryStore<H, K> {
    /// 기본 상한으로 store를 엽니다. `keyer`는 mirror cache와 같아야 합니다.
    pub fn new(root: LibraryCacheRoot, hasher: H, keyer: K) -> LibraryEntryStore<H, K> {
        LibraryEntryStore {
            root,
            hasher,
            keyer,
            limits: MaterializeLimits::default(),
        }
    }

    /// 상한을 바꿉니다.
    pub fn with_limits(mut self, limits: MaterializeLimits) -> LibraryEntryStore<H, K> {
        self.limits = limits;
        self
    }

    /// cache root.
    pub fn root(&self) -> &LibraryCacheRoot {
        &self.root
    }

    /// content hash의 entry 디렉터리(`entries/sha256-<hex>`).
    pub fn entry_dir(&self, hash: &ContentHashText) -> PathBuf {
        let hex = hash
            .as_str()
            .strip_prefix(crate::content_hash::SHA256_PREFIX)
            .unwrap_or(hash.as_str());
        self.root
            .entries_dir()
            .join(format!("{ENTRY_DIR_PREFIX}{hex}"))
    }

    /// privacy index에서 `(repository, commit)` entry를 찾습니다(디렉터리가 있을 때만).
    pub fn lookup(
        &self,
        repository: &RepositoryUrl,
        commit: &CommitId,
    ) -> Result<Option<PrivateIndexEntry>, MaterializeError> {
        let index = PrivateCacheIndex::read_from(&self.root.index_path())
            .map_err(MaterializeError::Index)?;
        Ok(index
            .get(&self.keyer, repository, commit)
            .filter(|e| self.entry_dir(&e.content_hash).is_dir())
            .cloned())
    }

    /// exact commit의 Stellar project files를 entry로 materialize하거나 기존 entry를 재사용합니다.
    ///
    /// 같은 commit이 index에 있으면 Git을 실행하지 않습니다(`CommitHit`). 없으면 `mirrors`에서 fetch된
    /// git dir을 얻어 읽고, 같은 내용 entry가 이미 있으면 새로 쓰지 않습니다(`ContentHit`).
    pub fn materialize<R, F>(
        &mut self,
        runner: &mut R,
        mirrors: &mut F,
        repository: &RepositoryUrl,
        commit: &CommitId,
        now_unix_ms: u64,
    ) -> Result<MaterializedEntry, MaterializeError>
    where
        R: GitCommandRunner + ?Sized,
        F: FetchedRefs + ?Sized,
    {
        let key = self.keyer.repository_key(repository);
        if let Some(hit) = self.lookup(repository, commit)? {
            self.touch(&key, commit, now_unix_ms)?;
            return Ok(MaterializedEntry {
                dir: self.entry_dir(&hit.content_hash),
                content_hash: hit.content_hash,
                size_bytes: hit.size_bytes,
                reuse: EntryReuse::CommitHit,
            });
        }

        let git_dir = mirrors
            .fetched_git_dir(repository)
            .map_err(MaterializeError::Mirror)?;
        self.verify_commit(runner, &git_dir, commit)?;
        let listing = self.git(
            runner,
            &git_dir,
            &["ls-tree", "-r", "-l", "-z", "--full-tree", commit.as_str()],
            "ls-tree",
        )?;
        let files = parse_tree(&listing)?;
        if !files.iter().any(|f| f.path == "stellar.yaml") {
            return Err(MaterializeError::NotAStellarProject);
        }
        if files.len() > self.limits.max_files {
            return Err(MaterializeError::TooManyFiles {
                max: self.limits.max_files,
            });
        }
        let total = files
            .iter()
            .try_fold(0u64, |acc, f| acc.checked_add(f.size))
            .filter(|t| *t <= self.limits.max_total_bytes)
            .ok_or(MaterializeError::TooLarge {
                max_bytes: self.limits.max_total_bytes,
            })?;

        let entries = self.root.entries_dir();
        std::fs::create_dir_all(&entries).map_err(io_error("create-entries"))?;
        let staging = Staging {
            path: entries.join(staging_name()),
            keep: false,
        };
        std::fs::create_dir(&staging.path).map_err(io_error("create-staging"))?;
        let records = self.write_files(runner, &git_dir, &files, &staging.path)?;
        let content_hash = entry_content_hash(&records, &self.hasher);
        let dir = self.entry_dir(&content_hash);

        let reuse = if dir.is_dir() {
            EntryReuse::ContentHit
        } else {
            match std::fs::rename(&staging.path, &dir) {
                Ok(()) => {
                    let mut staging = staging;
                    staging.keep = true;
                    sync_dir(&entries).map_err(io_error("sync-entries"))?;
                    EntryReuse::Created
                }
                // 다른 materializer가 같은 내용을 먼저 공개함.
                Err(_) if dir.is_dir() => EntryReuse::ContentHit,
                Err(e) => return Err(io_error("publish")(e)),
            }
        };

        // index·source config read-modify-write는 짧은 index lock 안에서(XV.3.6). 다른 프로세스의
        // 갱신을 잃지 않도록 lock을 잡은 뒤 다시 읽습니다.
        let _index_lock = self.locks().index().map_err(MaterializeError::Lock)?;
        let index_path = self.root.index_path();
        let mut index =
            PrivateCacheIndex::read_from(&index_path).map_err(MaterializeError::Index)?;
        index.upsert(PrivateIndexEntry {
            repository_key: key.clone(),
            commit: commit.clone(),
            content_hash: content_hash.clone(),
            size_bytes: total,
            last_used_unix_ms: now_unix_ms,
        });
        index
            .write_to(&index_path)
            .map_err(MaterializeError::Index)?;
        let sources_path = self
            .root
            .path()
            .join(crate::cache_privacy::SOURCE_CONFIG_FILE);
        let mut sources =
            LocalSourceConfig::read_from(&sources_path).map_err(MaterializeError::Index)?;
        if sources.upsert(key, repository.clone()) {
            sources
                .write_to(&sources_path)
                .map_err(MaterializeError::Index)?;
        }
        Ok(MaterializedEntry {
            content_hash,
            dir,
            size_bytes: total,
            reuse,
        })
    }

    /// cache lock 디렉터리(XV.3.6).
    pub fn locks(&self) -> CacheLocks {
        CacheLocks::new(&self.root)
    }

    /// 최근 사용 시각을 index lock 안에서 갱신합니다.
    fn touch(
        &self,
        key: &crate::mirror::RepositoryKey,
        commit: &CommitId,
        now_unix_ms: u64,
    ) -> Result<(), MaterializeError> {
        let _index_lock = self.locks().index().map_err(MaterializeError::Lock)?;
        let index_path = self.root.index_path();
        let mut index =
            PrivateCacheIndex::read_from(&index_path).map_err(MaterializeError::Index)?;
        if index.touch(key, commit, now_unix_ms) {
            index
                .write_to(&index_path)
                .map_err(MaterializeError::Index)?;
        }
        Ok(())
    }

    /// 이미 materialize된 `(repository, commit)` entry를 **공유 read lock**을 잡고 엽니다(XV.3.6).
    ///
    /// Git·fetch lock을 쓰지 않으므로 다른 프로세스가 fetch·materialize 중이어도 기다리지 않습니다.
    /// entry 삭제(XV.8)는 read lock이 하나라도 있으면 진행하지 못합니다. lock을 잡은 뒤 디렉터리가
    /// 사라졌으면(그 사이 삭제) `None`입니다.
    pub fn open_entry(
        &self,
        repository: &RepositoryUrl,
        commit: &CommitId,
        now_unix_ms: u64,
    ) -> Result<Option<OpenedEntry>, MaterializeError> {
        let Some(hit) = self.lookup(repository, commit)? else {
            return Ok(None);
        };
        let guard = self
            .locks()
            .read_entry(&hit.content_hash)
            .map_err(MaterializeError::Lock)?;
        let dir = self.entry_dir(&hit.content_hash);
        if !dir.is_dir() {
            return Ok(None);
        }
        self.touch(&self.keyer.repository_key(repository), commit, now_unix_ms)?;
        Ok(Some(OpenedEntry {
            content_hash: hit.content_hash,
            dir,
            size_bytes: hit.size_bytes,
            _read_lock: guard,
        }))
    }

    fn git<R: GitCommandRunner + ?Sized>(
        &self,
        runner: &mut R,
        git_dir: &Path,
        args: &[&str],
        operation: &'static str,
    ) -> Result<Vec<u8>, MaterializeError> {
        let mut full = vec![OsString::from("--git-dir"), git_dir.as_os_str().to_owned()];
        full.extend(git_args(args.iter().copied()));
        let out = runner
            .run_git(&full, git_dir)
            .map_err(|_| MaterializeError::Git {
                operation,
                exit_code: None,
            })?;
        if !out.success() {
            return Err(MaterializeError::Git {
                operation,
                exit_code: out.exit_code,
            });
        }
        Ok(out.stdout)
    }

    fn verify_commit<R: GitCommandRunner + ?Sized>(
        &self,
        runner: &mut R,
        git_dir: &Path,
        commit: &CommitId,
    ) -> Result<(), MaterializeError> {
        match self.git(
            runner,
            git_dir,
            &["cat-file", "-t", commit.as_str()],
            "cat-file",
        ) {
            Ok(kind) if kind.trim_ascii() == b"commit" => Ok(()),
            // 실행은 됐지만 object가 없거나(종료 코드 ≠ 0) commit이 아님.
            Ok(_)
            | Err(MaterializeError::Git {
                exit_code: Some(_), ..
            }) => Err(MaterializeError::CommitNotFound),
            Err(other) => Err(other),
        }
    }

    /// blob을 batch로 읽어 staging에 쓰고 파일별 record를 만듭니다.
    fn write_files<R: GitCommandRunner + ?Sized>(
        &self,
        runner: &mut R,
        git_dir: &Path,
        files: &[TreeFile],
        staging: &Path,
    ) -> Result<Vec<EntryFileRecord>, MaterializeError> {
        let mut records = Vec::with_capacity(files.len());
        let mut start = 0;
        while start < files.len() {
            let mut end = start;
            let mut bytes = 0u64;
            while end < files.len()
                && end - start < MAX_BATCH_OBJECTS
                && (end == start || bytes + files[end].size <= self.limits.max_batch_bytes)
            {
                bytes += files[end].size;
                end += 1;
            }
            let batch: Vec<&TreeFile> = files[start..end].iter().collect();
            let mut input = Vec::new();
            for file in &batch {
                input.extend_from_slice(file.oid.as_bytes());
                input.push(b'\n');
            }
            let args = vec![
                OsString::from("--git-dir"),
                git_dir.as_os_str().to_owned(),
                OsString::from("cat-file"),
                OsString::from("--batch"),
            ];
            let out = runner
                .run_git_with_input(&args, git_dir, &input)
                .map_err(|_| MaterializeError::Git {
                    operation: "cat-file",
                    exit_code: None,
                })?;
            if !out.success() {
                return Err(MaterializeError::Git {
                    operation: "cat-file",
                    exit_code: out.exit_code,
                });
            }
            let contents = parse_batch(&out.stdout, &batch)?;
            for (file, content) in batch.iter().zip(contents) {
                write_entry_file(staging, file, content)?;
                records.push(EntryFileRecord {
                    path: file.path.clone(),
                    mode: file.mode,
                    size: file.size,
                    sha256: self.hasher.sha256(content),
                });
            }
            start = end;
        }
        Ok(records)
    }
}

/// staging에 파일 하나를 쓰고 읽기 전용으로 만듭니다.
fn write_entry_file(
    staging: &Path,
    file: &TreeFile,
    content: &[u8],
) -> Result<(), MaterializeError> {
    let mut path = staging.to_path_buf();
    for component in file.path.split('/') {
        path.push(component);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_error("create-dir"))?;
    }
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(io_error("create-file"))?;
    out.write_all(content).map_err(io_error("write-file"))?;
    out.sync_all().map_err(io_error("sync-file"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            &path,
            std::fs::Permissions::from_mode(file.mode.read_only_permissions()),
        )
        .map_err(io_error("permissions"))?;
    }
    Ok(())
}

fn staging_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        ".staging-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

impl<H: Sha256Digest, K: RepositoryKeyer> CachedContentSource for LibraryEntryStore<H, K> {
    fn recompute_content_hash(
        &mut self,
        repository: &RepositoryUrl,
        commit: &CommitId,
    ) -> Result<Option<ContentHashText>, CacheReadFailure> {
        let entry = self
            .lookup(repository, commit)
            .map_err(|e| CacheReadFailure {
                kind: match e {
                    MaterializeError::Index(CacheIndexError::Io { kind }) => kind,
                    _ => std::io::ErrorKind::InvalidData,
                },
            })?;
        let Some(entry) = entry else {
            return Ok(None);
        };
        match recompute_entry_hash(&self.entry_dir(&entry.content_hash), &self.hasher) {
            Ok((hash, _, _)) => Ok(Some(hash)),
            Err(EntryScanError::Io { kind }) => Err(CacheReadFailure { kind }),
            // 예상하지 않은 항목(symlink 등)이 생긴 entry는 손상입니다. 읽기 실패로 막아(fail closed)
            // 검사를 통과시키지 않습니다.
            Err(EntryScanError::UnexpectedEntry) => Err(CacheReadFailure {
                kind: std::io::ErrorKind::InvalidData,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::fixtures::{decl, library};
    use crate::declaration::LogicalName;
    use crate::graph::DependencyPath;
    use crate::lock::{LockDocument, LockEntry};
    use crate::lock_integrity::{verify_lock_against_cache, LockIntegrityError};
    use crate::mirror::MirrorCache;
    use crate::node::DeclNode;
    use crate::test_support::{FakeSha256, FnvKeyer, SystemGit, TempDir};

    /// library 저장소 fixture: Stellar project files + 제외 대상 파일.
    struct Library {
        path: PathBuf,
        url: RepositoryUrl,
    }

    fn commit_all(git: &mut SystemGit, dir: &Path, message: &str) -> CommitId {
        git.ok(dir, ["add", "-A"]);
        git.ok(dir, ["commit", "-q", "-m", message]);
        git.ok(dir, ["rev-parse", "HEAD"]).trim().parse().unwrap()
    }

    fn library_repo(git: &mut SystemGit, tmp: &Path) -> Library {
        let path = tmp.join("lib");
        std::fs::create_dir_all(path.join("design/objects")).unwrap();
        std::fs::create_dir_all(path.join("assets/sha256")).unwrap();
        git.ok(&path, ["init", "-q", "-b", "main"]);
        git.ok(&path, ["config", "user.name", "Fixture"]);
        git.ok(&path, ["config", "user.email", "fixture@example.com"]);
        git.ok(&path, ["config", "commit.gpgsign", "false"]);
        std::fs::write(path.join("stellar.yaml"), "schema_version: 1\n").unwrap();
        std::fs::write(
            path.join("stellar.lock.yaml"),
            "schema_version: 1\nresolved: []\n",
        )
        .unwrap();
        std::fs::write(path.join("design/current.stellarref"), "sha256-00\n").unwrap();
        // binary chunk(NUL·고 byte 포함).
        let chunk: Vec<u8> = (0..=255u8).chain([0, 0, 10, 13]).collect();
        std::fs::write(path.join("design/objects/chunk.starchunk"), &chunk).unwrap();
        std::fs::write(
            path.join("assets/sha256/icon.png"),
            [0x89, b'P', b'N', b'G'],
        )
        .unwrap();
        std::fs::write(path.join("README.md"), "not a project file\n").unwrap();
        std::fs::write(path.join(".gitignore"), ".stellar/\n").unwrap();
        let url = RepositoryUrl::new(path.to_str().unwrap()).unwrap();
        Library { path, url }
    }

    fn store(root: &LibraryCacheRoot) -> LibraryEntryStore<FakeSha256, FnvKeyer> {
        LibraryEntryStore::new(root.clone(), FakeSha256, FnvKeyer::new(7))
    }

    fn mirrors(tmp: &Path, root: &LibraryCacheRoot) -> MirrorCache<SystemGit, FnvKeyer> {
        MirrorCache::new(
            root.clone(),
            SystemGit::find(tmp).unwrap(),
            FnvKeyer::new(7),
        )
    }

    fn read_tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for item in std::fs::read_dir(&d).unwrap() {
                let p = item.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let rel = p.strip_prefix(dir).unwrap().to_str().unwrap().to_string();
                    out.push((rel, std::fs::read(&p).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    #[cfg(unix)]
    fn inode(path: &Path) -> u64 {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path).unwrap().ino()
    }

    #[test]
    fn same_commit_rerequest_reuses_entry_bytes() {
        let tmp = TempDir::new("entry-reuse");
        let Some(mut fixture) = SystemGit::find(tmp.path()) else {
            return;
        };
        let lib = library_repo(&mut fixture, tmp.path());
        std::fs::set_permissions(lib.path.join("design/objects/chunk.starchunk"), {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::Permissions::from_mode(0o755)
            }
            #[cfg(not(unix))]
            {
                std::fs::metadata(lib.path.join("stellar.yaml"))
                    .unwrap()
                    .permissions()
            }
        })
        .unwrap();
        let first = commit_all(&mut fixture, &lib.path, "v1");
        let root = LibraryCacheRoot::in_app_data(&tmp.path().join("app")).unwrap();

        let mut runner = SystemGit::find(tmp.path()).unwrap();
        let mut mirror = mirrors(tmp.path(), &root);
        let created = store(&root)
            .materialize(&mut runner, &mut mirror, &lib.url, &first, 1_000)
            .unwrap();
        assert_eq!(created.reuse, EntryReuse::Created);
        assert!(created.dir.starts_with(root.entries_dir()));
        // Stellar project files만, bytes 그대로, `.git`·README·.gitignore 없음.
        let tree = read_tree(&created.dir);
        let names: Vec<&str> = tree.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "assets/sha256/icon.png",
                "design/current.stellarref",
                "design/objects/chunk.starchunk",
                "stellar.lock.yaml",
                "stellar.yaml",
            ]
        );
        for (name, bytes) in &tree {
            assert_eq!(
                bytes,
                &std::fs::read(lib.path.join(name)).unwrap(),
                "{name}"
            );
        }
        assert_eq!(
            created.size_bytes,
            tree.iter().map(|(_, b)| b.len() as u64).sum::<u64>()
        );
        // 디스크 재계산 hash = materialize hash, 실행 bit 보존·읽기 전용.
        let (rehash, size, count) = recompute_entry_hash(&created.dir, &FakeSha256).unwrap();
        assert_eq!(
            (rehash, size, count),
            (created.content_hash.clone(), created.size_bytes, 5)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &str| {
                std::fs::metadata(created.dir.join(p))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777
            };
            assert_eq!(mode("stellar.yaml"), 0o444);
            assert_eq!(mode("design/objects/chunk.starchunk"), 0o555);
        }

        // 같은 commit 재요청: 새 store·새 실행기에서도 Git 실행 0회, 같은 디렉터리·같은 파일 재사용.
        #[cfg(unix)]
        let ino = inode(&created.dir.join("design/objects/chunk.starchunk"));
        let mut quiet = SystemGit::find(tmp.path()).unwrap();
        let mut quiet_mirror = mirrors(tmp.path(), &root);
        let again = store(&root)
            .materialize(&mut quiet, &mut quiet_mirror, &lib.url, &first, 2_000)
            .unwrap();
        assert_eq!(again.reuse, EntryReuse::CommitHit);
        assert_eq!(again.dir, created.dir);
        assert_eq!(again.content_hash, created.content_hash);
        assert!(quiet.calls.is_empty(), "{:?}", quiet.calls);
        assert!(quiet_mirror.runner().calls.is_empty());
        #[cfg(unix)]
        assert_eq!(
            inode(&again.dir.join("design/objects/chunk.starchunk")),
            ino
        );
        assert_eq!(read_tree(&again.dir), tree);
        // 최근 사용 시각 갱신, index에는 URL 원문 없음(privacy mode).
        let index_bytes = std::fs::read(root.index_path()).unwrap();
        assert!(!String::from_utf8_lossy(&index_bytes).contains(lib.path.to_str().unwrap()));
        let entry = store(&root).lookup(&lib.url, &first).unwrap().unwrap();
        assert_eq!(entry.last_used_unix_ms, 2_000);

        // 다른 commit이지만 project files가 같음(README만 변경) → 같은 entry 재사용.
        std::fs::write(lib.path.join("README.md"), "changed\n").unwrap();
        let second = commit_all(&mut fixture, &lib.path, "readme");
        let mut runner = SystemGit::find(tmp.path()).unwrap();
        let mut mirror = mirrors(tmp.path(), &root);
        let dedup = store(&root)
            .materialize(&mut runner, &mut mirror, &lib.url, &second, 3_000)
            .unwrap();
        assert_eq!(dedup.reuse, EntryReuse::ContentHit);
        assert_eq!(dedup.dir, created.dir);
        #[cfg(unix)]
        assert_eq!(
            inode(&dedup.dir.join("design/objects/chunk.starchunk")),
            ino
        );

        // project file 변경 → 다른 entry.
        std::fs::write(lib.path.join("stellar.yaml"), "schema_version: 2\n").unwrap();
        let third = commit_all(&mut fixture, &lib.path, "v2");
        // 새 commit은 새 fetch 세션에서 mirror로 들어옵니다(MirrorCache는 세션당 1회 fetch).
        let mut mirror = mirrors(tmp.path(), &root);
        let changed = store(&root)
            .materialize(&mut runner, &mut mirror, &lib.url, &third, 4_000)
            .unwrap();
        assert_eq!(changed.reuse, EntryReuse::Created);
        assert_ne!(changed.content_hash, created.content_hash);
        // staging 잔여물 없음, entry 2개.
        let names: Vec<String> = std::fs::read_dir(root.entries_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.iter().all(|n| n.starts_with(ENTRY_DIR_PREFIX)));
    }

    #[test]
    fn lock_integrity_detects_tampered_materialized_entry() {
        let tmp = TempDir::new("entry-tamper");
        let Some(mut fixture) = SystemGit::find(tmp.path()) else {
            return;
        };
        let lib = library_repo(&mut fixture, tmp.path());
        let commit = commit_all(&mut fixture, &lib.path, "v1");
        let root = LibraryCacheRoot::in_app_data(&tmp.path().join("app")).unwrap();
        let mut runner = SystemGit::find(tmp.path()).unwrap();
        let mut mirror = mirrors(tmp.path(), &root);
        let mut entries = store(&root);
        let entry = entries
            .materialize(&mut runner, &mut mirror, &lib.url, &commit, 1)
            .unwrap();

        // 실제 materialize hash로 만든 lock은 통과.
        let lock_entry = LockEntry {
            library_id: library(1),
            source: crate::declaration::SourceKind::Git,
            repository: lib.url.clone(),
            requested: decl("lib", 1, lib.url.reveal(), "v1").version,
            exact_commit: commit.clone(),
            content_hash: entry.content_hash.clone(),
            schema_version: 1,
            dependency_path: DependencyPath::from_names(vec![LogicalName::new("lib").unwrap()]),
            dependencies: Vec::new(),
        };
        let lock = LockDocument::from_node(&DeclNode::Mapping(vec![
            ("schema_version".to_string(), DeclNode::scalar("1")),
            (
                "resolved".to_string(),
                DeclNode::Sequence(vec![lock_entry.to_node()]),
            ),
        ]))
        .unwrap();
        assert_eq!(
            verify_lock_against_cache(&lock, &mut entries)
                .unwrap()
                .len(),
            1
        );

        // entry 파일 변조 → 재계산 hash 불일치 → Corrupted(다른 commit 선택 없음).
        let target = entry.dir.join("stellar.yaml");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        std::fs::write(&target, "schema_version: 9\n").unwrap();
        assert!(matches!(
            verify_lock_against_cache(&lock, &mut entries),
            Err(LockIntegrityError::Corrupted { .. })
        ));

        // entry 안에 끼어든 symlink는 따르지 않고 손상으로 막음(fail closed).
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/hosts", entry.dir.join("design/link")).unwrap();
            assert_eq!(
                recompute_entry_hash(&entry.dir, &FakeSha256),
                Err(EntryScanError::UnexpectedEntry)
            );
            assert!(matches!(
                verify_lock_against_cache(&lock, &mut entries),
                Err(LockIntegrityError::ReadFailed {
                    failure: CacheReadFailure {
                        kind: std::io::ErrorKind::InvalidData
                    },
                    ..
                })
            ));
        }
    }

    #[test]
    fn unsupported_trees_are_rejected_before_publishing() {
        let tmp = TempDir::new("entry-reject");
        let Some(mut fixture) = SystemGit::find(tmp.path()) else {
            return;
        };
        let root = LibraryCacheRoot::in_app_data(&tmp.path().join("app")).unwrap();
        let attempt = |fixture: &mut SystemGit, setup: &dyn Fn(&mut SystemGit, &Path)| {
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = tmp
                .path()
                .join(format!("repo-{}", N.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&dir).unwrap();
            fixture.ok(&dir, ["init", "-q", "-b", "main"]);
            fixture.ok(&dir, ["config", "user.name", "Fixture"]);
            fixture.ok(&dir, ["config", "user.email", "fixture@example.com"]);
            fixture.ok(&dir, ["config", "commit.gpgsign", "false"]);
            setup(fixture, &dir);
            // `update-index`로 넣은 항목(작업 트리에 없음)을 지우지 않도록 삭제는 stage하지 않음.
            fixture.ok(&dir, ["add", "--ignore-removal", "."]);
            fixture.ok(&dir, ["commit", "-q", "-m", "c"]);
            let commit: CommitId = fixture
                .ok(&dir, ["rev-parse", "HEAD"])
                .trim()
                .parse()
                .unwrap();
            let url = RepositoryUrl::new(dir.to_str().unwrap()).unwrap();
            let mut runner = SystemGit::find(tmp.path()).unwrap();
            let mut mirror = mirrors(tmp.path(), &root);
            store(&root).materialize(&mut runner, &mut mirror, &url, &commit, 1)
        };

        // 일반 Git 저장소(stellar.yaml 없음).
        let plain = attempt(&mut fixture, &|_, dir| {
            std::fs::write(dir.join("README.md"), "x").unwrap();
        });
        assert_eq!(plain.unwrap_err(), MaterializeError::NotAStellarProject);

        // design 아래 symlink.
        #[cfg(unix)]
        {
            let symlink = attempt(&mut fixture, &|_, dir| {
                std::fs::write(dir.join("stellar.yaml"), "schema_version: 1\n").unwrap();
                std::fs::create_dir_all(dir.join("design")).unwrap();
                std::os::unix::fs::symlink("/etc/hosts", dir.join("design/link")).unwrap();
            });
            assert_eq!(
                symlink.unwrap_err(),
                MaterializeError::UnsupportedEntry {
                    kind: UnsupportedEntryKind::Symlink
                }
            );
        }

        // design 아래 submodule(gitlink).
        let submodule = attempt(&mut fixture, &|git, dir| {
            std::fs::write(dir.join("stellar.yaml"), "schema_version: 1\n").unwrap();
            let oid = "1".repeat(40);
            git.ok(
                dir,
                [
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("160000,{oid},design/sub"),
                ],
            );
        });
        assert_eq!(
            submodule.unwrap_err(),
            MaterializeError::UnsupportedEntry {
                kind: UnsupportedEntryKind::Submodule
            }
        );

        // 제어 문자 경로, 대소문자 충돌.
        let control = attempt(&mut fixture, &|_, dir| {
            std::fs::write(dir.join("stellar.yaml"), "schema_version: 1\n").unwrap();
            std::fs::create_dir_all(dir.join("design")).unwrap();
            std::fs::write(dir.join("design/a\nb"), "x").unwrap();
        });
        assert_eq!(
            control.unwrap_err(),
            MaterializeError::UnsupportedPath {
                reason: UnsupportedPathReason::BadCharacter
            }
        );
        let collision = attempt(&mut fixture, &|git, dir| {
            std::fs::write(dir.join("stellar.yaml"), "schema_version: 1\n").unwrap();
            let blob = git
                .ok(dir, ["hash-object", "-w", "stellar.yaml"])
                .trim()
                .to_string();
            for path in ["design/Chunk", "design/chunk"] {
                git.ok(
                    dir,
                    [
                        "update-index",
                        "--add",
                        "--cacheinfo",
                        &format!("100644,{blob},{path}"),
                    ],
                );
            }
        });
        assert_eq!(collision.unwrap_err(), MaterializeError::PathCollision);

        // 공개된 entry·staging 잔여물 없음.
        let leftovers: Vec<_> = std::fs::read_dir(root.entries_dir())
            .map(|d| d.map(|e| e.unwrap().file_name()).collect())
            .unwrap_or_default();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn limits_and_unknown_commit_are_rejected() {
        let tmp = TempDir::new("entry-limits");
        let Some(mut fixture) = SystemGit::find(tmp.path()) else {
            return;
        };
        let lib = library_repo(&mut fixture, tmp.path());
        let commit = commit_all(&mut fixture, &lib.path, "v1");
        let root = LibraryCacheRoot::in_app_data(&tmp.path().join("app")).unwrap();
        let mut runner = SystemGit::find(tmp.path()).unwrap();
        let mut mirror = mirrors(tmp.path(), &root);
        let limited = |files, bytes| {
            store(&root).with_limits(MaterializeLimits {
                max_files: files,
                max_total_bytes: bytes,
                max_batch_bytes: 1,
            })
        };
        assert_eq!(
            limited(4, u64::MAX)
                .materialize(&mut runner, &mut mirror, &lib.url, &commit, 1)
                .unwrap_err(),
            MaterializeError::TooManyFiles { max: 4 }
        );
        assert_eq!(
            limited(10, 100)
                .materialize(&mut runner, &mut mirror, &lib.url, &commit, 1)
                .unwrap_err(),
            MaterializeError::TooLarge { max_bytes: 100 }
        );
        // batch 상한 1 byte → 파일마다 cat-file 1회(5회)로도 같은 결과.
        let before = runner.count("cat-file");
        let entry = limited(10, u64::MAX)
            .materialize(&mut runner, &mut mirror, &lib.url, &commit, 1)
            .unwrap();
        assert_eq!(runner.count("cat-file") - before, 1 + 5);
        assert_eq!(entry.reuse, EntryReuse::Created);
        let missing: CommitId = "f".repeat(40).parse().unwrap();
        assert_eq!(
            store(&root)
                .materialize(&mut runner, &mut mirror, &lib.url, &missing, 1)
                .unwrap_err(),
            MaterializeError::CommitNotFound
        );
    }

    fn tree_file(oid: &str, size: u64) -> TreeFile {
        TreeFile {
            path: "stellar.yaml".to_string(),
            mode: EntryFileMode::Regular,
            oid: oid.to_string(),
            size,
        }
    }

    #[test]
    fn batch_and_tree_parsers_reject_malformed_output() {
        let oid = "a".repeat(40);
        let file = tree_file(&oid, 3);
        let ok = format!("{oid} blob 3\nabc\n");
        assert_eq!(parse_batch(ok.as_bytes(), &[&file]).unwrap(), [b"abc"]);
        for bad in [
            format!("{oid} missing\n"),
            format!("{} blob 3\nabc\n", "b".repeat(40)),
            format!("{oid} tree 3\nabc\n"),
            format!("{oid} blob 4\nabcd\n"),
        ] {
            assert_eq!(
                parse_batch(bad.as_bytes(), &[&file]),
                Err(MaterializeError::BlobMismatch),
                "{bad:?}"
            );
        }
        // 크기 overflow·잘림·끝 LF 없음·남는 bytes는 panic 없이 해석 오류.
        let huge = tree_file(&oid, u64::MAX);
        let overflow = format!("{oid} blob {}\nabc\n", u64::MAX);
        assert!(parse_batch(overflow.as_bytes(), &[&huge]).is_err());
        for bad in [
            format!("{oid} blob 3\nab"),
            format!("{oid} blob 3\nabcX"),
            format!("{oid} blob 3\nabc\nextra"),
            format!("{oid} blob x\nabc\n"),
            format!("{oid} blob 3"),
        ] {
            assert_eq!(
                parse_batch(bad.as_bytes(), &[&file]),
                Err(MaterializeError::MalformedOutput {
                    operation: "cat-file"
                }),
                "{bad:?}"
            );
        }

        let record = |meta: &str, path: &str| format!("{meta}\t{path}\0");
        let listing = [
            record(&format!("100644 blob {oid}     3"), "stellar.yaml"),
            record(&format!("100755 blob {oid}     3"), "design/run"),
            record(&format!("120000 blob {oid}     3"), "README.md"),
            record(&format!("160000 commit {oid}       -"), "vendor/sub"),
        ]
        .concat();
        let files = parse_tree(listing.as_bytes()).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].mode, EntryFileMode::Executable);
        for (bad, error) in [
            (
                record(&format!("100644 blob {oid} 3"), "design/.GIT/x"),
                MaterializeError::UnsupportedPath {
                    reason: UnsupportedPathReason::DotGit,
                },
            ),
            (
                record(&format!("100644 blob {oid} 3"), "design//x"),
                MaterializeError::UnsupportedPath {
                    reason: UnsupportedPathReason::BadComponent,
                },
            ),
            (
                record(&format!("100644 blob {oid} 3"), "design/a\\b"),
                MaterializeError::UnsupportedPath {
                    reason: UnsupportedPathReason::BadCharacter,
                },
            ),
            (
                record(&format!("100644 blob {oid} x"), "design/x"),
                MaterializeError::MalformedOutput {
                    operation: "ls-tree",
                },
            ),
            (
                format!("100644 blob {oid} 3 design/x\0"),
                MaterializeError::MalformedOutput {
                    operation: "ls-tree",
                },
            ),
            (
                [
                    record(&format!("100644 blob {oid} 3"), "design/x"),
                    record(&format!("100644 blob {oid} 3"), "design/X/y"),
                ]
                .concat(),
                MaterializeError::PathCollision,
            ),
        ] {
            assert_eq!(parse_tree(bad.as_bytes()), Err(error), "{bad:?}");
        }
        // non-UTF-8 경로.
        let mut non_utf8 = format!("100644 blob {oid} 3\tdesign/").into_bytes();
        non_utf8.extend_from_slice(&[0xff, 0]);
        assert_eq!(
            parse_tree(&non_utf8),
            Err(MaterializeError::UnsupportedPath {
                reason: UnsupportedPathReason::NotUtf8
            })
        );
    }

    #[test]
    fn listing_is_canonical_and_order_independent() {
        let rec = |path: &str, mode| EntryFileRecord {
            path: path.to_string(),
            mode,
            size: 2,
            sha256: [0xab; 32],
        };
        let a = [
            rec("stellar.yaml", EntryFileMode::Regular),
            rec("design/x", EntryFileMode::Executable),
        ];
        let b = [a[1].clone(), a[0].clone()];
        assert_eq!(entry_listing(&a), entry_listing(&b));
        let text = String::from_utf8(entry_listing(&a)).unwrap();
        assert_eq!(
            text,
            format!(
                "stellar-library-entry 1\n100755 2 {h} design/x\n100644 2 {h} stellar.yaml\n",
                h = "ab".repeat(32)
            )
        );
        assert_eq!(
            entry_content_hash(&a, &FakeSha256),
            entry_content_hash(&b, &FakeSha256)
        );
    }
}

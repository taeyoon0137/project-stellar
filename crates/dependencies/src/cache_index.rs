//! library cache index (XV.3.1).
//!
//! cache root의 `index.stlcache`는 materialize된 cache entry 목록입니다. entry는 (repository, exact
//! commit)으로 식별하고 content hash·크기·최근 사용 시각을 기록합니다. 형식은 canonical LE binary
//! 입니다(같은 index는 항상 같은 bytes).
//!
//! ```text
//! magic        8  b"STLDCIDX"
//! version      u16 LE (= 1)
//! mode         u8      (0 = plain: repository URL 원문, 1 = private: keyed identifier — XV.3.5)
//! entry_count  u32 LE
//! entry*       (repository 비교 키, commit) 오름차순, 중복 금지
//!   repository   u32 LE 길이 + UTF-8
//!   commit       u8 길이 + ASCII hex
//!   content_hash u8 길이 + ASCII
//!   size_bytes   u64 LE
//!   last_used_ms u64 LE (Unix epoch ms)
//! ```
//!
//! 쓰기는 같은 디렉터리의 임시 파일 → fsync → rename으로 원자적입니다.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::content_hash::ContentHashText;
use crate::declaration::{CommitId, RepositoryUrl};

/// 파일 magic.
pub const CACHE_INDEX_MAGIC: &[u8; 8] = b"STLDCIDX";
/// 현재 형식 버전.
pub const CACHE_INDEX_VERSION: u16 = 1;
/// plain mode(repository URL 원문 저장).
pub const MODE_PLAIN: u8 = 0;

/// cache index entry 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheIndexEntry {
    /// repository.
    pub repository: RepositoryUrl,
    /// exact commit.
    pub commit: CommitId,
    /// materialize된 entry content hash.
    pub content_hash: ContentHashText,
    /// entry 크기(byte).
    pub size_bytes: u64,
    /// 최근 사용 시각(Unix epoch ms).
    pub last_used_unix_ms: u64,
}

/// cache index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheIndex {
    entries: BTreeMap<(String, CommitId), CacheIndexEntry>,
}

/// cache index 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheIndexError {
    /// magic 불일치.
    BadMagic,
    /// 지원하지 않는 형식 버전.
    UnsupportedVersion {
        /// 파일의 버전.
        version: u16,
    },
    /// 지원하지 않는 mode.
    UnsupportedMode {
        /// 파일의 mode.
        mode: u8,
    },
    /// 데이터가 중간에 끝남.
    Truncated,
    /// 마지막 entry 뒤에 남는 bytes.
    TrailingBytes,
    /// 필드 값이 규칙을 위반함.
    InvalidField {
        /// 필드 이름.
        field: &'static str,
    },
    /// entry가 canonical 순서가 아니거나 중복됨.
    NotCanonicalOrder,
    /// 길이가 형식 한도를 넘음.
    TooLong {
        /// 필드 이름.
        field: &'static str,
    },
    /// 파일 I/O 실패.
    Io {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
}

impl fmt::Display for CacheIndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheIndexError::BadMagic => f.write_str("cache index magic이 다릅니다"),
            CacheIndexError::UnsupportedVersion { version } => {
                write!(f, "지원하지 않는 cache index 버전: {version}")
            }
            CacheIndexError::UnsupportedMode { mode } => {
                write!(f, "지원하지 않는 cache index mode: {mode}")
            }
            CacheIndexError::Truncated => f.write_str("cache index가 중간에 끝납니다"),
            CacheIndexError::TrailingBytes => {
                f.write_str("cache index 끝에 남는 데이터가 있습니다")
            }
            CacheIndexError::InvalidField { field } => {
                write!(f, "잘못된 cache index 필드: {field}")
            }
            CacheIndexError::NotCanonicalOrder => {
                f.write_str("cache index entry 순서가 canonical이 아닙니다")
            }
            CacheIndexError::TooLong { field } => {
                write!(f, "cache index 필드가 너무 깁니다: {field}")
            }
            CacheIndexError::Io { kind } => write!(f, "cache index I/O 실패: {kind:?}"),
        }
    }
}

impl std::error::Error for CacheIndexError {}

impl CacheIndex {
    /// 빈 index.
    pub fn new() -> CacheIndex {
        CacheIndex::default()
    }

    /// entry를 추가하거나 교체합니다.
    pub fn upsert(&mut self, entry: CacheIndexEntry) {
        let key = (entry.repository.comparison_key(), entry.commit.clone());
        self.entries.insert(key, entry);
    }

    /// (repository, commit) entry 조회.
    pub fn get(&self, repository: &RepositoryUrl, commit: &CommitId) -> Option<&CacheIndexEntry> {
        self.entries
            .get(&(repository.comparison_key(), commit.clone()))
    }

    /// 최근 사용 시각을 갱신합니다. entry가 없으면 `false`.
    pub fn touch(
        &mut self,
        repository: &RepositoryUrl,
        commit: &CommitId,
        now_unix_ms: u64,
    ) -> bool {
        match self
            .entries
            .get_mut(&(repository.comparison_key(), commit.clone()))
        {
            Some(entry) => {
                entry.last_used_unix_ms = entry.last_used_unix_ms.max(now_unix_ms);
                true
            }
            None => false,
        }
    }

    /// 모든 entry(canonical 순).
    pub fn entries(&self) -> impl Iterator<Item = &CacheIndexEntry> {
        self.entries.values()
    }

    /// entry 수.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// canonical bytes로 인코딩합니다.
    pub fn encode(&self) -> Result<Vec<u8>, CacheIndexError> {
        let mut out = Vec::new();
        out.extend_from_slice(CACHE_INDEX_MAGIC);
        out.extend_from_slice(&CACHE_INDEX_VERSION.to_le_bytes());
        out.push(MODE_PLAIN);
        let count = u32::try_from(self.entries.len()).map_err(|_| CacheIndexError::TooLong {
            field: "entry_count",
        })?;
        out.extend_from_slice(&count.to_le_bytes());
        for entry in self.entries.values() {
            put_u32_bytes(&mut out, entry.repository.reveal().as_bytes(), "repository")?;
            put_u8_bytes(&mut out, entry.commit.as_str().as_bytes(), "commit")?;
            put_u8_bytes(
                &mut out,
                entry.content_hash.as_str().as_bytes(),
                "content_hash",
            )?;
            out.extend_from_slice(&entry.size_bytes.to_le_bytes());
            out.extend_from_slice(&entry.last_used_unix_ms.to_le_bytes());
        }
        Ok(out)
    }

    /// bytes를 엄격하게 디코딩합니다(canonical 순서·필드 규칙·남는 bytes 검사).
    pub fn decode(bytes: &[u8]) -> Result<CacheIndex, CacheIndexError> {
        let mut r = Reader { bytes, pos: 0 };
        if r.take(8)? != CACHE_INDEX_MAGIC {
            return Err(CacheIndexError::BadMagic);
        }
        let version = r.u16()?;
        if version != CACHE_INDEX_VERSION {
            return Err(CacheIndexError::UnsupportedVersion { version });
        }
        let mode = r.u8()?;
        if mode != MODE_PLAIN {
            return Err(CacheIndexError::UnsupportedMode { mode });
        }
        let count = r.u32()?;
        let mut index = CacheIndex::new();
        let mut previous: Option<(String, CommitId)> = None;
        for _ in 0..count {
            let len = r.u32()? as usize;
            let repository = std::str::from_utf8(r.take(len)?)
                .ok()
                .and_then(|s| RepositoryUrl::new(s).ok())
                .ok_or(CacheIndexError::InvalidField {
                    field: "repository",
                })?;
            let commit = r
                .short_text("commit")?
                .parse::<CommitId>()
                .map_err(|_| CacheIndexError::InvalidField { field: "commit" })?;
            let content_hash = r
                .short_text("content_hash")?
                .parse::<ContentHashText>()
                .map_err(|_| CacheIndexError::InvalidField {
                    field: "content_hash",
                })?;
            let size_bytes = r.u64()?;
            let last_used_unix_ms = r.u64()?;
            let key = (repository.comparison_key(), commit.clone());
            if previous.as_ref().is_some_and(|p| p >= &key) {
                return Err(CacheIndexError::NotCanonicalOrder);
            }
            previous = Some(key.clone());
            index.entries.insert(
                key,
                CacheIndexEntry {
                    repository,
                    commit,
                    content_hash,
                    size_bytes,
                    last_used_unix_ms,
                },
            );
        }
        if r.pos != bytes.len() {
            return Err(CacheIndexError::TrailingBytes);
        }
        Ok(index)
    }

    /// 파일에서 읽습니다. 파일이 없으면 빈 index입니다.
    pub fn read_from(path: &Path) -> Result<CacheIndex, CacheIndexError> {
        match std::fs::read(path) {
            Ok(bytes) => CacheIndex::decode(&bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CacheIndex::new()),
            Err(e) => Err(CacheIndexError::Io { kind: e.kind() }),
        }
    }

    /// 파일에 원자적으로 씁니다(임시 파일 → fsync → rename).
    pub fn write_to(&self, path: &Path) -> Result<(), CacheIndexError> {
        write_atomic(path, &self.encode()?)
    }
}

/// 같은 디렉터리 임시 파일에 쓰고 fsync한 뒤 rename합니다.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), CacheIndexError> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let io = |e: std::io::Error| CacheIndexError::Io { kind: e.kind() };
    let parent = path.parent().ok_or(CacheIndexError::Io {
        kind: std::io::ErrorKind::InvalidInput,
    })?;
    std::fs::create_dir_all(parent).map_err(io)?;
    let file_name = path.file_name().ok_or(CacheIndexError::Io {
        kind: std::io::ErrorKind::InvalidInput,
    })?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = parent.join(tmp_name);
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(io(e));
    }
    // rename 자체가 crash 뒤에도 남도록 부모 디렉터리 항목을 fsync합니다.
    sync_dir(parent).map_err(io)
}

/// 디렉터리 항목(생성·rename·삭제)을 디스크에 반영합니다.
///
/// Unix에서는 디렉터리를 열어 `fsync`합니다. Windows는 디렉터리 handle fsync를 같은 방식으로
/// 지원하지 않으며 지원 보류 상태(ADR-0006)라 아무것도 하지 않습니다.
pub(crate) fn sync_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

pub(crate) fn put_u32_bytes(
    out: &mut Vec<u8>,
    bytes: &[u8],
    field: &'static str,
) -> Result<(), CacheIndexError> {
    let len = u32::try_from(bytes.len()).map_err(|_| CacheIndexError::TooLong { field })?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

pub(crate) fn put_u8_bytes(
    out: &mut Vec<u8>,
    bytes: &[u8],
    field: &'static str,
) -> Result<(), CacheIndexError> {
    let len = u8::try_from(bytes.len()).map_err(|_| CacheIndexError::TooLong { field })?;
    out.push(len);
    out.extend_from_slice(bytes);
    Ok(())
}

/// 경계 검사 reader.
pub(crate) struct Reader<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], CacheIndexError> {
        let end = self.pos.checked_add(n).ok_or(CacheIndexError::Truncated)?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(CacheIndexError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, CacheIndexError> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16, CacheIndexError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, CacheIndexError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub(crate) fn u64(&mut self) -> Result<u64, CacheIndexError> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    pub(crate) fn short_text(&mut self, field: &'static str) -> Result<&'a str, CacheIndexError> {
        let len = self.u8()? as usize;
        std::str::from_utf8(self.take(len)?).map_err(|_| CacheIndexError::InvalidField { field })
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! 테스트 공용 index entry.

    use super::*;
    use crate::declaration::fixtures::commit;

    pub(crate) fn hash(n: u8) -> ContentHashText {
        ContentHashText::from_sha256_digest([n; 32])
    }

    pub(crate) fn entry(url: &str, commit_no: u8, size: u64, used: u64) -> CacheIndexEntry {
        CacheIndexEntry {
            repository: RepositoryUrl::new(url).unwrap(),
            commit: commit(commit_no),
            content_hash: hash(commit_no),
            size_bytes: size,
            last_used_unix_ms: used,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{entry, hash};
    use super::*;
    use crate::cache_root::LibraryCacheRoot;
    use crate::declaration::fixtures::commit;
    use crate::test_support::{SystemGit, TempDir};

    fn sample() -> CacheIndex {
        let mut index = CacheIndex::new();
        index.upsert(entry("https://e.com/b.git", 0x02, 2048, 1_700_000_000_002));
        index.upsert(entry("https://e.com/a.git", 0x01, 1024, 1_700_000_000_001));
        index
    }

    #[test]
    fn encode_decode_round_trip_is_canonical() {
        let index = sample();
        let bytes = index.encode().unwrap();
        assert_eq!(&bytes[..8], CACHE_INDEX_MAGIC);
        let decoded = CacheIndex::decode(&bytes).unwrap();
        assert_eq!(decoded, index);
        assert_eq!(decoded.encode().unwrap(), bytes);
        // 삽입 순서와 무관한 bytes.
        let mut other = CacheIndex::new();
        let mut reversed: Vec<CacheIndexEntry> = index.entries().cloned().collect();
        reversed.reverse();
        for e in reversed {
            other.upsert(e);
        }
        assert_eq!(other.encode().unwrap(), bytes);
        // header golden: magic + version 1 + mode 0 + count 2.
        assert_eq!(&bytes[8..15], &[1, 0, 0, 2, 0, 0, 0]);
    }

    #[test]
    fn decode_rejects_malformed_bytes() {
        let bytes = sample().encode().unwrap();
        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'X';
        assert_eq!(
            CacheIndex::decode(&bad_magic),
            Err(CacheIndexError::BadMagic)
        );
        let mut bad_version = bytes.clone();
        bad_version[8] = 9;
        assert_eq!(
            CacheIndex::decode(&bad_version),
            Err(CacheIndexError::UnsupportedVersion { version: 9 })
        );
        let mut bad_mode = bytes.clone();
        bad_mode[10] = 7;
        assert_eq!(
            CacheIndex::decode(&bad_mode),
            Err(CacheIndexError::UnsupportedMode { mode: 7 })
        );
        assert_eq!(
            CacheIndex::decode(&bytes[..bytes.len() - 1]),
            Err(CacheIndexError::Truncated)
        );
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            CacheIndex::decode(&trailing),
            Err(CacheIndexError::TrailingBytes)
        );
        // 순서를 바꾼 entry → canonical 아님.
        let mut a = CacheIndex::new();
        a.upsert(entry("https://e.com/a.git", 0x01, 1, 1));
        let mut b = CacheIndex::new();
        b.upsert(entry("https://e.com/b.git", 0x02, 2, 2));
        let a_bytes = a.encode().unwrap();
        let b_bytes = b.encode().unwrap();
        let mut swapped = a_bytes[..11].to_vec();
        swapped.extend_from_slice(&2u32.to_le_bytes());
        swapped.extend_from_slice(&b_bytes[15..]);
        swapped.extend_from_slice(&a_bytes[15..]);
        assert_eq!(
            CacheIndex::decode(&swapped),
            Err(CacheIndexError::NotCanonicalOrder)
        );
        // 잘못된 content hash 필드.
        let mut bad_hash = a_bytes.clone();
        let pos = bad_hash.len() - 16 - 1;
        bad_hash[pos] = b'Z';
        assert_eq!(
            CacheIndex::decode(&bad_hash),
            Err(CacheIndexError::InvalidField {
                field: "content_hash"
            })
        );
    }

    #[test]
    fn touch_and_lookup() {
        let mut index = sample();
        let url = RepositoryUrl::new("https://e.com/a").unwrap();
        // 비교 키로 조회(.git 유무 무관).
        assert_eq!(
            index.get(&url, &commit(0x01)).unwrap().content_hash,
            hash(0x01)
        );
        assert!(index.touch(&url, &commit(0x01), 1_800_000_000_000));
        assert_eq!(
            index.get(&url, &commit(0x01)).unwrap().last_used_unix_ms,
            1_800_000_000_000
        );
        // 과거 시각으로는 되돌리지 않음.
        assert!(index.touch(&url, &commit(0x01), 1));
        assert_eq!(
            index.get(&url, &commit(0x01)).unwrap().last_used_unix_ms,
            1_800_000_000_000
        );
        assert!(!index.touch(&url, &commit(0x09), 1));
    }

    fn list_files(root: &Path) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                out.push(path.strip_prefix(root).unwrap().display().to_string());
                if path.is_dir() {
                    stack.push(path);
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn cache_lives_in_app_data_and_project_git_status_is_unchanged() {
        let project = TempDir::new("cache-project");
        let app_data = TempDir::new("cache-appdata");
        let mut git = SystemGit::find(project.path());
        if let Some(git) = git.as_mut() {
            git.ok(project.path(), ["init", "-q", "-b", "main"]);
            git.ok(project.path(), ["config", "user.name", "Fixture"]);
            git.ok(
                project.path(),
                ["config", "user.email", "fixture@example.com"],
            );
            std::fs::write(project.path().join("stellar.yaml"), "schema_version: 1\n").unwrap();
            git.ok(project.path(), ["add", "stellar.yaml"]);
            git.ok(project.path(), ["commit", "-q", "-m", "init"]);
        } else {
            std::fs::write(project.path().join("stellar.yaml"), "schema_version: 1\n").unwrap();
        }
        let status = |git: &mut Option<SystemGit>| {
            git.as_mut().map(|g| {
                g.ok(
                    project.path(),
                    [
                        "status",
                        "--porcelain=v1",
                        "--untracked-files=all",
                        "--ignored",
                    ],
                )
            })
        };
        let status_before = status(&mut git);
        let files_before = list_files(project.path());

        let cache = LibraryCacheRoot::in_app_data(app_data.path()).unwrap();
        cache.ensure_outside_project(project.path()).unwrap();
        cache.create_dirs().unwrap();
        let index = sample();
        index.write_to(&cache.index_path()).unwrap();
        assert_eq!(CacheIndex::read_from(&cache.index_path()).unwrap(), index);
        assert!(cache.index_path().starts_with(app_data.path()));

        // 프로젝트 폴더·Git status 불변.
        assert_eq!(list_files(project.path()), files_before);
        let status_after = status(&mut git);
        assert_eq!(status_after, status_before);
        if let Some(text) = status_after {
            assert_eq!(text, "", "프로젝트 Git status가 깨끗해야 합니다");
        }
        // 임시 파일이 남지 않음.
        let leftovers: Vec<String> = list_files(cache.path())
            .into_iter()
            .filter(|p| p.contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        // 없는 파일은 빈 index.
        assert!(CacheIndex::read_from(&app_data.path().join("none"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn atomic_write_syncs_parent_directory() {
        // 리뷰 P7 회귀: rename 뒤 부모 디렉터리를 fsync합니다. 디렉터리를 열 수 없으면(없음)
        // 성공으로 보고하지 않습니다.
        let dir = TempDir::new("sync-dir");
        assert!(sync_dir(dir.path()).is_ok());
        assert!(sync_dir(&dir.path().join("missing")).is_err());
        let path = dir.path().join("nested/index.stlcache");
        write_atomic(&path, b"bytes").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"bytes");
        let leftovers: Vec<String> = list_files(dir.path())
            .into_iter()
            .filter(|p| p.contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn cache_root_inside_project_is_rejected() {
        let project = TempDir::new("cache-overlap");
        let inside = LibraryCacheRoot::in_app_data(&project.path().join(".stellar/app")).unwrap();
        assert_eq!(
            inside.ensure_outside_project(project.path()),
            Err(crate::cache_root::CacheRootError::InsideProject)
        );
        let nested_project = project.path().join("library-cache/sub");
        std::fs::create_dir_all(&nested_project).unwrap();
        let outer = LibraryCacheRoot::in_app_data(project.path()).unwrap();
        assert_eq!(
            outer.ensure_outside_project(&nested_project),
            Err(crate::cache_root::CacheRootError::ProjectInsideCache)
        );
        assert_eq!(
            LibraryCacheRoot::in_app_data(Path::new("relative/app")),
            Err(crate::cache_root::CacheRootError::NotNormalizedAbsolute)
        );
        assert_eq!(
            LibraryCacheRoot::in_app_data(&project.path().join("../x")),
            Err(crate::cache_root::CacheRootError::NotNormalizedAbsolute)
        );
    }
}

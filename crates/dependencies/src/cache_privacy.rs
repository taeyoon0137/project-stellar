//! cache index privacy mode와 local source config (XV.3.5).
//!
//! privacy mode index(mode 1)는 repository URL 원문 대신 [`RepositoryKey`](keyed identifier)와
//! commit·content hash·크기·최근 사용 시각만 저장합니다. 원본 URL은 별도 **local source config**
//! (`sources.stlsecure`, 소유자 전용 권한)에만 둡니다. 진단 export([`PrivateCacheIndex::export_diagnostic`])는
//! privacy index만 사용하므로 URL을 포함할 수 없습니다(아키텍처 39장 "Git URL 수집 금지").
//!
//! ```text
//! privacy index: magic "STLDCIDX" | version u16=1 | mode u8=1 | count u32 |
//!                entry* (key, commit) 오름차순 = key(u8 길이+hex) commit(u8+hex)
//!                content_hash(u8+ASCII) size u64 last_used_ms u64
//! source config: magic "STLDSRCS" | version u16=1 | count u32 |
//!                entry* key 오름차순 = key(u8 길이+hex) url(u32 길이+UTF-8)
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::cache_index::{
    put_u32_bytes, put_u8_bytes, CacheIndex, CacheIndexError, Reader, CACHE_INDEX_MAGIC,
    CACHE_INDEX_VERSION,
};
use crate::content_hash::ContentHashText;
use crate::declaration::{CommitId, RepositoryUrl};
use crate::mirror::{RepositoryKey, RepositoryKeyer};

/// privacy mode 값.
pub const MODE_PRIVATE: u8 = 1;
/// local source config magic.
pub const SOURCE_CONFIG_MAGIC: &[u8; 8] = b"STLDSRCS";
/// local source config 형식 버전.
pub const SOURCE_CONFIG_VERSION: u16 = 1;
/// local source config 파일 이름(cache root 아래, index와 분리).
pub const SOURCE_CONFIG_FILE: &str = "sources.stlsecure";

/// privacy index entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateIndexEntry {
    /// repository keyed identifier.
    pub repository_key: RepositoryKey,
    /// exact commit.
    pub commit: CommitId,
    /// content hash.
    pub content_hash: ContentHashText,
    /// 크기(byte).
    pub size_bytes: u64,
    /// 최근 사용 시각(Unix epoch ms).
    pub last_used_unix_ms: u64,
}

/// URL 원문이 없는 privacy mode cache index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivateCacheIndex {
    entries: BTreeMap<(RepositoryKey, CommitId), PrivateIndexEntry>,
}

/// keyed identifier → 원본 repository URL(로컬 UI 표시·fetch 전용).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalSourceConfig {
    sources: BTreeMap<RepositoryKey, RepositoryUrl>,
}

impl CacheIndex {
    /// plain index를 privacy index + local source config로 분리합니다.
    pub fn split_private<K: RepositoryKeyer>(
        &self,
        keyer: &K,
    ) -> (PrivateCacheIndex, LocalSourceConfig) {
        let mut private = PrivateCacheIndex::default();
        let mut sources = LocalSourceConfig::default();
        for entry in self.entries() {
            let key = keyer.repository_key(&entry.repository);
            sources
                .sources
                .insert(key.clone(), entry.repository.clone());
            private.upsert(PrivateIndexEntry {
                repository_key: key,
                commit: entry.commit.clone(),
                content_hash: entry.content_hash.clone(),
                size_bytes: entry.size_bytes,
                last_used_unix_ms: entry.last_used_unix_ms,
            });
        }
        (private, sources)
    }
}

impl PrivateCacheIndex {
    /// entry를 추가하거나 교체합니다.
    pub fn upsert(&mut self, entry: PrivateIndexEntry) {
        self.entries
            .insert((entry.repository_key.clone(), entry.commit.clone()), entry);
    }

    /// repository URL로 조회합니다(keyer로 key 계산, URL은 저장하지 않음).
    pub fn get<K: RepositoryKeyer>(
        &self,
        keyer: &K,
        repository: &RepositoryUrl,
        commit: &CommitId,
    ) -> Option<&PrivateIndexEntry> {
        self.entries
            .get(&(keyer.repository_key(repository), commit.clone()))
    }

    /// 모든 entry(canonical 순).
    pub fn entries(&self) -> impl Iterator<Item = &PrivateIndexEntry> {
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

    /// canonical bytes(mode 1).
    pub fn encode(&self) -> Result<Vec<u8>, CacheIndexError> {
        let mut out = Vec::new();
        out.extend_from_slice(CACHE_INDEX_MAGIC);
        out.extend_from_slice(&CACHE_INDEX_VERSION.to_le_bytes());
        out.push(MODE_PRIVATE);
        let count = u32::try_from(self.entries.len()).map_err(|_| CacheIndexError::TooLong {
            field: "entry_count",
        })?;
        out.extend_from_slice(&count.to_le_bytes());
        for entry in self.entries.values() {
            put_u8_bytes(
                &mut out,
                entry.repository_key.as_str().as_bytes(),
                "repository_key",
            )?;
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

    /// 엄격 디코딩(mode 1만 허용).
    pub fn decode(bytes: &[u8]) -> Result<PrivateCacheIndex, CacheIndexError> {
        let mut r = Reader { bytes, pos: 0 };
        if r.take(8)? != CACHE_INDEX_MAGIC {
            return Err(CacheIndexError::BadMagic);
        }
        let version = r.u16()?;
        if version != CACHE_INDEX_VERSION {
            return Err(CacheIndexError::UnsupportedVersion { version });
        }
        let mode = r.u8()?;
        if mode != MODE_PRIVATE {
            return Err(CacheIndexError::UnsupportedMode { mode });
        }
        let count = r.u32()?;
        let mut index = PrivateCacheIndex::default();
        let mut previous: Option<(RepositoryKey, CommitId)> = None;
        for _ in 0..count {
            let repository_key = RepositoryKey::from_hex(r.short_text("repository_key")?).ok_or(
                CacheIndexError::InvalidField {
                    field: "repository_key",
                },
            )?;
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
            let key = (repository_key.clone(), commit.clone());
            if previous.as_ref().is_some_and(|p| p >= &key) {
                return Err(CacheIndexError::NotCanonicalOrder);
            }
            previous = Some(key.clone());
            index.entries.insert(
                key,
                PrivateIndexEntry {
                    repository_key,
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

    /// 진단 export 텍스트: entry마다 key·commit·content hash·크기·최근 사용 시각. URL 없음.
    pub fn export_diagnostic(&self) -> String {
        let mut out = format!("library-cache entries={}\n", self.entries.len());
        for e in self.entries.values() {
            out.push_str(&format!(
                "repo={} commit={} content={} size={} last_used_ms={}\n",
                e.repository_key, e.commit, e.content_hash, e.size_bytes, e.last_used_unix_ms
            ));
        }
        out
    }

    /// 파일에 원자적으로 씁니다.
    pub fn write_to(&self, path: &Path) -> Result<(), CacheIndexError> {
        crate::cache_index::write_atomic(path, &self.encode()?)
    }
}

impl LocalSourceConfig {
    /// keyed identifier의 원본 URL(로컬 UI 표시용).
    pub fn source_for(&self, key: &RepositoryKey) -> Option<&RepositoryUrl> {
        self.sources.get(key)
    }

    /// 등록 수.
    pub fn len(&self) -> usize {
        self.sources.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// canonical bytes.
    pub fn encode(&self) -> Result<Vec<u8>, CacheIndexError> {
        let mut out = Vec::new();
        out.extend_from_slice(SOURCE_CONFIG_MAGIC);
        out.extend_from_slice(&SOURCE_CONFIG_VERSION.to_le_bytes());
        let count = u32::try_from(self.sources.len()).map_err(|_| CacheIndexError::TooLong {
            field: "entry_count",
        })?;
        out.extend_from_slice(&count.to_le_bytes());
        for (key, url) in &self.sources {
            put_u8_bytes(&mut out, key.as_str().as_bytes(), "repository_key")?;
            put_u32_bytes(&mut out, url.reveal().as_bytes(), "repository")?;
        }
        Ok(out)
    }

    /// 엄격 디코딩.
    pub fn decode(bytes: &[u8]) -> Result<LocalSourceConfig, CacheIndexError> {
        let mut r = Reader { bytes, pos: 0 };
        if r.take(8)? != SOURCE_CONFIG_MAGIC {
            return Err(CacheIndexError::BadMagic);
        }
        let version = r.u16()?;
        if version != SOURCE_CONFIG_VERSION {
            return Err(CacheIndexError::UnsupportedVersion { version });
        }
        let count = r.u32()?;
        let mut config = LocalSourceConfig::default();
        let mut previous: Option<RepositoryKey> = None;
        for _ in 0..count {
            let key = RepositoryKey::from_hex(r.short_text("repository_key")?).ok_or(
                CacheIndexError::InvalidField {
                    field: "repository_key",
                },
            )?;
            let len = r.u32()? as usize;
            let url = std::str::from_utf8(r.take(len)?)
                .ok()
                .and_then(|s| RepositoryUrl::new(s).ok())
                .ok_or(CacheIndexError::InvalidField {
                    field: "repository",
                })?;
            if previous.as_ref().is_some_and(|p| p >= &key) {
                return Err(CacheIndexError::NotCanonicalOrder);
            }
            previous = Some(key.clone());
            config.sources.insert(key, url);
        }
        if r.pos != bytes.len() {
            return Err(CacheIndexError::TrailingBytes);
        }
        Ok(config)
    }

    /// 소유자 전용 권한(unix 0600)으로 원자적으로 씁니다.
    pub fn write_to(&self, path: &Path) -> Result<(), CacheIndexError> {
        write_owner_only(path, &self.encode()?)
    }
}

impl fmt::Display for PrivateCacheIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.export_diagnostic())
    }
}

/// 소유자 전용 임시 파일 → fsync → rename.
fn write_owner_only(path: &Path, bytes: &[u8]) -> Result<(), CacheIndexError> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let io = |e: std::io::Error| CacheIndexError::Io { kind: e.kind() };
    let invalid = CacheIndexError::Io {
        kind: std::io::ErrorKind::InvalidInput,
    };
    let parent = path.parent().ok_or(invalid.clone())?;
    let file_name = path.file_name().ok_or(invalid)?;
    std::fs::create_dir_all(parent).map_err(io)?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = parent.join(tmp_name);
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(io(e));
    }
    // rename 자체가 crash 뒤에도 남도록 부모 디렉터리 항목을 fsync합니다.
    crate::cache_index::sync_dir(parent).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache_index::fixtures::entry;
    use crate::cache_root::LibraryCacheRoot;
    use crate::declaration::fixtures::commit;
    use crate::test_support::{FnvKeyer, TempDir};

    const URL_A: &str = "git@secret-host.example:team/private-ds.git";
    const URL_B: &str = "https://secret-host.example/team/private-icons";

    fn plain() -> CacheIndex {
        let mut index = CacheIndex::new();
        index.upsert(entry(URL_A, 0x01, 4096, 10));
        index.upsert(entry(URL_A, 0x02, 8192, 20));
        index.upsert(entry(URL_B, 0x03, 1024, 30));
        index
    }

    fn assert_no_url(bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        for needle in ["secret-host", "private-ds", "private-icons", "team/"] {
            assert!(!text.contains(needle), "{needle:?} 노출");
        }
    }

    #[test]
    fn exported_diagnostic_and_private_index_contain_no_url() {
        let keyer = FnvKeyer::new(42);
        let (private, sources) = plain().split_private(&keyer);
        assert_eq!(private.len(), 3);
        assert_eq!(sources.len(), 2);

        let diagnostic = private.export_diagnostic();
        assert_no_url(diagnostic.as_bytes());
        assert!(diagnostic.contains("entries=3"));
        assert!(diagnostic.contains(&format!("commit={}", commit(0x02))));
        assert_no_url(format!("{private:?}").as_bytes());
        assert_no_url(private.to_string().as_bytes());

        let bytes = private.encode().unwrap();
        assert_no_url(&bytes);
        assert_eq!(bytes[10], MODE_PRIVATE);
        assert_eq!(PrivateCacheIndex::decode(&bytes).unwrap(), private);
        // plain decoder는 privacy index를, privacy decoder는 plain index를 거부.
        assert_eq!(
            CacheIndex::decode(&bytes),
            Err(CacheIndexError::UnsupportedMode { mode: 1 })
        );
        assert_eq!(
            PrivateCacheIndex::decode(&plain().encode().unwrap()),
            Err(CacheIndexError::UnsupportedMode { mode: 0 })
        );

        // URL로 조회는 keyer로 가능(.git 표기 차이 무관), 원문 URL은 local source config에만.
        let url_a = RepositoryUrl::new("git@secret-host.example:team/private-ds").unwrap();
        let found = private.get(&keyer, &url_a, &commit(0x02)).unwrap();
        assert_eq!(found.size_bytes, 8192);
        assert_eq!(
            sources.source_for(&found.repository_key).unwrap().reveal(),
            URL_A
        );
        // source config Debug도 URL을 가림.
        assert_no_url(format!("{sources:?}").as_bytes());
        // 다른 key(비밀)로는 같은 index를 찾을 수 없음.
        assert!(private
            .get(&FnvKeyer::new(43), &url_a, &commit(0x02))
            .is_none());
    }

    #[test]
    fn files_are_separate_and_source_config_is_owner_only() {
        let app = TempDir::new("privacy");
        let root = LibraryCacheRoot::in_app_data(app.path()).unwrap();
        let keyer = FnvKeyer::new(5);
        let (private, sources) = plain().split_private(&keyer);
        private.write_to(&root.index_path()).unwrap();
        let config_path = root.path().join(SOURCE_CONFIG_FILE);
        sources.write_to(&config_path).unwrap();

        let index_bytes = std::fs::read(root.index_path()).unwrap();
        assert_no_url(&index_bytes);
        assert_eq!(PrivateCacheIndex::decode(&index_bytes).unwrap(), private);
        let config_bytes = std::fs::read(&config_path).unwrap();
        assert_eq!(LocalSourceConfig::decode(&config_bytes).unwrap(), sources);
        assert_eq!(
            LocalSourceConfig::decode(&config_bytes)
                .unwrap()
                .encode()
                .unwrap(),
            config_bytes
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&config_path)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // 잘린 config 거부.
        assert_eq!(
            LocalSourceConfig::decode(&config_bytes[..config_bytes.len() - 1]),
            Err(CacheIndexError::Truncated)
        );
    }
}

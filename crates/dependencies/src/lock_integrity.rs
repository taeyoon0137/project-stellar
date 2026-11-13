//! lockfile content hash와 cache entry 무결성 검사 (XV.4.4).
//!
//! lock의 각 node(LibraryId + exact commit)에 대해 cache entry 내용으로 content hash를 **다시 계산**해
//! lock 값과 비교합니다. 다르면 cache 손상(`Corrupted`)으로 즉시 실패합니다. 이 검사는 선택자
//! 해석기를 받지 않으므로 구조적으로 **다른 commit을 자동 선택할 수 없습니다** — 손상 복구(재fetch·
//! 재materialize 또는 사용자의 명시적 update)는 호출자가 결정합니다.
//!
//! hash 재계산(materialize된 entry bytes의 SHA-256)은 cache entry store(XV.3.3)와 storage SHA-256을
//! 쓰는 project가 [`CachedContentSource`]로 구현합니다.

use std::collections::BTreeSet;
use std::fmt;

use crate::content_hash::ContentHashText;
use crate::declaration::{CommitId, RepositoryUrl};
use crate::graph::{DependencyNodeKey, DependencyPath};
use crate::lock::LockDocument;

/// cache entry 읽기 실패.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheReadFailure {
    /// io 오류 종류.
    pub kind: std::io::ErrorKind,
}

/// cache entry 내용 hash 재계산기.
pub trait CachedContentSource {
    /// `(repository, commit)` cache entry 내용에서 content hash를 다시 계산합니다. entry가 없으면
    /// `Ok(None)`.
    fn recompute_content_hash(
        &mut self,
        repository: &RepositoryUrl,
        commit: &CommitId,
    ) -> Result<Option<ContentHashText>, CacheReadFailure>;
}

/// lock-cache 무결성 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockIntegrityError {
    /// cache entry 내용 hash가 lock 값과 다름(손상·변조).
    Corrupted {
        /// 대상 node.
        node: DependencyNodeKey,
        /// lock entry 경로.
        path: DependencyPath,
    },
    /// lock이 가리키는 cache entry가 없음.
    MissingEntry {
        /// 대상 node.
        node: DependencyNodeKey,
        /// lock entry 경로.
        path: DependencyPath,
    },
    /// cache entry를 읽지 못함.
    ReadFailed {
        /// 대상 node.
        node: DependencyNodeKey,
        /// 원인.
        failure: CacheReadFailure,
    },
}

impl fmt::Display for LockIntegrityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockIntegrityError::Corrupted { node, path } => write!(
                f,
                "{path}: cache entry {node}의 content hash가 lockfile과 다릅니다(손상)"
            ),
            LockIntegrityError::MissingEntry { node, path } => {
                write!(f, "{path}: cache entry {node}가 없습니다")
            }
            LockIntegrityError::ReadFailed { node, failure } => {
                write!(f, "cache entry {node} 읽기 실패: {:?}", failure.kind)
            }
        }
    }
}

impl std::error::Error for LockIntegrityError {}

/// lock의 모든 node를 cache와 대조합니다. 성공하면 검증한 node 목록(정렬, 중복 제거)을 반환합니다.
///
/// node마다 한 번만 재계산하고, 첫 불일치에서 멈춥니다. lock에 없는 commit은 읽지 않습니다.
pub fn verify_lock_against_cache(
    lock: &LockDocument,
    cache: &mut dyn CachedContentSource,
) -> Result<Vec<DependencyNodeKey>, LockIntegrityError> {
    let mut verified = BTreeSet::new();
    for entry in lock.entries() {
        let node = entry.key();
        if verified.contains(&node) {
            continue;
        }
        let actual = cache
            .recompute_content_hash(&entry.repository, &entry.exact_commit)
            .map_err(|failure| LockIntegrityError::ReadFailed {
                node: node.clone(),
                failure,
            })?;
        match actual {
            None => {
                return Err(LockIntegrityError::MissingEntry {
                    node,
                    path: entry.dependency_path.clone(),
                })
            }
            Some(hash) if hash != entry.content_hash => {
                return Err(LockIntegrityError::Corrupted {
                    node,
                    path: entry.dependency_path.clone(),
                })
            }
            Some(_) => {
                verified.insert(node);
            }
        }
    }
    Ok(verified.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::declaration::fixtures::{commit, decl, library};
    use crate::declaration::LogicalName;
    use crate::resolver::fake::{resolve, Catalog};
    use crate::test_support::TempDir;

    const A: &str = "https://e.com/a.git";
    const C: &str = "https://e.com/c.git";

    /// `<root>/<commit>/` 디렉터리를 entry로 쓰는 테스트 cache. hash는 정렬된 (경로, bytes)의
    /// FNV-1a 기반 결정적 32바이트(암호학적 아님 — 운영은 storage SHA-256).
    struct DirCache {
        root: PathBuf,
        reads: Vec<CommitId>,
    }

    impl DirCache {
        fn entry_dir(&self, commit: &CommitId) -> PathBuf {
            self.root.join(commit.as_str())
        }

        fn put(&self, commit: &CommitId, files: &[(&str, &[u8])]) {
            let dir = self.entry_dir(commit);
            std::fs::create_dir_all(&dir).unwrap();
            for (name, bytes) in files {
                std::fs::write(dir.join(name), bytes).unwrap();
            }
        }

        fn hash_dir(dir: &Path) -> std::io::Result<ContentHashText> {
            let mut files: Vec<(String, Vec<u8>)> = Vec::new();
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                files.push((
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path())?,
                ));
            }
            files.sort();
            let mut digest = [0u8; 32];
            for (lane, chunk) in digest.chunks_mut(8).enumerate() {
                let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ lane as u64;
                for (name, bytes) in &files {
                    for b in name.as_bytes().iter().chain([0u8].iter()).chain(bytes) {
                        h ^= u64::from(*b);
                        h = h.wrapping_mul(0x0000_0100_0000_01b3);
                    }
                }
                chunk.copy_from_slice(&h.to_le_bytes());
            }
            Ok(ContentHashText::from_sha256_digest(digest))
        }
    }

    impl CachedContentSource for DirCache {
        fn recompute_content_hash(
            &mut self,
            _: &RepositoryUrl,
            commit: &CommitId,
        ) -> Result<Option<ContentHashText>, CacheReadFailure> {
            self.reads.push(commit.clone());
            let dir = self.entry_dir(commit);
            if !dir.is_dir() {
                return Ok(None);
            }
            DirCache::hash_dir(&dir)
                .map(Some)
                .map_err(|e| CacheReadFailure { kind: e.kind() })
        }
    }

    fn key(lib: u8, c: u8) -> DependencyNodeKey {
        DependencyNodeKey::new(library(lib), commit(c))
    }

    /// root → a v1 → c v1. cache에는 a v1, c v1, 그리고 lock에 없는 c v2가 있습니다.
    fn setup(tmp: &TempDir) -> (LockDocument, DirCache) {
        let cache = DirCache {
            root: tmp.path().to_path_buf(),
            reads: Vec::new(),
        };
        cache.put(
            &commit(0xa1),
            &[("stellar.yaml", b"a v1"), ("design.bin", b"\x00\x01")],
        );
        cache.put(
            &commit(0xc1),
            &[("stellar.yaml", b"c v1"), ("tokens.bin", b"t1")],
        );
        cache.put(
            &commit(0xc2),
            &[("stellar.yaml", b"c v2"), ("tokens.bin", b"t2")],
        );
        let mut catalog = Catalog::default();
        catalog.publish(C, 3, "v1", 0xc1, vec![]);
        catalog.publish(A, 1, "v1", 0xa1, vec![decl("c", 3, C, "v1")]);
        let graph = resolve(&catalog, &[decl("a", 1, A, "v1")]).unwrap();
        let hashes: BTreeMap<_, _> = [(key(1, 0xa1), 0xa1), (key(3, 0xc1), 0xc1)]
            .into_iter()
            .map(|(k, c)| (k, DirCache::hash_dir(&cache.entry_dir(&commit(c))).unwrap()))
            .collect();
        (LockDocument::from_graph(&graph, &hashes).unwrap(), cache)
    }

    #[test]
    fn intact_cache_matches_lock() {
        let tmp = TempDir::new("lock-integrity-ok");
        let (lock, mut cache) = setup(&tmp);
        assert_eq!(
            verify_lock_against_cache(&lock, &mut cache).unwrap(),
            [key(1, 0xa1), key(3, 0xc1)]
        );
    }

    #[test]
    fn tampered_cache_fails_as_corruption_without_choosing_another_commit() {
        let tmp = TempDir::new("lock-integrity-tamper");
        let (lock, mut cache) = setup(&tmp);
        // c v1 entry의 파일 하나를 변조.
        std::fs::write(cache.entry_dir(&commit(0xc1)).join("tokens.bin"), b"evil").unwrap();
        let err = verify_lock_against_cache(&lock, &mut cache).unwrap_err();
        assert_eq!(
            err,
            LockIntegrityError::Corrupted {
                node: key(3, 0xc1),
                path: DependencyPath::from_names(vec![
                    LogicalName::new("a").unwrap(),
                    LogicalName::new("c").unwrap()
                ]),
            }
        );
        assert!(err.to_string().contains("손상"));
        // 같은 library의 유효한 다른 commit(c v2)이 cache에 있어도 읽거나 선택하지 않음.
        assert!(!cache.reads.contains(&commit(0xc2)));
        assert_eq!(cache.reads, [commit(0xa1), commit(0xc1)]);

        // 파일 추가도 변조로 판정.
        let (lock2, mut cache2) = setup(&TempDir::new("lock-integrity-extra"));
        cache2.put(&commit(0xa1), &[("injected.bin", b"x")]);
        assert!(matches!(
            verify_lock_against_cache(&lock2, &mut cache2),
            Err(LockIntegrityError::Corrupted { node, .. }) if node == key(1, 0xa1)
        ));
    }

    #[test]
    fn missing_entry_is_reported_separately() {
        let tmp = TempDir::new("lock-integrity-missing");
        let (lock, mut cache) = setup(&tmp);
        std::fs::remove_dir_all(cache.entry_dir(&commit(0xc1))).unwrap();
        assert!(matches!(
            verify_lock_against_cache(&lock, &mut cache),
            Err(LockIntegrityError::MissingEntry { node, .. }) if node == key(3, 0xc1)
        ));
    }
}

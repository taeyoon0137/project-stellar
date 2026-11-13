//! repository별 bare mirror cache (XV.3.2).
//!
//! cache root의 `mirrors/<repository-key>.git`에 repository마다 bare mirror 하나를 두고, fetch는
//! [`GitCommandRunner`](시스템 Git)로 수행합니다. 한 [`MirrorCache`] 세션 안에서 같은 repository(비교
//! 키 기준)는 **한 번만 fetch**합니다 — 같은 저장소의 여러 의존성(다중 버전 포함)이 네트워크를
//! 중복 사용하지 않습니다.
//!
//! 디렉터리 이름은 URL 원문이 아니라 [`RepositoryKeyer`]가 만든 keyed identifier입니다(XV.1.4·
//! XV.3.5). 운영 keyer(로컬 비밀 key 기반 keyed hash)는 storage의 SHA-256을 쓰는 project가
//! 연결합니다.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use crate::cache_root::LibraryCacheRoot;
use crate::declaration::RepositoryUrl;
use crate::git::{git_args, GitCommandRunner};
use crate::git_refs::FetchedRefs;
use crate::selection::SelectorError;

/// repository keyed identifier(소문자 hex 16~64자). 디렉터리 이름·privacy index key로 씁니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepositoryKey(String);

impl RepositoryKey {
    /// 소문자 hex 16~64자만 받습니다.
    pub fn from_hex(text: &str) -> Option<RepositoryKey> {
        let ok = (16..=64).contains(&text.len())
            && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        ok.then(|| RepositoryKey(text.to_string()))
    }

    /// hex 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepositoryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// repository → keyed identifier. 같은 비교 키([`RepositoryUrl::comparison_key`])는 같은 key여야
/// 합니다.
pub trait RepositoryKeyer {
    /// keyed identifier.
    fn repository_key(&self, repository: &RepositoryUrl) -> RepositoryKey;
}

/// mirror 작업 실패. stderr·URL은 담지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorError {
    /// mirror 디렉터리 준비 실패.
    Io {
        /// io 오류 종류.
        kind: std::io::ErrorKind,
    },
    /// `git init --bare` 실패.
    InitFailed {
        /// 종료 코드.
        exit_code: Option<i32>,
    },
    /// `git fetch` 실패.
    FetchFailed {
        /// 종료 코드.
        exit_code: Option<i32>,
    },
}

impl fmt::Display for MirrorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MirrorError::Io { kind } => write!(f, "mirror 디렉터리 준비 실패: {kind:?}"),
            MirrorError::InitFailed { exit_code } => {
                write!(f, "mirror 초기화 실패(종료 코드 {exit_code:?})")
            }
            MirrorError::FetchFailed { exit_code } => {
                write!(f, "mirror fetch 실패(종료 코드 {exit_code:?})")
            }
        }
    }
}

impl std::error::Error for MirrorError {}

/// repository의 mirror 경로(`<cache root>/mirrors/<key>.git`). 디스크는 건드리지 않습니다.
///
/// [`MirrorCache`]와 같은 root·keyer를 쓰면 같은 경로입니다. 이미 fetch된 mirror를 읽기만 하는
/// 쪽(예: commit의 library 매니페스트 읽기)이 fetch 세션 없이 경로를 얻을 때 씁니다.
pub fn mirror_path<K: RepositoryKeyer + ?Sized>(
    root: &LibraryCacheRoot,
    keyer: &K,
    repository: &RepositoryUrl,
) -> PathBuf {
    let key = keyer.repository_key(repository);
    root.mirrors_dir().join(format!("{key}.git"))
}

/// repository별 bare mirror cache.
pub struct MirrorCache<R, K> {
    root: LibraryCacheRoot,
    runner: R,
    keyer: K,
    fetched: BTreeSet<RepositoryKey>,
}

impl<R: GitCommandRunner, K: RepositoryKeyer> MirrorCache<R, K> {
    /// 세션을 시작합니다(아직 아무것도 fetch하지 않음).
    pub fn new(root: LibraryCacheRoot, runner: R, keyer: K) -> MirrorCache<R, K> {
        MirrorCache {
            root,
            runner,
            keyer,
            fetched: BTreeSet::new(),
        }
    }

    /// 실행기.
    pub fn runner(&self) -> &R {
        &self.runner
    }

    /// repository의 mirror 경로(`mirrors/<key>.git`).
    pub fn mirror_dir(&self, repository: &RepositoryUrl) -> PathBuf {
        mirror_path(&self.root, &self.keyer, repository)
    }

    /// 이 세션에서 fetch를 마친 repository key.
    pub fn fetched_keys(&self) -> impl Iterator<Item = &RepositoryKey> {
        self.fetched.iter()
    }

    /// mirror가 이 세션에서 fetch되었음을 보장합니다. 이미 fetch했으면 Git을 실행하지 않습니다.
    pub fn ensure_fetched(&mut self, repository: &RepositoryUrl) -> Result<PathBuf, MirrorError> {
        let key = self.keyer.repository_key(repository);
        let dir = self.root.mirrors_dir().join(format!("{key}.git"));
        if self.fetched.contains(&key) {
            return Ok(dir);
        }
        // 같은 mirror를 여러 프로세스가 동시에 init·fetch하지 않도록 repository fetch lock을
        // fetch 동안만 잡습니다(XV.3.6). entry를 읽는 쪽은 이 lock을 쓰지 않습니다.
        let _fetch_lock = crate::cache_lock::CacheLocks::new(&self.root)
            .fetch(&key)
            .map_err(|e| MirrorError::Io {
                kind: match e {
                    crate::cache_lock::CacheLockError::Io { kind } => kind,
                    crate::cache_lock::CacheLockError::Busy => std::io::ErrorKind::WouldBlock,
                },
            })?;
        let mirrors = self.root.mirrors_dir();
        std::fs::create_dir_all(&mirrors).map_err(|e| MirrorError::Io { kind: e.kind() })?;
        if !dir.join("HEAD").is_file() {
            // 로컬 작업(네트워크 없음). template을 비워 hook 등을 복사하지 않습니다.
            let mut args = git_args(["init", "--bare", "--quiet", "--template="]);
            args.push(dir.as_os_str().to_owned());
            let out = self
                .runner
                .run_git(&args, &mirrors)
                .map_err(|_| MirrorError::InitFailed { exit_code: None })?;
            if !out.success() {
                return Err(MirrorError::InitFailed {
                    exit_code: out.exit_code,
                });
            }
        }
        let mut args = vec![OsString::from("--git-dir"), dir.as_os_str().to_owned()];
        args.extend(git_args(["fetch", "--quiet", "--prune", "--"]));
        args.push(OsString::from(repository.reveal()));
        args.extend(git_args([
            "+refs/heads/*:refs/heads/*",
            "+refs/tags/*:refs/tags/*",
        ]));
        let out = self
            .runner
            .run_git(&args, &mirrors)
            .map_err(|_| MirrorError::FetchFailed { exit_code: None })?;
        if !out.success() {
            return Err(MirrorError::FetchFailed {
                exit_code: out.exit_code,
            });
        }
        self.fetched.insert(key);
        Ok(dir)
    }
}

impl<R: GitCommandRunner, K: RepositoryKeyer> FetchedRefs for MirrorCache<R, K> {
    fn fetched_git_dir(&mut self, repository: &RepositoryUrl) -> Result<PathBuf, SelectorError> {
        self.ensure_fetched(repository)
            .map_err(|e| SelectorError::GitFailed {
                operation: "fetch",
                exit_code: match e {
                    MirrorError::InitFailed { exit_code }
                    | MirrorError::FetchFailed { exit_code } => exit_code,
                    MirrorError::Io { .. } => None,
                },
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::fixtures::{decl, library};
    use crate::declaration::CommitId;
    use crate::git_refs::GitSelectorResolver;
    use crate::resolver::{
        resolve_dependency_graph, LibraryManifest, LibraryManifestSource, ManifestSourceError,
    };
    use crate::test_support::{fixture_repo, FnvKeyer, SystemGit, TempDir};

    /// 모든 commit에서 library 1, 전이 의존성 없음.
    struct LeafManifests;

    impl LibraryManifestSource for LeafManifests {
        fn manifest_at(
            &mut self,
            _: &RepositoryUrl,
            _: &CommitId,
        ) -> Result<LibraryManifest, ManifestSourceError> {
            Ok(LibraryManifest {
                library_id: library(1),
                schema_version: 1,
                dependencies: vec![],
            })
        }
    }

    #[test]
    fn two_dependencies_on_one_repository_fetch_once() {
        let tmp = TempDir::new("mirror");
        let Some(mut fixture_git) = SystemGit::find(tmp.path()) else {
            return;
        };
        let repo = fixture_repo(&mut fixture_git, tmp.path(), "remote");
        let app_data = tmp.path().join("app-data");
        let root = LibraryCacheRoot::in_app_data(&app_data).unwrap();
        let neutral = tmp.path().join("neutral");
        std::fs::create_dir_all(&neutral).unwrap();

        let url = repo.path.to_str().unwrap().to_string();
        // 같은 저장소의 두 의존성(v1, v2), 하나는 끝에 '/'를 붙인 표기.
        let root_decls = [
            decl("old", 1, &url, "v1"),
            decl("new", 1, &format!("{url}/"), "v2"),
        ];

        let mirror_git = SystemGit::find(tmp.path()).unwrap();
        let mirrors = MirrorCache::new(root.clone(), mirror_git, FnvKeyer::new(7));
        let refs_git = SystemGit::find(tmp.path()).unwrap();
        let mut resolver = GitSelectorResolver::new(refs_git, mirrors, &neutral);
        let graph =
            resolve_dependency_graph(&root_decls, &mut resolver, &mut LeafManifests).unwrap();
        assert_eq!(graph.node_count(), 2);

        // fetch(네트워크)는 1회, ls-remote도 repository당 1회, init 1회.
        let mirrors = resolver.fetched();
        assert_eq!(mirrors.runner().count("fetch"), 1);
        assert_eq!(mirrors.runner().count("init"), 1);
        assert_eq!(resolver.runner().count("ls-remote"), 1);
        assert_eq!(mirrors.fetched_keys().count(), 1);

        // mirror는 cache root 아래, 이름에 URL 텍스트 없음.
        let repo_url = RepositoryUrl::new(&url).unwrap();
        let dir = mirrors.mirror_dir(&repo_url);
        assert!(dir.starts_with(root.mirrors_dir()));
        assert!(dir.join("HEAD").is_file());
        let name = dir.file_name().unwrap().to_str().unwrap().to_string();
        assert!(!name.contains("remote"), "{name}");
        assert_eq!(
            mirrors.mirror_dir(&RepositoryUrl::new(&format!("{url}/")).unwrap()),
            dir
        );

        // 새 세션은 기존 mirror를 재사용(init 없음)하고 세션당 1회 fetch.
        let mut next = MirrorCache::new(
            root.clone(),
            SystemGit::find(tmp.path()).unwrap(),
            FnvKeyer::new(7),
        );
        assert_eq!(next.ensure_fetched(&repo_url).unwrap(), dir);
        assert_eq!(next.ensure_fetched(&repo_url).unwrap(), dir);
        assert_eq!(next.runner().count("init"), 0);
        assert_eq!(next.runner().count("fetch"), 1);
    }

    #[test]
    fn failed_fetch_is_not_marked_fetched() {
        let tmp = TempDir::new("mirror-fail");
        let Some(git) = SystemGit::find(tmp.path()) else {
            return;
        };
        let root = LibraryCacheRoot::in_app_data(&tmp.path().join("app")).unwrap();
        let mut mirrors = MirrorCache::new(root, git, FnvKeyer::new(1));
        let missing = RepositoryUrl::new(tmp.path().join("nope").to_str().unwrap()).unwrap();
        let first = mirrors.ensure_fetched(&missing).unwrap_err();
        assert!(matches!(first, MirrorError::FetchFailed { exit_code: Some(c) } if c != 0));
        assert!(!first.to_string().contains("nope"));
        assert_eq!(mirrors.fetched_keys().count(), 0);
        // 다시 시도하면 다시 fetch(실패를 캐시하지 않음).
        assert!(mirrors.ensure_fetched(&missing).is_err());
        assert_eq!(mirrors.runner().count("fetch"), 2);
        assert!(matches!(
            mirrors.fetched_git_dir(&missing),
            Err(SelectorError::GitFailed {
                operation: "fetch",
                ..
            })
        ));
    }

    #[test]
    fn repository_key_format() {
        assert!(RepositoryKey::from_hex("0123456789abcdef").is_some());
        assert!(RepositoryKey::from_hex(&"a".repeat(64)).is_some());
        for bad in [
            "",
            "0123456789abcde",
            "0123456789ABCDEF",
            "../../etc/passwd0",
            &"a".repeat(65),
        ] {
            assert!(RepositoryKey::from_hex(bad).is_none(), "{bad}");
        }
    }
}

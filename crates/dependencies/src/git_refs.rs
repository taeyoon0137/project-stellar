//! 시스템 Git 기반 version 선택자 해석 (XV.2.4).
//!
//! 해석 순서(Git 객체를 직접 파싱하지 않음):
//!
//! 1. tag·branch: `git ls-remote -- <url> refs/heads/* refs/tags/*` 출력에서 이름을 정확히 찾습니다.
//!    annotated tag는 peeled 줄(`refs/tags/<t>^{}`)의 commit을 씁니다. 결과는 repository별로 세션
//!    memo합니다.
//! 2. fetch된 refs: [`FetchedRefs`]가 해당 repository를 fetch한 bare git dir을 제공합니다(XV.3.2
//!    mirror).
//! 3. commit 검증: `git --git-dir <mirror> cat-file -t <oid>`가 `commit`이어야 합니다. commit
//!    선택자는 ls-remote 없이 이 검증만 수행합니다.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::declaration::{CommitId, RepositoryUrl, SelectorKind, VersionSelector};
use crate::git::{git_args, GitCommandRunner};
use crate::selection::{SelectorError, SelectorResolver};

/// `ls-remote` 출력의 ref 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteRef {
    /// object id(commit 또는 tag object — 검증 전, 형식만 확인).
    pub oid: CommitId,
    /// 전체 ref 이름(peeled 줄은 `^{}` 접미사 포함).
    pub name: String,
}

/// `git ls-remote` 출력(`<oid>\t<ref>` 줄)을 파싱합니다. 형식이 다르면 `MalformedOutput`.
pub fn parse_ls_remote(stdout: &[u8]) -> Result<Vec<RemoteRef>, SelectorError> {
    let malformed = SelectorError::MalformedOutput {
        operation: "ls-remote",
    };
    let text = std::str::from_utf8(stdout).map_err(|_| malformed.clone())?;
    let mut refs = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let (oid, name) = line.split_once('\t').ok_or_else(|| malformed.clone())?;
        let oid = oid.parse::<CommitId>().map_err(|_| malformed.clone())?;
        if name.is_empty() || name.contains('\t') {
            return Err(malformed);
        }
        refs.push(RemoteRef {
            oid,
            name: name.to_string(),
        });
    }
    Ok(refs)
}

/// 선택자에 맞는 ref의 object id를 찾습니다(commit 선택자는 자기 자신).
pub fn select_object(
    refs: &[RemoteRef],
    selector: &VersionSelector,
) -> Result<CommitId, SelectorError> {
    let find = |name: &str| refs.iter().find(|r| r.name == name).map(|r| r.oid.clone());
    match selector {
        VersionSelector::Tag(tag) => {
            let full = format!("refs/tags/{tag}");
            find(&format!("{full}^{{}}")).or_else(|| find(&full)).ok_or(
                SelectorError::RefNotFound {
                    kind: SelectorKind::Tag,
                },
            )
        }
        VersionSelector::Branch(branch) => {
            find(&format!("refs/heads/{branch}")).ok_or(SelectorError::RefNotFound {
                kind: SelectorKind::Branch,
            })
        }
        VersionSelector::Commit(commit) => Ok(commit.clone()),
    }
}

/// repository를 fetch한 bare git dir 공급자(XV.3.2 mirror cache가 구현).
pub trait FetchedRefs {
    /// `repository`의 objects·refs가 fetch된 bare git dir. 필요하면 fetch를 수행합니다.
    fn fetched_git_dir(&mut self, repository: &RepositoryUrl) -> Result<PathBuf, SelectorError>;
}

/// 시스템 Git 선택자 해석기.
pub struct GitSelectorResolver<R, F> {
    runner: R,
    fetched: F,
    neutral_dir: PathBuf,
    remote_refs: BTreeMap<String, Vec<RemoteRef>>,
}

impl<R: GitCommandRunner, F: FetchedRefs> GitSelectorResolver<R, F> {
    /// 해석기를 만듭니다. `neutral_dir`은 저장소 밖의 작업 디렉터리입니다(저장소 config 영향 차단).
    pub fn new(runner: R, fetched: F, neutral_dir: &Path) -> GitSelectorResolver<R, F> {
        GitSelectorResolver {
            runner,
            fetched,
            neutral_dir: neutral_dir.to_path_buf(),
            remote_refs: BTreeMap::new(),
        }
    }

    /// 실행기.
    pub fn runner(&self) -> &R {
        &self.runner
    }

    /// fetch 공급자.
    pub fn fetched(&self) -> &F {
        &self.fetched
    }

    fn list_remote(&mut self, repository: &RepositoryUrl) -> Result<&[RemoteRef], SelectorError> {
        let key = repository.comparison_key();
        if !self.remote_refs.contains_key(&key) {
            let mut args = git_args(["ls-remote", "--"]);
            args.push(OsString::from(repository.reveal()));
            args.extend(git_args(["refs/heads/*", "refs/tags/*"]));
            let out = self.runner.run_git(&args, &self.neutral_dir).map_err(|_| {
                SelectorError::GitFailed {
                    operation: "ls-remote",
                    exit_code: None,
                }
            })?;
            if !out.success() {
                return Err(SelectorError::GitFailed {
                    operation: "ls-remote",
                    exit_code: out.exit_code,
                });
            }
            let refs = parse_ls_remote(&out.stdout)?;
            self.remote_refs.insert(key.clone(), refs);
        }
        Ok(self.remote_refs.get(&key).map(Vec::as_slice).unwrap_or(&[]))
    }

    fn verify_commit(&mut self, git_dir: &Path, oid: &CommitId) -> Result<(), SelectorError> {
        let mut args = vec![OsString::from("--git-dir"), git_dir.as_os_str().to_owned()];
        args.extend(git_args(["cat-file", "-t", oid.as_str()]));
        let out = self
            .runner
            .run_git(&args, git_dir)
            .map_err(|_| SelectorError::GitFailed {
                operation: "cat-file",
                exit_code: None,
            })?;
        if !out.success() {
            return Err(SelectorError::CommitNotFound);
        }
        match out.stdout.as_slice() {
            b"commit\n" | b"commit" => Ok(()),
            _ => Err(SelectorError::NotACommit),
        }
    }
}

impl<R: GitCommandRunner, F: FetchedRefs> SelectorResolver for GitSelectorResolver<R, F> {
    fn resolve(
        &mut self,
        repository: &RepositoryUrl,
        selector: &VersionSelector,
    ) -> Result<CommitId, SelectorError> {
        let oid = match selector {
            VersionSelector::Commit(commit) => commit.clone(),
            other => {
                let refs = self.list_remote(repository)?;
                select_object(refs, other)?
            }
        };
        let git_dir = self.fetched.fetched_git_dir(repository)?;
        self.verify_commit(&git_dir, &oid)?;
        Ok(oid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration::RefName;
    use crate::test_support::{fixture_repo, SystemGit, TempDir};

    fn tag(name: &str) -> VersionSelector {
        VersionSelector::Tag(RefName::new(name).unwrap())
    }

    fn branch(name: &str) -> VersionSelector {
        VersionSelector::Branch(RefName::new(name).unwrap())
    }

    #[test]
    fn parse_ls_remote_output() {
        let a = "a".repeat(40);
        let b = "B".repeat(40);
        let text = format!("{a}\trefs/heads/main\n{b}\trefs/tags/v1^{{}}\n\n");
        let refs = parse_ls_remote(text.as_bytes()).unwrap();
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[1].oid.as_str(), "b".repeat(40));
        assert_eq!(refs[1].name, "refs/tags/v1^{}");
        for bad in [
            "abc\trefs/heads/main\n".to_string(),
            format!("{a} refs/heads/main\n"),
            format!("{a}\t\n"),
        ] {
            assert_eq!(
                parse_ls_remote(bad.as_bytes()),
                Err(SelectorError::MalformedOutput {
                    operation: "ls-remote"
                }),
                "{bad:?}"
            );
        }
        assert!(parse_ls_remote(&[0xff, 0xfe]).is_err());
    }

    #[test]
    fn select_prefers_peeled_tag_and_exact_names() {
        let refs = parse_ls_remote(
            format!(
                "{}\trefs/tags/v2\n{}\trefs/tags/v2^{{}}\n{}\trefs/heads/main\n{}\trefs/heads/x/main\n",
                "1".repeat(40),
                "2".repeat(40),
                "3".repeat(40),
                "4".repeat(40)
            )
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(
            select_object(&refs, &tag("v2")).unwrap().as_str(),
            "2".repeat(40)
        );
        assert_eq!(
            select_object(&refs, &branch("main")).unwrap().as_str(),
            "3".repeat(40)
        );
        // 꼬리 일치가 아니라 정확한 이름만.
        assert_eq!(
            select_object(&refs, &branch("ain")),
            Err(SelectorError::RefNotFound {
                kind: SelectorKind::Branch
            })
        );
        assert_eq!(
            select_object(&refs, &tag("v1")),
            Err(SelectorError::RefNotFound {
                kind: SelectorKind::Tag
            })
        );
    }

    /// 미리 fetch해 둔 mirror를 돌려주는 공급자.
    struct PreFetched {
        dir: PathBuf,
        calls: usize,
    }

    impl FetchedRefs for PreFetched {
        fn fetched_git_dir(&mut self, _: &RepositoryUrl) -> Result<PathBuf, SelectorError> {
            self.calls += 1;
            Ok(self.dir.clone())
        }
    }

    #[test]
    fn tag_branch_commit_fixture_resolve_with_system_git() {
        let tmp = TempDir::new("git-refs");
        let Some(mut git) = SystemGit::find(tmp.path()) else {
            return;
        };
        let repo = fixture_repo(&mut git, tmp.path(), "remote");
        let mirror = tmp.path().join("mirror.git");
        git.ok(tmp.path(), ["init", "-q", "--bare", "mirror.git"]);
        git.ok(
            &mirror,
            [
                "fetch",
                "-q",
                "--",
                repo.path.to_str().unwrap(),
                "+refs/heads/*:refs/heads/*",
                "+refs/tags/*:refs/tags/*",
            ],
        );
        let url = RepositoryUrl::new(repo.path.to_str().unwrap()).unwrap();
        let neutral = tmp.path().join("neutral");
        std::fs::create_dir_all(&neutral).unwrap();
        let fetched = PreFetched {
            dir: mirror.clone(),
            calls: 0,
        };
        let mut resolver = GitSelectorResolver::new(git, fetched, &neutral);

        let first: CommitId = repo.first.parse().unwrap();
        let second: CommitId = repo.second.parse().unwrap();
        // lightweight tag, annotated tag(peeled), branch 두 개, exact commit.
        assert_eq!(resolver.resolve(&url, &tag("v1")).unwrap(), first);
        assert_eq!(resolver.resolve(&url, &tag("v2")).unwrap(), second);
        assert_eq!(resolver.resolve(&url, &branch("main")).unwrap(), second);
        assert_eq!(resolver.resolve(&url, &branch("feature")).unwrap(), first);
        assert_eq!(
            resolver
                .resolve(&url, &VersionSelector::Commit(first.clone()))
                .unwrap(),
            first
        );

        // 실패: 없는 tag·branch, fetch된 저장소에 없는 commit, commit이 아닌 tag.
        assert_eq!(
            resolver.resolve(&url, &tag("v9")),
            Err(SelectorError::RefNotFound {
                kind: SelectorKind::Tag
            })
        );
        assert_eq!(
            resolver.resolve(&url, &branch("nope")),
            Err(SelectorError::RefNotFound {
                kind: SelectorKind::Branch
            })
        );
        let absent: CommitId = "0123456789abcdef0123456789abcdef01234567".parse().unwrap();
        assert_eq!(
            resolver.resolve(&url, &VersionSelector::Commit(absent)),
            Err(SelectorError::CommitNotFound)
        );
        assert_eq!(
            resolver.resolve(&url, &tag("tree-tag")),
            Err(SelectorError::NotACommit)
        );

        // ls-remote는 repository당 1회(memo), commit 선택자는 ls-remote 없이 검증만.
        assert_eq!(resolver.runner().count("ls-remote"), 1);
        assert_eq!(resolver.runner().count("cat-file"), 7);
        assert_eq!(resolver.fetched().calls, 7);
    }

    #[test]
    fn ls_remote_failure_is_reported_without_stderr() {
        let tmp = TempDir::new("git-refs-fail");
        let Some(git) = SystemGit::find(tmp.path()) else {
            return;
        };
        let missing = tmp.path().join("does-not-exist");
        let url = RepositoryUrl::new(missing.to_str().unwrap()).unwrap();
        let fetched = PreFetched {
            dir: tmp.path().to_path_buf(),
            calls: 0,
        };
        let mut resolver = GitSelectorResolver::new(git, fetched, tmp.path());
        let err = resolver.resolve(&url, &tag("v1")).unwrap_err();
        assert!(matches!(
            err,
            SelectorError::GitFailed {
                operation: "ls-remote",
                exit_code: Some(code)
            } if code != 0
        ));
        assert!(!err.to_string().contains("does-not-exist"));
        assert_eq!(resolver.fetched().calls, 0);
    }
}

//! version 선택자 해석 계약과 lock pin (XV.1.2).
//!
//! branch 선택자는 개발 편의 선언으로 허용합니다. 그러나 lockfile에는 선택자 종류와 관계없이
//! 해석된 **exact commit**만 기록합니다. [`LockPin::commit`]은 축약을 허용하지 않는 [`CommitId`]
//! 타입이라 branch·tag 이름이나 축약 hash가 lock 값으로 들어갈 수 없습니다. 실제 해석(시스템
//! Git `ls-remote`·fetch·commit 검증)은 [`SelectorResolver`]의 Git 구현(XV.2.4)이 담당합니다.

use std::fmt;

use crate::declaration::{
    CommitId, DependencyDeclaration, LibraryId, LogicalName, RepositoryUrl, SelectorKind,
    VersionSelector,
};

/// 선택자 해석 실패. Git stderr 원문은 담지 않습니다(URL이 섞일 수 있음, XV.1.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorError {
    /// tag·branch를 원격에서 찾지 못함.
    RefNotFound {
        /// 찾은 선택자 종류.
        kind: SelectorKind,
    },
    /// commit이 fetch된 저장소에 없음.
    CommitNotFound,
    /// 해석 결과가 commit object가 아님(예: tree를 가리키는 tag).
    NotACommit,
    /// Git 실행 실패.
    GitFailed {
        /// 실패한 작업 이름.
        operation: &'static str,
        /// 종료 코드(시그널 종료면 `None`).
        exit_code: Option<i32>,
    },
    /// Git 출력 해석 실패.
    MalformedOutput {
        /// 출력을 낸 작업 이름.
        operation: &'static str,
    },
}

impl fmt::Display for SelectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelectorError::RefNotFound { kind } => {
                write!(f, "원격에서 {}를 찾지 못했습니다", kind.as_str())
            }
            SelectorError::CommitNotFound => f.write_str("commit이 저장소에 없습니다"),
            SelectorError::NotACommit => f.write_str("선택자가 commit을 가리키지 않습니다"),
            SelectorError::GitFailed {
                operation,
                exit_code,
            } => match exit_code {
                Some(code) => write!(f, "Git {operation} 실패(종료 코드 {code})"),
                None => write!(f, "Git {operation} 실패(시그널 종료)"),
            },
            SelectorError::MalformedOutput { operation } => {
                write!(f, "Git {operation} 출력을 해석할 수 없습니다")
            }
        }
    }
}

impl std::error::Error for SelectorError {}

/// repository + 선택자 → exact commit 해석기.
pub trait SelectorResolver {
    /// 선택자를 exact commit으로 해석합니다.
    fn resolve(
        &mut self,
        repository: &RepositoryUrl,
        selector: &VersionSelector,
    ) -> Result<CommitId, SelectorError>;
}

impl VersionSelector {
    /// branch처럼 시간이 지나면 다른 commit으로 해석될 수 있는 개발 편의 선택자인지.
    pub fn is_floating(&self) -> bool {
        matches!(self, VersionSelector::Branch(_))
    }
}

/// lockfile에 기록할 exact pin.
///
/// 선언 선택자(`requested`)는 "선택자가 바뀌었는지" 비교용으로만 보존하고, 해석 결과는 항상
/// exact commit입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockPin {
    /// 의존성 logical name.
    pub name: LogicalName,
    /// 라이브러리 식별자.
    pub library_id: LibraryId,
    /// 선언 당시 선택자.
    pub requested: VersionSelector,
    /// 해석된 exact commit.
    pub commit: CommitId,
}

impl LockPin {
    /// 개발 편의(branch) 선언에서 나온 pin인지.
    pub fn is_floating(&self) -> bool {
        self.requested.is_floating()
    }
}

/// 선언 하나를 exact commit pin으로 해석합니다.
pub fn pin_declaration(
    declaration: &DependencyDeclaration,
    resolver: &mut dyn SelectorResolver,
) -> Result<LockPin, SelectorError> {
    let commit = resolver.resolve(&declaration.repository, &declaration.version)?;
    Ok(LockPin {
        name: declaration.name.clone(),
        library_id: declaration.library_id,
        requested: declaration.version.clone(),
        commit,
    })
}

#[cfg(test)]
pub(crate) mod fake {
    //! 테스트용 in-memory 선택자 해석기.

    use std::collections::BTreeMap;

    use super::*;

    /// `(repository, 선택자)` → commit 표. 호출 횟수를 기록합니다.
    #[derive(Default)]
    pub(crate) struct TableResolver {
        pub(crate) table: BTreeMap<(String, VersionSelector), CommitId>,
        pub(crate) calls: usize,
    }

    impl TableResolver {
        pub(crate) fn with(
            mut self,
            url: &str,
            selector: VersionSelector,
            commit: CommitId,
        ) -> Self {
            self.table.insert((url.to_string(), selector), commit);
            self
        }
    }

    impl SelectorResolver for TableResolver {
        fn resolve(
            &mut self,
            repository: &RepositoryUrl,
            selector: &VersionSelector,
        ) -> Result<CommitId, SelectorError> {
            self.calls += 1;
            if let VersionSelector::Commit(commit) = selector {
                if self
                    .table
                    .contains_key(&(repository.as_str().to_string(), selector.clone()))
                {
                    return Ok(commit.clone());
                }
                return Err(SelectorError::CommitNotFound);
            }
            self.table
                .get(&(repository.as_str().to_string(), selector.clone()))
                .cloned()
                .ok_or(SelectorError::RefNotFound {
                    kind: selector.kind(),
                })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::TableResolver;
    use super::*;
    use crate::declaration::fixtures::{commit, decl};
    use crate::declaration::RefName;

    fn branch(name: &str) -> VersionSelector {
        VersionSelector::Branch(RefName::new(name).unwrap())
    }

    #[test]
    fn branch_resolves_to_exact_commit_in_lock_pin() {
        let mut declaration = decl("ds", 1, "https://e/ds.git", "unused");
        declaration.version = branch("main");
        let head = commit(0x5a);
        let mut resolver =
            TableResolver::default().with("https://e/ds.git", branch("main"), head.clone());

        let pin = pin_declaration(&declaration, &mut resolver).expect("pin");
        assert!(pin.is_floating());
        // lock 값은 branch 이름이 아니라 exact 40자리 hash.
        assert_eq!(pin.commit, head);
        assert_eq!(pin.commit.as_str().len(), 40);
        assert!(pin.commit.as_str().bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(pin.commit.as_str(), "main");
        // 선언 선택자는 비교용으로만 보존.
        assert_eq!(pin.requested, branch("main"));

        // branch가 이동하면 새 해석은 새 exact commit(이전 lock 값은 그대로 유효한 hash).
        let moved = commit(0x6b);
        let mut later =
            TableResolver::default().with("https://e/ds.git", branch("main"), moved.clone());
        let repinned = pin_declaration(&declaration, &mut later).unwrap();
        assert_eq!(repinned.commit, moved);
        assert_ne!(repinned.commit, pin.commit);
    }

    #[test]
    fn tag_and_commit_selectors_also_pin_exact_commits() {
        let tagged = decl("ds", 1, "https://e/ds.git", "v1.0.0");
        let target = commit(0x11);
        let mut resolver = TableResolver::default()
            .with("https://e/ds.git", tagged.version.clone(), target.clone())
            .with(
                "https://e/ds.git",
                VersionSelector::Commit(target.clone()),
                target.clone(),
            );
        let pin = pin_declaration(&tagged, &mut resolver).unwrap();
        assert!(!pin.is_floating());
        assert_eq!(pin.commit, target);

        let mut exact = tagged.clone();
        exact.version = VersionSelector::Commit(target.clone());
        assert_eq!(
            pin_declaration(&exact, &mut resolver).unwrap().commit,
            target
        );
    }

    #[test]
    fn resolution_failure_produces_no_pin() {
        let mut declaration = decl("ds", 1, "https://e/ds.git", "unused");
        declaration.version = branch("gone");
        let mut resolver = TableResolver::default();
        assert_eq!(
            pin_declaration(&declaration, &mut resolver),
            Err(SelectorError::RefNotFound {
                kind: SelectorKind::Branch
            })
        );
        // 축약 hash는 CommitId로 만들 수 없어 lock 값이 될 수 없습니다.
        assert!("5a5a5a5".parse::<CommitId>().is_err());
    }
}

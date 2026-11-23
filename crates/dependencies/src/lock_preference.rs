//! lockfile가 있으면 선택자가 바뀌지 않은 의존성은 network 없이 lock commit을 씁니다 (XV.4.3, ADR-0137 결정 3).
//!
//! [`LockPreferringResolver`]는 다른 [`SelectorResolver`](시스템 Git 구현, XV.2.4)를 감쌉니다. resolver가
//! `(repository, 선택자)`를 물으면 기존 lock entry 가운데 **같은 repository 비교 키와 같은 선언 선택자**를 기록한 entry의
//! exact commit을 돌려주고, 안쪽 resolver(= `ls-remote`·fetch)를 부르지 않습니다. 선택자가 바뀌었거나(lock entry의
//! `requested`와 다름) lock에 없는 의존성만 안쪽 resolver가 해석합니다.
//!
//! - 같은 `(repository, 선택자)`를 기록한 entry들이 서로 다른 commit을 가리키면(손상된 lock) 어느 것도 고르지 않고 안쪽
//!   resolver에 맡깁니다(fail closed: 임의의 commit을 고르지 않음).
//! - update처럼 같은 선택자를 다시 해석해야 하는 경우(branch 이동 확인)는 [`LockPreferringResolver::refresh`]로 그
//!   `(repository, 선택자)`의 lock 우선을 끕니다.
//! - 이 해석기는 commit만 정합니다. 해석한 commit의 매니페스트·cache entry도 network 없이 읽으려면 호출자가 cache를 먼저
//!   보는 공급자를 씁니다(project `library_resolve`).

use std::collections::{BTreeMap, BTreeSet};

use crate::declaration::{CommitId, RepositoryUrl, VersionSelector};
use crate::lock::LockDocument;
use crate::selection::{SelectorError, SelectorResolver};

/// lock pin 표의 key: (repository 비교 키, 선언 선택자).
type PinKey = (String, VersionSelector);

/// lock commit을 먼저 쓰는 선택자 해석기.
pub struct LockPreferringResolver<S> {
    inner: S,
    pins: BTreeMap<PinKey, CommitId>,
    refreshed: BTreeSet<PinKey>,
    pinned_hits: usize,
}

impl<S: SelectorResolver> LockPreferringResolver<S> {
    /// `lock`(없으면 pin 없음)의 entry로 pin 표를 만듭니다.
    pub fn new(lock: Option<&LockDocument>, inner: S) -> LockPreferringResolver<S> {
        let mut pins: BTreeMap<PinKey, CommitId> = BTreeMap::new();
        let mut conflicting: BTreeSet<PinKey> = BTreeSet::new();
        for entry in lock.map(LockDocument::entries).unwrap_or_default() {
            let key = (entry.repository.comparison_key(), entry.requested.clone());
            match pins.get(&key) {
                Some(commit) if commit != &entry.exact_commit => {
                    conflicting.insert(key);
                }
                Some(_) => {}
                None => {
                    pins.insert(key, entry.exact_commit.clone());
                }
            }
        }
        for key in conflicting {
            pins.remove(&key);
        }
        LockPreferringResolver {
            inner,
            pins,
            refreshed: BTreeSet::new(),
            pinned_hits: 0,
        }
    }

    /// `(repository, selector)`는 lock commit을 쓰지 않고 다시 해석합니다(update).
    pub fn refresh(mut self, repository: &RepositoryUrl, selector: &VersionSelector) -> Self {
        self.refreshed
            .insert((repository.comparison_key(), selector.clone()));
        self
    }

    /// lock commit으로 답한 횟수.
    pub fn pinned_hits(&self) -> usize {
        self.pinned_hits
    }

    /// 안쪽 해석기.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// 안쪽 해석기를 돌려받습니다.
    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<S: SelectorResolver> SelectorResolver for LockPreferringResolver<S> {
    fn resolve(
        &mut self,
        repository: &RepositoryUrl,
        selector: &VersionSelector,
    ) -> Result<CommitId, SelectorError> {
        let key = (repository.comparison_key(), selector.clone());
        if !self.refreshed.contains(&key) {
            if let Some(commit) = self.pins.get(&key) {
                self.pinned_hits += 1;
                return Ok(commit.clone());
            }
        }
        self.inner.resolve(repository, selector)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::content_hash::ContentHashText;
    use crate::declaration::fixtures::{commit, library};
    use crate::declaration::{RefName, SourceKind};
    use crate::graph::DependencyPath;
    use crate::lock::LockEntry;
    use crate::node::DeclNode;

    /// 호출 횟수를 세는 해석기(항상 같은 commit).
    struct Counting {
        calls: usize,
        answer: CommitId,
    }

    impl SelectorResolver for Counting {
        fn resolve(
            &mut self,
            _: &RepositoryUrl,
            _: &VersionSelector,
        ) -> Result<CommitId, SelectorError> {
            self.calls += 1;
            Ok(self.answer.clone())
        }
    }

    fn tag(name: &str) -> VersionSelector {
        VersionSelector::Tag(RefName::new(name).unwrap())
    }

    fn entry(name: &str, url: &str, requested: VersionSelector, at: u8) -> LockEntry {
        LockEntry {
            library_id: library(1),
            source: SourceKind::Git,
            repository: RepositoryUrl::new(url).unwrap(),
            requested,
            exact_commit: commit(at),
            content_hash: format!("sha256:{}", "a".repeat(64))
                .parse::<ContentHashText>()
                .unwrap(),
            schema_version: 4,
            dependency_path: DependencyPath::from_names(vec![
                crate::declaration::LogicalName::new(name).unwrap(),
            ]),
            dependencies: vec![],
        }
    }

    fn lock(mut entries: Vec<LockEntry>) -> LockDocument {
        entries.sort_by_key(|e| {
            (
                e.library_id,
                e.exact_commit.clone(),
                e.dependency_path.clone(),
            )
        });
        LockDocument::from_node(&DeclNode::Mapping(vec![
            (
                "schema_version".to_string(),
                DeclNode::scalar(crate::lock::LOCK_SCHEMA_VERSION.to_string()),
            ),
            (
                "resolved".to_string(),
                DeclNode::Sequence(entries.iter().map(LockEntry::to_node).collect()),
            ),
        ]))
        .unwrap()
    }

    #[test]
    fn unchanged_selector_uses_lock_commit_without_inner_resolver() {
        let url = RepositoryUrl::new("https://e.com/ds.git").unwrap();
        // 같은 저장소의 다른 표기(비교 키 같음)도 같은 pin입니다.
        let other_spelling = RepositoryUrl::new("HTTPS://E.COM/ds").unwrap();
        let document = lock(vec![entry("ds", "https://e.com/ds.git", tag("v1"), 0x11)]);
        let mut resolver = LockPreferringResolver::new(
            Some(&document),
            Counting {
                calls: 0,
                answer: commit(0x22),
            },
        );
        assert_eq!(resolver.resolve(&url, &tag("v1")).unwrap(), commit(0x11));
        assert_eq!(
            resolver.resolve(&other_spelling, &tag("v1")).unwrap(),
            commit(0x11)
        );
        assert_eq!(resolver.inner().calls, 0, "lock commit은 Git을 부르지 않음");
        assert_eq!(resolver.pinned_hits(), 2);
        // 선택자가 바뀌면 안쪽 해석기가 해석합니다.
        assert_eq!(resolver.resolve(&url, &tag("v2")).unwrap(), commit(0x22));
        assert_eq!(resolver.inner().calls, 1);
        // refresh한 (저장소, 선택자)는 lock을 쓰지 않습니다.
        let mut refreshed = LockPreferringResolver::new(
            Some(&document),
            Counting {
                calls: 0,
                answer: commit(0x33),
            },
        )
        .refresh(&url, &tag("v1"));
        assert_eq!(refreshed.resolve(&url, &tag("v1")).unwrap(), commit(0x33));
        assert_eq!(refreshed.into_inner().calls, 1);
    }

    #[test]
    fn conflicting_lock_pins_and_missing_lock_fall_back_to_inner_resolver() {
        let url = RepositoryUrl::new("https://e.com/ds.git").unwrap();
        let mut entries = BTreeMap::new();
        entries.insert("a", entry("a", "https://e.com/ds.git", tag("v1"), 0x11));
        entries.insert("b", entry("b", "https://e.com/ds.git", tag("v1"), 0x12));
        let document = lock(entries.into_values().collect());
        let mut resolver = LockPreferringResolver::new(
            Some(&document),
            Counting {
                calls: 0,
                answer: commit(0x22),
            },
        );
        assert_eq!(resolver.resolve(&url, &tag("v1")).unwrap(), commit(0x22));
        assert_eq!(
            resolver.pinned_hits(),
            0,
            "서로 다른 commit pin은 고르지 않음"
        );
        let mut none = LockPreferringResolver::new(
            None,
            Counting {
                calls: 0,
                answer: commit(0x44),
            },
        );
        assert_eq!(none.resolve(&url, &tag("v1")).unwrap(), commit(0x44));
    }
}

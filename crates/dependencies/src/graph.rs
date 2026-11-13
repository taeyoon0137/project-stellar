//! 의존성 resolver 그래프 (XV.2).
//!
//! node는 [`DependencyNodeKey`](LibraryId + 해석된 exact commit)로 식별합니다. 같은 library의 여러
//! 버전은 서로 다른 node입니다(아키텍처 30.3 "여러 버전 동시 로드", 자동 단일 버전 통합 금지).

use std::fmt;

use crate::declaration::{CommitId, LibraryId};
use crate::selection::LockPin;

/// 해석된 의존성 node 식별자(XV.2.1).
///
/// 정렬은 LibraryId → commit 순서입니다. 같은 LibraryId라도 commit이 다르면 다른 key입니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DependencyNodeKey {
    /// 라이브러리 식별자.
    pub library_id: LibraryId,
    /// 해석된 exact commit.
    pub commit: CommitId,
}

impl DependencyNodeKey {
    /// key를 만듭니다.
    pub fn new(library_id: LibraryId, commit: CommitId) -> DependencyNodeKey {
        DependencyNodeKey { library_id, commit }
    }

    /// lock pin의 node key.
    pub fn of_pin(pin: &LockPin) -> DependencyNodeKey {
        DependencyNodeKey::new(pin.library_id, pin.commit.clone())
    }
}

impl fmt::Display for DependencyNodeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.library_id, self.commit)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};

    use super::*;
    use crate::declaration::fixtures::{commit, library};
    use crate::declaration::{LogicalName, RefName, VersionSelector};

    #[test]
    fn same_library_two_versions_are_distinct_keys() {
        let v1 = DependencyNodeKey::new(library(3), commit(0x01));
        let v2 = DependencyNodeKey::new(library(3), commit(0x02));
        assert_ne!(v1, v2);
        assert_eq!(v1, DependencyNodeKey::new(library(3), commit(0x01)));

        // 두 버전이 한 map에 동시에 존재.
        let mut nodes = BTreeMap::new();
        nodes.insert(v2.clone(), "v2");
        nodes.insert(v1.clone(), "v1");
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes.values().copied().collect::<Vec<_>>(), ["v1", "v2"]);
        let set: HashSet<_> = [v1.clone(), v2.clone(), v1.clone()].into_iter().collect();
        assert_eq!(set.len(), 2);

        // 정렬: LibraryId가 commit보다 우선.
        let other = DependencyNodeKey::new(library(4), commit(0x00));
        let mut keys = vec![other.clone(), v2.clone(), v1.clone()];
        keys.sort();
        assert_eq!(keys, [v1.clone(), v2, other]);

        assert_eq!(
            v1.to_string(),
            format!("{}@{}", library(3), "01".repeat(20))
        );
    }

    #[test]
    fn key_from_lock_pin() {
        let pin = LockPin {
            name: LogicalName::new("ds").unwrap(),
            library_id: library(1),
            requested: VersionSelector::Tag(RefName::new("v1").unwrap()),
            commit: commit(0x0a),
        };
        assert_eq!(
            DependencyNodeKey::of_pin(&pin),
            DependencyNodeKey::new(library(1), commit(0x0a))
        );
    }
}

//! SVG id 생성 (XX.3.3).
//!
//! id는 순회 순서 번호가 아니라 **StableId에서** 만듭니다. 그래서 같은 snapshot은 항상 같은 id가
//! 되고, 관계없는 node를 추가·삭제해도 다른 node의 id가 바뀌지 않습니다.
//!
//! | 대상 | id | 비고 |
//! |---|---|---|
//! | node 그룹 | `n-<uuid>` | `<uuid>`는 StableId 표준 소문자 hyphen 문자열 |
//! | 같은 StableId의 k번째(k ≥ 2) scene node | `n-<uuid>-d<k>` | pre-order 등장 순서. component instance처럼 한 document node가 여러 scene node를 만들 때 |
//! | scene clip 정의 | `<node id>-c<depth>` | 그 clip을 처음 쓴 node(pre-order)와 clip stack 깊이(1부터) |
//! | Inside stroke 자기 기하 clip | `<node id>-g` | node당 하나 |
//!
//! 모든 id는 [`is_safe_id`](XML `ID`·CSS 식별자 공통 부분 집합)를 만족합니다. 형식상 겹칠 수 없지만,
//! [`SvgIdAllocator`]가 발급한 id를 모두 기억해 중복 발급을 [`SvgIdError::Collision`]으로 거부합니다.

use std::collections::{HashMap, HashSet};
use std::fmt;

use stellar_core_types::ids::StableId;

use crate::svg::format::is_safe_id;

/// 검증된 SVG id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SvgId(String);

impl SvgId {
    /// id 문자열.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SvgId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// id 발급 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SvgIdError {
    /// 이미 발급한 id입니다.
    Collision {
        /// 겹친 id.
        id: String,
    },
    /// 안전 문자 규칙을 어깁니다.
    Unsafe {
        /// 문제 id(최대 128 byte로 자름).
        id: String,
    },
    /// 한 StableId의 등장 횟수가 `u32`를 넘었습니다.
    OccurrenceOverflow {
        /// StableId.
        source: StableId,
    },
}

impl fmt::Display for SvgIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SvgIdError::Collision { id } => write!(f, "SVG id {id}가 중복되었습니다"),
            SvgIdError::Unsafe { id } => write!(f, "SVG id {id}가 안전한 문자 규칙을 어깁니다"),
            SvgIdError::OccurrenceOverflow { source } => {
                write!(f, "node {source}의 scene 등장 횟수가 너무 많습니다")
            }
        }
    }
}

impl std::error::Error for SvgIdError {}

/// 문서 하나의 id 발급기.
#[derive(Debug, Default)]
pub struct SvgIdAllocator {
    issued: HashSet<String>,
    occurrences: HashMap<StableId, u32>,
}

impl SvgIdAllocator {
    /// 빈 발급기.
    pub fn new() -> SvgIdAllocator {
        SvgIdAllocator::default()
    }

    /// 발급한 id 수.
    pub fn len(&self) -> usize {
        self.issued.len()
    }

    /// 발급한 id가 없으면 참입니다.
    pub fn is_empty(&self) -> bool {
        self.issued.is_empty()
    }

    /// `id`를 검사해 발급합니다. 안전 규칙 위반·중복은 거부합니다.
    pub fn claim(&mut self, id: String) -> Result<SvgId, SvgIdError> {
        if !is_safe_id(&id) {
            let mut end = id.len().min(crate::svg::format::MAX_ID_BYTES);
            while !id.is_char_boundary(end) {
                end -= 1;
            }
            return Err(SvgIdError::Unsafe {
                id: id[..end].to_string(),
            });
        }
        if self.issued.contains(&id) {
            return Err(SvgIdError::Collision { id });
        }
        self.issued.insert(id.clone());
        Ok(SvgId(id))
    }

    /// scene node 그룹 id. 같은 StableId가 다시 나오면 등장 순서 suffix를 붙입니다.
    pub fn node(&mut self, source: StableId) -> Result<SvgId, SvgIdError> {
        let count = self.occurrences.entry(source).or_insert(0);
        *count = count
            .checked_add(1)
            .ok_or(SvgIdError::OccurrenceOverflow { source })?;
        let id = if *count == 1 {
            format!("n-{source}")
        } else {
            format!("n-{source}-d{count}")
        };
        self.claim(id)
    }

    /// `owner` node가 처음 쓴 깊이 `depth` scene clip의 정의 id.
    pub fn clip(&mut self, owner: &SvgId, depth: usize) -> Result<SvgId, SvgIdError> {
        self.claim(format!("{owner}-c{depth}"))
    }

    /// `owner` node의 Inside stroke 자기 기하 clip id.
    pub fn geometry_clip(&mut self, owner: &SvgId) -> Result<SvgId, SvgIdError> {
        self.claim(format!("{owner}-g"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures as fx;

    #[test]
    fn node_ids_come_from_stable_ids() {
        let mut ids = SvgIdAllocator::new();
        let id: StableId = "0195c530-3acd-7bc1-8414-c213f61cb965".parse().unwrap();
        let node = ids.node(id).unwrap();
        assert_eq!(node.as_str(), "n-0195c530-3acd-7bc1-8414-c213f61cb965");
        assert_eq!(
            ids.clip(&node, 2).unwrap().as_str(),
            "n-0195c530-3acd-7bc1-8414-c213f61cb965-c2"
        );
        assert_eq!(
            ids.geometry_clip(&node).unwrap().to_string(),
            "n-0195c530-3acd-7bc1-8414-c213f61cb965-g"
        );
        assert_eq!(ids.len(), 3);
        assert!(!ids.is_empty());
    }

    #[test]
    fn repeated_sources_get_occurrence_suffixes() {
        let mut ids = SvgIdAllocator::new();
        let source = fx::node_id(1);
        let first = ids.node(source).unwrap();
        let second = ids.node(source).unwrap();
        let third = ids.node(source).unwrap();
        assert_eq!(second.as_str(), format!("{first}-d2"));
        assert_eq!(third.as_str(), format!("{first}-d3"));
        // 다른 StableId는 자기 첫 id를 받습니다.
        assert_eq!(
            ids.node(fx::node_id(2)).unwrap().as_str(),
            format!("n-{}", fx::node_id(2))
        );
        // 중복 node의 파생 id도 겹치지 않습니다.
        let a = ids.clip(&first, 1).unwrap();
        let b = ids.clip(&second, 1).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn collisions_and_unsafe_ids_are_rejected() {
        let mut ids = SvgIdAllocator::new();
        let node = ids.node(fx::node_id(1)).unwrap();
        assert_eq!(
            ids.claim(node.as_str().to_string()),
            Err(SvgIdError::Collision {
                id: node.as_str().to_string()
            })
        );
        ids.clip(&node, 1).unwrap();
        assert_eq!(
            ids.clip(&node, 1),
            Err(SvgIdError::Collision {
                id: format!("{node}-c1")
            })
        );
        ids.geometry_clip(&node).unwrap();
        assert!(matches!(
            ids.geometry_clip(&node),
            Err(SvgIdError::Collision { .. })
        ));
        for bad in ["", "1n", "n\"", "n x", "n:x"] {
            assert!(matches!(
                ids.claim(bad.to_string()),
                Err(SvgIdError::Unsafe { .. })
            ));
        }
        match ids.claim("가".repeat(100)) {
            Err(SvgIdError::Unsafe { id }) => assert!(id.len() <= 128),
            other => panic!("unexpected {other:?}"),
        }
        assert!(!SvgIdError::OccurrenceOverflow {
            source: fx::node_id(1)
        }
        .to_string()
        .is_empty());
    }

    #[test]
    fn id_shapes_cannot_overlap() {
        // node id(`n-<36>` / `n-<36>-d<k>`)와 파생 id(`-c<k>`, `-g`) 형식이 서로 겹치지 않음을
        // 많은 조합으로 확인합니다.
        let mut ids = SvgIdAllocator::new();
        for n in 0..200u64 {
            let source = fx::node_id(n % 50);
            let node = ids.node(source).unwrap();
            for depth in 1..4 {
                ids.clip(&node, depth).unwrap();
            }
            ids.geometry_clip(&node).unwrap();
        }
        assert_eq!(ids.len(), 200 * 5);
    }
}

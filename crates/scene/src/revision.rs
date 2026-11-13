//! DocumentRevision → SceneRevision 매핑과 snapshot 세대 (XI.1.3).
//!
//! [`SceneRevision`]은 snapshot의 **세대 번호**입니다. 오래된 렌더 결과는 이 번호로 폐기합니다
//! (아키텍처 §9.3). 규칙은 다음과 같습니다.
//!
//! - snapshot을 하나 만들 때마다 scene revision이 정확히 1씩 증가합니다. 첫 값은 1이며 0은
//!   "아직 없음"으로 남겨 둡니다.
//! - document revision은 감소할 수 없습니다. 같은 document revision에서 snapshot을 여러 번
//!   만들 수 있습니다(예: 비동기 shaping·decode 결과 반영). 이때 scene revision은 달라지지만
//!   내용이 같으면 [`SceneSnapshot::semantic_hash`]는 같습니다.
//! - 매핑 이력은 보관하지 않습니다. 각 snapshot이 자기 [`SceneStamp`](두 revision)를 들고
//!   있으므로 별도 표 없이 역추적할 수 있습니다.

use std::fmt;
use std::sync::Arc;

use stellar_core_types::revision::{DocumentRevision, SceneRevision};

use crate::node::{SceneEditError, SceneNode, SceneTreeError};
use crate::snapshot::SceneSnapshot;

/// snapshot 하나에 붙는 revision 쌍.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SceneStamp {
    /// 원본 document revision.
    pub document: DocumentRevision,
    /// scene revision(세대 번호).
    pub scene: SceneRevision,
}

/// revision 할당 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneRevisionError {
    /// document revision이 이전 snapshot보다 작습니다.
    DocumentRevisionRegressed {
        /// 마지막으로 매핑한 document revision.
        last: DocumentRevision,
        /// 요청한 document revision.
        requested: DocumentRevision,
    },
    /// scene revision 공간(`u64`)을 다 썼습니다.
    Exhausted,
}

impl fmt::Display for SceneRevisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SceneRevisionError::DocumentRevisionRegressed { last, requested } => write!(
                f,
                "document revision이 감소했습니다(마지막 {}, 요청 {})",
                last.0, requested.0
            ),
            SceneRevisionError::Exhausted => f.write_str("scene revision을 더 할당할 수 없습니다"),
        }
    }
}

impl std::error::Error for SceneRevisionError {}

/// snapshot 생성 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneGenerateError {
    /// revision 할당 실패.
    Revision(SceneRevisionError),
    /// scene tree 불변식 위반.
    Tree(SceneTreeError),
    /// 구조 공유 갱신 실패(XI.1.4).
    Edit(SceneEditError),
}

impl fmt::Display for SceneGenerateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SceneGenerateError::Revision(error) => write!(f, "scene revision 오류: {error}"),
            SceneGenerateError::Tree(error) => write!(f, "scene tree 오류: {error}"),
            SceneGenerateError::Edit(error) => write!(f, "scene 갱신 오류: {error}"),
        }
    }
}

impl std::error::Error for SceneGenerateError {}

/// document revision을 scene revision으로 매핑하고 snapshot 세대를 발급합니다.
///
/// document thread가 하나를 소유합니다(유일한 snapshot 작성자).
#[derive(Debug, Clone, Default)]
pub struct SceneRevisionMap {
    last: Option<SceneStamp>,
}

impl SceneRevisionMap {
    /// 아직 snapshot이 없는 매핑.
    pub const fn new() -> SceneRevisionMap {
        SceneRevisionMap { last: None }
    }

    /// 마지막으로 발급한 stamp.
    pub const fn last(&self) -> Option<SceneStamp> {
        self.last
    }

    /// 다음 stamp를 계산합니다(상태는 바꾸지 않음).
    pub fn peek(&self, document: DocumentRevision) -> Result<SceneStamp, SceneRevisionError> {
        match self.last {
            None => Ok(SceneStamp {
                document,
                scene: SceneRevision(1),
            }),
            Some(last) => {
                if document < last.document {
                    return Err(SceneRevisionError::DocumentRevisionRegressed {
                        last: last.document,
                        requested: document,
                    });
                }
                let scene = last
                    .scene
                    .0
                    .checked_add(1)
                    .ok_or(SceneRevisionError::Exhausted)?;
                Ok(SceneStamp {
                    document,
                    scene: SceneRevision(scene),
                })
            }
        }
    }

    /// 다음 stamp를 발급합니다.
    pub fn next(&mut self, document: DocumentRevision) -> Result<SceneStamp, SceneRevisionError> {
        let stamp = self.peek(document)?;
        self.last = Some(stamp);
        Ok(stamp)
    }

    /// `document` revision에서 만든 최상위 node로 새 세대 snapshot을 만듭니다.
    ///
    /// tree가 불변식을 어기면 revision을 소비하지 않고 오류를 반환합니다.
    pub fn generate(
        &mut self,
        document: DocumentRevision,
        roots: Vec<Arc<SceneNode>>,
    ) -> Result<SceneSnapshot, SceneGenerateError> {
        let stamp = self.peek(document).map_err(SceneGenerateError::Revision)?;
        let snapshot = SceneSnapshot::new(stamp.document, stamp.scene, roots)
            .map_err(SceneGenerateError::Tree)?;
        self.last = Some(stamp);
        Ok(snapshot)
    }

    /// `base`의 `path` 위치 node만 바꾼 새 세대 snapshot을 만듭니다(XI.1.4, 구조 공유).
    ///
    /// 갱신이 실패하면 revision을 소비하지 않습니다.
    pub fn derive(
        &mut self,
        base: &SceneSnapshot,
        document: DocumentRevision,
        path: &[usize],
        node: Arc<SceneNode>,
    ) -> Result<SceneSnapshot, SceneGenerateError> {
        let stamp = self.peek(document).map_err(SceneGenerateError::Revision)?;
        let snapshot = base
            .replace_node(stamp.document, stamp.scene, path, node)
            .map_err(SceneGenerateError::Edit)?;
        self.last = Some(stamp);
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec;
    use crate::fixtures::{self, leaf, FixtureConfig};
    use crate::node::ZOrder;

    fn roots_of(snapshot: &SceneSnapshot) -> Vec<Arc<SceneNode>> {
        snapshot.roots().cloned().collect()
    }

    #[test]
    fn scene_revisions_increase_and_document_revisions_never_regress() {
        let mut map = SceneRevisionMap::new();
        assert_eq!(map.last(), None);
        let a = map.next(DocumentRevision(10)).unwrap();
        assert_eq!(a.scene, SceneRevision(1));
        // 같은 document revision에서 다시 생성: scene revision만 증가.
        let b = map.next(DocumentRevision(10)).unwrap();
        assert_eq!(
            b,
            SceneStamp {
                document: DocumentRevision(10),
                scene: SceneRevision(2)
            }
        );
        let c = map.next(DocumentRevision(12)).unwrap();
        assert_eq!(c.scene, SceneRevision(3));
        assert_eq!(
            map.next(DocumentRevision(11)),
            Err(SceneRevisionError::DocumentRevisionRegressed {
                last: DocumentRevision(12),
                requested: DocumentRevision(11)
            })
        );
        // 실패는 상태를 바꾸지 않습니다.
        assert_eq!(map.last(), Some(c));

        let mut full = SceneRevisionMap {
            last: Some(SceneStamp {
                document: DocumentRevision(1),
                scene: SceneRevision(u64::MAX),
            }),
        };
        assert_eq!(
            full.next(DocumentRevision(1)),
            Err(SceneRevisionError::Exhausted)
        );
    }

    #[test]
    fn same_document_revision_yields_same_semantic_hash() {
        // 검증 기준(XI.1.3): 동일 document revision에서 동일 scene semantic hash.
        let config = FixtureConfig {
            seed: 0x1_3,
            roots: 3,
            children: 3,
            depth: 3,
        };
        let mut map = SceneRevisionMap::new();
        // 같은 document 상태(같은 fixture 입력)에서 독립적으로 두 번 생성합니다.
        let first = map
            .generate(DocumentRevision(5), roots_of(&fixtures::generate(config)))
            .unwrap();
        let second = map
            .generate(DocumentRevision(5), roots_of(&fixtures::generate(config)))
            .unwrap();
        assert_eq!(first.scene_revision(), SceneRevision(1));
        assert_eq!(second.scene_revision(), SceneRevision(2));
        assert_eq!(first.document_revision(), second.document_revision());
        assert_eq!(first.semantic_hash(), second.semantic_hash());

        // Arc 공유 구조가 달라도(decode로 모든 공유가 끊긴 복제본) hash는 같습니다.
        let unshared = codec::decode(&codec::encode(&first)).unwrap();
        assert!(!Arc::ptr_eq(
            unshared.root(0).unwrap(),
            first.root(0).unwrap()
        ));
        assert_eq!(unshared.semantic_hash(), first.semantic_hash());

        // 내용이 바뀌면 hash가 바뀝니다.
        let changed = map
            .generate(
                DocumentRevision(6),
                roots_of(&fixtures::generate(FixtureConfig { seed: 9, ..config })),
            )
            .unwrap();
        assert_ne!(changed.semantic_hash(), first.semantic_hash());
    }

    #[test]
    fn invalid_tree_does_not_consume_a_revision() {
        let mut map = SceneRevisionMap::new();
        map.generate(DocumentRevision(1), vec![leaf(1, 1, 1.0, 1.0)])
            .unwrap();
        let error = map
            .generate(
                DocumentRevision(2),
                vec![leaf(1, 5, 1.0, 1.0), leaf(2, 4, 1.0, 1.0)],
            )
            .unwrap_err();
        assert_eq!(
            error,
            SceneGenerateError::Tree(SceneTreeError::ZOrderNotIncreasing {
                previous: ZOrder(5),
                next: ZOrder(4)
            })
        );
        assert_eq!(map.last().unwrap().scene, SceneRevision(1));
        assert_eq!(
            map.generate(DocumentRevision(0), Vec::new()).unwrap_err(),
            SceneGenerateError::Revision(SceneRevisionError::DocumentRevisionRegressed {
                last: DocumentRevision(1),
                requested: DocumentRevision(0)
            })
        );
        let next = map.generate(DocumentRevision(2), Vec::new()).unwrap();
        assert_eq!(next.scene_revision(), SceneRevision(2));
    }
}

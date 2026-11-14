//! Semantic Status 분류 (XIV.4.1~).
//!
//! Git 변경은 편집 이벤트가 아니라 세 snapshot의 의미 차이입니다(아키텍처 §26.1).
//!
//! | 영역 | 비교 | 뜻 |
//! |---|---|---|
//! | [`StatusArea::Staged`] | HEAD → Index | 다음 commit에 들어갈 변경 |
//! | [`StatusArea::Unstaged`] | Index → WorkingTree | 디스크에 저장했지만 아직 stage하지 않은 변경 |
//!
//! - 기준은 HEAD와 Index이고, 둘 다 Git이 가진 상태입니다. Git index가 staged 상태의 유일한
//!   원본이므로(§26.3) staged 목록은 Index snapshot에서만 계산합니다. UI cache를 원본으로 쓰지
//!   않습니다.
//! - WorkingTree snapshot은 디스크에 **저장된** 파일 기준입니다(XIV.2.7). 편집기의 저장 대기 상태는
//!   입력이 아닙니다.
//! - HEAD가 없으면(unborn branch) 호출자가 빈 snapshot을 넘깁니다. 그러면 Index의 모든 객체가
//!   staged 추가입니다.
//! - 두 영역은 각각 [`diff`]의 결과라 같은 결정론(XIV.3.8)과 적용 가능성(XIV.3.9)을 가집니다:
//!   `apply(staged, HEAD) == Index`, `apply(unstaged, Index) == WorkingTree`.

use crate::diff::{diff, SemanticDiff};
use crate::snapshot::SemanticSnapshot;

/// status 영역.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StatusArea {
    /// HEAD → Index.
    Staged,
    /// Index → WorkingTree.
    Unstaged,
}

impl StatusArea {
    /// 안정 numeric code.
    pub fn code(self) -> u8 {
        match self {
            StatusArea::Staged => 1,
            StatusArea::Unstaged => 2,
        }
    }

    /// code에서 복원합니다.
    pub fn from_code(code: u8) -> Option<StatusArea> {
        match code {
            1 => Some(StatusArea::Staged),
            2 => Some(StatusArea::Unstaged),
            _ => None,
        }
    }
}

/// status 계산에 쓰는 세 snapshot 중 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SnapshotSide {
    /// HEAD commit.
    Head,
    /// Git index(stage 0).
    Index,
    /// 디스크에 저장된 작업 트리.
    WorkingTree,
}

/// 세 snapshot의 semantic status.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SemanticStatus {
    staged: SemanticDiff,
    unstaged: SemanticDiff,
}

impl SemanticStatus {
    /// HEAD → Index를 staged, Index → WorkingTree를 unstaged로 분류합니다.
    pub fn classify(
        head: &SemanticSnapshot,
        index: &SemanticSnapshot,
        working: &SemanticSnapshot,
    ) -> SemanticStatus {
        SemanticStatus {
            staged: diff(head, index),
            unstaged: diff(index, working),
        }
    }

    /// staged 변경(HEAD → Index, canonical 순서).
    pub fn staged(&self) -> &SemanticDiff {
        &self.staged
    }

    /// unstaged 변경(Index → WorkingTree, canonical 순서).
    pub fn unstaged(&self) -> &SemanticDiff {
        &self.unstaged
    }

    /// 영역의 변경.
    pub fn area(&self, area: StatusArea) -> &SemanticDiff {
        match area {
            StatusArea::Staged => &self.staged,
            StatusArea::Unstaged => &self.unstaged,
        }
    }

    /// 두 영역 모두 변경이 없는지.
    pub fn is_clean(&self) -> bool {
        self.staged.is_empty() && self.unstaged.is_empty()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::apply::apply;
    use crate::build::tests::{build, head_records, node, rect, sid};
    use crate::change::{SemanticChange, SemanticChangeKind};
    use crate::keys;
    use stellar_core_types::ids::StableId;
    use stellar_core_types::property::PropertyPath;
    use stellar_core_types::value::{Rect, Size2, Vec2};
    use stellar_document::record_codec::SubtreeRecords;
    use stellar_document::records::{DocumentRecords, NodePayload};
    use stellar_document::shape::{EllipseParams, ShapeGeometry};

    /// Git blob materialize 경로 모사: View subtree chunk bytes로 직렬화 후 다시 읽습니다.
    pub(crate) fn through_chunks(records: &DocumentRecords) -> DocumentRecords {
        let subtrees = records
            .view_subtrees()
            .unwrap()
            .into_iter()
            .map(|(_, subtree)| SubtreeRecords::decode(&subtree.encode()).unwrap())
            .collect();
        DocumentRecords::from_subtrees(records.definition.clone(), subtrees)
    }

    /// 부분 staged fixture.
    ///
    /// - HEAD: [`head_records`](frame 10 아래 rect 11·image 12·fragment 13).
    /// - Index: rect 11 크기 변경 + ellipse 14 추가(staged).
    /// - WorkingTree: Index에서 rect 11 숨김 + fragment 13 삭제(unstaged). rect 11은 같은 chunk의 같은
    ///   객체에서 한 속성은 staged, 다른 속성은 unstaged입니다.
    pub(crate) fn partial_staged() -> [DocumentRecords; 3] {
        let head = head_records();
        let mut index = head.clone();
        index.nodes[1].payload = rect(30.0, 20.0);
        index.nodes.push(node(
            14,
            Some(10),
            0xE0,
            "dot",
            NodePayload::Shape(ShapeGeometry::Ellipse(EllipseParams {
                bounds: Rect::new(Vec2::new(0.0, 0.0).unwrap(), Size2::new(4.0, 4.0).unwrap()),
            })),
        ));
        let mut working = index.clone();
        working.nodes[1].visible = false;
        working.nodes.retain(|n| n.id != sid(13));
        [head, index, working]
    }

    /// 변경 요약: (종류, 객체, 속성 key).
    pub(crate) fn summary(
        changes: &[SemanticChange],
    ) -> Vec<(SemanticChangeKind, StableId, Option<u32>)> {
        changes
            .iter()
            .map(|c| {
                (
                    c.kind(),
                    c.object_id(),
                    c.property_path()
                        .and_then(|p| p.keys().first())
                        .map(|k| k.0),
                )
            })
            .collect()
    }

    #[test]
    fn partial_staged_fixture_is_classified_exactly() {
        let [head, index, working] = partial_staged();
        let snapshots: Vec<SemanticSnapshot> = [&head, &index, &working]
            .into_iter()
            .map(|records| {
                let snapshot = build(&through_chunks(records));
                assert_eq!(snapshot, build(records), "chunk 경유 snapshot 동일");
                snapshot
            })
            .collect();
        let status = SemanticStatus::classify(&snapshots[0], &snapshots[1], &snapshots[2]);

        assert_eq!(
            summary(status.staged()),
            vec![
                (
                    SemanticChangeKind::SetProperty,
                    sid(11),
                    Some(keys::RECT_SIZE.0)
                ),
                (SemanticChangeKind::AddObject, sid(14), None),
            ]
        );
        assert_eq!(
            summary(status.unstaged()),
            vec![
                (
                    SemanticChangeKind::SetProperty,
                    sid(11),
                    Some(keys::VISIBLE.0)
                ),
                (SemanticChangeKind::RemoveObject, sid(13), None),
            ]
        );
        assert_eq!(status.area(StatusArea::Staged), status.staged());
        assert_eq!(status.area(StatusArea::Unstaged), status.unstaged());
        assert!(!status.is_clean());

        // 각 영역은 자기 기준 snapshot에 적용하면 다음 snapshot을 재현합니다.
        assert_eq!(apply(&snapshots[0], status.staged()).unwrap(), snapshots[1]);
        assert_eq!(
            apply(&snapshots[1], status.unstaged()).unwrap(),
            snapshots[2]
        );

        // staged 값은 Index 기준: rect 11 크기의 이후 값이 Index 값이고, unstaged 쪽에는 없습니다.
        let size = PropertyPath::with_keys(sid(11), vec![keys::RECT_SIZE]);
        let staged_size = status
            .staged()
            .iter()
            .find_map(|c| match c {
                SemanticChange::SetProperty(p) if p.path == size => p.new.clone(),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            Some(&staged_size),
            snapshots[1].get(sid(11)).unwrap().property(&size)
        );
        assert!(status
            .unstaged()
            .iter()
            .all(|c| c.property_path() != Some(&size)));
    }

    #[test]
    fn unborn_head_makes_every_index_object_staged_and_clean_tree_is_clean() {
        let [_, index, _] = partial_staged();
        let index = build(&index);
        let status = SemanticStatus::classify(&SemanticSnapshot::new(), &index, &index);
        assert_eq!(status.staged().len(), index.len());
        assert!(status
            .staged()
            .iter()
            .all(|c| c.kind() == SemanticChangeKind::AddObject));
        assert!(status.unstaged().is_empty());

        let clean = SemanticStatus::classify(&index, &index, &index);
        assert!(clean.is_clean());
        assert_eq!(clean, SemanticStatus::default());
    }

    #[test]
    fn area_codes_round_trip() {
        for area in [StatusArea::Staged, StatusArea::Unstaged] {
            assert_eq!(StatusArea::from_code(area.code()), Some(area));
        }
        assert_eq!(StatusArea::from_code(0), None);
        assert_eq!(StatusArea::from_code(3), None);
    }
}

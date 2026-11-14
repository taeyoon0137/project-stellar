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
//!
//! **세 값 보존(XIV.4.2)**: 같은 `StableId + PropertyPath`를 staged와 unstaged에서 연속으로 바꾸면
//! (예: HEAD=1, Index=2, WorkingTree=3) 두 영역의 변경만으로는 한 영역에 한 쌍씩만 보입니다. 그래서
//! 어느 영역에서든 바뀐 모든 속성 경로에 대해 HEAD·Index·WorkingTree 세 값을 [`PropertyLayers`]로
//! 함께 보존합니다. 값은 snapshot에서 직접 읽으므로 객체 추가·삭제와 겹쳐도(객체 없음 =
//! [`LayerValue::NoObject`]) 정확합니다.
//!
//! **untracked Stellar 파일(XIV.4.3)**: Git이 추적하지 않는 새 chunk·asset 파일은 WorkingTree
//! snapshot에는 들어가고(XIV.2.7) Index snapshot에는 없습니다. 그 안의 객체는 unstaged
//! AddObject/AddToken이며 [`SemanticStatus::working_tree_additions`]로 모아 봅니다. 파일 단위
//! 표시(`Untracked` file change)는 git-porcelain `file_status`가 담당합니다.
//!
//! **손상된 snapshot(XIV.4.5)**: 어느 snapshot이든 읽지 못하면(예: 손상된 WorkingTree chunk) 일부
//! 영역만 계산해 내놓지 않고 [`SemanticStatus::try_classify`]가 실패한 쪽과 원인을 모두
//! [`StatusFailure`]로 돌려줍니다. raw Git status는 이와 별개로 계속 제공되며(git-porcelain
//! `project_status`), 이 모듈은 snapshot을 고치지 않습니다.

use std::collections::BTreeMap;

use stellar_core_types::property::PropertyPath;

use crate::change::{SemanticChange, SemanticChangeKind};
use crate::diff::{diff, SemanticDiff};
use crate::snapshot::SemanticSnapshot;
use crate::value::SemanticValue;

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

/// 한 snapshot에서 본 속성 상태(XIV.4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerValue {
    /// 속성을 가진 객체가 그 snapshot에 없음.
    NoObject,
    /// 객체는 있으나 속성이 없음.
    Unset,
    /// 속성 값.
    Value(SemanticValue),
}

impl LayerValue {
    /// snapshot에서 경로의 상태를 읽습니다.
    pub fn read(snapshot: &SemanticSnapshot, path: &PropertyPath) -> LayerValue {
        match snapshot.get(path.root()) {
            None => LayerValue::NoObject,
            Some(object) => match object.property(path) {
                None => LayerValue::Unset,
                Some(value) => LayerValue::Value(value.clone()),
            },
        }
    }

    /// 값(없으면 `None`).
    pub fn value(&self) -> Option<&SemanticValue> {
        match self {
            LayerValue::Value(value) => Some(value),
            _ => None,
        }
    }

    /// canonical 비교(float bit 단위, XIV.3.4).
    pub fn canonical_eq(&self, other: &LayerValue) -> bool {
        match (self, other) {
            (LayerValue::Value(a), LayerValue::Value(b)) => a.canonical_eq(b),
            (LayerValue::NoObject, LayerValue::NoObject)
            | (LayerValue::Unset, LayerValue::Unset) => true,
            _ => false,
        }
    }
}

/// 속성 하나의 HEAD·Index·WorkingTree 세 값(XIV.4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyLayers {
    /// 속성 경로.
    pub path: PropertyPath,
    /// HEAD 값(base).
    pub head: LayerValue,
    /// Index 값(staged).
    pub index: LayerValue,
    /// WorkingTree 값.
    pub working: LayerValue,
}

impl PropertyLayers {
    /// 세 snapshot에서 경로의 값을 읽습니다.
    pub fn read(
        path: &PropertyPath,
        head: &SemanticSnapshot,
        index: &SemanticSnapshot,
        working: &SemanticSnapshot,
    ) -> PropertyLayers {
        PropertyLayers {
            path: path.clone(),
            head: LayerValue::read(head, path),
            index: LayerValue::read(index, path),
            working: LayerValue::read(working, path),
        }
    }

    /// 한 쪽 값.
    pub fn side(&self, side: SnapshotSide) -> &LayerValue {
        match side {
            SnapshotSide::Head => &self.head,
            SnapshotSide::Index => &self.index,
            SnapshotSide::WorkingTree => &self.working,
        }
    }

    /// HEAD와 Index가 다른지(staged 변경).
    pub fn is_staged(&self) -> bool {
        !self.head.canonical_eq(&self.index)
    }

    /// Index와 WorkingTree가 다른지(unstaged 변경).
    pub fn is_unstaged(&self) -> bool {
        !self.index.canonical_eq(&self.working)
    }

    /// staged와 unstaged에서 연속 수정됐는지.
    pub fn is_layered(&self) -> bool {
        self.is_staged() && self.is_unstaged()
    }
}

/// semantic status 계산 실패(XIV.4.5): 읽지 못한 snapshot 쪽과 원인.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusFailure<E> {
    /// 실패한 쪽과 원인(Head → Index → WorkingTree 순, 하나 이상).
    pub failures: Vec<(SnapshotSide, E)>,
}

impl<E> StatusFailure<E> {
    /// 그 쪽이 실패했는지.
    pub fn failed(&self, side: SnapshotSide) -> bool {
        self.failures.iter().any(|(s, _)| *s == side)
    }
}

/// 세 snapshot의 semantic status.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SemanticStatus {
    staged: SemanticDiff,
    unstaged: SemanticDiff,
    properties: BTreeMap<PropertyPath, PropertyLayers>,
}

impl SemanticStatus {
    /// 세 snapshot 읽기 결과로 status를 분류합니다(XIV.4.5).
    ///
    /// 하나라도 실패하면 일부 영역만 계산하지 않고 실패한 모든 쪽을 돌려줍니다. 손상을 고치거나
    /// 빈 snapshot으로 대체하지 않습니다.
    pub fn try_classify<E>(
        head: Result<&SemanticSnapshot, E>,
        index: Result<&SemanticSnapshot, E>,
        working: Result<&SemanticSnapshot, E>,
    ) -> Result<SemanticStatus, StatusFailure<E>> {
        match (head, index, working) {
            (Ok(head), Ok(index), Ok(working)) => {
                Ok(SemanticStatus::classify(head, index, working))
            }
            (head, index, working) => {
                let failures = [
                    (SnapshotSide::Head, head.err()),
                    (SnapshotSide::Index, index.err()),
                    (SnapshotSide::WorkingTree, working.err()),
                ]
                .into_iter()
                .filter_map(|(side, error)| error.map(|e| (side, e)))
                .collect();
                Err(StatusFailure { failures })
            }
        }
    }

    /// HEAD → Index를 staged, Index → WorkingTree를 unstaged로 분류합니다. 어느 영역에서든 바뀐
    /// 속성 경로는 세 값을 함께 보존합니다(XIV.4.2).
    pub fn classify(
        head: &SemanticSnapshot,
        index: &SemanticSnapshot,
        working: &SemanticSnapshot,
    ) -> SemanticStatus {
        let staged = diff(head, index);
        let unstaged = diff(index, working);
        let mut properties = BTreeMap::new();
        for change in staged.iter().chain(unstaged.iter()) {
            if let Some(path) = change.property_path() {
                if !properties.contains_key(path) {
                    let layers = PropertyLayers::read(path, head, index, working);
                    properties.insert(path.clone(), layers);
                }
            }
        }
        SemanticStatus {
            staged,
            unstaged,
            properties,
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

    /// 어느 영역에서든 속성 변경이 있는 경로의 세 값(경로 오름차순).
    pub fn properties(&self) -> impl Iterator<Item = &PropertyLayers> + '_ {
        self.properties.values()
    }

    /// 경로의 세 값. 어느 영역에서도 그 경로의 속성 변경이 없으면 `None`.
    pub fn property(&self, path: &PropertyPath) -> Option<&PropertyLayers> {
        self.properties.get(path)
    }

    /// staged와 unstaged에서 연속 수정된 속성(경로 오름차순).
    pub fn layered_properties(&self) -> impl Iterator<Item = &PropertyLayers> + '_ {
        self.properties.values().filter(|p| p.is_layered())
    }

    /// WorkingTree에만 있고 Index에는 없는 객체의 추가(unstaged AddObject·AddToken, canonical 순서).
    ///
    /// untracked Stellar 파일의 객체가 여기에 나옵니다(XIV.4.3). 같은 StableId를 다른 종류로 바꾼
    /// 교체(삭제 + 추가)의 추가 쪽은 Index에도 객체가 있으므로 포함하지 않습니다.
    pub fn working_tree_additions(&self) -> impl Iterator<Item = &SemanticChange> + '_ {
        let removed: std::collections::BTreeSet<_> = self
            .unstaged
            .iter()
            .filter(|c| {
                matches!(
                    c.kind(),
                    SemanticChangeKind::RemoveObject | SemanticChangeKind::RemoveToken
                )
            })
            .map(SemanticChange::object_id)
            .collect();
        self.unstaged.iter().filter(move |c| {
            matches!(
                c.kind(),
                SemanticChangeKind::AddObject | SemanticChangeKind::AddToken
            ) && !removed.contains(&c.object_id())
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::apply::apply;
    use crate::build::tests::{build, head_records, node, rect, sid};
    use crate::build::{AssetEntry, SnapshotBuilder};
    use crate::keys;
    use crate::snapshot::{ObjectKind, Origin, SemanticObject};
    use stellar_core_types::ids::StableId;
    use stellar_core_types::property::PropertyPath;
    use stellar_core_types::reference::AssetRef;
    use stellar_core_types::value::{FiniteF32, Rect, Size2, Vec2};
    use stellar_document::definitions::{PageDefinition, ViewDefinition};
    use stellar_document::frame::FramePayload;
    use stellar_document::image::ImagePayload;
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

    fn width_layers(widths: [f32; 3]) -> SemanticStatus {
        let [head, index, working] = widths.map(|w| {
            let mut records = head_records();
            records.nodes[1].payload = rect(w, 20.0);
            build(&records)
        });
        SemanticStatus::classify(&head, &index, &working)
    }

    fn width(w: f32) -> LayerValue {
        LayerValue::Value(SemanticValue::Floats(vec![
            FiniteF32::new(w).unwrap(),
            FiniteF32::new(20.0).unwrap(),
        ]))
    }

    #[test]
    fn same_property_staged_then_modified_keeps_three_values() {
        // H=1, I=2, W=3.
        let status = width_layers([1.0, 2.0, 3.0]);
        let size = PropertyPath::with_keys(sid(11), vec![keys::RECT_SIZE]);
        let layers = status.property(&size).unwrap();
        assert_eq!(layers.head, width(1.0));
        assert_eq!(layers.index, width(2.0));
        assert_eq!(layers.working, width(3.0));
        assert_eq!(layers.side(SnapshotSide::Head), &width(1.0));
        assert_eq!(layers.side(SnapshotSide::Index), &width(2.0));
        assert_eq!(layers.side(SnapshotSide::WorkingTree), &width(3.0));
        assert!(layers.is_staged() && layers.is_unstaged() && layers.is_layered());
        let layered: Vec<&PropertyLayers> = status.layered_properties().collect();
        assert_eq!(layered, vec![layers]);
        // 두 영역에 같은 경로의 변경이 하나씩: staged 1→2, unstaged 2→3.
        let values = |changes: &SemanticDiff| {
            changes
                .iter()
                .filter_map(|c| match c {
                    SemanticChange::SetProperty(p) if p.path == size => {
                        Some((p.old.clone(), p.new.clone()))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let pair = |a: f32, b: f32| (width(a).value().cloned(), width(b).value().cloned());
        assert_eq!(values(status.staged()), vec![pair(1.0, 2.0)]);
        assert_eq!(values(status.unstaged()), vec![pair(2.0, 3.0)]);

        // staged만(1,2,2)·unstaged만(1,1,3)은 한 영역만 표시합니다.
        let staged_only = width_layers([1.0, 2.0, 2.0]);
        let layers = staged_only.property(&size).unwrap();
        assert!(layers.is_staged() && !layers.is_unstaged());
        assert_eq!(layers.working, width(2.0));
        assert_eq!(staged_only.layered_properties().count(), 0);
        let unstaged_only = width_layers([1.0, 1.0, 3.0]);
        let layers = unstaged_only.property(&size).unwrap();
        assert!(!layers.is_staged() && layers.is_unstaged());
        assert_eq!((&layers.head, &layers.index), (&width(1.0), &width(1.0)));
        // HEAD 값으로 되돌린 경우(1,2,1)도 세 값 모두 보존합니다.
        let reverted = width_layers([1.0, 2.0, 1.0]);
        let layers = reverted.property(&size).unwrap();
        assert!(layers.is_layered());
        assert_eq!(layers.working, width(1.0));
        // 바뀌지 않은 경로는 없습니다.
        let name = PropertyPath::with_keys(sid(11), vec![keys::DISPLAY_NAME]);
        assert_eq!(reverted.property(&name), None);
    }

    #[test]
    fn layers_cover_object_addition_and_removal() {
        // 부분 staged fixture: 14는 staged 추가, 13은 unstaged 삭제, 11은 영역별 다른 속성.
        let [head, index, working] = partial_staged().map(|r| build(&r));
        let status = SemanticStatus::classify(&head, &index, &working);
        let size = PropertyPath::with_keys(sid(11), vec![keys::RECT_SIZE]);
        let visible = PropertyPath::with_keys(sid(11), vec![keys::VISIBLE]);
        let paths: Vec<&PropertyPath> = status.properties().map(|p| &p.path).collect();
        assert_eq!(paths, vec![&visible, &size]);
        let v = status.property(&visible).unwrap();
        assert_eq!(v.head, LayerValue::Value(SemanticValue::Bool(true)));
        assert_eq!(v.index, LayerValue::Value(SemanticValue::Bool(true)));
        assert_eq!(v.working, LayerValue::Value(SemanticValue::Bool(false)));

        // 추가된 객체의 속성을 WorkingTree에서 다시 바꾸면 HEAD 쪽은 객체 없음입니다.
        let mut later = working.clone();
        let mut dot = later.remove(sid(14)).unwrap();
        dot.set_property(
            dot.path(&[keys::DISPLAY_NAME]),
            SemanticValue::Text("점".into()),
        )
        .unwrap();
        later.insert(dot).unwrap();
        let status = SemanticStatus::classify(&head, &index, &later);
        let name = PropertyPath::with_keys(sid(14), vec![keys::DISPLAY_NAME]);
        let layers = status.property(&name).unwrap();
        assert_eq!(layers.head, LayerValue::NoObject);
        assert_eq!(
            layers.index,
            LayerValue::Value(SemanticValue::Text("dot".into()))
        );
        assert_eq!(
            layers.working,
            LayerValue::Value(SemanticValue::Text("점".into()))
        );
        assert!(layers.is_layered());
        let unset = PropertyPath::with_keys(sid(11), vec![keys::IMAGE_SIZE]);
        assert_eq!(LayerValue::read(&head, &unset), LayerValue::Unset);
        assert!(LayerValue::Unset.canonical_eq(&LayerValue::Unset));
        assert!(!LayerValue::Unset.canonical_eq(&LayerValue::NoObject));
    }

    #[test]
    fn untracked_chunk_and_asset_objects_are_working_tree_additions() {
        // Index: HEAD 그대로. WorkingTree: 새 View subtree(새 chunk 파일: 페이지 4·View 5·frame 20·
        // rect 21)와 untracked asset 91, 그 asset을 쓰는 image 12 참조 변경.
        let index = head_records();
        let mut working = index.clone();
        let mut page = PageDefinition::new(sid(4), "새 페이지".into());
        page.add_view(ViewDefinition::new(sid(5), "새 View".into(), sid(20)))
            .unwrap();
        working.definition.add_page(page).unwrap();
        working.nodes.push(node(
            20,
            None,
            0x80,
            "new-root",
            NodePayload::Frame(FramePayload::default()),
        ));
        working
            .nodes
            .push(node(21, Some(20), 0x80, "new-box", rect(5.0, 5.0)));
        working.nodes[2].payload = NodePayload::Image(ImagePayload::new(
            AssetRef::new(sid(91)),
            Size2::new(64.0, 64.0).unwrap(),
        ));
        // 새 View subtree는 별도 chunk로 직렬화됩니다.
        assert_eq!(working.view_subtrees().unwrap().len(), 2);
        let working = through_chunks(&working);

        let snapshot = |records: &DocumentRecords, assets: &[u64]| {
            let mut builder = SnapshotBuilder::new();
            builder.add_document(records).unwrap();
            for id in assets {
                builder
                    .add_asset(&AssetEntry {
                        id: sid(*id),
                        content_hash: vec![*id as u8; 32],
                        media_type: "image/png".into(),
                    })
                    .unwrap();
            }
            builder.finish()
        };
        let head = snapshot(&index, &[90]);
        let index_snapshot = snapshot(&index, &[90]);
        let working_snapshot = snapshot(&working, &[90, 91]);
        let status = SemanticStatus::classify(&head, &index_snapshot, &working_snapshot);
        assert!(status.staged().is_empty());

        let added: Vec<_> = status
            .working_tree_additions()
            .map(|c| c.object_id())
            .collect();
        assert_eq!(added, vec![sid(91), sid(4), sid(5), sid(20), sid(21)]);
        for id in &added {
            assert!(!index_snapshot.contains(*id), "Index에 없음");
            assert!(working_snapshot.contains(*id), "WorkingTree에 있음");
        }
        assert!(status
            .working_tree_additions()
            .all(|c| c.kind() == SemanticChangeKind::AddObject));
        // 기존 객체의 참조·목록 변경은 추가가 아니라 속성 변경입니다.
        let others = summary(status.unstaged())
            .into_iter()
            .filter(|(kind, _, _)| *kind != SemanticChangeKind::AddObject)
            .collect::<Vec<_>>();
        assert_eq!(
            others,
            vec![
                (
                    SemanticChangeKind::SetProperty,
                    sid(1),
                    Some(keys::DOCUMENT_PAGES.0)
                ),
                (
                    SemanticChangeKind::AssetRefChange,
                    sid(12),
                    Some(keys::IMAGE_ASSET.0)
                ),
            ]
        );

        // 교체(같은 id를 다른 종류로)의 추가 쪽은 untracked 추가가 아닙니다.
        let mut replaced = working_snapshot.clone();
        let old = replaced.remove(sid(21)).unwrap();
        replaced
            .insert(
                SemanticObject::new(sid(21), ObjectKind::Token, Origin::Project)
                    .with_position(old.parent(), None),
            )
            .unwrap();
        let status = SemanticStatus::classify(&head, &working_snapshot, &replaced);
        assert_eq!(status.working_tree_additions().count(), 0);
    }

    #[test]
    fn corrupt_snapshot_fails_semantic_status_without_partial_result() {
        let [head, index, _] = partial_staged().map(|r| build(&r));
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct Corrupt(&'static str);
        // 손상된 WorkingTree chunk: chunk bytes를 해석할 수 없음.
        let [_, _, working] = partial_staged();
        let subtree = working.view_subtrees().unwrap().remove(0).1;
        let mut bytes = subtree.encode();
        bytes.truncate(bytes.len() / 2);
        let read_working = SubtreeRecords::decode(&bytes)
            .map(|_| unreachable!("잘린 chunk는 해석되지 않아야 합니다"))
            .map_err(|_| Corrupt("design/objects/aa/view.starchunk"));
        let failure = SemanticStatus::try_classify(Ok(&head), Ok(&index), read_working)
            .expect_err("WorkingTree 손상은 semantic status 실패");
        assert_eq!(
            failure.failures,
            vec![(
                SnapshotSide::WorkingTree,
                Corrupt("design/objects/aa/view.starchunk")
            )]
        );
        assert!(failure.failed(SnapshotSide::WorkingTree));
        assert!(!failure.failed(SnapshotSide::Index));

        // 여러 쪽이 실패하면 모두 보고합니다(Head → Index → WorkingTree 순).
        let failure = SemanticStatus::try_classify(
            Err(Corrupt("head")),
            Ok(&index),
            Err::<&SemanticSnapshot, _>(Corrupt("wt")),
        )
        .unwrap_err();
        assert_eq!(
            failure.failures,
            vec![
                (SnapshotSide::Head, Corrupt("head")),
                (SnapshotSide::WorkingTree, Corrupt("wt"))
            ]
        );

        // 모두 읽히면 classify와 같습니다.
        let ok = SemanticStatus::try_classify::<Corrupt>(Ok(&head), Ok(&index), Ok(&index));
        assert_eq!(ok, Ok(SemanticStatus::classify(&head, &index, &index)));
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

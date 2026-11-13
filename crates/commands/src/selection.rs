//! 창별 선택 상태 (VII.3).
//!
//! 선택은 **문서 모델이 아니라** 창의 [`WindowViewState`]에 있습니다(기술 아키텍처 §22.1·§23.2:
//! 선택 변경은 프로젝트 파일·Git 변경에 포함하지 않음). 따라서 선택을 바꿔도 문서 레코드와 저장
//! bytes는 그대로입니다.
//!
//! [`Selection`]은 선택된 StableId의 **순서 있는 집합**(처음 선택한 순서, 중복 없음)과 primary
//! selection(집합의 원소이거나 없음)입니다.
//!
//! 선택 변경은 문서 command가 아니라 history event [`SetSelection`](VII.3.2)입니다. 적용할 때 문서에
//! 없는 id와 숨김 정책([`SelectionPolicy`])상 선택할 수 없는 id를 거르고, 이전 선택을 역 event로
//! 돌려줍니다.

use stellar_core_types::ids::StableId;
use stellar_document::model::DocumentModel;

use crate::change::ChangeSet;
use crate::codec::{CommandCodecError, Reader, Writer};
use crate::id::WindowSessionId;
use crate::tree::parent_of;

/// 순서 있는 선택 집합 + primary.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    ids: Vec<StableId>,
    primary: Option<StableId>,
}

impl Selection {
    /// 빈 선택.
    pub fn empty() -> Selection {
        Selection::default()
    }

    /// 선택을 만듭니다. 중복은 처음 위치만 남기고 NIL은 버립니다. primary가 집합의 원소가
    /// 아니면 마지막 원소(가장 최근에 더한 node)가 primary입니다(빈 집합이면 없음).
    pub fn new(ids: impl IntoIterator<Item = StableId>, primary: Option<StableId>) -> Selection {
        let mut ordered: Vec<StableId> = Vec::new();
        for id in ids {
            if id != StableId::NIL && !ordered.contains(&id) {
                ordered.push(id);
            }
        }
        let primary = match primary {
            Some(primary) if ordered.contains(&primary) => Some(primary),
            _ => ordered.last().copied(),
        };
        Selection {
            ids: ordered,
            primary,
        }
    }

    /// 선택된 id(선택 순서).
    pub fn ids(&self) -> &[StableId] {
        &self.ids
    }

    /// primary selection.
    pub fn primary(&self) -> Option<StableId> {
        self.primary
    }

    /// `id`가 선택되었는지.
    pub fn contains(&self, id: StableId) -> bool {
        self.ids.contains(&id)
    }

    /// 선택 수.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// `keep`을 만족하는 원소만 남긴 선택. primary가 빠지면 primary 규칙을 다시 적용합니다.
    pub fn retain(&self, mut keep: impl FnMut(StableId) -> bool) -> Selection {
        let ids: Vec<StableId> = self.ids.iter().copied().filter(|id| keep(*id)).collect();
        Selection::new(ids, self.primary)
    }
}

/// 한 창의 view 상태. 문서와 분리되어 저장 경로에 들어가지 않습니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowViewState {
    window: WindowSessionId,
    selection: Selection,
    area: Option<AreaPreview>,
}

/// 삭제된 node를 선택에서 지운 system event(VII.3.4). 사용자 history 항목이 아닙니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionPruned {
    /// 정리한 창.
    pub window: WindowSessionId,
    /// 선택에서 빠진 id(원래 선택 순서).
    pub removed: Vec<StableId>,
    /// 정리 전 선택.
    pub before: Selection,
    /// 정리 후 선택.
    pub after: Selection,
}

/// 영역 선택 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaSelectionMode {
    /// 영역에 걸린 node로 선택을 교체합니다.
    Replace,
    /// 시작 전 선택에 영역에 걸린 node를 더합니다.
    Add,
}

/// 진행 중인 영역 선택의 transient preview. history·저장 대상이 아닙니다.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AreaPreview {
    mode: AreaSelectionMode,
    current: Selection,
}

/// 영역 선택 API 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaSelectionError {
    /// 이미 영역 선택이 진행 중입니다.
    AlreadyActive,
    /// 진행 중인 영역 선택이 없습니다.
    NotActive,
}

impl std::fmt::Display for AreaSelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AreaSelectionError::AlreadyActive => f.write_str("영역 선택이 이미 진행 중입니다"),
            AreaSelectionError::NotActive => f.write_str("진행 중인 영역 선택이 없습니다"),
        }
    }
}

impl std::error::Error for AreaSelectionError {}

impl WindowViewState {
    /// 빈 선택의 view 상태.
    pub fn new(window: WindowSessionId) -> WindowViewState {
        WindowViewState {
            window,
            selection: Selection::empty(),
            area: None,
        }
    }

    /// 소유 창.
    pub fn window(&self) -> WindowSessionId {
        self.window
    }

    /// 현재 선택.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// 선택을 바꾸고 이전 선택을 반환합니다(검증·history는 selection command의 몫).
    pub(crate) fn replace_selection(&mut self, selection: Selection) -> Selection {
        std::mem::replace(&mut self.selection, selection)
    }

    /// undo·redo가 기록해 둔 선택 상태를 **그대로** 복원하고 이전 선택을 반환합니다(VII.5.5).
    ///
    /// 사용자 선택(`SetSelection::apply`)과 달리 숨김 정책을 다시 적용하지 않습니다 — 복원 대상은 그 시점의
    /// 실제 view 상태이기 때문입니다. 문서에 더 없는 id만 뺍니다. history에 기록하지 않는 경로입니다.
    pub fn restore_selection(&mut self, selection: &Selection, model: &DocumentModel) -> Selection {
        let restored = selection.retain(|id| model.contains(id));
        self.replace_selection(restored)
    }

    /// 화면에 보일 선택: 영역 선택 중이면 transient preview, 아니면 확정 선택.
    pub fn displayed_selection(&self) -> &Selection {
        self.area
            .as_ref()
            .map_or(&self.selection, |area| &area.current)
    }

    /// 영역 선택이 진행 중인지.
    pub fn area_selection_active(&self) -> bool {
        self.area.is_some()
    }

    /// pointer down: 영역 선택 preview를 시작합니다(VII.3.3). 확정 선택은 바뀌지 않습니다.
    pub fn begin_area_selection(
        &mut self,
        mode: AreaSelectionMode,
    ) -> Result<(), AreaSelectionError> {
        if self.area.is_some() {
            return Err(AreaSelectionError::AlreadyActive);
        }
        self.area = Some(AreaPreview {
            mode,
            current: self.selection.clone(),
        });
        Ok(())
    }

    /// pointer move: 영역에 걸린 node로 preview만 교체합니다. history event를 만들지 않습니다.
    pub fn update_area_selection(
        &mut self,
        hits: impl IntoIterator<Item = StableId>,
    ) -> Result<(), AreaSelectionError> {
        let base = self.selection.clone();
        let area = self.area.as_mut().ok_or(AreaSelectionError::NotActive)?;
        area.current = match area.mode {
            AreaSelectionMode::Replace => Selection::new(hits, None),
            AreaSelectionMode::Add => {
                let ids: Vec<StableId> = base.ids().iter().copied().chain(hits).collect();
                Selection::new(ids, base.primary())
            }
        };
        Ok(())
    }

    /// pointer up: preview를 끝내고 기록할 **최종 SetSelection 하나**를 돌려줍니다. 확정 선택과
    /// 같으면 기록할 것이 없어 `None`입니다. 반환 event의 적용·기록은 history가 합니다.
    pub fn finish_area_selection(&mut self) -> Result<Option<SetSelection>, AreaSelectionError> {
        let area = self.area.take().ok_or(AreaSelectionError::NotActive)?;
        if area.current == self.selection {
            Ok(None)
        } else {
            Ok(Some(SetSelection::new(area.current)))
        }
    }

    /// node 삭제 결과(change set의 removed nodes)를 선택에서 지웁니다(VII.3.4).
    ///
    /// 사용자 history 항목이 아니라 **system event**([`SelectionPruned`])를 돌려줍니다. 진행 중인 영역
    /// 선택 preview도 함께 정리합니다. 지울 것이 없으면 `None`입니다.
    pub fn prune_removed(&mut self, changes: &ChangeSet) -> Option<SelectionPruned> {
        let removed = changes.removed_nodes();
        if removed.is_empty() {
            return None;
        }
        self.prune_where(|id| !removed.contains(&id))
    }

    /// 문서에 더 없는 id를 선택에서 지웁니다(재로드·외부 변경 반영 후, VII.3.4). system event입니다.
    pub fn prune_missing(&mut self, model: &DocumentModel) -> Option<SelectionPruned> {
        self.prune_where(|id| model.contains(id))
    }

    fn prune_where(&mut self, keep: impl Fn(StableId) -> bool) -> Option<SelectionPruned> {
        if let Some(area) = self.area.as_mut() {
            area.current = area.current.retain(&keep);
        }
        let after = self.selection.retain(&keep);
        if after == self.selection {
            return None;
        }
        let removed: Vec<StableId> = self
            .selection
            .ids()
            .iter()
            .copied()
            .filter(|id| !after.contains(*id))
            .collect();
        let before = std::mem::replace(&mut self.selection, after.clone());
        Some(SelectionPruned {
            window: self.window,
            removed,
            before,
            after,
        })
    }

    /// 영역 선택 취소: preview를 버리고 아무 event도 만들지 않습니다.
    pub fn cancel_area_selection(&mut self) -> Result<(), AreaSelectionError> {
        self.area
            .take()
            .map(|_| ())
            .ok_or(AreaSelectionError::NotActive)
    }
}

impl Selection {
    /// canonical bytes를 덧붙입니다: 수 `u32` + id* + primary(flag + id).
    pub(crate) fn encode_into(&self, out: &mut Writer) {
        out.len(self.ids.len());
        for id in &self.ids {
            out.id(*id);
        }
        out.opt_id(self.primary);
    }

    /// [`Selection::encode_into`]의 역. 형식을 정규화 규칙으로 다시 검사합니다.
    pub(crate) fn decode_from(input: &mut Reader<'_>) -> Result<Selection, CommandCodecError> {
        let count = input.len()?;
        let mut ids = Vec::with_capacity(count);
        for _ in 0..count {
            ids.push(input.id()?);
        }
        let primary = input.opt_id()?;
        let selection = Selection::new(ids.clone(), primary);
        if selection.ids != ids || selection.primary != primary {
            return Err(CommandCodecError::Malformed {
                reason: "정규형이 아닌 선택",
            });
        }
        Ok(selection)
    }
}

/// 선택 가능 정책.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SelectionPolicy {
    /// 숨김 node(자신 또는 조상이 숨김)도 선택할 수 있는지. 기본은 거부입니다.
    pub include_hidden: bool,
}

impl SelectionPolicy {
    /// `id`를 선택할 수 있는지: 문서에 있고, 숨김 정책을 통과해야 합니다.
    pub fn is_selectable(&self, model: &DocumentModel, id: StableId) -> bool {
        if !model.contains(id) {
            return false;
        }
        if self.include_hidden {
            return true;
        }
        let mut current = Some(id);
        while let Some(node) = current {
            let visible = model
                .runtime_id(node)
                .and_then(|handle| model.columns().get(handle))
                .map(|fields| fields.visible);
            if visible != Some(true) {
                return false;
            }
            current = parent_of(model, node).flatten();
        }
        true
    }
}

/// 선택 변경 history event(VII.3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetSelection {
    /// 요청 선택.
    pub selection: Selection,
}

/// 적용된 선택 event: 실제로 적용한(걸러진) 정방향과 이전 선택으로의 역방향.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedSelection {
    /// 걸러진 결과로 다시 적용하면 같은 선택이 되는 event.
    pub forward: SetSelection,
    /// 이전 선택으로 되돌리는 event.
    pub inverse: SetSelection,
}

impl SetSelection {
    /// 선택 event를 만듭니다.
    pub fn new(selection: Selection) -> SetSelection {
        SetSelection { selection }
    }

    /// 문서에 없거나 정책상 선택할 수 없는 id를 거른 선택(primary 규칙 재적용).
    pub fn filtered(&self, model: &DocumentModel, policy: SelectionPolicy) -> Selection {
        self.selection.retain(|id| policy.is_selectable(model, id))
    }

    /// view 상태에 적용합니다. 문서는 바꾸지 않습니다.
    pub fn apply(
        &self,
        view: &mut WindowViewState,
        model: &DocumentModel,
        policy: SelectionPolicy,
    ) -> AppliedSelection {
        let selection = self.filtered(model, policy);
        let previous = view.replace_selection(selection.clone());
        AppliedSelection {
            forward: SetSelection::new(selection),
            inverse: SetSelection::new(previous),
        }
    }

    /// canonical bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Writer::new();
        self.selection.encode_into(&mut out);
        out.into_bytes()
    }

    /// [`SetSelection::encode`]의 역.
    pub fn decode(bytes: &[u8]) -> Result<SetSelection, CommandCodecError> {
        let mut input = Reader::new(bytes);
        let selection = Selection::decode_from(&mut input)?;
        input.expect_end()?;
        Ok(SetSelection { selection })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_fixtures::{sid, small_model};
    use crate::state::DocumentState;
    use stellar_document::model::DocumentModel;
    use stellar_document::record_codec::encode_node_record;

    /// 문서 저장 bytes(정의 + node 레코드 + modifier stack canonical bytes).
    fn save_bytes(model: &DocumentModel) -> Vec<u8> {
        let records = model.to_records().unwrap();
        let mut bytes = records.definition.encode();
        for node in &records.nodes {
            bytes.extend(encode_node_record(node));
        }
        for stack in &records.modifier_stacks {
            bytes.extend(stack.encode());
        }
        bytes
    }

    #[test]
    fn selection_is_not_part_of_save_bytes() {
        // 검증 기준(VII.3.1): 선택이 save bytes에 포함되지 않습니다.
        let state = DocumentState::new(small_model());
        let before = save_bytes(state.model());
        let mut view = WindowViewState::new(WindowSessionId::from_stable_id(sid(500)));
        view.replace_selection(Selection::new([sid(11), sid(13)], Some(sid(11))));
        assert_eq!(view.selection().len(), 2);
        assert_eq!(
            save_bytes(state.model()),
            before,
            "선택 후에도 저장 bytes 동일"
        );
        // 창 id·선택 id는 저장 bytes 어디에도 새로 생기지 않습니다(문서 정의·레코드만 기록).
        let window_bytes = sid(500).to_bytes();
        assert!(!before.windows(16).any(|w| w == window_bytes));
    }

    #[test]
    fn ordered_set_and_primary_rules() {
        let selection = Selection::new([sid(3), sid(1), sid(3), StableId::NIL, sid(2)], None);
        assert_eq!(
            selection.ids(),
            &[sid(3), sid(1), sid(2)],
            "선택 순서, 중복·NIL 제거"
        );
        assert_eq!(selection.primary(), Some(sid(2)), "primary 미지정 → 마지막");
        let selection = Selection::new([sid(3), sid(1)], Some(sid(3)));
        assert_eq!(selection.primary(), Some(sid(3)));
        let selection = Selection::new([sid(3), sid(1)], Some(sid(9)));
        assert_eq!(
            selection.primary(),
            Some(sid(1)),
            "원소가 아닌 primary는 무시"
        );
        assert_eq!(Selection::new([], Some(sid(1))).primary(), None);
        let kept = Selection::new([sid(1), sid(2), sid(3)], Some(sid(2))).retain(|id| id != sid(2));
        assert_eq!(kept.ids(), &[sid(1), sid(3)]);
        assert_eq!(kept.primary(), Some(sid(3)), "primary가 빠지면 마지막으로");
        assert!(kept.contains(sid(1)) && !kept.contains(sid(2)));
        assert!(Selection::empty().is_empty());
    }

    fn hidden_model() -> DocumentModel {
        // small_model + 숨김 12, 12 아래 20(자신은 보임), 11 아래 숨김 21.
        use crate::command::test_fixtures::frame;
        let mut model = small_model();
        let mut hidden = frame(12, Some(10), 0x20);
        hidden.visible = false;
        model.remove_leaf(sid(12)).unwrap();
        model.insert_node(hidden).unwrap();
        model.insert_node(frame(20, Some(12), 0x10)).unwrap();
        let mut hidden_leaf = frame(21, Some(11), 0x20);
        hidden_leaf.visible = false;
        model.insert_node(hidden_leaf).unwrap();
        model
    }

    #[test]
    fn set_selection_filters_and_inverts() {
        // 검증 기준(VII.3.2): apply/inverse와 primary selection 규칙 통과.
        let model = hidden_model();
        let mut view = WindowViewState::new(WindowSessionId::from_stable_id(sid(500)));
        let policy = SelectionPolicy::default();
        let first = SetSelection::new(Selection::new([sid(11), sid(13)], Some(sid(11))));
        let applied = first.apply(&mut view, &model, policy);
        assert_eq!(applied.inverse, SetSelection::new(Selection::empty()));
        assert_eq!(view.selection().ids(), &[sid(11), sid(13)]);
        assert_eq!(view.selection().primary(), Some(sid(11)));

        // 없는 99, 숨김 12, 숨김 조상 아래 20, 숨김 21은 걸러집니다. primary(20)가 빠지면 마지막.
        let request = SetSelection::new(Selection::new(
            [sid(99), sid(12), sid(13), sid(20), sid(21), sid(10)],
            Some(sid(20)),
        ));
        let applied = request.apply(&mut view, &model, policy);
        assert_eq!(view.selection().ids(), &[sid(13), sid(10)]);
        assert_eq!(view.selection().primary(), Some(sid(10)));
        assert_eq!(applied.forward.selection, view.selection().clone());
        // 걸러진 정방향을 다시 적용해도 같은 선택(redo 결정론).
        let mut replay = WindowViewState::new(WindowSessionId::from_stable_id(sid(501)));
        applied.forward.apply(&mut replay, &model, policy);
        assert_eq!(replay.selection(), view.selection());
        // 역방향은 이전 선택(primary 포함)으로 되돌립니다.
        let back = applied.inverse.apply(&mut view, &model, policy);
        assert_eq!(view.selection().ids(), &[sid(11), sid(13)]);
        assert_eq!(view.selection().primary(), Some(sid(11)));
        assert_eq!(back.inverse.selection.ids(), &[sid(13), sid(10)]);

        // 숨김 허용 정책.
        let permissive = SelectionPolicy {
            include_hidden: true,
        };
        request.apply(&mut view, &model, permissive);
        assert_eq!(
            view.selection().ids(),
            &[sid(12), sid(13), sid(20), sid(21), sid(10)]
        );
        assert_eq!(view.selection().primary(), Some(sid(20)));
    }

    #[test]
    fn selection_codec_round_trips_and_rejects_non_canonical() {
        let event = SetSelection::new(Selection::new([sid(3), sid(1), sid(2)], Some(sid(1))));
        let bytes = event.encode();
        assert_eq!(SetSelection::decode(&bytes).unwrap(), event);
        assert_eq!(
            SetSelection::decode(&SetSelection::new(Selection::empty()).encode()).unwrap(),
            SetSelection::new(Selection::empty())
        );
        // 중복 id(정규형 아님) 거부.
        let mut out = Writer::new();
        out.len(2);
        out.id(sid(1));
        out.id(sid(1));
        out.opt_id(Some(sid(1)));
        assert!(SetSelection::decode(&out.into_bytes()).is_err());
        let mut trailing = bytes;
        trailing.push(0);
        assert!(SetSelection::decode(&trailing).is_err());
    }

    #[test]
    fn area_selection_produces_one_event_for_many_moves() {
        // 검증 기준(VII.3.3): 1,000 move event에서 history 1건.
        let model = small_model();
        let policy = SelectionPolicy::default();
        let mut view = WindowViewState::new(WindowSessionId::from_stable_id(sid(500)));
        let mut history: Vec<SetSelection> = Vec::new();
        view.begin_area_selection(AreaSelectionMode::Replace)
            .unwrap();
        let candidates = [sid(11), sid(12), sid(13)];
        for step in 0..1_000usize {
            let hits = candidates.iter().copied().take(1 + step % 3);
            view.update_area_selection(hits).unwrap();
            // preview는 보이지만 확정 선택·history는 그대로입니다.
            assert!(view.selection().is_empty());
            assert!(!view.displayed_selection().is_empty());
        }
        assert!(history.is_empty());
        if let Some(event) = view.finish_area_selection().unwrap() {
            event.apply(&mut view, &model, policy);
            history.push(event);
        }
        assert_eq!(history.len(), 1, "1,000 move → history 1건");
        // 마지막 move(999 % 3 = 0 → 1개)가 최종 선택.
        assert_eq!(view.selection().ids(), &[sid(11)]);
        assert!(!view.area_selection_active());
        assert_eq!(view.displayed_selection(), view.selection());
    }

    #[test]
    fn area_selection_add_cancel_and_no_change() {
        let model = small_model();
        let policy = SelectionPolicy::default();
        let mut view = WindowViewState::new(WindowSessionId::from_stable_id(sid(500)));
        SetSelection::new(Selection::new([sid(12)], None)).apply(&mut view, &model, policy);
        // Add 모드: 기존 선택 + 영역.
        view.begin_area_selection(AreaSelectionMode::Add).unwrap();
        assert_eq!(
            view.begin_area_selection(AreaSelectionMode::Add),
            Err(AreaSelectionError::AlreadyActive)
        );
        view.update_area_selection([sid(13), sid(12)]).unwrap();
        assert_eq!(view.displayed_selection().ids(), &[sid(12), sid(13)]);
        assert_eq!(view.displayed_selection().primary(), Some(sid(12)));
        let event = view.finish_area_selection().unwrap().unwrap();
        assert_eq!(event.selection.ids(), &[sid(12), sid(13)]);
        // 취소는 event 없음, 확정 선택 유지.
        view.begin_area_selection(AreaSelectionMode::Replace)
            .unwrap();
        view.update_area_selection([sid(11)]).unwrap();
        view.cancel_area_selection().unwrap();
        assert_eq!(view.selection().ids(), &[sid(12)]);
        assert_eq!(
            view.cancel_area_selection(),
            Err(AreaSelectionError::NotActive)
        );
        assert_eq!(
            view.update_area_selection([sid(11)]),
            Err(AreaSelectionError::NotActive)
        );
        // 결과가 확정 선택과 같으면 기록할 것이 없습니다.
        view.begin_area_selection(AreaSelectionMode::Replace)
            .unwrap();
        view.update_area_selection([sid(12)]).unwrap();
        assert_eq!(view.finish_area_selection().unwrap(), None);
    }

    #[test]
    fn deletion_prunes_every_window_without_user_history() {
        // 검증 기준(VII.3.4): 여러 창 fixture에서 stale selection 없음.
        use crate::command::Command;
        use crate::id::{CommandId, CommandOrigin};
        use crate::node_ops::DeleteSubtree;
        use crate::request::CommandRequest;
        use crate::state::DocumentState;
        use crate::transaction::CommandTransaction;

        let policy = SelectionPolicy::default();
        // 창마다 독립 문서 상태(§22.2). A·B는 같은 문서를 열었고 C는 다른 node를 선택.
        let mut docs: Vec<DocumentState> =
            (0..3).map(|_| DocumentState::new(small_model())).collect();
        let mut views: Vec<WindowViewState> = (0..3u64)
            .map(|i| WindowViewState::new(WindowSessionId::from_stable_id(sid(500 + i))))
            .collect();
        let selections = [
            Selection::new([sid(13), sid(12)], Some(sid(13))),
            Selection::new([sid(11), sid(13)], Some(sid(11))),
            Selection::new([sid(12)], None),
        ];
        for ((view, doc), selection) in views.iter_mut().zip(&docs).zip(selections) {
            SetSelection::new(selection).apply(view, doc.model(), policy);
        }
        // B는 영역 선택 preview 중에도 13을 담고 있습니다.
        views[1]
            .begin_area_selection(AreaSelectionMode::Add)
            .unwrap();
        views[1].update_area_selection([sid(13), sid(12)]).unwrap();

        // 창 A에서 11 subtree(11, 13) 삭제.
        let delete = Command::DeleteSubtree(DeleteSubtree { root: sid(11) });
        let tx = CommandTransaction::single(
            CommandId::from_stable_id(sid(900)),
            CommandOrigin::User,
            CommandRequest::at(&docs[0], delete.clone()),
        );
        let applied = docs[0].execute_transaction(&tx).unwrap();
        // 정리 결과는 SetSelection(사용자 history event)이 아니라 SelectionPruned system event입니다.
        let mut system_events: Vec<SelectionPruned> = Vec::new();
        system_events.extend(views[0].prune_removed(&applied.changes));
        // B는 A의 저장 generation을 다시 읽은 뒤(같은 삭제가 반영된 문서) 누락 id를 정리합니다.
        let reload = CommandTransaction::single(
            CommandId::from_stable_id(sid(901)),
            CommandOrigin::System,
            CommandRequest::at(&docs[1], delete),
        );
        docs[1].execute_transaction(&reload).unwrap();
        system_events.extend(views[1].prune_missing(docs[1].model()));
        // C의 문서·선택에는 삭제가 없습니다.
        assert_eq!(views[2].prune_missing(docs[2].model()), None);

        assert_eq!(system_events.len(), 2);
        assert_eq!(system_events[0].removed, vec![sid(13)]);
        assert_eq!(system_events[0].after.ids(), &[sid(12)]);
        assert_eq!(
            system_events[0].after.primary(),
            Some(sid(12)),
            "primary 재적용"
        );
        assert_eq!(system_events[1].removed, vec![sid(11), sid(13)]);
        assert!(system_events[1].after.is_empty());
        // 어떤 창에도 stale id가 없습니다(preview 포함).
        for (view, doc) in views.iter().zip(&docs) {
            for id in view.selection().ids() {
                assert!(doc.model().contains(*id), "stale {id}");
            }
            for id in view.displayed_selection().ids() {
                assert!(doc.model().contains(*id), "stale preview {id}");
            }
        }
        assert_eq!(views[1].displayed_selection().ids(), &[sid(12)]);
        assert_eq!(views[2].selection().ids(), &[sid(12)]);
        // 삭제가 없는 change set은 아무것도 하지 않습니다.
        let rename = CommandRequest::at(&docs[2], Command::rename_node(sid(12), "x"));
        let outcome = docs[2].execute(&rename).unwrap();
        assert_eq!(views[2].prune_removed(&outcome.changes), None);
    }
}

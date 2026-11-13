//! 창별 선택 상태 (VII.3).
//!
//! 선택은 **문서 모델이 아니라** 창의 [`WindowViewState`]에 있습니다(기술 아키텍처 §22.1·§23.2:
//! 선택 변경은 프로젝트 파일·Git 변경에 포함하지 않음). 따라서 선택을 바꿔도 문서 레코드와 저장
//! bytes는 그대로입니다.
//!
//! [`Selection`]은 선택된 StableId의 **순서 있는 집합**(처음 선택한 순서, 중복 없음)과 primary
//! selection(집합의 원소이거나 없음)입니다.

use stellar_core_types::ids::StableId;

use crate::id::WindowSessionId;

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
}

impl WindowViewState {
    /// 빈 선택의 view 상태.
    pub fn new(window: WindowSessionId) -> WindowViewState {
        WindowViewState {
            window,
            selection: Selection::empty(),
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
    // SetSelection(VII.3.2)이 유일한 사용처가 되기 전까지는 테스트만 씁니다.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn replace_selection(&mut self, selection: Selection) -> Selection {
        std::mem::replace(&mut self.selection, selection)
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
}

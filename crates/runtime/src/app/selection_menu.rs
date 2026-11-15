//! 표시한 선택에 대한 코어 능력 mask와 문맥 메뉴를 열 때의 선택(UX.3.2·3.4).
//!
//! 메뉴 막대·캔버스 문맥 메뉴·레이어 문맥 메뉴는 이 mask 하나로 활성과 제목(Show·Unlock)을 정합니다. 실행 경로와 같은
//! 규칙 함수(`editable_selected_roots`, `unlocked_selected_roots`, `flag_targets`, `paste_parent`)를 쓰므로 활성인 항목은 코어가 받습니다.
use super::node_delete::{editable_selected_roots, unlocked_selected_roots, SelectionEditTarget};
use super::sidebar::{flag_targets, path_locked, view_root, LayerFlag};
use super::*;

/// 선택 subtree를 삭제할 수 있습니다(View root가 아니고, 자신·조상·자손이 잠기지 않음).
pub const CAN_DELETE: u32 = 1;
/// 단일 선택 subtree를 복제할 수 있습니다(View root가 아니고 자신·조상이 잠기지 않음. 잠긴 자손은 막지 않음).
pub const CAN_DUPLICATE: u32 = 2;
/// 단일 선택 subtree를 클립보드로 복사할 수 있습니다(복제와 같은 규칙).
pub const CAN_COPY: u32 = 4;
/// 지금 선택에서 정한 부모에 붙여 넣을 수 있습니다(클립보드 내용은 셸이 확인).
pub const CAN_PASTE: u32 = 8;
/// 선택의 표시를 토글할 수 있습니다(D8).
pub const CAN_HIDE: u32 = 16;
/// 선택의 잠금을 토글할 수 있습니다(D8).
pub const CAN_LOCK: u32 = 32;
/// 표시 토글이 선택을 보이게 합니다(하나라도 숨겨져 있음). 메뉴 제목 Show.
pub const HIDE_SHOWS: u32 = 64;
/// 잠금 토글이 선택을 풉니다(모두 잠겨 있음). 메뉴 제목 Unlock.
pub const LOCK_UNLOCKS: u32 = 128;

/// 마지막으로 계산한 mask. 문서 revision·활성 View·선택이 같으면 다시 계산하지 않습니다(삭제 검사는 subtree를 훑음).
#[derive(Debug, Default)]
pub(crate) struct SelectionCapsCache {
    key: Option<(DocumentRevision, StableId, Vec<StableId>)>,
    flags: u32,
}

impl SelectionCapsCache {
    pub(crate) fn get(&mut self, window: &ProjectWindowSession) -> u32 {
        let Some(view) = window.tabs().active_view() else {
            self.key = None;
            return 0;
        };
        let revision = window.revision();
        let selected = window.view().selection().ids();
        if let Some((cached_revision, cached_view, cached)) = &self.key {
            if *cached_revision == revision && *cached_view == view && cached == selected {
                return self.flags;
            }
        }
        self.flags = capabilities(window, view, revision);
        self.key = Some((revision, view, selected.to_vec()));
        self.flags
    }
}

fn capabilities(window: &ProjectWindowSession, view: StableId, revision: DocumentRevision) -> u32 {
    let Some(model) = window.document().map(|document| document.model()) else {
        return 0;
    };
    let Some(root) = view_root(model, view) else {
        return 0;
    };
    let selected = window.view().selection().ids();
    let mut flags = 0;
    if super::node_paste::paste_parent(model, root, selected).is_some() {
        flags |= CAN_PASTE;
    }
    if selected.is_empty() {
        return flags;
    }
    if editable_selected_roots(window, view, revision).is_ok() {
        flags |= CAN_DELETE;
    }
    // 복제·복사는 원본을 바꾸지 않으므로 잠긴 자손을 막지 않습니다(ADR-0070 결정 5).
    if let Ok(roots) = unlocked_selected_roots(window, view, revision) {
        if selected.len() == 1 && roots.len() == 1 {
            flags |= CAN_DUPLICATE | CAN_COPY;
        }
    }
    if let Ok(targets) = flag_targets(model, root, selected, LayerFlag::Visible) {
        flags |= CAN_HIDE;
        if !targets.iter().all(|(_, visible)| *visible) {
            flags |= HIDE_SHOWS;
        }
    }
    if let Ok(targets) = flag_targets(model, root, selected, LayerFlag::Locked) {
        flags |= CAN_LOCK;
        if targets.iter().all(|(_, locked)| *locked) {
            flags |= LOCK_UNLOCKS;
        }
    }
    flags
}

impl AppRuntime {
    /// 캔버스 문맥 메뉴를 열기 전의 선택. 누른 노드나 그 조상(View root 제외)이 이미 선택에 있으면 선택을 그대로 두고,
    /// 아니면 그 노드만 선택합니다. `node`가 없으면(빈 곳) 선택을 비웁니다. 판단은 실행 시점의 전체 선택으로 합니다.
    pub fn context_select(
        &self,
        target: SelectionEditTarget,
        node: Option<StableId>,
    ) -> Result<RequestId, RequestError> {
        self.submit(move |ctx, request| {
            let window = window_mut(ctx, target.session)?;
            target.verify(window)?;
            if let Some(ids) = context_selection(window, target.view, node)? {
                window
                    .select(stellar_commands::selection::Selection::new(ids, None))
                    .map_err(view_failure)?;
            }
            Ok(RuntimeEvent::WindowViewChanged {
                request,
                session: target.session,
            })
        })
    }
}

/// 바꿀 선택. 그대로 두면 `None`입니다.
fn context_selection(
    window: &ProjectWindowSession,
    view: StableId,
    node: Option<StableId>,
) -> Result<Option<Vec<StableId>>, Failure> {
    let stale = || {
        Failure::new(
            RequestFailure::UnknownView,
            "누른 노드가 활성 View에 없습니다",
        )
    };
    let selected = window.view().selection().ids();
    let Some(node) = node else {
        return Ok((!selected.is_empty()).then(Vec::new));
    };
    let model = window.document().ok_or_else(stale)?.model();
    let root = view_root(model, view).ok_or_else(stale)?;
    // 활성 View 트리 안의 노드인지 확인합니다.
    path_locked(model, node, root).ok_or_else(stale)?;
    if selected.contains(&node) {
        return Ok(None);
    }
    let mut cursor = node;
    while cursor != root {
        cursor = model
            .runtime_id(cursor)
            .and_then(|id| model.columns().parent(id))
            .and_then(|id| model.stable_id(id))
            .ok_or_else(stale)?;
        if cursor != root && selected.contains(&cursor) {
            return Ok(None);
        }
    }
    // 선택 밖을 우클릭하면 왼쪽 클릭과 같은 규칙(⌘·더블클릭 없는 누름)으로 고릅니다. 그래서 가장 안쪽 node가 아니라
    // 지금 선택과 같은 단계, 선택이 없으면 root의 직계 자식이 선택됩니다(UX 명세 5.3).
    let target = super::canvas_gesture::point_target(model, root, selected, node, false, false)
        .ok_or_else(stale)?;
    Ok(Some(vec![target]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_session::test_fixtures::{
        identity, sample_document, saved_project_with, sid, window,
    };
    use stellar_commands::selection::Selection;

    #[test]
    fn capabilities_follow_root_lock_and_selection_rules() {
        // 10 root, 11·12 자식. 30은 12 아래의 잠긴 노드, 31은 30 아래.
        let mut model = sample_document();
        let mut locked = model.record(sid(12)).unwrap();
        locked.locked = true;
        locked.id = sid(30);
        locked.parent = Some(sid(12));
        model.insert_node(locked).unwrap();
        let mut child = model.record(sid(11)).unwrap();
        child.id = sid(31);
        child.parent = Some(sid(30));
        model.insert_node(child).unwrap();
        let dir = saved_project_with("selection-caps", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        let mut cache = SelectionCapsCache::default();
        let mut caps = |win: &mut ProjectWindowSession, ids: &[u64]| {
            win.select(Selection::new(ids.iter().map(|n| sid(*n)), None))
                .unwrap();
            cache.get(win)
        };
        // 빈 선택은 root에 붙여넣기만 됩니다.
        assert_eq!(caps(&mut session, &[]), CAN_PASTE);
        // View root는 레이어 명령을 받지 않습니다.
        assert_eq!(caps(&mut session, &[10]) & !CAN_PASTE, 0);
        // 보통 노드 하나는 모두 되고, 다 보이고 안 잠겼으니 Hide·Lock 제목입니다.
        let one = caps(&mut session, &[11]);
        assert_eq!(
            one,
            CAN_DELETE | CAN_DUPLICATE | CAN_COPY | CAN_PASTE | CAN_HIDE | CAN_LOCK
        );
        // 다중 선택은 복제·복사·붙여넣기를 받지 않고, 잠긴 자손(30)이 있는 12 때문에 삭제도 안 됩니다.
        assert_eq!(caps(&mut session, &[11, 12]), CAN_HIDE | CAN_LOCK);
        // 잠긴 자손이 있는 부모는 삭제가 안 되지만 복제·복사와 표시·잠금은 됩니다(ADR-0070 결정 5: 복제·복사는
        // 원본을 바꾸지 않아 Option 드래그·CLI duplicate-subtree와 같게 자손을 보지 않음. 자손 잠금은 부모 토글도 막지 않음).
        assert_eq!(
            caps(&mut session, &[12]),
            CAN_DUPLICATE | CAN_COPY | CAN_PASTE | CAN_HIDE | CAN_LOCK
        );
        // 잠긴 노드는 잠금 해제만 됩니다(제목 Unlock). 그 아래 노드는 아무것도 안 됩니다.
        assert_eq!(caps(&mut session, &[30]), CAN_LOCK | LOCK_UNLOCKS);
        assert_eq!(caps(&mut session, &[31]), 0);
        // 잠긴 노드와 보통 노드를 함께 고르면 표시는 막히고 잠금은 모두 잠그는 쪽입니다.
        assert_eq!(caps(&mut session, &[11, 30]), CAN_LOCK);
    }

    #[test]
    fn cache_recomputes_when_the_document_changes() {
        let dir = saved_project_with("selection-caps-cache", &[&sample_document()]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session.select(Selection::new([sid(11)], None)).unwrap();
        let mut cache = SelectionCapsCache::default();
        assert_eq!(cache.get(&session) & HIDE_SHOWS, 0);
        let revision = session.revision();
        assert!(super::super::sidebar::set_flags(
            &mut session,
            RequestId(1),
            sid(3),
            revision,
            &[sid(11)],
            LayerFlag::Visible,
            None,
        )
        .is_ok());
        assert_ne!(cache.get(&session) & HIDE_SHOWS, 0);
    }

    #[test]
    fn context_selection_keeps_a_selection_that_contains_the_pressed_node() {
        let mut model = sample_document();
        let mut child = model.record(sid(12)).unwrap();
        child.id = sid(32);
        child.parent = Some(sid(11));
        model.insert_node(child).unwrap();
        let dir = saved_project_with("context-select", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        // Ok(None)은 선택 유지, Ok(Some)은 바꿀 선택, Err는 None입니다.
        let next = |win: &ProjectWindowSession, node: Option<u64>| {
            context_selection(win, sid(3), node.map(sid)).ok()
        };
        // 빈 선택에서 빈 곳은 그대로입니다. 노드를 누르면 가장 안쪽(32)이 아니라 왼쪽 클릭처럼 root의 직계 자식(11)을
        // 고릅니다(UX 명세 5.3).
        assert_eq!(next(&session, None), Some(None));
        assert_eq!(next(&session, Some(32)), Some(Some(vec![sid(11)])));
        assert_eq!(next(&session, Some(12)), Some(Some(vec![sid(12)])));
        // 같은 단계에 형제가 선택되어 있으면 그 단계에서 고릅니다.
        session.select(Selection::new([sid(12)], None)).unwrap();
        assert_eq!(next(&session, Some(32)), Some(Some(vec![sid(11)])));
        // 다중 선택 안의 노드나 선택한 노드의 자손을 누르면 선택을 유지합니다.
        session
            .select(Selection::new([sid(11), sid(12)], None))
            .unwrap();
        assert_eq!(next(&session, Some(12)), Some(None));
        assert_eq!(next(&session, Some(32)), Some(None));
        // View root가 선택되어 있어도 자식을 누르면 그 자식만 고릅니다.
        session.select(Selection::new([sid(10)], None)).unwrap();
        assert_eq!(next(&session, Some(11)), Some(Some(vec![sid(11)])));
        assert_eq!(next(&session, Some(10)), Some(None));
        // 빈 곳은 선택을 비웁니다. 다른 View의 노드는 거부합니다.
        assert_eq!(next(&session, None), Some(Some(vec![])));
        assert_eq!(next(&session, Some(20)), None);
    }

    #[test]
    fn untrusted_window_publishes_mask_toggles_flags_and_context_selects() {
        use crate::app_window_tests::{next_event, open_project, open_window};
        let dir = saved_project_with("selection-menu-app", &[&sample_document()]);
        let app = AppRuntime::start().unwrap();
        let owner = open_project(&app, dir.path());
        let (session, _, _) = open_window(&app, owner);
        let target = || {
            let state = app.state();
            let window = state.window(session).unwrap();
            SelectionEditTarget {
                session,
                view: window.active_view.unwrap(),
                revision: window.revision,
                history_sequence: window.history_sequence,
            }
        };
        // 우클릭한 노드만 선택하고, 그 선택의 mask를 게시합니다(신뢰하지 않은 프로젝트).
        app.context_select(target(), Some(sid(11))).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        let window = app.state().window(session).unwrap().clone();
        assert_eq!(window.selection_len, 1);
        assert_ne!(window.selection_flags & CAN_HIDE, 0);
        // D1: 표시·잠금 토글은 신뢰 없이도 Undo 가능한 편집입니다.
        app.edit_selection(target(), SelectionEdit::ToggleVisibility, None)
            .unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        let window = app.state().window(session).unwrap().clone();
        assert!(window.has_unsaved_changes);
        assert_ne!(window.selection_flags & HIDE_SHOWS, 0);
        // 이미 선택한 노드를 다시 우클릭하면 선택과 history가 그대로입니다.
        let before = target();
        app.context_select(before, Some(sid(11))).unwrap();
        assert!(matches!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged { .. }
        ));
        assert_eq!(target().history_sequence, before.history_sequence);
    }
}

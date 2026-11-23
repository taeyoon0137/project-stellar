//! 깊은 선택과 편집 범위 이동 (XII.10.4, ADR-0104 결정 3.4).
//!
//! - 더블클릭: 고른 node가 이미 선택된 컨테이너(Frame, Fragment)이면 그 안으로 들어가 커서 아래 직계 자식을 고릅니다.
//!   커서 아래 자식이 없으면 선택 없이 범위만 들어갑니다. 선택되지 않은 node면 클릭과 같습니다. Text·Path는 내용 편집
//!   요청입니다.
//! - Return: 선택한 컨테이너의 선택 가능 자식 전체를 고릅니다. 컨테이너가 없고 Text·Path 하나면 내용 편집 요청입니다.
//! - Shift-Return: 선택 node의 부모(View root 바로 아래 node는 그대로)를 고릅니다.
//! - Tab·Shift-Tab: primary 부모의 선택 가능 자식을 OrderKey 순으로 순환합니다. 선택이 없으면 범위의 첫·마지막 자식입니다.
//! - Shift-더블클릭: 커서 아래 가장 깊은 node를 선택에 더합니다.
//! - Escape의 범위 위로 단계는 [`super::select::escape`]입니다.
//!
//! 범위는 새 선택에 맞춰 [`EditingScope::refit`]으로 다시 정합니다. 범위 변경은 저장·history에 없고, 선택 변경만
//! `SetSelection` 하나입니다.

use stellar_commands::selection::Selection;
use stellar_core_types::ids::StableId;
use stellar_document::node_kind::NodeKind;

use super::hit::PointStack;
use super::select::{click_pick, resolve_click, EditingScope, SelectTree, SelectionChange};
use super::tool::Modifiers;

/// 범위 이동 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Navigation {
    /// 선택·범위가 바뀜(선택이 같으면 범위만).
    Changed(SelectionChange),
    /// Text·Path 내용 편집 모드로 들어가라는 요청(XII.10.29, XII.10.33이 받음).
    EditContent(StableId),
    /// 할 일이 없음.
    Unchanged,
}

fn content_editable(tree: &SelectTree<'_>, node: StableId) -> bool {
    matches!(tree.kind(node), Some(NodeKind::Text | NodeKind::Path))
}

fn changed(tree: &SelectTree<'_>, scope: EditingScope, selection: Selection) -> Navigation {
    let scope = scope.refit(tree, &selection);
    Navigation::Changed(SelectionChange { selection, scope })
}

/// 더블클릭. `modifiers.shift`면 깊은 node 추가(Shift-더블클릭)입니다.
pub fn double_click(
    tree: &SelectTree<'_>,
    scope: EditingScope,
    current: &Selection,
    stack: &PointStack,
    modifiers: Modifiers,
) -> Navigation {
    if modifiers.shift {
        return add_deep(tree, scope, current, stack);
    }
    let Some(pick) = click_pick(tree, scope, stack, modifiers.command) else {
        return Navigation::Changed(resolve_click(tree, scope, current, stack, modifiers));
    };
    if !current.contains(pick.target) {
        return Navigation::Changed(resolve_click(tree, scope, current, stack, modifiers));
    }
    if !tree.is_container(pick.target) {
        return if content_editable(tree, pick.target) {
            Navigation::EditContent(pick.target)
        } else {
            Navigation::Unchanged
        };
    }
    let inner = EditingScope::at(tree, pick.target);
    match click_pick(tree, inner, stack, false) {
        Some(child) if tree.parent(child.target) == Some(pick.target) => {
            Navigation::Changed(SelectionChange {
                selection: Selection::new([child.target], Some(child.target)),
                scope: inner,
            })
        }
        _ => Navigation::Changed(SelectionChange {
            selection: Selection::empty(),
            scope: inner,
        }),
    }
}

/// Shift-더블클릭: 커서 아래 가장 깊은 선택 가능 node를 더합니다(이미 있으면 그대로).
pub fn add_deep(
    tree: &SelectTree<'_>,
    scope: EditingScope,
    current: &Selection,
    stack: &PointStack,
) -> Navigation {
    let Some(pick) = click_pick(tree, scope, stack, true) else {
        return Navigation::Unchanged;
    };
    if current.contains(pick.target) {
        return Navigation::Unchanged;
    }
    let selection = Selection::new(
        current.ids().iter().copied().chain([pick.target]),
        Some(pick.target),
    );
    changed(tree, scope, selection)
}

/// Return: 선택한 컨테이너 안으로 한 단계 들어갑니다.
pub fn enter(tree: &SelectTree<'_>, scope: EditingScope, current: &Selection) -> Navigation {
    let containers: Vec<StableId> = current
        .ids()
        .iter()
        .copied()
        .filter(|id| tree.is_container(*id))
        .collect();
    if containers.is_empty() {
        return match current.ids() {
            [single] if content_editable(tree, *single) => Navigation::EditContent(*single),
            _ => Navigation::Unchanged,
        };
    }
    let children: Vec<StableId> = containers
        .iter()
        .flat_map(|container| tree.selectable_children(*container))
        .collect();
    if children.is_empty() {
        return Navigation::Unchanged;
    }
    changed(tree, scope, Selection::new(children, None))
}

/// Shift-Return: 부모를 고릅니다. View root 바로 아래 node는 그대로 둡니다.
pub fn exit_to_parent(
    tree: &SelectTree<'_>,
    scope: EditingScope,
    current: &Selection,
) -> Navigation {
    if current.is_empty() {
        return Navigation::Unchanged;
    }
    let lift = |id: StableId| match tree.parent(id) {
        Some(parent) if parent != tree.root() && tree.selectable(parent) => parent,
        _ => id,
    };
    let lifted: Vec<StableId> = current.ids().iter().map(|id| lift(*id)).collect();
    // primary는 primary를 올린 node입니다(올리지 못한 최상위 node면 그 node 그대로).
    let primary = current.primary().map(lift);
    let selection = Selection::new(lifted, primary);
    if selection.ids() == current.ids() {
        return Navigation::Unchanged;
    }
    changed(tree, scope, selection)
}

/// Tab(`backward`면 Shift-Tab): primary의 형제를 OrderKey 순으로 순환합니다.
pub fn cycle_sibling(
    tree: &SelectTree<'_>,
    scope: EditingScope,
    current: &Selection,
    backward: bool,
) -> Navigation {
    let (parent, from) = match current.primary() {
        Some(primary) => match tree.parent(primary) {
            Some(parent) => (parent, Some(primary)),
            None => return Navigation::Unchanged,
        },
        None => (scope.node(tree), None),
    };
    let siblings = tree.selectable_children(parent);
    if siblings.is_empty() {
        return Navigation::Unchanged;
    }
    let next = match from.and_then(|f| siblings.iter().position(|s| *s == f)) {
        Some(index) if backward => siblings[(index + siblings.len() - 1) % siblings.len()],
        Some(index) => siblings[(index + 1) % siblings.len()],
        None if backward => siblings[siblings.len() - 1],
        None => siblings[0],
    };
    changed(tree, scope, Selection::new([next], Some(next)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::hit::StackEntry;
    use crate::canvas::select::{escape, EscapeStep};
    use crate::canvas::select_fixtures::{nested, save_bytes, sid};
    use crate::canvas::tool::CanvasToolState;
    use crate::window_session::test_fixtures::window;
    use stellar_commands::selection::SetSelection;
    use stellar_commands::state::DocumentState;
    use stellar_history::window::WindowHistory;

    fn stack(ids: &[u64]) -> PointStack {
        PointStack::new(ids.iter().map(|n| StackEntry {
            node: sid(*n),
            locked: false,
        }))
    }

    fn sel(ids: &[u64]) -> Selection {
        Selection::new(ids.iter().map(|n| sid(*n)), None)
    }

    fn change(nav: Navigation) -> SelectionChange {
        match nav {
            Navigation::Changed(change) => change,
            other => panic!("변경 기대: {other:?}"),
        }
    }

    fn ids(change: &SelectionChange) -> Vec<StableId> {
        change.selection.ids().to_vec()
    }

    const NONE: Modifiers = Modifiers::NONE;

    #[test]
    fn enter_exit_and_escape_walk_the_nested_scopes() {
        // 검증 기준(XII.10.4): `canvas/select-scope-nested`에서 진입·이탈 후 범위 결과.
        let fixture = nested();
        let tree = SelectTree::new(&fixture.model, fixture.root);
        let top = EditingScope::top(&tree);
        let deep_point = stack(&[17, 14, 11, 10]);
        // 첫 클릭은 11, 같은 곳 더블클릭은 11 안의 14, 다시 더블클릭은 14 안의 17.
        let first = resolve_click(&tree, top, &Selection::empty(), &deep_point, NONE);
        assert_eq!(ids(&first), vec![sid(11)]);
        let second = change(double_click(
            &tree,
            first.scope,
            &first.selection,
            &deep_point,
            NONE,
        ));
        assert_eq!(
            (ids(&second), second.scope.node(&tree)),
            (vec![sid(14)], sid(11))
        );
        let third = change(double_click(
            &tree,
            second.scope,
            &second.selection,
            &deep_point,
            NONE,
        ));
        assert_eq!(
            (ids(&third), third.scope.node(&tree)),
            (vec![sid(17)], sid(14))
        );
        // Rectangle 더블클릭은 내용 편집 대상이 아니라 그대로.
        assert_eq!(
            double_click(&tree, third.scope, &third.selection, &deep_point, NONE),
            Navigation::Unchanged
        );
        // Shift-Return: 부모 14, 범위 11.
        let up = change(exit_to_parent(&tree, third.scope, &third.selection));
        assert_eq!((ids(&up), up.scope.node(&tree)), (vec![sid(14)], sid(11)));
        // Return: 14의 자식 전체.
        let down = change(enter(&tree, up.scope, &up.selection));
        assert_eq!(
            (ids(&down), down.scope.node(&tree)),
            (vec![sid(17)], sid(14))
        );
        // Escape: 범위가 최상위가 아니면 범위 node를 고르고 한 단계 위.
        let mut tools = CanvasToolState::new();
        let EscapeStep::ScopeUp(esc) = escape(&tree, down.scope, &down.selection, &mut tools)
        else {
            panic!("범위 위로");
        };
        assert_eq!((ids(&esc), esc.scope.node(&tree)), (vec![sid(14)], sid(11)));
        let EscapeStep::ScopeUp(esc) = escape(&tree, esc.scope, &esc.selection, &mut tools) else {
            panic!("범위 위로");
        };
        assert_eq!((ids(&esc), esc.scope.node(&tree)), (vec![sid(11)], sid(10)));
        assert!(matches!(
            escape(&tree, esc.scope, &esc.selection, &mut tools),
            EscapeStep::Deselect(_)
        ));
        // 최상위 node의 Shift-Return은 그대로, 빈 선택도 그대로.
        assert_eq!(
            exit_to_parent(&tree, top, &sel(&[11])),
            Navigation::Unchanged
        );
        assert_eq!(
            exit_to_parent(&tree, top, &Selection::empty()),
            Navigation::Unchanged
        );
        // 섞인 깊이: 최상위 12가 primary면 올린 뒤에도 12가 primary입니다.
        let mixed = change(exit_to_parent(
            &tree,
            top,
            &Selection::new([sid(12), sid(13)], Some(sid(12))),
        ));
        assert_eq!(ids(&mixed), vec![sid(12), sid(11)]);
        assert_eq!(mixed.selection.primary(), Some(sid(12)));
        let mixed = change(exit_to_parent(
            &tree,
            top,
            &Selection::new([sid(13), sid(12)], Some(sid(13))),
        ));
        assert_eq!(mixed.selection.primary(), Some(sid(11)));
        // 컨테이너 몸체 더블클릭은 선택 없이 범위만 들어갑니다.
        let body = change(double_click(
            &tree,
            top,
            &sel(&[11]),
            &stack(&[11, 10]),
            NONE,
        ));
        assert!(body.selection.is_empty());
        assert_eq!(body.scope.node(&tree), sid(11));
        // 선택되지 않은 node 더블클릭은 클릭과 같습니다.
        let fresh = change(double_click(
            &tree,
            top,
            &Selection::empty(),
            &deep_point,
            NONE,
        ));
        assert_eq!(ids(&fresh), vec![sid(11)]);
        // Fragment도 들어갈 수 있습니다.
        let frag = change(enter(&tree, top, &sel(&[12])));
        assert_eq!(
            (ids(&frag), frag.scope.node(&tree)),
            (vec![sid(18), sid(19)], sid(12))
        );
    }

    #[test]
    fn tab_cycles_siblings_in_order_key_order_and_shift_double_click_adds_deep() {
        let fixture = nested();
        let tree = SelectTree::new(&fixture.model, fixture.root);
        let top = EditingScope::top(&tree);
        // 선택이 없으면 범위의 첫 자식, Shift-Tab은 마지막.
        assert_eq!(
            ids(&change(cycle_sibling(
                &tree,
                top,
                &Selection::empty(),
                false
            ))),
            vec![sid(11)]
        );
        assert_eq!(
            ids(&change(cycle_sibling(
                &tree,
                top,
                &Selection::empty(),
                true
            ))),
            vec![sid(21)]
        );
        // 11 → 12 → 21 → 11 (잠김·숨김 없음).
        let mut current = sel(&[11]);
        let mut order = Vec::new();
        for _ in 0..3 {
            let next = change(cycle_sibling(&tree, top, &current, false));
            order.push(next.selection.ids()[0]);
            current = next.selection;
        }
        assert_eq!(order, vec![sid(12), sid(21), sid(11)]);
        // 범위 11 안: 13 ↔ 14(잠긴 15, 숨긴 16은 건너뜀), Shift-Tab은 거꾸로.
        let inside = EditingScope::at(&tree, sid(11));
        let next = change(cycle_sibling(&tree, inside, &sel(&[14]), false));
        assert_eq!(
            (ids(&next), next.scope.node(&tree)),
            (vec![sid(13)], sid(11))
        );
        let back = change(cycle_sibling(&tree, inside, &sel(&[13]), true));
        assert_eq!(ids(&back), vec![sid(14)]);
        // Shift-더블클릭: 깊은 node 추가, 범위는 공통 조상.
        let added = change(double_click(
            &tree,
            top,
            &sel(&[12]),
            &stack(&[17, 14, 11, 10]),
            Modifiers::SHIFT,
        ));
        assert_eq!(ids(&added), vec![sid(12), sid(17)]);
        assert_eq!(added.selection.primary(), Some(sid(17)));
        assert_eq!(added.scope.node(&tree), sid(10));
        assert_eq!(
            add_deep(&tree, top, &added.selection, &stack(&[17, 14, 11, 10])),
            Navigation::Unchanged
        );
    }

    #[test]
    fn sidebar_deep_selection_refits_scope_and_scope_moves_stay_out_of_history_and_save() {
        // 검증 기준(XII.10.4): 사이드바 깊은 선택 후 범위 결과, 범위 변경이 save bytes와 history에 없음.
        let fixture = nested();
        let root = fixture.root;
        let doc = DocumentState::new(fixture.model);
        let before = save_bytes(&doc);
        let tree = SelectTree::new(doc.model(), root);
        let mut history = WindowHistory::new(window(2));
        // 사이드바에서 17을 고름: 선택 event 하나, 범위는 17의 부모 14.
        history
            .record_selection(&doc, &SetSelection::new(sel(&[17])))
            .unwrap();
        let scope = EditingScope::top(&tree).refit(&tree, history.view().selection());
        assert_eq!(scope.node(&tree), sid(14));
        // 범위만 바뀌는 이동(Return으로 들어갔다가 선택이 같은 경우 등)은 기록하지 않습니다.
        let len = history.history().len();
        let moved = EditingScope::at(&tree, sid(11)).refit(&tree, history.view().selection());
        assert_eq!(moved.node(&tree), sid(14));
        assert_eq!(history.history().len(), len);
        // 선택이 바뀌는 범위 이동만 SetSelection 하나.
        let up = change(exit_to_parent(&tree, scope, history.view().selection()));
        let event = up.event(history.view().selection()).unwrap();
        history.record_selection(&doc, &event).unwrap();
        assert_eq!(history.history().len(), len + 1);
        assert_eq!(
            save_bytes(&doc),
            before,
            "범위·선택 이동은 저장 bytes를 바꾸지 않음"
        );
    }

    #[test]
    fn text_and_path_request_content_editing() {
        use crate::canvas::select_fixtures::record;
        use stellar_document::path::{IdentifiedPath, PathData, WindingRule};
        use stellar_document::records::NodePayload;
        use stellar_document::text::TextPayload;
        let mut fixture = nested();
        fixture
            .model
            .insert_node(record(
                30,
                11,
                0x50,
                NodePayload::Text(TextPayload::empty(0)),
            ))
            .unwrap();
        let path =
            IdentifiedPath::new(PathData::new(WindingRule::NonZero, vec![]).unwrap(), vec![])
                .unwrap();
        fixture
            .model
            .insert_node(record(31, 11, 0x60, NodePayload::Path(path)))
            .unwrap();
        let tree = SelectTree::new(&fixture.model, fixture.root);
        let inside = EditingScope::at(&tree, sid(11));
        assert_eq!(
            enter(&tree, inside, &sel(&[30])),
            Navigation::EditContent(sid(30))
        );
        assert_eq!(
            enter(&tree, inside, &sel(&[31])),
            Navigation::EditContent(sid(31))
        );
        assert_eq!(
            double_click(&tree, inside, &sel(&[30]), &stack(&[30, 11, 10]), NONE),
            Navigation::EditContent(sid(30))
        );
        assert_eq!(enter(&tree, inside, &sel(&[13])), Navigation::Unchanged);
    }
}

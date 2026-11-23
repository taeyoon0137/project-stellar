//! 캔버스 클릭 선택과 편집 범위 (XII.10.3, ADR-0104 결정 3).
//!
//! 선택 규칙은 문서 tree([`SelectTree`])와 커서 아래 node 목록([`PointStack`])만 보는 순수 함수입니다. 결과는 새 선택과
//! 새 편집 범위([`SelectionChange`])이고, 기록할 history event는 [`SelectionChange::event`]가 만드는 `SetSelection` 하나
//! (바뀌지 않으면 없음)입니다. 편집 범위는 저장·history에 들어가지 않습니다.
//!
//! # 클릭 규칙
//!
//! 커서 아래 가장 위의 잠기지 않은 node를 leaf라 합니다.
//!
//! - leaf가 없거나 View root이면 빈 곳입니다. 선택을 비우고 범위를 View root로 돌립니다(Shift는 그대로).
//! - leaf가 편집 범위 S 자신이거나 S의 조상이면 leaf를 고르고 범위는 leaf의 부모입니다.
//! - 아니면 leaf 조상 사슬에서 처음 만나는 S의 조상(S 포함) A를 새 범위로 하고 A의 직계 자식을 고릅니다. S 안의
//!   클릭은 S의 직계 자식(Fragment는 묶음, ComponentInstance는 한 덩어리)이고 S 밖 클릭은 가장 가까운 공통 조상으로
//!   범위를 맞춥니다.
//! - ⌘(깊은 선택)는 leaf 자신, 선택할 수 없으면 가장 가까운 선택 가능 조상을 고르고 범위는 그 부모입니다.
//! - Shift는 고른 node를 넣고 뺍니다. 범위는 선택한 node 부모들의 가장 가까운 공통 조상으로 맞춥니다.
//! - Option은 다중 선택에서 이미 선택된 node를 primary로 지정합니다(선택 집합은 그대로).
//! - ⌘A는 범위의 선택 가능 자식 전체입니다.

use stellar_commands::lock::NodeLock;
use stellar_commands::selection::{Selection, SelectionPolicy, SetSelection};
use stellar_core_types::ids::StableId;
use stellar_document::model::DocumentModel;
use stellar_document::node_kind::NodeKind;
use stellar_document::records::NodePayload;

use super::hit::PointStack;
use super::tool::{CanvasToolState, Modifiers};

/// View root subtree를 선택 규칙으로 읽는 보기.
#[derive(Debug, Clone, Copy)]
pub struct SelectTree<'a> {
    model: &'a DocumentModel,
    root: StableId,
    policy: SelectionPolicy,
}

impl<'a> SelectTree<'a> {
    /// root(View root Frame) 아래 tree. 숨김 node는 선택하지 않습니다(기본 정책).
    pub fn new(model: &'a DocumentModel, root: StableId) -> SelectTree<'a> {
        SelectTree {
            model,
            root,
            policy: SelectionPolicy::default(),
        }
    }

    /// View `view`의 root 아래 tree. View가 없으면 `None`.
    pub fn for_view(model: &'a DocumentModel, view: StableId) -> Option<SelectTree<'a>> {
        let root = model
            .definition()
            .pages
            .iter()
            .flat_map(|page| &page.views)
            .find(|v| v.id == view)?
            .root_frame;
        model.contains(root).then(|| SelectTree::new(model, root))
    }

    /// 문서.
    pub fn model(&self) -> &'a DocumentModel {
        self.model
    }

    /// View root.
    pub fn root(&self) -> StableId {
        self.root
    }

    /// 선택 정책.
    pub fn policy(&self) -> SelectionPolicy {
        self.policy
    }

    /// 부모. root나 없는 node는 `None`.
    pub fn parent(&self, node: StableId) -> Option<StableId> {
        if node == self.root {
            return None;
        }
        let fields = self
            .model
            .runtime_id(node)
            .and_then(|id| self.model.columns().get(id))?;
        fields.parent.and_then(|id| self.model.stable_id(id))
    }

    /// `[node, 부모, …, root]`. root 아래가 아니면 `None`.
    pub fn ancestry(&self, node: StableId) -> Option<Vec<StableId>> {
        if !self.model.contains(node) {
            return None;
        }
        let mut chain = vec![node];
        let mut current = node;
        let mut remaining = self.model.node_count();
        while current != self.root {
            if remaining == 0 {
                return None;
            }
            remaining -= 1;
            current = self.parent(current)?;
            chain.push(current);
        }
        Some(chain)
    }

    /// root 아래(root 포함)인지.
    pub fn contains(&self, node: StableId) -> bool {
        self.ancestry(node).is_some()
    }

    /// `ancestor`가 `node`의 조상이거나 자신인지.
    pub fn is_ancestor_or_self(&self, ancestor: StableId, node: StableId) -> bool {
        self.ancestry(node)
            .is_some_and(|chain| chain.contains(&ancestor))
    }

    /// node 종류.
    pub fn kind(&self, node: StableId) -> Option<NodeKind> {
        self.model.payload(node).map(NodePayload::kind)
    }

    /// 들어갈 수 있는 컨테이너(Frame, Fragment)인지.
    pub fn is_container(&self, node: StableId) -> bool {
        matches!(self.kind(node), Some(NodeKind::Frame | NodeKind::Fragment))
    }

    /// 캔버스에서 고를 수 있는지: root가 아닌 root 아래 node이고, 숨김 정책을 통과하고, 자신·조상이 잠기지 않았습니다.
    pub fn selectable(&self, node: StableId) -> bool {
        node != self.root
            && self.contains(node)
            && self.policy.is_selectable(self.model, node)
            && NodeLock::of(self.model, node).is_some_and(|lock| !lock.any())
    }

    /// 자식(OrderKey 순, 뒤에 그린 것부터).
    pub fn children(&self, node: StableId) -> Vec<StableId> {
        self.model.children_in_order(node)
    }

    /// 선택 가능한 자식(OrderKey 순).
    pub fn selectable_children(&self, node: StableId) -> Vec<StableId> {
        self.children(node)
            .into_iter()
            .filter(|child| self.selectable(*child))
            .collect()
    }

    /// 여러 node의 가장 가까운 공통 조상(자신 포함). 비었거나 root 밖 node가 있으면 `None`.
    pub fn common_ancestor(&self, nodes: impl IntoIterator<Item = StableId>) -> Option<StableId> {
        let mut common: Option<Vec<StableId>> = None;
        for node in nodes {
            let mut chain = self.ancestry(node)?;
            chain.reverse(); // root → node
            common = Some(match common {
                None => chain,
                Some(prefix) => prefix
                    .into_iter()
                    .zip(chain)
                    .take_while(|(a, b)| a == b)
                    .map(|(a, _)| a)
                    .collect(),
            });
        }
        common.and_then(|chain| chain.last().copied())
    }
}

/// 창의 편집 범위(Editing Scope). View root 또는 그 아래 컨테이너입니다. 저장·history 대상이 아닙니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EditingScope {
    node: StableId,
}

impl EditingScope {
    /// 최상위 범위(View root).
    pub fn top(tree: &SelectTree<'_>) -> EditingScope {
        EditingScope { node: tree.root() }
    }

    /// `node`를 범위로 합니다. root 아래 컨테이너가 아니면 최상위입니다.
    pub fn at(tree: &SelectTree<'_>, node: StableId) -> EditingScope {
        if node == tree.root() || (tree.contains(node) && tree.is_container(node)) {
            EditingScope { node }
        } else {
            EditingScope::top(tree)
        }
    }

    /// 범위 node. 문서가 바뀌어 범위가 사라졌으면 최상위로 봅니다.
    pub fn node(&self, tree: &SelectTree<'_>) -> StableId {
        EditingScope::at(tree, self.node).node
    }

    /// 최상위 범위인지.
    pub fn is_top(&self, tree: &SelectTree<'_>) -> bool {
        self.node(tree) == tree.root()
    }

    /// 선택에 맞춘 범위: 모든 선택 node의 부모가 지금 범위이면 그대로, 아니면 부모들의 가장 가까운 공통 조상입니다.
    /// 선택이 비었으면 그대로입니다(사이드바 깊은 선택 뒤에도 씁니다).
    pub fn refit(&self, tree: &SelectTree<'_>, selection: &Selection) -> EditingScope {
        let current = self.node(tree);
        let parents: Vec<StableId> = selection
            .ids()
            .iter()
            .filter_map(|id| tree.parent(*id))
            .collect();
        if parents.is_empty() || parents.iter().all(|p| *p == current) {
            return EditingScope { node: current };
        }
        match tree.common_ancestor(parents) {
            Some(ancestor) => EditingScope::at(tree, ancestor),
            None => EditingScope::top(tree),
        }
    }
}

/// 선택 규칙의 결과: 새 선택과 새 편집 범위.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionChange {
    /// 새 선택.
    pub selection: Selection,
    /// 새 편집 범위.
    pub scope: EditingScope,
}

impl SelectionChange {
    /// 기록할 선택 event. `current`와 같으면 기록할 것이 없어 `None`입니다.
    pub fn event(&self, current: &Selection) -> Option<SetSelection> {
        (self.selection != *current).then(|| SetSelection::new(self.selection.clone()))
    }
}

/// 클릭이 고를 node와 그때의 범위.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pick {
    /// 고를 node.
    pub target: StableId,
    /// 고른 뒤의 편집 범위 node.
    pub scope: StableId,
}

/// 클릭(또는 hover, XII.10.7)이 고를 node. 빈 곳이면 `None`입니다. 모듈 문서의 클릭 규칙입니다.
pub fn click_pick(
    tree: &SelectTree<'_>,
    scope: EditingScope,
    stack: &PointStack,
    deep: bool,
) -> Option<Pick> {
    let leaf = stack.leaf().filter(|leaf| *leaf != tree.root())?;
    let chain = tree.ancestry(leaf)?;
    if deep {
        let target = chain.iter().copied().find(|node| tree.selectable(*node))?;
        return Some(Pick {
            target,
            scope: tree.parent(target)?,
        });
    }
    let scope = scope.node(tree);
    let scope_chain = tree.ancestry(scope)?;
    if scope_chain.contains(&leaf) {
        // 범위 자신이나 범위의 조상 몸체: 그 node를 고르고 한 단계 위 범위로.
        return tree.selectable(leaf).then(|| Pick {
            target: leaf,
            scope: tree.parent(leaf).unwrap_or(tree.root()),
        });
    }
    let index = chain.iter().position(|node| scope_chain.contains(node))?;
    let target = chain[index - 1];
    tree.selectable(target).then_some(Pick {
        target,
        scope: chain[index],
    })
}

/// 클릭 하나의 결과(모듈 문서의 클릭 규칙). `modifiers.command`는 깊은 선택, `shift`는 토글, `option`은 primary
/// 지정입니다.
pub fn resolve_click(
    tree: &SelectTree<'_>,
    scope: EditingScope,
    current: &Selection,
    stack: &PointStack,
    modifiers: Modifiers,
) -> SelectionChange {
    let current = current.retain(|id| tree.contains(id));
    let pick = click_pick(tree, scope, stack, modifiers.command);
    match pick {
        None if modifiers.shift => SelectionChange {
            selection: current,
            scope: EditingScope::at(tree, scope.node(tree)),
        },
        None => SelectionChange {
            selection: Selection::empty(),
            scope: EditingScope::top(tree),
        },
        Some(pick) if modifiers.option && current.len() > 1 && current.contains(pick.target) => {
            SelectionChange {
                selection: Selection::new(current.ids().iter().copied(), Some(pick.target)),
                scope: EditingScope::at(tree, scope.node(tree)),
            }
        }
        Some(pick) if modifiers.shift => {
            let selection = toggled(&current, pick.target);
            let scope = EditingScope::at(tree, pick.scope).refit(tree, &selection);
            SelectionChange { selection, scope }
        }
        Some(pick) => SelectionChange {
            selection: Selection::new([pick.target], Some(pick.target)),
            scope: EditingScope::at(tree, pick.scope),
        },
    }
}

/// `target`을 넣거나 뺀 선택. 넣으면 primary가 되고, 빼면 primary 규칙(마지막 원소)을 다시 적용합니다.
pub(crate) fn toggled(current: &Selection, target: StableId) -> Selection {
    if current.contains(target) {
        let primary = current.primary().filter(|p| *p != target);
        Selection::new(
            current.ids().iter().copied().filter(|id| *id != target),
            primary,
        )
    } else {
        Selection::new(current.ids().iter().copied().chain([target]), Some(target))
    }
}

/// ⌘A: 범위의 선택 가능 자식 전체.
pub fn select_all(tree: &SelectTree<'_>, scope: EditingScope) -> SelectionChange {
    let scope_node = scope.node(tree);
    let children = tree.selectable_children(scope_node);
    SelectionChange {
        selection: Selection::new(children, None),
        scope: EditingScope::at(tree, scope_node),
    }
}

/// Escape 한 번의 단계(ADR-0104 결정 3.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscapeStep {
    /// 진행 중 제스처를 취소합니다(호출자가 제스처 기록기·마퀴를 취소).
    CancelGesture,
    /// Move가 아닌 도구를 Move로 돌렸습니다.
    ResetTool,
    /// 범위 node를 고르고 범위를 한 단계 위로 올립니다(XII.10.4).
    ScopeUp(SelectionChange),
    /// 최상위 범위에서 선택을 해제합니다.
    Deselect(SelectionChange),
    /// 할 일이 없습니다.
    Nothing,
}

/// Escape: 진행 중 제스처 취소 → Move가 아닌 도구에서 Move로 → 범위가 최상위가 아니면 한 단계 위 → 최상위면 선택
/// 해제.
pub fn escape(
    tree: &SelectTree<'_>,
    scope: EditingScope,
    current: &Selection,
    tools: &mut CanvasToolState,
) -> EscapeStep {
    if tools.gesture_active() {
        return EscapeStep::CancelGesture;
    }
    if tools.reset_to_move() {
        return EscapeStep::ResetTool;
    }
    let scope_node = scope.node(tree);
    if scope_node != tree.root() {
        let selection = if tree.selectable(scope_node) {
            Selection::new([scope_node], Some(scope_node))
        } else {
            Selection::empty()
        };
        return EscapeStep::ScopeUp(SelectionChange {
            selection,
            scope: EditingScope::at(tree, tree.parent(scope_node).unwrap_or(tree.root())),
        });
    }
    if current.is_empty() {
        EscapeStep::Nothing
    } else {
        EscapeStep::Deselect(SelectionChange {
            selection: Selection::empty(),
            scope: EditingScope::top(tree),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::hit::{point_stack, StackEntry};
    use crate::canvas::select_fixtures::{nested, scene_of, sid, view_point};
    use crate::canvas::tool::{CanvasTool, ToolInput};
    use crate::window_session::test_fixtures::window;
    use stellar_commands::state::DocumentState;
    use stellar_history::window::WindowHistory;
    use stellar_scene::narrow_phase::{LayoutBoxesOnly, NarrowPhaseOptions};

    fn stack(ids: &[u64]) -> PointStack {
        PointStack::new(ids.iter().map(|n| StackEntry {
            node: sid(*n),
            locked: false,
        }))
    }

    fn sel(ids: &[u64]) -> Selection {
        Selection::new(ids.iter().map(|n| sid(*n)), None)
    }

    const NONE: Modifiers = Modifiers::NONE;

    #[test]
    fn nested_fixture_click_matrix_from_scene_hits() {
        // 검증 기준(XII.10.3): `canvas/select-scope-nested`에서 클릭 위치별 선택 결과.
        let fixture = nested();
        let tree = SelectTree::new(&fixture.model, fixture.root);
        let scene = scene_of(&fixture.model, fixture.root);
        let top = EditingScope::top(&tree);
        let at = |x: f32, y: f32| {
            point_stack(
                &scene.index,
                &scene.viewport,
                view_point(x, y),
                &NarrowPhaseOptions::EXACT,
                &LayoutBoxesOnly,
            )
        };
        let inside_11 = EditingScope::at(&tree, sid(11));
        // (클릭 위치, 범위, 수식키) → (선택, 범위).
        let cases: [((f32, f32), EditingScope, Modifiers, &[u64], u64); 9] = [
            // 최상위: Rectangle 13 위를 눌러도 범위 직계 자식 Frame 11.
            ((130., 130.), top, NONE, &[11], 10),
            ((220., 220.), top, NONE, &[11], 10),
            // Fragment는 묶음 단위.
            ((310., 30.), top, NONE, &[12], 10),
            // Root 빈 영역·캔버스 밖은 빈 곳.
            ((700., 50.), top, NONE, &[], 10),
            ((900., 900.), top, NONE, &[], 10),
            // 범위 11 안: 직계 자식.
            ((130., 130.), inside_11, NONE, &[13], 11),
            ((220., 220.), inside_11, NONE, &[14], 11),
            // 범위 11 밖 클릭은 공통 조상(root)으로 범위를 맞춤.
            ((310., 30.), inside_11, NONE, &[12], 10),
            // ⌘ 깊은 선택: 잠긴 15를 통과한 가장 깊은 node.
            ((220., 220.), top, Modifiers::COMMAND, &[17], 14),
        ];
        for ((x, y), scope, modifiers, selected, scope_after) in cases {
            let change = resolve_click(&tree, scope, &Selection::empty(), &at(x, y), modifiers);
            assert_eq!(
                change.selection.ids(),
                sel(selected).ids(),
                "({x}, {y}) {modifiers:?}"
            );
            assert_eq!(change.scope.node(&tree), sid(scope_after), "({x}, {y})");
        }
        // 범위 11의 몸체(자식 없는 곳)를 누르면 11 자신을 고르고 범위는 한 단계 위.
        let change = resolve_click(&tree, inside_11, &Selection::empty(), &at(150., 280.), NONE);
        assert_eq!(change.selection.ids(), &[sid(11)]);
        assert_eq!(change.scope.node(&tree), sid(10));
    }

    #[test]
    fn instance_is_one_block_and_hidden_locked_never_selected() {
        let fixture = nested();
        let tree = SelectTree::new(&fixture.model, fixture.root);
        let top = EditingScope::top(&tree);
        // 인스턴스 안 scene node의 source는 인스턴스입니다.
        let change = resolve_click(&tree, top, &Selection::empty(), &stack(&[21, 10]), NONE);
        assert_eq!(change.selection.ids(), &[sid(21)]);
        let deep = resolve_click(
            &tree,
            top,
            &Selection::empty(),
            &stack(&[21, 10]),
            Modifiers::COMMAND,
        );
        assert_eq!(
            deep.selection.ids(),
            &[sid(21)],
            "인스턴스 안으로 깊게 들어가지 않음"
        );
        assert!(!tree.selectable(sid(15)), "잠김");
        assert!(!tree.selectable(sid(16)), "숨김");
        assert!(!tree.selectable(sid(10)), "View root");
        let locked_top = PointStack::new([
            StackEntry {
                node: sid(15),
                locked: true,
            },
            StackEntry {
                node: sid(13),
                locked: false,
            },
        ]);
        let change = resolve_click(
            &tree,
            EditingScope::at(&tree, sid(11)),
            &Selection::empty(),
            &locked_top,
            NONE,
        );
        assert_eq!(change.selection.ids(), &[sid(13)]);
    }

    #[test]
    fn shift_toggles_option_sets_primary_and_select_all_takes_scope_children() {
        let fixture = nested();
        let tree = SelectTree::new(&fixture.model, fixture.root);
        let top = EditingScope::top(&tree);
        let first = resolve_click(&tree, top, &Selection::empty(), &stack(&[13, 11, 10]), NONE);
        let shift = Modifiers::SHIFT;
        let added = resolve_click(
            &tree,
            first.scope,
            &first.selection,
            &stack(&[18, 12, 10]),
            shift,
        );
        assert_eq!(added.selection.ids(), &[sid(11), sid(12)]);
        assert_eq!(added.selection.primary(), Some(sid(12)));
        // Option-클릭: 이미 선택된 11을 primary로, 집합은 그대로.
        let primary = resolve_click(
            &tree,
            added.scope,
            &added.selection,
            &stack(&[13, 11, 10]),
            Modifiers::OPTION,
        );
        assert_eq!(primary.selection.ids(), &[sid(11), sid(12)]);
        assert_eq!(primary.selection.primary(), Some(sid(11)));
        // Shift로 다시 누르면 뺍니다. 빈 곳 Shift는 그대로.
        let removed = resolve_click(&tree, top, &primary.selection, &stack(&[18, 12, 10]), shift);
        assert_eq!(removed.selection.ids(), &[sid(11)]);
        let kept = resolve_click(&tree, top, &removed.selection, &stack(&[10]), shift);
        assert_eq!(kept.selection, removed.selection);
        // 빈 곳 클릭: 해제, 기록할 event 하나.
        let cleared = resolve_click(&tree, top, &removed.selection, &PointStack::empty(), NONE);
        assert!(cleared.selection.is_empty());
        assert_eq!(
            cleared.event(&removed.selection),
            Some(SetSelection::new(Selection::empty()))
        );
        assert_eq!(cleared.event(&Selection::empty()), None, "같으면 기록 없음");
        // ⌘A: 범위 형제 전체(잠김·숨김 제외).
        assert_eq!(
            select_all(&tree, top).selection.ids(),
            &[sid(11), sid(12), sid(21)]
        );
        assert_eq!(
            select_all(&tree, EditingScope::at(&tree, sid(11)))
                .selection
                .ids(),
            &[sid(13), sid(14)]
        );
        // 범위가 다른 깊이의 Shift 추가는 공통 조상으로 범위를 맞춥니다.
        let mixed = resolve_click(
            &tree,
            EditingScope::at(&tree, sid(11)),
            &sel(&[13]),
            &stack(&[18, 12, 10]),
            shift,
        );
        assert_eq!(mixed.selection.ids(), &[sid(13), sid(12)]);
        assert_eq!(mixed.scope.node(&tree), sid(10));
    }

    #[test]
    fn top_scope_escape_deselects_once_and_records_one_entry() {
        // 검증 기준(XII.10.3): 최상위 편집 범위 Escape의 선택 해제 fixture, 결과는 SetSelection 하나.
        let fixture = nested();
        let root = fixture.root;
        let doc = DocumentState::new(fixture.model);
        let tree = SelectTree::new(doc.model(), root);
        let mut history = WindowHistory::new(window(1));
        let mut tools = CanvasToolState::new();
        let top = EditingScope::top(&tree);
        let click = resolve_click(
            &tree,
            top,
            history.view().selection(),
            &stack(&[13, 11, 10]),
            NONE,
        );
        let event = click.event(history.view().selection()).unwrap();
        history.record_selection(&doc, &event).unwrap();
        assert_eq!(history.history().len(), 1, "클릭 하나 = 선택 항목 하나");
        // 제스처 중이면 먼저 제스처 취소, 그다음 도구 복귀.
        tools.handle(ToolInput::GestureStarted);
        let current = history.view().selection().clone();
        assert_eq!(
            escape(&tree, click.scope, &current, &mut tools),
            EscapeStep::CancelGesture
        );
        tools.handle(ToolInput::GestureEnded);
        tools.select(CanvasTool::Frame);
        assert_eq!(
            escape(&tree, click.scope, &current, &mut tools),
            EscapeStep::ResetTool
        );
        let EscapeStep::Deselect(change) = escape(&tree, click.scope, &current, &mut tools) else {
            panic!("최상위 범위는 선택 해제");
        };
        history
            .record_selection(&doc, &change.event(&current).unwrap())
            .unwrap();
        assert!(history.view().selection().is_empty());
        assert_eq!(history.history().len(), 2);
        assert_eq!(
            escape(&tree, change.scope, history.view().selection(), &mut tools),
            EscapeStep::Nothing
        );
    }

    #[test]
    fn scope_refits_to_the_common_ancestor_and_falls_back_to_top() {
        let fixture = nested();
        let tree = SelectTree::new(&fixture.model, fixture.root);
        let top = EditingScope::top(&tree);
        assert_eq!(top.refit(&tree, &sel(&[17])).node(&tree), sid(14));
        assert_eq!(top.refit(&tree, &sel(&[17, 13])).node(&tree), sid(11));
        assert_eq!(top.refit(&tree, &sel(&[18, 13])).node(&tree), sid(10));
        assert_eq!(
            EditingScope::at(&tree, sid(14))
                .refit(&tree, &Selection::empty())
                .node(&tree),
            sid(14)
        );
        // 컨테이너가 아니거나 없는 node는 범위가 될 수 없습니다.
        assert!(EditingScope::at(&tree, sid(13)).is_top(&tree));
        assert!(EditingScope::at(&tree, sid(999)).is_top(&tree));
        assert_eq!(tree.common_ancestor([sid(17), sid(19)]), Some(sid(10)));
    }
}

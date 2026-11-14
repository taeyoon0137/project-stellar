//! 접근성 진단 (IX.8.5).
//!
//! [`diagnose`]는 접근성 트리의 의미 문제를 찾습니다(기술 아키텍처 §18 진단 예). 트리 구조 자체의 유효성
//! (cycle·orphan 등)은 [`AccessibilityTree::validate`]가 맡고, 이 모듈은 root에서 도달하는 node만 봅니다.
//!
//! | 진단 | code | 심각도 | 규칙 |
//! |---|---|---|---|
//! | label 누락 | `missing-label` | 오류 | `Button`·`Radio`·`TextField`·`Image`·`Dialog`에 사용할 수 있는 label이 없음. `Button`·`Radio`는 자손 `StaticText`·`Image`의 label로 이름을 계산할 수 있으면 통과 |
//! | 잘못된 중첩 | `invalid-nesting` | 오류 | 상호작용 역할(`Button`·`TextField`·`Radio`) 안의 상호작용 역할, `List` 밖 `ListItem`, `RadioGroup` 밖 `Radio`(`Generic` 조상은 건너뜀) |
//! | focus 가능하지만 동작 없음 | `focusable-without-action` | 경고 | 상수 `Focusable = true`인데 `Activate`·`SetValue`·`Select`·`Dismiss`가 없음. 탐색 container(`List`·`RadioGroup`·`Dialog`)는 제외 |
//! | 동작이 있지만 focus 불가 | `action-without-focus` | 오류 | `Activate`·`SetValue`가 있는데 `Focusable`이 없거나 상수 `false`(키보드 접근 불가) |
//! | 읽기 순서 중복 | `duplicate-reading-order` | 오류 | 같은 부모(또는 root 목록)의 형제가 같은 명시 읽기 순서를 가짐 |
//!
//! "사용할 수 있는 label"은 비어 있지 않은 상수 문자열이거나, 정적으로 값을 알 수 없는 binding(속성·상태 등)
//! 입니다. 빈·공백 문자열과 문자열이 아닌 상수는 label로 보지 않습니다.
//!
//! 순회는 반복 preorder이며 결과 순서는 root 목록 중복 검사 → node preorder 순(node 자신의 진단 → 그 자식의 읽기
//! 순서 중복 검사)입니다. 같은 tree는 항상 같은 목록을 냅니다.
//!
//! 조상 문맥(가장 가까운 상호작용 조상·비-`Generic` 조상)은 preorder stack으로 물려주고, 내용 이름은 강결합 요소마다
//! 한 번 계산합니다. 따라서 깊은 tree에서도 시간이 node·간선 수에 비례합니다(node마다 조상·자손을 다시 훑지 않음).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;

use crate::node::{
    AccessibilityBinding, AccessibilityNode, AccessibilityRole, ActionKind, StateKind,
};
use crate::tree::{AccessibilityTree, Components};

/// 심각도.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// 오류.
    Error,
    /// 경고.
    Warning,
}

/// 잘못된 중첩 규칙.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NestingRule {
    /// 상호작용 역할 안의 상호작용 역할.
    InteractiveInsideInteractive,
    /// `List` 밖의 `ListItem`.
    ListItemOutsideList,
    /// `RadioGroup` 밖의 `Radio`.
    RadioOutsideRadioGroup,
}

/// 진단 문제.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessibilityProblem {
    /// 접근 가능한 이름이 없습니다.
    MissingLabel {
        /// 역할.
        role: AccessibilityRole,
    },
    /// 의미 중첩이 잘못됐습니다.
    InvalidNesting {
        /// 위반 규칙.
        rule: NestingRule,
        /// 문제의 조상(상호작용 조상 또는 가장 가까운 비-`Generic` 조상, 없으면 `None`).
        ancestor: Option<StableId>,
    },
    /// focus 가능하지만 실행할 동작이 없습니다.
    FocusableWithoutAction,
    /// 동작이 있지만 keyboard focus를 받을 수 없습니다.
    ActionWithoutFocus {
        /// 동작.
        action: ActionKind,
    },
    /// 형제가 같은 명시 읽기 순서를 가집니다.
    DuplicateReadingOrder {
        /// 부모(root 목록이면 `None`).
        parent: Option<StableId>,
        /// 중복 순서 값.
        order: u32,
        /// 그 순서를 가진 형제(형제 순서대로).
        nodes: Vec<StableId>,
    },
}

/// 진단 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibilityIssue {
    /// 대상 node(읽기 순서 중복은 그 순서를 가진 첫 형제).
    pub node: StableId,
    /// 문제.
    pub problem: AccessibilityProblem,
}

impl AccessibilityIssue {
    /// 안정 code.
    pub fn code(&self) -> &'static str {
        match self.problem {
            AccessibilityProblem::MissingLabel { .. } => "missing-label",
            AccessibilityProblem::InvalidNesting { .. } => "invalid-nesting",
            AccessibilityProblem::FocusableWithoutAction => "focusable-without-action",
            AccessibilityProblem::ActionWithoutFocus { .. } => "action-without-focus",
            AccessibilityProblem::DuplicateReadingOrder { .. } => "duplicate-reading-order",
        }
    }

    /// 심각도.
    pub fn severity(&self) -> Severity {
        match self.problem {
            AccessibilityProblem::FocusableWithoutAction => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

impl fmt::Display for AccessibilityIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let node = self.node;
        match &self.problem {
            AccessibilityProblem::MissingLabel { role } => {
                write!(f, "{role} {node}에 접근 가능한 이름이 없습니다")
            }
            AccessibilityProblem::InvalidNesting { rule, .. } => match rule {
                NestingRule::InteractiveInsideInteractive => {
                    write!(f, "상호작용 node {node}가 다른 상호작용 node 안에 있습니다")
                }
                NestingRule::ListItemOutsideList => {
                    write!(f, "목록 항목 {node}가 목록 밖에 있습니다")
                }
                NestingRule::RadioOutsideRadioGroup => {
                    write!(f, "radio {node}가 radio group 밖에 있습니다")
                }
            },
            AccessibilityProblem::FocusableWithoutAction => {
                write!(f, "node {node}는 focus를 받지만 실행할 동작이 없습니다")
            }
            AccessibilityProblem::ActionWithoutFocus { action } => {
                write!(
                    f,
                    "node {node}의 {action} 동작은 keyboard로 접근할 수 없습니다"
                )
            }
            AccessibilityProblem::DuplicateReadingOrder { order, nodes, .. } => {
                write!(
                    f,
                    "형제 {}개가 같은 읽기 순서 {order}를 가집니다",
                    nodes.len()
                )
            }
        }
    }
}

const fn is_interactive(role: AccessibilityRole) -> bool {
    matches!(
        role,
        AccessibilityRole::Button | AccessibilityRole::TextField | AccessibilityRole::Radio
    )
}

const fn requires_label(role: AccessibilityRole) -> bool {
    matches!(
        role,
        AccessibilityRole::Button
            | AccessibilityRole::Radio
            | AccessibilityRole::TextField
            | AccessibilityRole::Image
            | AccessibilityRole::Dialog
    )
}

const fn name_from_content(role: AccessibilityRole) -> bool {
    matches!(role, AccessibilityRole::Button | AccessibilityRole::Radio)
}

const fn is_navigation_container(role: AccessibilityRole) -> bool {
    matches!(
        role,
        AccessibilityRole::List | AccessibilityRole::RadioGroup | AccessibilityRole::Dialog
    )
}

const USER_ACTIONS: [ActionKind; 4] = [
    ActionKind::Activate,
    ActionKind::SetValue,
    ActionKind::Select,
    ActionKind::Dismiss,
];

/// 사용할 수 있는 label인지(비어 있지 않은 상수 문자열 또는 정적으로 알 수 없는 binding).
fn usable_label(label: Option<&AccessibilityBinding>) -> bool {
    match label {
        None => false,
        Some(AccessibilityBinding::Constant(TypedValue::String(text))) => !text.trim().is_empty(),
        Some(AccessibilityBinding::Constant(_)) => false,
        Some(_) => true,
    }
}

/// 자손(간선 하나 이상으로 도달하는 node) `StaticText`·`Image`의 label로 이름을 계산할 수 있는 node 집합.
///
/// 강결합 요소를 역위상 순서로 한 번씩 계산하므로 node·간선 수에 비례하고, cycle이 있어도 자손 전체를 봅니다.
fn content_named(tree: &AccessibilityTree) -> BTreeSet<StableId> {
    let Components {
        ids,
        adjacency,
        component,
        members,
    } = tree.components();
    let named_leaf: Vec<bool> = ids
        .iter()
        .map(|id| {
            tree.node(*id).is_some_and(|node| {
                matches!(
                    node.role,
                    AccessibilityRole::StaticText | AccessibilityRole::Image
                ) && usable_label(node.label.as_ref())
            })
        })
        .collect();
    // 요소에서 도달하는 다른 요소는 역위상 순서상 먼저 계산되어 있습니다.
    let mut named = vec![false; members.len()];
    for (number, group) in members.iter().enumerate() {
        let value = group.iter().any(|member| {
            adjacency[*member].iter().any(|child| {
                named_leaf[*child] || (component[*child] != number && named[component[*child]])
            })
        });
        named[number] = value;
    }
    ids.iter()
        .enumerate()
        .filter(|(index, _)| named[component[*index]])
        .map(|(_, id)| *id)
        .collect()
}

/// preorder 경로에서 물려받는 조상 문맥.
#[derive(Debug, Clone, Copy, Default)]
struct Ancestry {
    /// 가장 가까운 상호작용 조상.
    interactive: Option<StableId>,
    /// 가장 가까운 비-`Generic` 조상과 그 역할.
    semantic: Option<(StableId, AccessibilityRole)>,
}

impl Ancestry {
    /// `node`의 자식이 물려받는 문맥.
    fn below(self, node: &AccessibilityNode) -> Ancestry {
        Ancestry {
            interactive: if is_interactive(node.role) {
                Some(node.source)
            } else {
                self.interactive
            },
            semantic: if node.role == AccessibilityRole::Generic {
                self.semantic
            } else {
                Some((node.source, node.role))
            },
        }
    }
}

fn duplicate_orders(
    tree: &AccessibilityTree,
    parent: Option<StableId>,
    siblings: &[StableId],
    issues: &mut Vec<AccessibilityIssue>,
) {
    let mut by_order: BTreeMap<u32, Vec<StableId>> = BTreeMap::new();
    let mut first_seen: Vec<u32> = Vec::new();
    for sibling in siblings {
        let Some(order) = tree.node(*sibling).and_then(|node| node.reading_order) else {
            continue;
        };
        let nodes = by_order.entry(order).or_default();
        if nodes.is_empty() {
            first_seen.push(order);
        }
        nodes.push(*sibling);
    }
    for order in first_seen {
        let nodes = by_order.remove(&order).unwrap_or_default();
        if nodes.len() > 1 {
            issues.push(AccessibilityIssue {
                node: nodes[0],
                problem: AccessibilityProblem::DuplicateReadingOrder {
                    parent,
                    order,
                    nodes,
                },
            });
        }
    }
}

fn node_issues(
    node: &AccessibilityNode,
    ancestry: Ancestry,
    content_named: &BTreeSet<StableId>,
    issues: &mut Vec<AccessibilityIssue>,
) {
    let mut push = |problem| {
        issues.push(AccessibilityIssue {
            node: node.source,
            problem,
        })
    };
    // label 누락.
    if requires_label(node.role)
        && !usable_label(node.label.as_ref())
        && !(name_from_content(node.role) && content_named.contains(&node.source))
    {
        push(AccessibilityProblem::MissingLabel { role: node.role });
    }
    // 잘못된 중첩.
    if is_interactive(node.role) {
        if let Some(ancestor) = ancestry.interactive {
            push(AccessibilityProblem::InvalidNesting {
                rule: NestingRule::InteractiveInsideInteractive,
                ancestor: Some(ancestor),
            });
        }
    }
    let semantic_parent = ancestry.semantic.map(|(id, _)| id);
    let parent_role = ancestry.semantic.map(|(_, role)| role);
    if node.role == AccessibilityRole::ListItem && parent_role != Some(AccessibilityRole::List) {
        push(AccessibilityProblem::InvalidNesting {
            rule: NestingRule::ListItemOutsideList,
            ancestor: semantic_parent,
        });
    }
    if node.role == AccessibilityRole::Radio && parent_role != Some(AccessibilityRole::RadioGroup) {
        push(AccessibilityProblem::InvalidNesting {
            rule: NestingRule::RadioOutsideRadioGroup,
            ancestor: semantic_parent,
        });
    }
    // focus와 동작.
    let has_user_action = USER_ACTIONS
        .iter()
        .any(|action| node.actions.contains_key(action));
    if node.is_state_constant_true(StateKind::Focusable)
        && !has_user_action
        && !is_navigation_container(node.role)
    {
        push(AccessibilityProblem::FocusableWithoutAction);
    }
    let may_focus = match node.states.get(&StateKind::Focusable) {
        None => false,
        Some(AccessibilityBinding::Constant(TypedValue::Boolean(value))) => *value,
        Some(_) => true,
    };
    if !may_focus {
        if let Some(action) = [ActionKind::Activate, ActionKind::SetValue]
            .into_iter()
            .find(|action| node.actions.contains_key(action))
        {
            push(AccessibilityProblem::ActionWithoutFocus { action });
        }
    }
}

/// 접근성 트리를 진단합니다. root에서 도달하는 node만 봅니다(cycle에도 종료).
pub fn diagnose(tree: &AccessibilityTree) -> Vec<AccessibilityIssue> {
    let mut issues = Vec::new();
    let content_named = content_named(tree);
    duplicate_orders(tree, None, tree.roots(), &mut issues);
    let mut visited = BTreeSet::new();
    let mut stack: Vec<(StableId, Ancestry)> = tree
        .roots()
        .iter()
        .rev()
        .map(|root| (*root, Ancestry::default()))
        .collect();
    while let Some((source, ancestry)) = stack.pop() {
        if !visited.insert(source) {
            continue;
        }
        let Some(node) = tree.node(source) else {
            continue;
        };
        node_issues(node, ancestry, &content_named, &mut issues);
        let children = tree.children(source);
        duplicate_orders(tree, Some(source), children, &mut issues);
        let below = ancestry.below(node);
        stack.extend(children.iter().rev().map(|child| (*child, below)));
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{SelectionMode, SourceControl};
    use crate::generate::generate;
    use crate::node::ActionTarget;
    use crate::source::{SourceKind, SourceNode, SourceTree};
    use crate::test_support::id;

    fn tree_of(
        nodes: Vec<AccessibilityNode>,
        edges: &[(u8, &[u8])],
        roots: &[u8],
    ) -> AccessibilityTree {
        let mut tree = AccessibilityTree::new();
        for node in nodes {
            tree.insert(node).expect("node");
        }
        for (parent, children) in edges {
            tree.set_children(id(*parent), children.iter().map(|n| id(*n)).collect());
        }
        for root in roots {
            tree.push_root(id(*root));
        }
        assert_eq!(tree.validate_structure(), Vec::new());
        tree
    }

    fn node(n: u8, role: AccessibilityRole) -> AccessibilityNode {
        AccessibilityNode::new(id(n), role)
    }

    fn focusable(node: AccessibilityNode) -> AccessibilityNode {
        node.with_state(StateKind::Focusable, AccessibilityBinding::flag(true))
            .with_action(ActionKind::Focus, ActionTarget::None)
    }

    fn button(n: u8) -> AccessibilityNode {
        focusable(node(n, AccessibilityRole::Button))
            .with_action(ActionKind::Activate, ActionTarget::SemanticAction(id(100)))
    }

    fn text(n: u8, label: &str) -> AccessibilityNode {
        node(n, AccessibilityRole::StaticText).with_label(AccessibilityBinding::text(label))
    }

    fn issue(n: u8, problem: AccessibilityProblem) -> AccessibilityIssue {
        AccessibilityIssue {
            node: id(n),
            problem,
        }
    }

    #[test]
    fn missing_label_fixture() {
        let tree = tree_of(
            vec![
                node(1, AccessibilityRole::Generic),
                node(2, AccessibilityRole::Image),
                button(3),
                text(4, "확인"),
                button(5),
                focusable(node(6, AccessibilityRole::TextField))
                    .with_action(ActionKind::SetValue, ActionTarget::State(id(101))),
                node(7, AccessibilityRole::Image).with_label(AccessibilityBinding::text("  ")),
                node(8, AccessibilityRole::Dialog)
                    .with_label(AccessibilityBinding::ComponentProperty(id(102))),
                node(9, AccessibilityRole::Image).with_label(AccessibilityBinding::flag(true)),
                button(10).with_label(AccessibilityBinding::text("닫기")),
            ],
            &[(1, &[2, 3, 5, 6, 7, 8, 9, 10]), (3, &[4])],
            &[1],
        );
        let missing = |n: u8, role| issue(n, AccessibilityProblem::MissingLabel { role });
        assert_eq!(
            diagnose(&tree),
            vec![
                missing(2, AccessibilityRole::Image),
                // 3은 자손 텍스트 "확인"으로 이름을 계산합니다.
                missing(5, AccessibilityRole::Button),
                missing(6, AccessibilityRole::TextField),
                missing(7, AccessibilityRole::Image),
                // 8은 binding label이라 정적으로 통과.
                missing(9, AccessibilityRole::Image),
            ]
        );
        assert!(diagnose(&tree)
            .iter()
            .all(|issue| issue.code() == "missing-label" && issue.severity() == Severity::Error));
    }

    #[test]
    fn invalid_nesting_fixture() {
        let tree = tree_of(
            vec![
                node(1, AccessibilityRole::Generic),
                button(2).with_label(AccessibilityBinding::text("바깥")),
                node(3, AccessibilityRole::Generic),
                button(4).with_label(AccessibilityBinding::text("안쪽")),
                node(5, AccessibilityRole::Group).with_label(AccessibilityBinding::text("묶음")),
                node(6, AccessibilityRole::Generic),
                node(7, AccessibilityRole::ListItem),
                focusable(node(8, AccessibilityRole::List)),
                node(9, AccessibilityRole::Generic),
                node(10, AccessibilityRole::ListItem),
                focusable(node(11, AccessibilityRole::Radio))
                    .with_label(AccessibilityBinding::text("홀로"))
                    .with_action(ActionKind::Select, ActionTarget::None),
                node(12, AccessibilityRole::RadioGroup),
                focusable(node(13, AccessibilityRole::Radio))
                    .with_label(AccessibilityBinding::text("하나"))
                    .with_action(ActionKind::Select, ActionTarget::None),
            ],
            &[
                (1, &[2, 5, 8, 11, 12]),
                (2, &[3]),
                (3, &[4]),
                (5, &[6]),
                (6, &[7]),
                (8, &[9]),
                (9, &[10]),
                (12, &[13]),
            ],
            &[1],
        );
        let nesting = |n: u8, rule, ancestor: Option<u8>| {
            issue(
                n,
                AccessibilityProblem::InvalidNesting {
                    rule,
                    ancestor: ancestor.map(id),
                },
            )
        };
        assert_eq!(
            diagnose(&tree),
            vec![
                // Generic 3을 건너 Button 2 안의 Button 4.
                nesting(4, NestingRule::InteractiveInsideInteractive, Some(2)),
                // Generic 6을 건너 가장 가까운 의미 조상은 Group 5.
                nesting(7, NestingRule::ListItemOutsideList, Some(5)),
                // 10은 Generic 9를 건너 List 8 아래라 통과. 11은 의미 조상이 없음.
                nesting(11, NestingRule::RadioOutsideRadioGroup, None),
            ]
        );
    }

    #[test]
    fn focus_and_action_fixture() {
        let tree = tree_of(
            vec![
                node(1, AccessibilityRole::Generic),
                focusable(node(2, AccessibilityRole::Generic)),
                node(3, AccessibilityRole::Button)
                    .with_label(AccessibilityBinding::text("저장"))
                    .with_action(ActionKind::Activate, ActionTarget::None),
                node(4, AccessibilityRole::TextField)
                    .with_label(AccessibilityBinding::text("이름"))
                    .with_state(StateKind::Focusable, AccessibilityBinding::flag(false))
                    .with_action(ActionKind::SetValue, ActionTarget::State(id(101))),
                // binding된 Focusable은 정적으로 focus 가능할 수 있다고 봅니다.
                node(5, AccessibilityRole::Button)
                    .with_label(AccessibilityBinding::text("다음"))
                    .with_state(StateKind::Focusable, AccessibilityBinding::State(id(103)))
                    .with_action(ActionKind::Activate, ActionTarget::None),
                // 탐색 container는 동작 없이 focus 가능해도 통과.
                focusable(node(6, AccessibilityRole::List)),
                // Dialog의 Dismiss는 focus 없이도 통과.
                node(7, AccessibilityRole::Dialog)
                    .with_label(AccessibilityBinding::text("창"))
                    .with_action(ActionKind::Dismiss, ActionTarget::None),
            ],
            &[(1, &[2, 3, 4, 5, 6, 7])],
            &[1],
        );
        let issues = diagnose(&tree);
        assert_eq!(
            issues,
            vec![
                issue(2, AccessibilityProblem::FocusableWithoutAction),
                issue(
                    3,
                    AccessibilityProblem::ActionWithoutFocus {
                        action: ActionKind::Activate
                    }
                ),
                issue(
                    4,
                    AccessibilityProblem::ActionWithoutFocus {
                        action: ActionKind::SetValue
                    }
                ),
            ]
        );
        assert_eq!(issues[0].severity(), Severity::Warning);
        assert_eq!(issues[0].code(), "focusable-without-action");
        assert_eq!(issues[1].severity(), Severity::Error);
        assert_eq!(issues[1].code(), "action-without-focus");
    }

    #[test]
    fn duplicate_reading_order_fixture() {
        let tree = tree_of(
            vec![
                text(1, "가").with_reading_order(1),
                text(2, "나").with_reading_order(1),
                node(3, AccessibilityRole::Generic),
                text(4, "다").with_reading_order(2),
                text(5, "라").with_reading_order(1),
                text(6, "마").with_reading_order(2),
                text(7, "바"),
                text(8, "사").with_reading_order(1),
            ],
            &[(3, &[4, 5, 6, 7, 8])],
            &[1, 2, 3],
        );
        let issues = diagnose(&tree);
        assert_eq!(
            issues,
            vec![
                issue(
                    1,
                    AccessibilityProblem::DuplicateReadingOrder {
                        parent: None,
                        order: 1,
                        nodes: vec![id(1), id(2)],
                    }
                ),
                issue(
                    4,
                    AccessibilityProblem::DuplicateReadingOrder {
                        parent: Some(id(3)),
                        order: 2,
                        nodes: vec![id(4), id(6)],
                    }
                ),
                issue(
                    5,
                    AccessibilityProblem::DuplicateReadingOrder {
                        parent: Some(id(3)),
                        order: 1,
                        nodes: vec![id(5), id(8)],
                    }
                ),
            ]
        );
        assert!(issues
            .iter()
            .all(|issue| issue.code() == "duplicate-reading-order"));
    }

    #[test]
    fn deep_trees_are_diagnosed_in_linear_time() {
        // 깊이 20,000 chain. 조상 문맥과 내용 이름을 node마다 다시 훑지 않으므로 node 수에 비례해 끝납니다.
        // 시간 상한은 debug에서도 넉넉한 참고용입니다(조상·자손을 node마다 다시 훑는 구현은 release에서도 넘습니다).
        const DEPTH: u32 = 20_000;
        let node_id = |n: u32| {
            let mut bytes = [0u8; 16];
            bytes[12..].copy_from_slice(&n.to_be_bytes());
            StableId::from_bytes(bytes)
        };
        let chain = |make: &dyn Fn(u32) -> AccessibilityNode| {
            let mut tree = AccessibilityTree::new();
            for n in 0..=DEPTH {
                tree.insert(make(n)).expect("node");
                if n < DEPTH {
                    tree.set_children(node_id(n), vec![node_id(n + 1)]);
                }
            }
            tree.push_root(node_id(0));
            tree
        };
        let started = std::time::Instant::now();

        // List > Generic × (DEPTH - 1) > ListItem: Generic을 건너 가장 가까운 의미 조상이 List라 진단 0건.
        let generic = chain(&|n| {
            let role = match n {
                0 => AccessibilityRole::List,
                DEPTH => AccessibilityRole::ListItem,
                _ => AccessibilityRole::Generic,
            };
            AccessibilityNode::new(node_id(n), role)
        });
        assert_eq!(diagnose(&generic), Vec::new());

        // label 없는 Button chain: 모두 missing-label, root 밖은 모두 바로 위 Button 안의 중첩.
        // 맨 아래 Button만 자식 텍스트로 이름을 계산하면 그 위 Button은 모두 내용 이름을 가집니다.
        let buttons = chain(&|n| {
            let node = AccessibilityNode::new(node_id(n), AccessibilityRole::Button)
                .with_state(StateKind::Focusable, AccessibilityBinding::flag(true))
                .with_action(ActionKind::Activate, ActionTarget::None);
            if n == DEPTH {
                AccessibilityNode::new(node_id(n), AccessibilityRole::StaticText)
                    .with_label(AccessibilityBinding::text("끝"))
            } else {
                node
            }
        });
        let mut expected = Vec::new();
        for n in 1..DEPTH {
            expected.push(AccessibilityIssue {
                node: node_id(n),
                problem: AccessibilityProblem::InvalidNesting {
                    rule: NestingRule::InteractiveInsideInteractive,
                    ancestor: Some(node_id(n - 1)),
                },
            });
        }
        assert_eq!(diagnose(&buttons), expected);

        let unnamed = chain(&|n| {
            AccessibilityNode::new(node_id(n), AccessibilityRole::Button)
                .with_state(StateKind::Focusable, AccessibilityBinding::flag(true))
                .with_action(ActionKind::Activate, ActionTarget::None)
        });
        let issues = diagnose(&unnamed);
        let missing = issues
            .iter()
            .filter(|issue| issue.code() == "missing-label")
            .count();
        assert_eq!(missing, DEPTH as usize + 1);
        assert_eq!(issues.len(), 2 * DEPTH as usize + 1);

        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn generated_control_trees_are_clean_and_diagnosis_is_deterministic() {
        let mut source = SourceTree::new();
        let nodes = [
            SourceNode::new(id(1), SourceKind::Frame)
                .with_label(AccessibilityBinding::text("크기"))
                .with_control(SourceControl::single_selection_group(id(2), Some(id(50))))
                .with_children([id(2), id(7)]),
            SourceNode::new(id(2), SourceKind::Frame)
                .with_control(SourceControl::list(id(2), SelectionMode::Single, None))
                .with_children([id(3), id(4)]),
            SourceNode::new(id(3), SourceKind::Frame)
                .with_control(SourceControl::button(id(13)))
                .with_children([id(5)]),
            SourceNode::new(id(4), SourceKind::Frame)
                .with_control(SourceControl::button(id(14)))
                .with_children([id(6)]),
            SourceNode::new(id(5), SourceKind::Text),
            SourceNode::new(id(6), SourceKind::Text),
            SourceNode::new(id(7), SourceKind::Frame)
                .with_control(SourceControl::text_input(id(30)))
                .with_label(AccessibilityBinding::text("메모")),
        ];
        for node in nodes {
            source.insert(node).expect("source");
        }
        source.push_root(id(1));
        let tree = generate(&source).expect("생성");
        assert_eq!(diagnose(&tree), Vec::new());

        // 진단은 cycle이 있는 tree에서도 종료하고 결정론적입니다.
        let mut cyclic = tree_of(vec![button(1), button(2)], &[(1, &[2])], &[1]);
        cyclic.push_child(id(2), id(1));
        let first = diagnose(&cyclic);
        assert_eq!(first, diagnose(&cyclic));
        assert!(!first.is_empty());

        // cycle 안 Button도 cycle을 거쳐 도달하는 자손 텍스트로 이름을 계산합니다(1 → (2, 3), 2 → 1).
        let mut named = tree_of(
            vec![button(1), button(2), text(3, "이름")],
            &[(1, &[2, 3])],
            &[1],
        );
        named.push_child(id(2), id(1));
        assert!(diagnose(&named)
            .iter()
            .all(|issue| issue.code() != "missing-label"));
    }
}

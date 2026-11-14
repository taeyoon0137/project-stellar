//! Control contract → 접근성 role·state·action 매핑 (IX.8.4).
//!
//! Control은 Frame에 부착하는 계약(IX.6)입니다. 이 crate는 document에 의존하지 않으므로, 상위 adapter가 문서의
//! `ControlContract`를 중립 [`SourceControl`]로 옮겨 [`SourceNode::control`](crate::source::SourceNode)에 넣습니다.
//! 계약 field 값의 대응: 상수 → [`AccessibilityBinding::Constant`], 컴포넌트 Property →
//! [`AccessibilityBinding::ComponentProperty`], 상태 변수 → [`AccessibilityBinding::State`], semantic action·node·
//! 상태 변수 참조 → StableId.
//!
//! | 계약(IX.6, kind) | 중립 field ← 계약 field(id) | 접근성 결과 |
//! |---|---|---|
//! | Button(0x0001) | activate(1), focused(2), disabled(3), pressed(4) | `Button`, state focused·disabled·pressed, action `Activate`(semantic action) |
//! | TextInput(0x0002) | value(1, 상태 변수), placeholder(2), focused(3) | `TextField`, value = 상태 변수, state `Editable`·focused, action `SetValue`(상태 변수), label 없으면 placeholder |
//! | Modal(0x0003) | dismiss(3) | `Dialog`, state `Modal`, action `Dismiss`(semantic action) |
//! | List(0x0004) | collection(1), selectionMode(3), selection(5) | `List`(Multiple이면 `Multiselectable`), collection 자식 = 항목 |
//! | SingleSelectionGroup(0x0005) | list(1), selected(2) | `RadioGroup`, 연결 List의 항목 Button = `Radio` |
//!
//! 공통: 상호작용 계약이 focus 가능하면 state `Focusable`과 action `Focus`를 더합니다. 계약의 IME 입력·overlay·
//! focus scope root·navigation 방향은 접근성 트리 의미가 아니어서 옮기지 않습니다. 모르는 계약 종류
//! ([`ControlSemantics::Other`])는 기본 매핑(IX.8.3)을 그대로 둡니다.
//!
//! **항목 문맥**(접근성 관계가 시각 계층과 다른 경우, IX.8.2):
//!
//! - List 항목: collection node의 시각 자식입니다. 의미 없는 항목(`Generic`·`Group`)은 `ListItem`이 됩니다.
//!   선택 방식이 None이 아니면 action `Select`(선택 상태 변수)를, Single이고 선택 상태 변수가 있으면 state
//!   `Selected = SelectionMatches(선택, 항목)`을 더합니다. collection이 List Frame 자신이 아니고 의미 없는
//!   Frame이면 그 node는 flatten되어 항목이 List 바로 아래에 옵니다.
//! - Radio: SingleSelectionGroup이 가리키는 List의 항목입니다. Button 항목은 `Radio`가 되고 pressed 대신
//!   state `Checked = SelectionMatches(선택, 항목)`과 action `Select`를 가집니다. 연결된 List node는 flatten되어
//!   radio가 radio group 바로 아래에 옵니다. 선택 상태 변수는 그룹의 selected, 없으면 List의 selection입니다.
//!
//! 항목 identity는 항목 source node StableId입니다(List itemIdentity NodeId 방식). DataKey 방식의 runtime key
//! 해석은 binding evaluator가 맡습니다. 한 node가 둘 이상 List의 항목이면 StableId가 작은 List가 정합니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::ids::StableId;

use crate::node::{
    AccessibilityBinding, AccessibilityNode, AccessibilityRole, ActionKind, ActionTarget, StateKind,
};
use crate::source::{SourceKind, SourceNode, SourceTree};

/// List 선택 방식(IX.6.5 `SELECTION_MODE_ENUM`: None 0·Single 1·Multiple 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum SelectionMode {
    /// 선택 없음.
    #[default]
    None,
    /// 한 항목.
    Single,
    /// 여러 항목.
    Multiple,
}

/// 계약 종류별 의미 field(중립 사본).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlSemantics {
    /// Button(IX.6.2).
    Button {
        /// 활성화 semantic action.
        activate: Option<StableId>,
        /// focus 상태.
        focused: Option<AccessibilityBinding>,
        /// 비활성 상태.
        disabled: Option<AccessibilityBinding>,
        /// 눌림 상태.
        pressed: Option<AccessibilityBinding>,
    },
    /// TextInput(IX.6.3).
    TextInput {
        /// 편집 값 상태 변수.
        value: Option<StableId>,
        /// placeholder.
        placeholder: Option<AccessibilityBinding>,
        /// focus 상태.
        focused: Option<AccessibilityBinding>,
    },
    /// Modal(IX.6.4).
    Modal {
        /// 닫힘 semantic action.
        dismiss: Option<StableId>,
    },
    /// List(IX.6.5).
    List {
        /// 항목 container node.
        collection: Option<StableId>,
        /// 선택 방식.
        selection_mode: SelectionMode,
        /// 선택된 항목 identity 상태 변수.
        selection: Option<StableId>,
    },
    /// SingleSelectionGroup(IX.6.6).
    SingleSelectionGroup {
        /// 단일 선택 List Frame.
        list: Option<StableId>,
        /// 선택된 항목 identity 상태 변수.
        selected: Option<StableId>,
    },
    /// 이 crate가 모르는 계약 종류(보존만).
    Other {
        /// 계약 종류 numeric ID.
        kind: u16,
    },
}

/// Frame에 부착된 control 계약(중립 사본).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceControl {
    /// 의미 계약.
    pub semantics: ControlSemantics,
    /// 상호작용 계약의 focus 가능 여부.
    pub focusable: bool,
}

impl SourceControl {
    /// 기본 Button: focus 가능, activate action.
    pub fn button(activate: StableId) -> SourceControl {
        SourceControl {
            semantics: ControlSemantics::Button {
                activate: Some(activate),
                focused: None,
                disabled: None,
                pressed: None,
            },
            focusable: true,
        }
    }

    /// 기본 TextInput: focus 가능, 편집 값 상태 변수.
    pub fn text_input(value: StableId) -> SourceControl {
        SourceControl {
            semantics: ControlSemantics::TextInput {
                value: Some(value),
                placeholder: None,
                focused: None,
            },
            focusable: true,
        }
    }

    /// 기본 Modal: focus 불가(focus는 scope 안 control이 받음).
    pub fn modal(dismiss: Option<StableId>) -> SourceControl {
        SourceControl {
            semantics: ControlSemantics::Modal { dismiss },
            focusable: false,
        }
    }

    /// 기본 List: focus 가능.
    pub fn list(
        collection: StableId,
        selection_mode: SelectionMode,
        selection: Option<StableId>,
    ) -> SourceControl {
        SourceControl {
            semantics: ControlSemantics::List {
                collection: Some(collection),
                selection_mode,
                selection,
            },
            focusable: true,
        }
    }

    /// 기본 SingleSelectionGroup: focus 불가(focus는 List·Button이 받음).
    pub fn single_selection_group(list: StableId, selected: Option<StableId>) -> SourceControl {
        SourceControl {
            semantics: ControlSemantics::SingleSelectionGroup {
                list: Some(list),
                selected,
            },
            focusable: false,
        }
    }
}

/// 항목 문맥.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemContext {
    /// List 항목.
    ListItem {
        selection_mode: SelectionMode,
        selection: Option<StableId>,
    },
    /// radio group 항목.
    Radio { selected: Option<StableId> },
}

/// 생성 전에 source tree 전체에서 계산하는 control 문맥: flatten할 node와 항목 문맥.
#[derive(Debug, Default)]
pub(crate) struct ControlContext {
    absorbed: BTreeSet<StableId>,
    items: BTreeMap<StableId, ItemContext>,
}

fn is_plain_container(node: &SourceNode) -> bool {
    matches!(node.kind, SourceKind::Frame | SourceKind::ComponentInstance)
        && node.control.is_none()
        && node.label.is_none()
}

fn list_semantics(
    node: &SourceNode,
) -> Option<(Option<StableId>, SelectionMode, Option<StableId>)> {
    match node.control.as_ref().map(|control| &control.semantics) {
        Some(ControlSemantics::List {
            collection,
            selection_mode,
            selection,
        }) => Some((*collection, *selection_mode, *selection)),
        _ => None,
    }
}

impl ControlContext {
    /// source tree의 모든 List·SingleSelectionGroup 계약에서 문맥을 만듭니다(StableId 순, 결정론).
    pub(crate) fn build(source: &SourceTree) -> ControlContext {
        let mut context = ControlContext::default();
        // radio group이 가리키는 List → 선택 상태 변수.
        let mut grouped_lists: BTreeMap<StableId, Option<StableId>> = BTreeMap::new();
        for node in source.nodes() {
            let Some(ControlSemantics::SingleSelectionGroup {
                list: Some(list),
                selected,
            }) = node.control.as_ref().map(|control| &control.semantics)
            else {
                continue;
            };
            if *list == node.id || grouped_lists.contains_key(list) {
                continue;
            }
            let Some((_, _, list_selection)) = source.node(*list).and_then(list_semantics) else {
                continue;
            };
            grouped_lists.insert(*list, selected.or(list_selection));
            context.absorbed.insert(*list);
        }
        for node in source.nodes() {
            let Some((Some(collection), selection_mode, selection)) = list_semantics(node) else {
                continue;
            };
            let Some(collection_node) = source.node(collection) else {
                continue;
            };
            if collection != node.id && is_plain_container(collection_node) {
                context.absorbed.insert(collection);
            }
            let item = match grouped_lists.get(&node.id) {
                Some(selected) => ItemContext::Radio {
                    selected: *selected,
                },
                None => ItemContext::ListItem {
                    selection_mode,
                    selection,
                },
            };
            for child in &collection_node.children {
                context.items.entry(*child).or_insert(item);
            }
        }
        context
    }

    /// 접근성 node를 만들지 않고 자식을 올릴 node인지.
    pub(crate) fn is_absorbed(&self, id: StableId) -> bool {
        self.absorbed.contains(&id)
    }

    /// 계약과 항목 문맥을 기본 매핑 node에 적용합니다.
    pub(crate) fn apply(&self, source: &SourceNode, node: &mut AccessibilityNode) {
        if let Some(control) = &source.control {
            apply_control(control, node);
        }
        if let Some(item) = self.items.get(&source.id) {
            apply_item(*item, node);
        }
    }
}

fn set_state(node: &mut AccessibilityNode, state: StateKind, value: &Option<AccessibilityBinding>) {
    if let Some(value) = value {
        node.states.insert(state, value.clone());
    }
}

fn apply_control(control: &SourceControl, node: &mut AccessibilityNode) {
    match &control.semantics {
        ControlSemantics::Button {
            activate,
            focused,
            disabled,
            pressed,
        } => {
            node.role = AccessibilityRole::Button;
            if let Some(action) = activate {
                node.actions
                    .insert(ActionKind::Activate, ActionTarget::SemanticAction(*action));
            }
            set_state(node, StateKind::Focused, focused);
            set_state(node, StateKind::Disabled, disabled);
            set_state(node, StateKind::Pressed, pressed);
        }
        ControlSemantics::TextInput {
            value,
            placeholder,
            focused,
        } => {
            node.role = AccessibilityRole::TextField;
            node.states
                .insert(StateKind::Editable, AccessibilityBinding::flag(true));
            if let Some(value) = value {
                node.value = Some(AccessibilityBinding::State(*value));
                node.actions
                    .insert(ActionKind::SetValue, ActionTarget::State(*value));
            }
            if node.label.is_none() {
                node.label = placeholder.clone();
            }
            set_state(node, StateKind::Focused, focused);
        }
        ControlSemantics::Modal { dismiss } => {
            node.role = AccessibilityRole::Dialog;
            node.states
                .insert(StateKind::Modal, AccessibilityBinding::flag(true));
            if let Some(action) = dismiss {
                node.actions
                    .insert(ActionKind::Dismiss, ActionTarget::SemanticAction(*action));
            }
        }
        ControlSemantics::List { selection_mode, .. } => {
            node.role = AccessibilityRole::List;
            if *selection_mode == SelectionMode::Multiple {
                node.states
                    .insert(StateKind::Multiselectable, AccessibilityBinding::flag(true));
            }
        }
        ControlSemantics::SingleSelectionGroup { .. } => {
            node.role = AccessibilityRole::RadioGroup;
        }
        ControlSemantics::Other { .. } => return,
    }
    if control.focusable {
        node.states
            .insert(StateKind::Focusable, AccessibilityBinding::flag(true));
        node.actions.insert(ActionKind::Focus, ActionTarget::None);
    }
}

fn apply_item(item: ItemContext, node: &mut AccessibilityNode) {
    match item {
        ItemContext::ListItem {
            selection_mode,
            selection,
        } => {
            if matches!(
                node.role,
                AccessibilityRole::Generic | AccessibilityRole::Group
            ) {
                node.role = AccessibilityRole::ListItem;
            }
            if selection_mode == SelectionMode::None {
                return;
            }
            let target = selection.map_or(ActionTarget::None, ActionTarget::State);
            node.actions.insert(ActionKind::Select, target);
            if let (SelectionMode::Single, Some(selection)) = (selection_mode, selection) {
                node.states.insert(
                    StateKind::Selected,
                    AccessibilityBinding::SelectionMatches {
                        selection,
                        item: node.source,
                    },
                );
            }
        }
        ItemContext::Radio { selected } => {
            if node.role != AccessibilityRole::Button {
                return;
            }
            node.role = AccessibilityRole::Radio;
            node.states.remove(&StateKind::Pressed);
            if let Some(selected) = selected {
                node.states.insert(
                    StateKind::Checked,
                    AccessibilityBinding::SelectionMatches {
                        selection: selected,
                        item: node.source,
                    },
                );
                node.actions
                    .insert(ActionKind::Select, ActionTarget::State(selected));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::generate;
    use crate::test_support::{id, outline};

    fn source(nodes: Vec<SourceNode>, root: u8) -> SourceTree {
        let mut tree = SourceTree::new();
        for node in nodes {
            tree.insert(node).expect("source node");
        }
        tree.push_root(id(root));
        tree
    }

    fn frame(n: u8) -> SourceNode {
        SourceNode::new(id(n), SourceKind::Frame)
    }

    fn text(n: u8) -> SourceNode {
        SourceNode::new(id(n), SourceKind::Text)
    }

    fn golden(source: &SourceTree) -> String {
        let tree = generate(source).expect("생성");
        assert_eq!(tree.validate(source), Vec::new());
        outline(&tree)
    }

    #[test]
    fn button_golden_tree() {
        let control = SourceControl {
            semantics: ControlSemantics::Button {
                activate: Some(id(10)),
                focused: None,
                disabled: Some(AccessibilityBinding::ComponentProperty(id(20))),
                pressed: Some(AccessibilityBinding::State(id(21))),
            },
            focusable: true,
        };
        let tree = source(
            vec![
                frame(1).with_control(control).with_children([id(2)]),
                text(2),
            ],
            1,
        );
        assert_eq!(
            golden(&tree),
            "\
button #1 state{focusable=true,disabled=prop:#20,pressed=state:#21} action{activate=action:#10,focus}
  staticText #2 label=text:#2
"
        );
    }

    #[test]
    fn text_input_golden_tree() {
        let control = SourceControl {
            semantics: ControlSemantics::TextInput {
                value: Some(id(30)),
                placeholder: Some(AccessibilityBinding::text("이메일")),
                focused: Some(AccessibilityBinding::State(id(31))),
            },
            focusable: true,
        };
        let placeholder_only = source(vec![frame(1).with_control(control.clone())], 1);
        assert_eq!(
            golden(&placeholder_only),
            "textField #1 label=\"이메일\" value=state:#30 state{focusable=true,focused=state:#31,editable=true} action{focus,setValue=state:#30}\n"
        );
        // 작성자 label이 placeholder보다 우선합니다.
        let labelled = source(
            vec![frame(1)
                .with_label(AccessibilityBinding::ComponentProperty(id(32)))
                .with_control(control)],
            1,
        );
        assert!(golden(&labelled).starts_with("textField #1 label=prop:#32 "));
    }

    #[test]
    fn modal_golden_tree() {
        let tree = source(
            vec![
                frame(1)
                    .with_label(AccessibilityBinding::text("설정"))
                    .with_control(SourceControl::modal(Some(id(11))))
                    .with_children([id(2), id(3)]),
                text(2),
                frame(3)
                    .with_control(SourceControl::button(id(12)))
                    .with_children([id(4)]),
                text(4),
            ],
            1,
        );
        assert_eq!(
            golden(&tree),
            "\
dialog #1 label=\"설정\" state{modal=true} action{dismiss=action:#11}
  staticText #2 label=text:#2
  button #3 state{focusable=true} action{activate=action:#12,focus}
    staticText #4 label=text:#4
"
        );
    }

    #[test]
    fn list_golden_tree() {
        // collection #2는 List Frame 아래의 의미 없는 Frame이라 flatten됩니다.
        let tree = source(
            vec![
                frame(1)
                    .with_control(SourceControl::list(
                        id(2),
                        SelectionMode::Single,
                        Some(id(40)),
                    ))
                    .with_children([id(2)]),
                frame(2).with_children([id(3), id(4)]),
                frame(3).with_children([id(5)]),
                frame(4).with_children([id(6)]),
                text(5),
                text(6),
            ],
            1,
        );
        assert_eq!(
            golden(&tree),
            "\
list #1 state{focusable=true} action{focus}
  listItem #3 state{selected=selection:#40==#3} action{select=state:#40}
    staticText #5 label=text:#5
  listItem #4 state{selected=selection:#40==#4} action{select=state:#40}
    staticText #6 label=text:#6
"
        );

        // Multiple: 목록은 multiselectable, 항목은 select만(단일 선택 상태 binding 없음).
        // 선택 None: 항목은 role만 ListItem.
        let multi = source(
            vec![
                frame(1)
                    .with_control(SourceControl::list(id(1), SelectionMode::Multiple, None))
                    .with_children([id(3), id(4)]),
                frame(3),
                frame(4).with_control(SourceControl {
                    semantics: ControlSemantics::List {
                        collection: Some(id(4)),
                        selection_mode: SelectionMode::None,
                        selection: None,
                    },
                    focusable: false,
                }),
            ],
            1,
        );
        assert_eq!(
            golden(&multi),
            "\
list #1 state{focusable=true,multiselectable=true} action{focus}
  listItem #3 action{select}
  list #4 action{select}
"
        );
    }

    #[test]
    fn single_selection_group_golden_tree() {
        // 그룹 #1 → List #2(collection = 자신) → Button 항목 #3·#4. List #2는 flatten됩니다.
        let tree = source(
            vec![
                frame(1)
                    .with_label(AccessibilityBinding::text("크기"))
                    .with_control(SourceControl::single_selection_group(id(2), Some(id(50))))
                    .with_children([id(2)]),
                frame(2)
                    .with_control(SourceControl::list(id(2), SelectionMode::Single, None))
                    .with_children([id(3), id(4)]),
                frame(3)
                    .with_control(SourceControl {
                        semantics: ControlSemantics::Button {
                            activate: Some(id(13)),
                            focused: None,
                            disabled: None,
                            pressed: Some(AccessibilityBinding::State(id(51))),
                        },
                        focusable: true,
                    })
                    .with_children([id(5)]),
                frame(4)
                    .with_control(SourceControl::button(id(14)))
                    .with_children([id(6)]),
                text(5),
                text(6),
            ],
            1,
        );
        assert_eq!(
            golden(&tree),
            "\
radioGroup #1 label=\"크기\"
  radio #3 state{focusable=true,checked=selection:#50==#3} action{activate=action:#13,focus,select=state:#50}
    staticText #5 label=text:#5
  radio #4 state{focusable=true,checked=selection:#50==#4} action{activate=action:#14,focus,select=state:#50}
    staticText #6 label=text:#6
"
        );

        // 그룹 selected가 없으면 List selection을 씁니다.
        let fallback = source(
            vec![
                frame(1)
                    .with_control(SourceControl::single_selection_group(id(2), None))
                    .with_children([id(2)]),
                frame(2)
                    .with_control(SourceControl::list(
                        id(2),
                        SelectionMode::Single,
                        Some(id(52)),
                    ))
                    .with_children([id(3)]),
                frame(3).with_control(SourceControl::button(id(15))),
            ],
            1,
        );
        assert!(
            golden(&fallback).contains("radio #3 state{focusable=true,checked=selection:#52==#3}")
        );
    }

    #[test]
    fn unknown_contract_keeps_base_mapping() {
        let tree = source(
            vec![frame(1).with_control(SourceControl {
                semantics: ControlSemantics::Other { kind: 0x7777 },
                focusable: true,
            })],
            1,
        );
        assert_eq!(golden(&tree), "generic #1\n");
    }
}

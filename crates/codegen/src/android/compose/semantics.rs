//! Control 의미·접근성 → Compose semantics (XXII.4.2, ADR-0107 결정 6).
//!
//! | 의미 | Compose |
//! |---|---|
//! | Button contract | `clickable(interactionSource, indication = null, enabled = !비활성, role = Role.Button) { onAction(activate) }` |
//! | radio(Button + 선택) | `selectable(selected = 선택 식, …, role = Role.RadioButton) { onAction; 선택 상태 쓰기 }` |
//! | SingleSelectionGroup·radioGroup | `selectableGroup()` |
//! | List | `collectionInfo = CollectionInfo(rowCount = 항목 수, columnCount = 1)`, 항목은 `collectionItemInfo` |
//! | 접근성 entry 없는 subtree | `clearAndSetSemantics {}` |
//! | button·radio·listItem | `semantics(mergeDescendants = true) { … }` |
//! | image | `role = Role.Image` |
//! | dialog·modal | `paneTitle = 이름`(근사, 노트) |
//! | label·value | `contentDescription`·`stateDescription`(staticText 자기 텍스트와 textField 값은 native가 제공) |
//! | disabled·selected·checked·focused | `disabled()`·`selected`·`toggleableState`·`focused`(control modifier가 이미 내면 생략) |
//! | pressed | `toggleableState`(근사, 노트) |
//! | multiselectable | 대응 없음(노트) |
//! | activate·select(control 밖) | `onClick { …; true }`, dismiss는 `dismiss { …; true }` |
//! | 읽기 순서(문서 순서와 다름) | 부모 `isTraversalGroup = true`, 자식 `traversalIndex`(Compose UI 1.6 이상) |
//!
//! `MutableInteractionSource`와 `indication = null`은 디자인에 없는 눌림 표시를 그리지 않기 위해서입니다.

use std::collections::BTreeMap;

use stellar_accessibility::node::{
    AccessibilityBinding, AccessibilityRole, ActionKind, ActionTarget, StateKind,
};
use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;
use stellar_target_profile::capability::ElementKind;

use super::{ui_at_least, Generator, KotlinType};
use crate::android::kotlin::{float, string_literal};
use crate::android::AndroidNoteKind;
use crate::apple::native::Control;
use crate::facet::Facet;
use crate::ir::ResolvedBinding;

/// node별 접근성 계획(IR node index 순).
#[derive(Debug, Clone, Default)]
pub(super) struct SemanticsPlan {
    /// 접근성 entry 없는 subtree의 맨 위.
    pub hidden: Vec<bool>,
    /// 읽기 순서를 다시 정하는 부모.
    pub traversal_group: Vec<bool>,
    /// 읽기 순서 위치.
    pub traversal_index: Vec<Option<u32>>,
    /// List 항목 위치(entry 부모가 list일 때).
    pub list_item: Vec<Option<u32>>,
    /// List 항목 수.
    pub list_rows: Vec<Option<u32>>,
}

impl Generator<'_> {
    /// IR 전체의 숨김·읽기 순서·목록 위치를 한 번 계산합니다.
    pub(super) fn semantics_plan(&mut self) -> SemanticsPlan {
        let ir = self.ir;
        let count = ir.nodes.len();
        let mut plan = SemanticsPlan {
            hidden: vec![false; count],
            traversal_group: vec![false; count],
            traversal_index: vec![None; count],
            list_item: vec![None; count],
            list_rows: vec![None; count],
        };
        let has = ir.semantic_subtrees();
        let parents = ir.element_parents();
        for (index, node) in ir.nodes.iter().enumerate() {
            if node.element == ElementKind::FRAGMENT || node.semantics.is_some() {
                continue;
            }
            let parent_has = parents[index].is_none_or(|p| has[p as usize]);
            plan.hidden[index] = !has[index] && parent_has;
        }
        let child_lists = ir.accessibility_child_lists();
        let is_descendant = |child: u32, ancestor: u32| {
            let mut current = ir.nodes[child as usize].parent;
            while let Some(p) = current {
                if p == ancestor {
                    return true;
                }
                current = ir.nodes[p as usize].parent;
            }
            false
        };
        for (entry_index, entry) in ir.accessibility.iter().enumerate() {
            let children: Vec<u32> = child_lists[entry_index]
                .iter()
                .map(|child| ir.accessibility[*child as usize].node)
                .collect();
            if entry.semantics.role == AccessibilityRole::List {
                plan.list_rows[entry.node as usize] = Some(children.len() as u32);
                for (position, child) in children.iter().enumerate() {
                    plan.list_item[*child as usize] = Some(position as u32);
                }
            }
            if children.is_empty() {
                continue;
            }
            let mut visual = children.clone();
            visual.sort_unstable();
            let reparented = children.iter().any(|c| !is_descendant(*c, entry.node));
            if visual == children && !reparented {
                continue;
            }
            if !ui_at_least(self.android, 1, 6) {
                self.note(
                    entry.node,
                    AndroidNoteKind::Unavailable(Facet::ReadingOrder),
                );
                continue;
            }
            plan.traversal_group[entry.node as usize] = true;
            for (position, child) in children.iter().enumerate() {
                plan.traversal_index[*child as usize] = Some(position as u32);
            }
            if reparented {
                self.note(
                    entry.node,
                    AndroidNoteKind::Approximated(Facet::ReadingOrder),
                );
            }
        }
        plan
    }

    fn state_name(&self, id: StableId, ty: KotlinType) -> Option<String> {
        match self.states.get(&id) {
            Some((name, found)) if *found == ty => Some(format!("state.{name}")),
            _ => None,
        }
    }

    fn flag_expr(
        &mut self,
        index: u32,
        binding: &AccessibilityBinding,
        facet: Facet,
    ) -> Option<String> {
        let resolved = self.ir.resolve_binding(binding);
        let out = match resolved {
            ResolvedBinding::Constant(TypedValue::Boolean(value)) => Some(value.to_string()),
            ResolvedBinding::Property(property) => match property.value {
                TypedValue::Boolean(value) => Some(value.to_string()),
                _ => None,
            },
            ResolvedBinding::State(state) => self.state_name(state.id, KotlinType::Boolean),
            ResolvedBinding::SelectionMatches { state, item } => self
                .state_name(state.id, KotlinType::String)
                .map(|name| format!("{name} == {}", string_literal(&item.to_string()))),
            ResolvedBinding::ComponentProperty(_) => {
                self.note(index, AndroidNoteKind::ComponentProperty(facet));
                return None;
            }
            _ => None,
        };
        if out.is_none() {
            self.note(index, AndroidNoteKind::UnresolvedBinding(facet));
        }
        out
    }

    fn text_expr(
        &mut self,
        index: u32,
        binding: &AccessibilityBinding,
        facet: Facet,
    ) -> Option<String> {
        let out = match self.ir.resolve_binding(binding) {
            ResolvedBinding::Constant(TypedValue::String(text)) => Some(string_literal(text)),
            ResolvedBinding::Property(property) => match &property.value {
                TypedValue::String(text) => Some(string_literal(text)),
                _ => None,
            },
            ResolvedBinding::NodeText {
                text: Some(text), ..
            } => Some(string_literal(text)),
            ResolvedBinding::State(state) => self.state_name(state.id, KotlinType::String),
            ResolvedBinding::ComponentProperty(_) => {
                self.note(index, AndroidNoteKind::ComponentProperty(facet));
                return None;
            }
            _ => None,
        };
        if out.is_none() {
            self.note(index, AndroidNoteKind::UnresolvedBinding(facet));
        }
        out
    }

    /// 동작 본문(activate는 onAction, select는 선택 상태 쓰기).
    fn action_body(&self, index: u32, actions: &BTreeMap<ActionKind, ActionTarget>) -> Vec<String> {
        let id = self.ir.nodes[index as usize].id;
        let mut body = Vec::new();
        for (action, target) in actions {
            match (action, target) {
                (ActionKind::Activate, ActionTarget::SemanticAction(action_id)) => body.push(
                    format!("onAction({})", string_literal(&action_id.to_string())),
                ),
                (ActionKind::Select, ActionTarget::State(state)) => {
                    if let Some(name) = self.state_name(*state, KotlinType::String) {
                        body.push(format!("{name} = {}", string_literal(&id.to_string())));
                    }
                }
                _ => {}
            }
        }
        body
    }

    /// Control interaction·접근성 modifier(체인에서 item·transform 뒤, 크기 앞).
    pub(super) fn semantics_modifiers(&mut self, index: u32, plan: &SemanticsPlan) -> Vec<String> {
        let mut out = Vec::new();
        if plan.hidden[index as usize] {
            self.use_name("clearAndSetSemantics");
            out.push("clearAndSetSemantics {}".to_string());
            return out;
        }
        let info = self.info(index).clone();
        let Some(entry) = self.ir.semantics_of(index).cloned() else {
            if plan.traversal_group[index as usize] {
                self.use_name("semantics");
                self.use_name("isTraversalGroup");
                out.push("semantics { isTraversalGroup = true }".to_string());
            }
            return out;
        };
        let semantics = &entry.semantics;
        let role = semantics.role;
        let mut handled_disabled = false;
        let mut handled_selected = false;
        let mut handled_activate = false;
        let disabled = match semantics.states.get(&StateKind::Disabled) {
            Some(binding) => self.flag_expr(index, binding, Facet::State(StateKind::Disabled)),
            None => None,
        };
        // Control interaction.
        if info.control == Some(Control::Button) {
            let body = self.action_body(index, &semantics.actions);
            self.use_name("remember");
            self.use_name("MutableInteractionSource");
            self.use_name("Role");
            let mut args = vec![
                "interactionSource = remember { MutableInteractionSource() }".to_string(),
                "indication = null".to_string(),
            ];
            if let Some(flag) = &disabled {
                args.push(format!("enabled = !({flag})"));
                handled_disabled = true;
            }
            let selected = semantics
                .states
                .get(&StateKind::Checked)
                .or_else(|| semantics.states.get(&StateKind::Selected))
                .cloned();
            let call = match (role, selected) {
                (AccessibilityRole::Radio | AccessibilityRole::ListItem, Some(binding)) => {
                    let flag = self
                        .flag_expr(index, &binding, Facet::State(StateKind::Selected))
                        .unwrap_or_else(|| "false".to_string());
                    handled_selected = true;
                    args.insert(0, format!("selected = {flag}"));
                    if role == AccessibilityRole::Radio {
                        args.push("role = Role.RadioButton".to_string());
                    }
                    self.use_name("selectable");
                    "selectable"
                }
                _ => {
                    args.push("role = Role.Button".to_string());
                    self.use_name("clickable");
                    "clickable"
                }
            };
            handled_activate = true;
            out.push(format!(
                "{call}({}) {{ {} }}",
                args.join(", "),
                body.join("; ")
            ));
        }
        if info.control == Some(Control::SelectionGroup) || role == AccessibilityRole::RadioGroup {
            self.use_name("selectableGroup");
            out.push("selectableGroup()".to_string());
        }
        // semantics 블록.
        let mut properties: Vec<String> = Vec::new();
        let merge = matches!(
            role,
            AccessibilityRole::Button | AccessibilityRole::Radio | AccessibilityRole::ListItem
        );
        match role {
            AccessibilityRole::Image => {
                self.use_name("role");
                self.use_name("Role");
                properties.push("role = Role.Image".to_string());
            }
            AccessibilityRole::Button if info.control != Some(Control::Button) => {
                self.use_name("role");
                self.use_name("Role");
                properties.push("role = Role.Button".to_string());
            }
            AccessibilityRole::Radio if info.control != Some(Control::Button) => {
                self.use_name("role");
                self.use_name("Role");
                properties.push("role = Role.RadioButton".to_string());
            }
            AccessibilityRole::Dialog => {
                self.note(index, AndroidNoteKind::Approximated(Facet::Role));
            }
            AccessibilityRole::List => {
                if let Some(rows) = plan.list_rows[index as usize] {
                    self.use_name("collectionInfo");
                    self.use_name("CollectionInfo");
                    properties.push(format!(
                        "collectionInfo = CollectionInfo(rowCount = {rows}, columnCount = 1)"
                    ));
                }
            }
            _ => {}
        }
        if let Some(position) = plan.list_item[index as usize] {
            self.use_name("collectionItemInfo");
            self.use_name("CollectionItemInfo");
            properties.push(format!(
                "collectionItemInfo = CollectionItemInfo(rowIndex = {position}, rowSpan = 1, columnIndex = 0, columnSpan = 1)"
            ));
        }
        let mut label_text = None;
        if let Some(label) = &semantics.label {
            let own_text = matches!(label, AccessibilityBinding::NodeText(target) if *target == self.ir.nodes[index as usize].id);
            if !(own_text && role == AccessibilityRole::StaticText) {
                if let Some(text) = self.text_expr(index, label, Facet::Label) {
                    self.use_name("contentDescription");
                    properties.push(format!("contentDescription = {text}"));
                    label_text = Some(text);
                }
            }
        }
        if let Some(value) = &semantics.value {
            if role != AccessibilityRole::TextField {
                if let Some(text) = self.text_expr(index, value, Facet::Value) {
                    self.use_name("stateDescription");
                    properties.push(format!("stateDescription = {text}"));
                }
            }
        }
        for (state, binding) in &semantics.states {
            let facet = Facet::State(*state);
            match state {
                StateKind::Focusable | StateKind::Editable => {}
                StateKind::Multiselectable => {
                    self.note(index, AndroidNoteKind::NoEquivalent(facet));
                }
                StateKind::Disabled => {
                    if handled_disabled {
                        continue;
                    }
                    if let Some(flag) = disabled.clone() {
                        self.use_name("disabled");
                        properties.push(if flag == "true" {
                            "disabled()".to_string()
                        } else {
                            format!("if ({flag}) disabled()")
                        });
                    }
                }
                StateKind::Selected | StateKind::Checked if handled_selected => {}
                StateKind::Selected => {
                    if let Some(flag) = self.flag_expr(index, binding, facet) {
                        self.use_name("selected");
                        properties.push(format!("selected = {flag}"));
                    }
                }
                StateKind::Checked | StateKind::Pressed => {
                    if *state == StateKind::Pressed {
                        self.note(index, AndroidNoteKind::Approximated(facet));
                    }
                    if let Some(flag) = self.flag_expr(index, binding, facet) {
                        self.use_name("toggleableState");
                        self.use_name("ToggleableState");
                        properties.push(format!("toggleableState = ToggleableState({flag})"));
                    }
                }
                StateKind::Focused => {
                    if let Some(flag) = self.flag_expr(index, binding, facet) {
                        self.use_name("focused");
                        properties.push(format!("focused = {flag}"));
                    }
                }
                StateKind::Modal => {
                    if self.ir.static_flag(binding) != Some(false) {
                        self.note(index, AndroidNoteKind::Approximated(facet));
                        if let Some(text) = &label_text {
                            self.use_name("paneTitle");
                            properties.push(format!("paneTitle = {text}"));
                        }
                    }
                }
            }
        }
        if !handled_activate {
            let body = self.action_body(index, &semantics.actions);
            if !body.is_empty() {
                self.use_name("onClick");
                properties.push(format!("onClick {{ {}; true }}", body.join("; ")));
            }
        }
        for (action, target) in &semantics.actions {
            match (action, target) {
                (ActionKind::Dismiss, ActionTarget::SemanticAction(action_id)) => {
                    self.use_name("dismiss");
                    properties.push(format!(
                        "dismiss {{ onAction({}); true }}",
                        string_literal(&action_id.to_string())
                    ));
                }
                (
                    ActionKind::Activate
                    | ActionKind::Select
                    | ActionKind::Focus
                    | ActionKind::SetValue,
                    _,
                )
                | (ActionKind::Dismiss, _) => {}
            }
        }
        if plan.traversal_group[index as usize] {
            self.use_name("isTraversalGroup");
            properties.push("isTraversalGroup = true".to_string());
        }
        if let Some(position) = plan.traversal_index[index as usize] {
            self.use_name("traversalIndex");
            properties.push(format!("traversalIndex = {}", float(position as f32)));
        }
        if !properties.is_empty() {
            self.use_name("semantics");
            let head = if merge {
                "semantics(mergeDescendants = true)"
            } else {
                "semantics"
            };
            out.push(format!("{head} {{ {} }}", properties.join("; ")));
        }
        out
    }
}

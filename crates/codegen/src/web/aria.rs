//! IR 접근성 tree → HTML/ARIA 매핑 (XXII.2.4).
//!
//! [`map_accessibility`]는 Fragment가 아닌 IR node마다 element 하나를 만들고(Fragment 자식은 부모 element에 바로
//! 붙음, layout flatten과 같음), 접근성 entry의 의미를 native HTML 의미 우선(ARIA in HTML)으로 옮깁니다.
//!
//! | role | element | 추가 속성 |
//! |---|---|---|
//! | generic | `div` | — |
//! | group | `div` | `role="group"` |
//! | button | `button type="button"` | `disabled`, `aria-pressed` |
//! | textField | 자식 없으면 `input type="text"`, 있으면 `div role="textbox"` | `value`·`readonly`(input), `aria-readonly`(div) |
//! | dialog | `dialog open` | modal은 노트(아래 상태) |
//! | list | 자식 element가 모두 항목이면 `ul`, 아니면 `div role="list"` | 선택 항목이 있거나 다중 선택이면 `role="listbox"`, `aria-multiselectable` |
//! | listItem | 부모 element가 `ul`이면 `li`, 아니면 `div role="listitem"` | listbox 안이면 `role="option"`, `aria-selected` |
//! | radioGroup | `div role="radiogroup"` | — |
//! | radio | `button type="button" role="radio"` | `aria-checked` |
//! | staticText | `span`(텍스트 내용) | — |
//! | image | 자식 없고 정적 이름이 있으면 `img alt`, 아니면 `div role="img"` | — |
//!
//! - **이름**: 상수·속성·상태 초기값 문자열은 `aria-label`(`img`는 `alt`), 다른 node의 텍스트는 `aria-labelledby`,
//!   자기 텍스트 내용은 내용 그대로입니다. staticText에 자기 텍스트가 아닌 이름이 있으면 `span`에 이름을 붙일 수 없어
//!   노트로 남깁니다.
//! - **상태**: 정적 값을 씁니다. 상태 변수에 묶인 값은 초기값을 쓰고 [`WebNoteKind::DynamicBinding`]으로 알립니다.
//!   `focusable`은 native로 focus를 받지 않는 element에 `tabindex="0"`, `focused`는 `autofocus`, `disabled`는 native
//!   control에 `disabled`·그 밖에 `aria-disabled`입니다. `modal`은 쓰지 않고 [`WebNoteKind::NoEquivalent`]로
//!   남깁니다(`<dialog open>`은 non-modal로 열리고 정적 HTML은 바깥 콘텐츠를 inert로 만들 수 없어, `aria-modal`은
//!   사실과 다른 상태를 알립니다. modal 표시는 `showModal()` 연결이 필요한 script 범위입니다).
//! - **숨김**: 자신과 하위에 접근성 entry가 없는 subtree는 맨 위 element에 `aria-hidden="true"`를 붙입니다(`img`는
//!   `alt=""`도). Fragment는 element가 없으므로 "맨 위"는 DOM 부모 element 기준입니다.
//! - **읽기 순서**: entry의 자식 읽기 순서가 DOM 순서와 다르거나 자식이 DOM 하위가 아니면(접근성 재부모) 그 entry
//!   element에 `aria-owns`로 자식 전체를 읽기 순서대로 적습니다.
//! - **action**: activate·select·setValue·focus·dismiss는 role과 native element(button·input·dialog)가 제공합니다.
//!
//! 참조가 필요한 element에만 `id="s-<StableId 16진>"`를 붙입니다. HTML로 표현할 수 없는 의미는 버리지 않고
//! [`WebNote`]로 남깁니다.

use std::collections::BTreeSet;

use stellar_accessibility::node::{AccessibilityRole, ActionKind, StateKind};
use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;

use crate::ir::snapshot::id_hex;
use crate::ir::{IrAccessibilityEntry, IrTree, ResolvedBinding};
use crate::web::html::{HtmlChild, HtmlDocument};
use crate::web::structure::map_structure;

pub use crate::facet::Facet;

/// 매핑 노트 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WebNoteKind {
    /// 상태 변수·컴포넌트 Property에 묶여 있어 정적 HTML에는 초기값만 썼거나(상태) 쓰지 못했습니다(컴포넌트
    /// Property).
    DynamicBinding(Facet),
    /// binding 대상이 IR에 없거나 값의 타입이 맞지 않아 쓰지 못했습니다.
    UnresolvedBinding(Facet),
    /// 이 role에 대응하는 HTML/ARIA 표현이 없어 쓰지 못했습니다.
    NoEquivalent(Facet),
    /// 접근성 root가 둘 이상이고 읽기 순서가 DOM 순서와 달라 표현하지 못했습니다.
    RootOrder,
}

/// 매핑 노트.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WebNote {
    /// IR node.
    pub node: StableId,
    /// 종류.
    pub kind: WebNoteKind,
}

/// 매핑 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebAccessibility {
    /// HTML 조각.
    pub document: HtmlDocument,
    /// 매핑 노트(node 문서 순서 → 종류 순).
    pub notes: Vec<WebNote>,
}

impl WebAccessibility {
    /// HTML 텍스트.
    pub fn to_html(&self) -> String {
        self.document.to_html()
    }
}

/// element의 참조 id.
pub fn element_id(node: StableId) -> String {
    format!("s-{}", id_hex(node))
}

struct Mapper<'a> {
    ir: &'a IrTree,
    doc: HtmlDocument,
    /// IR node index → element index(Fragment는 `None`).
    element_of: Vec<Option<u32>>,
    notes: Vec<WebNote>,
    referenced: BTreeSet<u32>,
    /// entry index → 자식 entry index(읽기 순서, 한 번만 계산).
    children: Vec<Vec<u32>>,
    /// entry index → listbox로 낼 List인지(한 번만 계산).
    listbox: Vec<bool>,
}

fn is_listbox(ir: &IrTree, children: &[Vec<u32>], entry: usize) -> bool {
    let list = &ir.accessibility[entry].semantics;
    if list.role != AccessibilityRole::List {
        return false;
    }
    list.states.contains_key(&StateKind::Multiselectable)
        || children[entry].iter().any(|child| {
            ir.accessibility[*child as usize]
                .semantics
                .actions
                .contains_key(&ActionKind::Select)
        })
}

impl<'a> Mapper<'a> {
    fn entry(&self, node: u32) -> Option<&'a IrAccessibilityEntry> {
        self.ir.semantics_of(node)
    }

    fn role(&self, node: u32) -> Option<AccessibilityRole> {
        self.entry(node).map(|entry| entry.semantics.role)
    }

    fn note(&mut self, node: u32, kind: WebNoteKind) {
        let id = self.ir.nodes[node as usize].id;
        self.notes.push(WebNote { node: id, kind });
    }

    fn element_children(&self, element: u32) -> Vec<u32> {
        self.doc
            .element(element)
            .children
            .iter()
            .filter_map(|child| match child {
                HtmlChild::Element(child) => Some(*child),
                HtmlChild::Text(_) => None,
            })
            .collect()
    }

    fn node_of(&self, element: u32) -> Option<u32> {
        let source = self.doc.element(element).source?;
        self.ir.node_index(source)
    }

    /// 구조([`map_structure`]) 위에서 접근성 entry가 있는 element의 tag와 role 속성을 role로 고칩니다. entry가 없는
    /// element는 구조 결과의 tag를 그대로 두고 숨김만 정합니다.
    fn assign_tags(&mut self, has_semantics: &[bool]) {
        let ir = self.ir;
        for index in 0..ir.nodes.len() {
            let Some(element) = self.element_of[index] else {
                continue;
            };
            let node = &ir.nodes[index];
            let has_children = !self.element_children(element).is_empty();
            let node_index = index as u32;
            if self.role(node_index).is_some() {
                // role이 element 의미를 정하므로 구조의 기본 속성(type·open·alt)을 버립니다.
                self.doc.clear_attributes(element);
            }
            match self.role(node_index) {
                Some(AccessibilityRole::Generic) => self.doc.set_tag(element, "div"),
                Some(AccessibilityRole::Group) => {
                    self.doc.set_tag(element, "div");
                    self.doc.set_attr(element, "role", "group");
                }
                Some(AccessibilityRole::Button) => {
                    self.doc.set_tag(element, "button");
                    self.doc.set_attr(element, "type", "button");
                }
                Some(AccessibilityRole::TextField) => {
                    if has_children || node.text.is_some() {
                        self.doc.set_tag(element, "div");
                        self.doc.set_attr(element, "role", "textbox");
                    } else {
                        self.doc.set_tag(element, "input");
                        self.doc.set_attr(element, "type", "text");
                    }
                }
                Some(AccessibilityRole::Dialog) => {
                    self.doc.set_tag(element, "dialog");
                    self.doc.set_attr(element, "open", "");
                }
                Some(AccessibilityRole::List) => {
                    let entry = node.semantics.unwrap_or_default();
                    let listbox = self.listbox[entry as usize];
                    let all_items = has_children
                        && self.element_children(element).into_iter().all(|child| {
                            self.node_of(child).and_then(|n| self.role(n))
                                == Some(AccessibilityRole::ListItem)
                        });
                    if all_items {
                        self.doc.set_tag(element, "ul");
                        if listbox {
                            self.doc.set_attr(element, "role", "listbox");
                        }
                    } else {
                        self.doc.set_tag(element, "div");
                        self.doc.set_attr(
                            element,
                            "role",
                            if listbox { "listbox" } else { "list" },
                        );
                    }
                }
                Some(AccessibilityRole::ListItem) => {
                    let in_listbox = ir
                        .semantics_of(node_index)
                        .and_then(|entry| entry.parent)
                        .is_some_and(|parent| self.listbox[parent as usize]);
                    // 부모 element tag는 부모가 먼저 처리되므로(preorder) 이미 정해졌습니다.
                    let parent_is_ul = self
                        .doc
                        .element(element)
                        .parent
                        .is_some_and(|parent| self.doc.element(parent).tag == "ul");
                    if parent_is_ul {
                        self.doc.set_tag(element, "li");
                        if in_listbox {
                            self.doc.set_attr(element, "role", "option");
                        }
                    } else {
                        self.doc.set_tag(element, "div");
                        self.doc.set_attr(
                            element,
                            "role",
                            if in_listbox { "option" } else { "listitem" },
                        );
                    }
                }
                Some(AccessibilityRole::RadioGroup) => {
                    self.doc.set_tag(element, "div");
                    self.doc.set_attr(element, "role", "radiogroup");
                }
                Some(AccessibilityRole::Radio) => {
                    self.doc.set_tag(element, "button");
                    self.doc.set_attr(element, "type", "button");
                    self.doc.set_attr(element, "role", "radio");
                }
                Some(AccessibilityRole::StaticText) => self.doc.set_tag(element, "span"),
                Some(AccessibilityRole::Image) => {
                    // img 여부는 이름 해석 뒤(assign_name)에 정합니다.
                    self.doc.set_tag(element, "div");
                    self.doc.set_attr(element, "role", "img");
                }
                None => {
                    if !has_semantics[index]
                        && matches!(self.doc.element(element).tag, "button" | "input" | "dialog")
                    {
                        // 접근성 entry가 없는 숨김 subtree에서는 native control이 focus를 받으면 안 되므로
                        // 구조의 control tag를 장식 `div`로 낮춥니다(정적 검사 FocusableInHidden).
                        self.doc.set_tag(element, "div");
                        self.doc.clear_attributes(element);
                    }
                    // Fragment는 DOM element가 없으므로 실제 element 부모를 기준으로 숨깁니다.
                    let parent_has = self
                        .doc
                        .element(element)
                        .parent
                        .and_then(|parent| self.node_of(parent))
                        .is_none_or(|parent| has_semantics[parent as usize]);
                    if !has_semantics[index] && parent_has {
                        self.doc.set_attr(element, "aria-hidden", "true");
                    }
                }
            }
        }
    }

    fn is_native_control(&self, element: u32) -> bool {
        matches!(self.doc.element(element).tag, "button" | "input")
    }

    fn assign_name(&mut self, node: u32, element: u32, entry: &IrAccessibilityEntry) {
        let ir = self.ir;
        let semantics = &entry.semantics;
        let image = semantics.role == AccessibilityRole::Image;
        let leaf = self.element_children(element).is_empty();
        let Some(label) = &semantics.label else {
            return;
        };
        let own_text = matches!(
            label,
            stellar_accessibility::node::AccessibilityBinding::NodeText(target)
                if Some(*target) == self.doc.element(element).source
        );
        if semantics.role == AccessibilityRole::StaticText && !own_text {
            // 정적 텍스트(span = generic)에는 이름을 붙일 수 없습니다(ARIA name prohibited).
            self.note(node, WebNoteKind::NoEquivalent(Facet::Label));
            return;
        }
        let static_name = match ir.resolve_binding(label) {
            ResolvedBinding::Constant(TypedValue::String(text)) => Some(text.clone()),
            ResolvedBinding::Property(property) => match &property.value {
                TypedValue::String(text) => Some(text.clone()),
                _ => {
                    self.note(node, WebNoteKind::UnresolvedBinding(Facet::Label));
                    return;
                }
            },
            ResolvedBinding::State(state) => match &state.initial {
                TypedValue::String(text) => {
                    self.note(node, WebNoteKind::DynamicBinding(Facet::Label));
                    Some(text.clone())
                }
                _ => {
                    self.note(node, WebNoteKind::UnresolvedBinding(Facet::Label));
                    return;
                }
            },
            ResolvedBinding::NodeText { node: target, text } => {
                if target == node && self.doc.element(element).tag == "span" {
                    // 이름 = 자기 텍스트 내용.
                    return;
                }
                if image && leaf {
                    if let Some(text) = text {
                        self.make_image(element, text.to_string());
                        return;
                    }
                }
                match self.element_of[target as usize] {
                    Some(target_element) => {
                        self.referenced.insert(target_element);
                        let id = element_id(ir.nodes[target as usize].id);
                        self.doc.set_attr(element, "aria-labelledby", id);
                    }
                    None => self.note(node, WebNoteKind::UnresolvedBinding(Facet::Label)),
                }
                return;
            }
            ResolvedBinding::ComponentProperty(_) => {
                self.note(node, WebNoteKind::DynamicBinding(Facet::Label));
                return;
            }
            ResolvedBinding::Constant(_)
            | ResolvedBinding::SelectionMatches { .. }
            | ResolvedBinding::Missing => {
                self.note(node, WebNoteKind::UnresolvedBinding(Facet::Label));
                return;
            }
        };
        if let Some(name) = static_name {
            if image && leaf {
                self.make_image(element, name);
            } else {
                self.doc.set_attr(element, "aria-label", name);
            }
        }
    }

    fn make_image(&mut self, element: u32, alt: String) {
        self.doc.set_tag(element, "img");
        self.doc.remove_attr(element, "role");
        self.doc.set_attr(element, "alt", alt);
    }

    fn static_flag(
        &mut self,
        node: u32,
        state: StateKind,
        entry: &IrAccessibilityEntry,
    ) -> Option<bool> {
        let binding = entry.semantics.states.get(&state)?;
        let ir = self.ir;
        match ir.resolve_binding(binding) {
            ResolvedBinding::Constant(TypedValue::Boolean(value)) => Some(*value),
            ResolvedBinding::Property(property) => match property.value {
                TypedValue::Boolean(value) => Some(value),
                _ => {
                    self.note(node, WebNoteKind::UnresolvedBinding(Facet::State(state)));
                    None
                }
            },
            ResolvedBinding::State(_) | ResolvedBinding::SelectionMatches { .. } => {
                match ir.static_flag(binding) {
                    Some(value) => {
                        self.note(node, WebNoteKind::DynamicBinding(Facet::State(state)));
                        Some(value)
                    }
                    None => {
                        self.note(node, WebNoteKind::UnresolvedBinding(Facet::State(state)));
                        None
                    }
                }
            }
            ResolvedBinding::ComponentProperty(_) => {
                self.note(node, WebNoteKind::DynamicBinding(Facet::State(state)));
                None
            }
            _ => {
                self.note(node, WebNoteKind::UnresolvedBinding(Facet::State(state)));
                None
            }
        }
    }

    fn assign_states(&mut self, node: u32, element: u32, entry: &IrAccessibilityEntry) {
        let tag = self.doc.element(element).tag;
        let role = self.doc.element(element).attr("role").map(str::to_string);
        let native = self.is_native_control(element);
        for state in entry.semantics.states.keys().copied().collect::<Vec<_>>() {
            let Some(value) = self.static_flag(node, state, entry) else {
                continue;
            };
            let flag = if value { "true" } else { "false" };
            match state {
                StateKind::Focusable => {
                    if value && !native {
                        self.doc.set_attr(element, "tabindex", "0");
                    }
                }
                StateKind::Focused => {
                    if value {
                        self.doc.set_attr(element, "autofocus", "");
                    }
                }
                StateKind::Disabled => {
                    if value {
                        if native {
                            self.doc.set_attr(element, "disabled", "");
                        } else {
                            self.doc.set_attr(element, "aria-disabled", "true");
                        }
                    }
                }
                StateKind::Pressed => {
                    if tag == "button" && role.is_none() {
                        self.doc.set_attr(element, "aria-pressed", flag);
                    } else {
                        self.note(node, WebNoteKind::NoEquivalent(Facet::State(state)));
                    }
                }
                StateKind::Selected => {
                    if role.as_deref() == Some("option") {
                        self.doc.set_attr(element, "aria-selected", flag);
                    } else {
                        self.note(node, WebNoteKind::NoEquivalent(Facet::State(state)));
                    }
                }
                StateKind::Checked => {
                    if role.as_deref() == Some("radio") {
                        self.doc.set_attr(element, "aria-checked", flag);
                    } else {
                        self.note(node, WebNoteKind::NoEquivalent(Facet::State(state)));
                    }
                }
                StateKind::Editable => match (tag, role.as_deref()) {
                    ("input", _) => {
                        if !value {
                            self.doc.set_attr(element, "readonly", "");
                        }
                    }
                    (_, Some("textbox")) => {
                        if !value {
                            self.doc.set_attr(element, "aria-readonly", "true");
                        }
                    }
                    _ => self.note(node, WebNoteKind::NoEquivalent(Facet::State(state))),
                },
                StateKind::Modal => {
                    // `<dialog open>`은 non-modal로 열리고 정적 HTML은 바깥 콘텐츠를 inert로 만들 수 없습니다.
                    // modal 표시는 `showModal()` 연결(script 범위)이 필요하므로 `aria-modal`을 쓰지 않습니다.
                    if tag != "dialog" || value {
                        self.note(node, WebNoteKind::NoEquivalent(Facet::State(state)));
                    }
                }
                StateKind::Multiselectable => {
                    if role.as_deref() == Some("listbox") {
                        if value {
                            self.doc.set_attr(element, "aria-multiselectable", "true");
                        }
                    } else {
                        self.note(node, WebNoteKind::NoEquivalent(Facet::State(state)));
                    }
                }
            }
        }
    }

    fn assign_value(&mut self, node: u32, element: u32, entry: &IrAccessibilityEntry) {
        let Some(binding) = &entry.semantics.value else {
            return;
        };
        if self.doc.element(element).tag != "input" {
            self.note(node, WebNoteKind::NoEquivalent(Facet::Value));
            return;
        }
        let ir = self.ir;
        let dynamic = matches!(
            ir.resolve_binding(binding),
            ResolvedBinding::State(_) | ResolvedBinding::ComponentProperty(_)
        );
        match ir.static_text(binding) {
            Some(text) => {
                if dynamic {
                    self.note(node, WebNoteKind::DynamicBinding(Facet::Value));
                }
                if !text.is_empty() {
                    self.doc.set_attr(element, "value", text);
                }
            }
            None if dynamic => self.note(node, WebNoteKind::DynamicBinding(Facet::Value)),
            None => self.note(node, WebNoteKind::UnresolvedBinding(Facet::Value)),
        }
    }

    /// entry element 아래 DOM preorder에서 만나는 이 entry의 자식 entry 순서(다른 entry를 만나면 내려가지 않음).
    fn dom_order(&self, entry: u32, element: u32) -> Vec<u32> {
        let ir = self.ir;
        let mut out = Vec::new();
        let mut stack: Vec<u32> = self.element_children(element).into_iter().rev().collect();
        while let Some(current) = stack.pop() {
            let current_entry = self
                .node_of(current)
                .and_then(|n| ir.nodes[n as usize].semantics);
            match current_entry {
                Some(found) => {
                    if ir.accessibility[found as usize].parent == Some(entry) {
                        out.push(found);
                    }
                }
                None => stack.extend(self.element_children(current).into_iter().rev()),
            }
        }
        out
    }

    fn assign_reading_order(&mut self) {
        let ir = self.ir;
        let child_lists = std::mem::take(&mut self.children);
        for (index, entry) in ir.accessibility.iter().enumerate() {
            let Some(element) = self.element_of[entry.node as usize] else {
                continue;
            };
            let children = &child_lists[index];
            if children.is_empty() || self.dom_order(index as u32, element) == *children {
                continue;
            }
            let ids: Vec<String> = children
                .iter()
                .filter_map(|child| {
                    let node = ir.accessibility[*child as usize].node;
                    let target = self.element_of[node as usize]?;
                    Some((target, element_id(ir.nodes[node as usize].id)))
                })
                .map(|(target, id)| {
                    self.referenced.insert(target);
                    id
                })
                .collect();
            self.doc.set_attr(element, "aria-owns", ids.join(" "));
        }
        self.children = child_lists;
        // 접근성 root가 여럿이면 DOM 순서로만 읽힙니다.
        let roots: Vec<u32> = ir
            .accessibility
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.parent.is_none())
            .map(|(index, _)| index as u32)
            .collect();
        if roots.len() > 1 {
            let dom: Vec<u32> = ir
                .nodes
                .iter()
                .filter_map(|node| node.semantics)
                .filter(|entry| ir.accessibility[*entry as usize].parent.is_none())
                .collect();
            if dom != roots {
                let first = ir.accessibility[roots[0] as usize].node;
                self.note(first, WebNoteKind::RootOrder);
            }
        }
    }
}

/// IR 접근성 tree를 HTML/ARIA 조각으로 매핑합니다(모듈 문서 규칙).
pub fn map_accessibility(ir: &IrTree) -> WebAccessibility {
    let children = ir.accessibility_child_lists();
    let listbox = (0..ir.accessibility.len())
        .map(|entry| is_listbox(ir, &children, entry))
        .collect();
    let structure = map_structure(ir);
    let mut mapper = Mapper {
        ir,
        doc: structure.document,
        element_of: structure.element_of,
        notes: Vec::new(),
        referenced: BTreeSet::new(),
        children,
        listbox,
    };
    let has_semantics = ir.semantic_subtrees();
    mapper.assign_tags(&has_semantics);
    for (index, node) in ir.nodes.iter().enumerate() {
        let (Some(element), Some(entry)) = (mapper.element_of[index], node.semantics) else {
            continue;
        };
        let entry = &ir.accessibility[entry as usize];
        mapper.assign_name(index as u32, element, entry);
        mapper.assign_value(index as u32, element, entry);
        mapper.assign_states(index as u32, element, entry);
    }
    mapper.assign_reading_order();
    for element in mapper.referenced.clone() {
        if let Some(source) = mapper.doc.element(element).source {
            mapper.doc.set_attr(element, "id", element_id(source));
        }
    }
    let mut notes = mapper.notes;
    let order = |id: StableId| ir.node_index(id).unwrap_or(u32::MAX);
    notes.sort_by_key(|note| (order(note.node), note.kind));
    notes.dedup();
    WebAccessibility {
        document: mapper.doc,
        notes,
    }
}

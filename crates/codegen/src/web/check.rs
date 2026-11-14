//! 생성 HTML 조각의 정적 접근성 검사 (XXII.2.4).
//!
//! 브라우저 없이 [`HtmlDocument`] 구조만으로 WAI-ARIA 1.2·ARIA in HTML 규칙 중 생성기가 지켜야 하는 것을
//! 검사합니다. 생성 결과를 golden과 비교하는 것과 별개로, 매핑이 바뀌어도 접근성 규칙을 어기지 않았는지 봅니다.
//!
//! | 규칙 | issue |
//! |---|---|
//! | role 값이 WAI-ARIA 1.2 concrete role이 아님 | [`WebA11yIssue::UnknownRole`] |
//! | tag에 허용되지 않은 명시 role(ARIA in HTML) | [`WebA11yIssue::RoleNotAllowed`] |
//! | role이 지원하지 않는 `aria-*`, 이름 금지 role의 `aria-label`·`aria-labelledby`, tag에 없는 HTML 속성 | [`WebA11yIssue::AttributeNotAllowed`] |
//! | `true`/`false`(일부 `mixed`) 아닌 상태 값, 정수 아닌 `tabindex`, 맞지 않는 `type` | [`WebA11yIssue::InvalidAttributeValue`] |
//! | 필수 상태 누락(radio의 `aria-checked`) | [`WebA11yIssue::MissingRequiredState`] |
//! | 이름이 필요한 role(button·radio·textbox·dialog·img·listbox·radiogroup·option)에 이름 없음 | [`WebA11yIssue::MissingAccessibleName`] |
//! | `aria-labelledby`·`aria-owns`가 없는 id를 가리킴, id 중복 | [`WebA11yIssue::BrokenIdReference`], [`WebA11yIssue::DuplicateId`] |
//! | option이 listbox 밖, listitem이 list 밖, radio가 radiogroup 밖 | [`WebA11yIssue::MissingRequiredContext`] |
//! | listbox·list·radiogroup이 필요한 자식 role을 하나도 소유하지 않음 | [`WebA11yIssue::MissingRequiredOwned`] |
//! | `button` 안의 interactive content | [`WebA11yIssue::NestedInteractive`] |
//! | `aria-hidden` subtree 안의 focus 가능한 element | [`WebA11yIssue::FocusableInHidden`] |
//! | 두 element가 같은 element를 `aria-owns`, 소유 순환 | [`WebA11yIssue::OwnedTwice`], [`WebA11yIssue::OwnsCycle`] |
//!
//! 접근성 부모는 `aria-owns` 소유자 또는 DOM 부모이며, generic·none·presentation role element는 건너뜁니다.

use std::collections::{BTreeMap, BTreeSet};

use crate::web::html::{HtmlChild, HtmlDocument, HtmlElement};

/// 정적 검사 issue. `element`는 문서 안 element index입니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum WebA11yIssue {
    /// 알 수 없는 role.
    UnknownRole {
        /// element.
        element: u32,
        /// role 값.
        role: String,
    },
    /// tag에 허용되지 않은 role.
    RoleNotAllowed {
        /// element.
        element: u32,
        /// role.
        role: String,
    },
    /// 허용되지 않은 속성.
    AttributeNotAllowed {
        /// element.
        element: u32,
        /// 속성 이름.
        attribute: String,
    },
    /// 잘못된 속성 값.
    InvalidAttributeValue {
        /// element.
        element: u32,
        /// 속성 이름.
        attribute: String,
    },
    /// 필수 상태 누락.
    MissingRequiredState {
        /// element.
        element: u32,
        /// 필요한 속성.
        attribute: &'static str,
    },
    /// 접근 가능한 이름 없음.
    MissingAccessibleName {
        /// element.
        element: u32,
        /// effective role.
        role: String,
    },
    /// 없는 id 참조.
    BrokenIdReference {
        /// element.
        element: u32,
        /// 속성 이름.
        attribute: &'static str,
        /// 없는 id.
        id: String,
    },
    /// 같은 id가 둘 이상.
    DuplicateId {
        /// 두 번째 element.
        element: u32,
        /// id.
        id: String,
    },
    /// 필수 문맥 role 밖.
    MissingRequiredContext {
        /// element.
        element: u32,
        /// effective role.
        role: String,
    },
    /// 필요한 자식 role을 소유하지 않음.
    MissingRequiredOwned {
        /// element.
        element: u32,
        /// effective role.
        role: String,
    },
    /// `button` 안의 interactive content.
    NestedInteractive {
        /// 안쪽 element.
        element: u32,
        /// 바깥 `button`.
        ancestor: u32,
    },
    /// `aria-hidden` 안의 focus 가능한 element.
    FocusableInHidden {
        /// element.
        element: u32,
    },
    /// 두 element가 같은 element를 소유.
    OwnedTwice {
        /// 소유된 element.
        element: u32,
    },
    /// 소유 순환.
    OwnsCycle {
        /// 순환에 걸린 element.
        element: u32,
    },
}

/// WAI-ARIA 1.2 concrete role(검사 대상 집합).
const KNOWN_ROLES: &[&str] = &[
    "alert",
    "alertdialog",
    "application",
    "article",
    "banner",
    "blockquote",
    "button",
    "caption",
    "cell",
    "checkbox",
    "code",
    "columnheader",
    "combobox",
    "complementary",
    "contentinfo",
    "definition",
    "deletion",
    "dialog",
    "directory",
    "document",
    "emphasis",
    "feed",
    "figure",
    "form",
    "generic",
    "grid",
    "gridcell",
    "group",
    "heading",
    "img",
    "insertion",
    "link",
    "list",
    "listbox",
    "listitem",
    "log",
    "main",
    "marquee",
    "math",
    "menu",
    "menubar",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "meter",
    "navigation",
    "none",
    "note",
    "option",
    "paragraph",
    "presentation",
    "progressbar",
    "radio",
    "radiogroup",
    "region",
    "row",
    "rowgroup",
    "rowheader",
    "scrollbar",
    "search",
    "searchbox",
    "separator",
    "slider",
    "spinbutton",
    "status",
    "strong",
    "subscript",
    "superscript",
    "switch",
    "tab",
    "table",
    "tablist",
    "tabpanel",
    "term",
    "textbox",
    "time",
    "timer",
    "toolbar",
    "tooltip",
    "tree",
    "treegrid",
    "treeitem",
];

/// 이름을 붙일 수 없는 role(WAI-ARIA 1.2 "Name prohibited").
const NAME_PROHIBITED: &[&str] = &[
    "caption",
    "code",
    "deletion",
    "emphasis",
    "generic",
    "insertion",
    "none",
    "paragraph",
    "presentation",
    "strong",
    "subscript",
    "superscript",
];

/// 이름이 필요한 role(생성기가 만드는 것 중).
const NAME_REQUIRED: &[&str] = &[
    "button",
    "dialog",
    "img",
    "listbox",
    "option",
    "radio",
    "radiogroup",
    "textbox",
];

/// 내용에서 이름을 계산하는 role(생성기가 만드는 것 중).
const NAME_FROM_CONTENT: &[&str] = &["button", "listitem", "option", "radio"];

/// widget role(interactive).
const WIDGET_ROLES: &[&str] = &[
    "button",
    "checkbox",
    "combobox",
    "link",
    "listbox",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "radio",
    "searchbox",
    "slider",
    "spinbutton",
    "switch",
    "tab",
    "textbox",
    "treeitem",
];

/// role별 지원 상태 속성(전역 속성 제외).
fn supported_role_attribute(role: &str, attribute: &str) -> bool {
    let roles: &[&str] = match attribute {
        "aria-checked" => &[
            "checkbox",
            "menuitemcheckbox",
            "menuitemradio",
            "option",
            "radio",
            "switch",
        ],
        "aria-selected" => &[
            "columnheader",
            "gridcell",
            "option",
            "row",
            "rowheader",
            "tab",
            "treeitem",
        ],
        "aria-pressed" => &["button"],
        "aria-modal" => &["alertdialog", "dialog"],
        "aria-multiselectable" => &["grid", "listbox", "tablist", "tree", "treegrid"],
        "aria-readonly" => &[
            "checkbox",
            "columnheader",
            "combobox",
            "grid",
            "gridcell",
            "listbox",
            "radiogroup",
            "rowheader",
            "searchbox",
            "slider",
            "spinbutton",
            "textbox",
        ],
        "aria-disabled" => &[
            "application",
            "button",
            "checkbox",
            "combobox",
            "grid",
            "gridcell",
            "group",
            "link",
            "listbox",
            "menuitem",
            "option",
            "radio",
            "radiogroup",
            "scrollbar",
            "searchbox",
            "separator",
            "slider",
            "spinbutton",
            "switch",
            "tab",
            "textbox",
            "toolbar",
            "treeitem",
        ],
        _ => return false,
    };
    roles.contains(&role)
}

const GLOBAL_ARIA: &[&str] = &[
    "aria-describedby",
    "aria-hidden",
    "aria-label",
    "aria-labelledby",
    "aria-owns",
];

/// tag별 HTML 속성(전역 `id`·`role`·`tabindex`·`autofocus`·`data-*` 제외).
fn html_attribute_allowed(tag: &str, attribute: &str) -> bool {
    match attribute {
        "id" | "role" | "tabindex" | "autofocus" => true,
        "type" => matches!(tag, "button" | "input"),
        "disabled" => matches!(tag, "button" | "input"),
        "readonly" | "value" => tag == "input",
        "alt" => tag == "img",
        "open" => tag == "dialog",
        _ => attribute.starts_with("data-"),
    }
}

/// ARIA in HTML: tag에 허용되는 명시 role(`None`이면 모든 role 허용).
fn allowed_roles(element: &HtmlElement) -> Option<&'static [&'static str]> {
    Some(match element.tag {
        "div" | "span" => return None,
        "button" => &[
            "checkbox",
            "combobox",
            "link",
            "menuitem",
            "menuitemcheckbox",
            "menuitemradio",
            "option",
            "radio",
            "switch",
            "tab",
        ],
        "input" => &["combobox", "searchbox", "spinbutton"],
        "ul" => &[
            "directory",
            "group",
            "listbox",
            "menu",
            "menubar",
            "none",
            "presentation",
            "radiogroup",
            "tablist",
            "toolbar",
            "tree",
        ],
        "li" => &[
            "menuitem",
            "menuitemcheckbox",
            "menuitemradio",
            "none",
            "option",
            "presentation",
            "radio",
            "separator",
            "tab",
            "treeitem",
        ],
        "img" if element.attr("alt") == Some("") => &["none", "presentation"],
        "img" => &[
            "button",
            "checkbox",
            "link",
            "menuitem",
            "menuitemcheckbox",
            "menuitemradio",
            "meter",
            "option",
            "progressbar",
            "radio",
            "scrollbar",
            "separator",
            "slider",
            "switch",
            "tab",
            "treeitem",
        ],
        "dialog" => &["alertdialog"],
        _ => return None,
    })
}

/// tag의 implicit role.
fn implicit_role(element: &HtmlElement) -> &'static str {
    match element.tag {
        "button" => "button",
        "input" => "textbox",
        "ul" => "list",
        "li" => "listitem",
        "img" if element.attr("alt") == Some("") => "presentation",
        "img" => "img",
        "dialog" => "dialog",
        _ => "generic",
    }
}

/// element의 effective role(명시 role의 첫 token, 없으면 implicit).
pub fn effective_role(element: &HtmlElement) -> String {
    match element
        .attr("role")
        .and_then(|role| role.split_whitespace().next())
    {
        Some(role) => role.to_string(),
        None => implicit_role(element).to_string(),
    }
}

fn ignored_role(role: &str) -> bool {
    matches!(role, "generic" | "none" | "presentation")
}

fn is_hidden_attr(element: &HtmlElement) -> bool {
    element.attr("aria-hidden") == Some("true")
}

fn is_focusable(element: &HtmlElement) -> bool {
    if let Some(index) = element.attr("tabindex") {
        return index.parse::<i32>().is_ok_and(|value| value >= 0);
    }
    matches!(element.tag, "button" | "input") && element.attr("disabled").is_none()
}

fn is_interactive(element: &HtmlElement) -> bool {
    matches!(element.tag, "button" | "input")
        || element.attr("tabindex").is_some()
        || WIDGET_ROLES.contains(&effective_role(element).as_str())
}

struct Checker<'a> {
    doc: &'a HtmlDocument,
    issues: Vec<WebA11yIssue>,
    ids: BTreeMap<&'a str, u32>,
    owner: BTreeMap<u32, u32>,
    hidden: Vec<bool>,
}

impl<'a> Checker<'a> {
    fn element(&self, index: u32) -> &'a HtmlElement {
        self.doc.element(index)
    }

    fn element_children(&self, index: u32) -> impl Iterator<Item = u32> + 'a {
        self.doc
            .element(index)
            .children
            .iter()
            .filter_map(|child| match child {
                HtmlChild::Element(child) => Some(*child),
                HtmlChild::Text(_) => None,
            })
    }

    fn collect_ids(&mut self) {
        for (index, element) in self.doc.elements().iter().enumerate() {
            if let Some(id) = element.attr("id") {
                if self.ids.insert(id, index as u32).is_some() {
                    self.issues.push(WebA11yIssue::DuplicateId {
                        element: index as u32,
                        id: id.to_string(),
                    });
                }
            }
        }
    }

    fn collect_hidden(&mut self) {
        // 부모가 먼저 계산되도록 root부터 DOM preorder로 훑습니다.
        let mut stack: Vec<(u32, bool)> = self.doc.roots().iter().map(|r| (*r, false)).collect();
        while let Some((index, inherited)) = stack.pop() {
            let hidden = inherited || is_hidden_attr(self.element(index));
            self.hidden[index as usize] = hidden;
            let children: Vec<u32> = self.element_children(index).collect();
            stack.extend(children.into_iter().map(|child| (child, hidden)));
        }
    }

    fn collect_owners(&mut self) {
        for (index, element) in self.doc.elements().iter().enumerate() {
            let Some(owns) = element.attr("aria-owns") else {
                continue;
            };
            for id in owns.split_whitespace() {
                match self.ids.get(id) {
                    Some(&target) => {
                        if self.owner.insert(target, index as u32).is_some() {
                            self.issues
                                .push(WebA11yIssue::OwnedTwice { element: target });
                        }
                    }
                    None => self.issues.push(WebA11yIssue::BrokenIdReference {
                        element: index as u32,
                        attribute: "aria-owns",
                        id: id.to_string(),
                    }),
                }
            }
        }
    }

    /// 접근성 부모(소유자 또는 DOM 부모, ignored role은 건너뜀). 순환이면 `Err(())`.
    fn accessible_parent(&self, index: u32) -> Result<Option<u32>, ()> {
        let limit = self.doc.elements().len() + 1;
        let mut current = index;
        for _ in 0..limit {
            let parent = self
                .owner
                .get(&current)
                .copied()
                .or(self.element(current).parent);
            let Some(parent) = parent else {
                return Ok(None);
            };
            if parent == index {
                return Err(());
            }
            if !ignored_role(&effective_role(self.element(parent))) {
                return Ok(Some(parent));
            }
            current = parent;
        }
        Err(())
    }

    /// 이름 계산용 텍스트(aria-hidden subtree 제외, 하위 element는 aria-label·alt 우선). 문서 순서입니다.
    fn text_content(&self, index: u32) -> String {
        enum Item<'t> {
            Element(u32),
            Text(&'t str),
        }
        let mut out = String::new();
        let mut stack = vec![Item::Element(index)];
        while let Some(item) = stack.pop() {
            let current = match item {
                Item::Text(text) => {
                    out.push_str(text);
                    out.push(' ');
                    continue;
                }
                Item::Element(current) => current,
            };
            let element = self.element(current);
            if current != index {
                if is_hidden_attr(element) {
                    continue;
                }
                if let Some(label) = element.attr("aria-label") {
                    out.push_str(label);
                    out.push(' ');
                    continue;
                }
                if element.tag == "img" {
                    out.push_str(element.attr("alt").unwrap_or_default());
                    out.push(' ');
                    continue;
                }
            }
            for child in element.children.iter().rev() {
                stack.push(match child {
                    HtmlChild::Element(child) => Item::Element(*child),
                    HtmlChild::Text(text) => Item::Text(text),
                });
            }
        }
        out
    }

    fn accessible_name(&self, index: u32, role: &str) -> String {
        let element = self.element(index);
        if let Some(ids) = element.attr("aria-labelledby") {
            let mut out = String::new();
            for id in ids.split_whitespace() {
                if let Some(&target) = self.ids.get(id) {
                    let target_element = self.element(target);
                    match target_element.attr("aria-label") {
                        Some(label) => out.push_str(label),
                        None => out.push_str(&self.text_content(target)),
                    }
                }
            }
            if !out.trim().is_empty() {
                return out;
            }
        }
        if let Some(label) = element.attr("aria-label") {
            if !label.trim().is_empty() {
                return label.to_string();
            }
        }
        if element.tag == "img" {
            if let Some(alt) = element.attr("alt") {
                return alt.to_string();
            }
        }
        if NAME_FROM_CONTENT.contains(&role) {
            return self.text_content(index);
        }
        String::new()
    }

    fn check_element(&mut self, index: u32) {
        let element = self.element(index);
        let role = effective_role(element);
        if let Some(explicit) = element.attr("role") {
            let first = explicit.split_whitespace().next().unwrap_or_default();
            if !KNOWN_ROLES.contains(&first) {
                self.issues.push(WebA11yIssue::UnknownRole {
                    element: index,
                    role: first.to_string(),
                });
            } else if let Some(allowed) = allowed_roles(element) {
                if !allowed.contains(&first) {
                    self.issues.push(WebA11yIssue::RoleNotAllowed {
                        element: index,
                        role: first.to_string(),
                    });
                }
            }
        }
        for (name, value) in &element.attributes {
            let name = name.as_str();
            let allowed = if name.starts_with("aria-") {
                if GLOBAL_ARIA.contains(&name) {
                    !(matches!(name, "aria-label" | "aria-labelledby")
                        && NAME_PROHIBITED.contains(&role.as_str()))
                } else {
                    supported_role_attribute(&role, name)
                }
            } else {
                html_attribute_allowed(element.tag, name)
            };
            if !allowed {
                self.issues.push(WebA11yIssue::AttributeNotAllowed {
                    element: index,
                    attribute: name.to_string(),
                });
                continue;
            }
            let valid = match name {
                "aria-checked" | "aria-pressed" => {
                    matches!(value.as_str(), "true" | "false" | "mixed")
                }
                "aria-selected"
                | "aria-modal"
                | "aria-multiselectable"
                | "aria-readonly"
                | "aria-disabled"
                | "aria-hidden" => matches!(value.as_str(), "true" | "false"),
                "tabindex" => value.parse::<i32>().is_ok(),
                "type" => match element.tag {
                    "button" => value == "button",
                    "input" => value == "text",
                    _ => true,
                },
                _ => true,
            };
            if !valid {
                self.issues.push(WebA11yIssue::InvalidAttributeValue {
                    element: index,
                    attribute: name.to_string(),
                });
            }
        }
        if let Some(ids) = element.attr("aria-labelledby") {
            for id in ids.split_whitespace() {
                if !self.ids.contains_key(id) {
                    self.issues.push(WebA11yIssue::BrokenIdReference {
                        element: index,
                        attribute: "aria-labelledby",
                        id: id.to_string(),
                    });
                }
            }
        }
        if self.hidden[index as usize] {
            if is_focusable(element) {
                self.issues
                    .push(WebA11yIssue::FocusableInHidden { element: index });
            }
            return;
        }
        if role == "radio" && element.attr("aria-checked").is_none() {
            self.issues.push(WebA11yIssue::MissingRequiredState {
                element: index,
                attribute: "aria-checked",
            });
        }
        if NAME_REQUIRED.contains(&role.as_str())
            && self.accessible_name(index, &role).trim().is_empty()
        {
            self.issues.push(WebA11yIssue::MissingAccessibleName {
                element: index,
                role: role.clone(),
            });
        }
        let context: Option<&[&str]> = match role.as_str() {
            "option" => Some(&["listbox", "group"]),
            "listitem" => Some(&["list", "directory", "group"]),
            "radio" => Some(&["radiogroup"]),
            _ => None,
        };
        if let Some(context) = context {
            match self.accessible_parent(index) {
                Ok(parent) => {
                    let ok = parent.is_some_and(|p| {
                        context.contains(&effective_role(self.element(p)).as_str())
                    });
                    if !ok {
                        self.issues.push(WebA11yIssue::MissingRequiredContext {
                            element: index,
                            role: role.clone(),
                        });
                    }
                }
                Err(()) => self.issues.push(WebA11yIssue::OwnsCycle { element: index }),
            }
        }
        // button 안 interactive content(DOM 조상 기준).
        if is_interactive(element) {
            let mut ancestor = element.parent;
            while let Some(current) = ancestor {
                if self.element(current).tag == "button" {
                    self.issues.push(WebA11yIssue::NestedInteractive {
                        element: index,
                        ancestor: current,
                    });
                    break;
                }
                ancestor = self.element(current).parent;
            }
        }
    }

    /// 소유자·DOM 부모를 따라 올라가다 자신으로 돌아오는지.
    fn in_cycle(&self, index: u32) -> bool {
        let mut current = index;
        for _ in 0..=self.doc.elements().len() {
            let parent = self
                .owner
                .get(&current)
                .copied()
                .or(self.element(current).parent);
            match parent {
                None => return false,
                Some(parent) if parent == index => return true,
                Some(parent) => current = parent,
            }
        }
        false
    }

    fn check_owned(&mut self) {
        let count = self.doc.elements().len();
        for index in 0..count as u32 {
            if self.in_cycle(index) {
                self.issues.push(WebA11yIssue::OwnsCycle { element: index });
            }
        }
        let mut owned_roles: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
        for index in 0..count as u32 {
            if self.hidden[index as usize] {
                continue;
            }
            let role = effective_role(self.element(index));
            if ignored_role(&role) {
                continue;
            }
            match self.accessible_parent(index) {
                Ok(Some(parent)) => {
                    owned_roles.entry(parent).or_default().insert(role);
                }
                Ok(None) => {}
                Err(()) => {
                    if !self
                        .issues
                        .contains(&WebA11yIssue::OwnsCycle { element: index })
                    {
                        self.issues.push(WebA11yIssue::OwnsCycle { element: index });
                    }
                }
            }
        }
        for index in 0..count as u32 {
            if self.hidden[index as usize] {
                continue;
            }
            let role = effective_role(self.element(index));
            let required: &[&str] = match role.as_str() {
                "listbox" => &["option", "group"],
                "list" => &["listitem"],
                "radiogroup" => &["radio"],
                _ => continue,
            };
            let owns = owned_roles
                .get(&index)
                .is_some_and(|roles| roles.iter().any(|r| required.contains(&r.as_str())));
            if !owns {
                self.issues.push(WebA11yIssue::MissingRequiredOwned {
                    element: index,
                    role,
                });
            }
        }
    }
}

/// 정적 접근성 검사. issue를 정렬해 돌려줍니다(없으면 빈 목록).
pub fn check_document(doc: &HtmlDocument) -> Vec<WebA11yIssue> {
    let mut checker = Checker {
        doc,
        issues: Vec::new(),
        ids: BTreeMap::new(),
        owner: BTreeMap::new(),
        hidden: vec![false; doc.elements().len()],
    };
    checker.collect_ids();
    checker.collect_hidden();
    checker.collect_owners();
    for index in 0..doc.elements().len() as u32 {
        checker.check_element(index);
    }
    checker.check_owned();
    let mut issues = checker.issues;
    issues.sort();
    issues.dedup();
    issues
}

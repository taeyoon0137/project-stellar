//! XXII.2.4 검증: HTML/ARIA golden, 정적 접근성 검사 fixture(통과·위반 규칙별).

use stellar_accessibility::node::StateKind;

use crate::fixture::{self as fx, sid, Fixture};
use crate::web::aria::{element_id, map_accessibility, Facet, WebNote, WebNoteKind};
use crate::web::check::{check_against_ir, check_document, WebA11yIssue};
use crate::web::html::HtmlDocument;

#[test]
fn site_profile_maps_to_golden_html_and_passes_static_check() {
    // 검증 기준(XXII.2.4): 정적 접근성 검사 fixture. fixture View의 접근성 tree를 HTML/ARIA로 매핑한 결과가
    // golden과 같고 정적 검사 issue가 없습니다.
    let fixture = Fixture::new();
    let ir = fixture.ir(&fx::web_profile());
    let web = map_accessibility(&ir);
    fx::check_golden("web_aria_site.html", &web.to_html());
    assert_eq!(check_document(&web.document), Vec::new());
    assert_eq!(check_against_ir(&ir, &web.document), Vec::new());
    // 상태 변수에 묶인 값은 초기값을 쓰고, 정적 HTML이 표현하지 못하는 modal은 노트로 남깁니다.
    assert_eq!(
        web.notes,
        vec![
            WebNote {
                node: sid(8),
                kind: WebNoteKind::DynamicBinding(Facet::State(StateKind::Checked)),
            },
            WebNote {
                node: sid(10),
                kind: WebNoteKind::DynamicBinding(Facet::State(StateKind::Checked)),
            },
            WebNote {
                node: sid(11),
                kind: WebNoteKind::DynamicBinding(Facet::Value),
            },
            WebNote {
                node: sid(12),
                kind: WebNoteKind::DynamicBinding(Facet::State(StateKind::Disabled)),
            },
            WebNote {
                node: sid(16),
                kind: WebNoteKind::NoEquivalent(Facet::State(StateKind::Modal)),
            },
        ]
    );
}

#[test]
fn every_profile_ir_passes_static_check_and_is_deterministic() {
    let fixture = Fixture::new();
    for profile in [fx::web_profile(), fx::ios_profile(), fx::android_profile()] {
        let ir = fixture.ir(&profile);
        let first = map_accessibility(&ir);
        assert_eq!(first, map_accessibility(&ir));
        assert_eq!(
            check_document(&first.document),
            Vec::new(),
            "{}",
            profile.id.as_str()
        );
        assert_eq!(
            check_against_ir(&ir, &first.document),
            Vec::new(),
            "{}",
            profile.id.as_str()
        );
    }
}

#[test]
fn fragment_is_transparent_for_hidden_subtrees() {
    // Fragment 아래에서 시작하는 entry 없는 subtree도 맨 위 element를 숨깁니다(Fragment는 element가 없음).
    let fixture = fx::fragment_fixture();
    let ir = fixture.ir(&fx::web_profile());
    let web = map_accessibility(&ir);
    let index = |n: u8| {
        web.document
            .elements()
            .iter()
            .position(|e| e.source == Some(sid(n)))
            .expect("element") as u32
    };
    let (decoration, visible) = (index(3), index(4));
    assert_eq!(
        web.document.element(decoration).attr("aria-hidden"),
        Some("true")
    );
    assert_eq!(web.document.element(visible).attr("aria-hidden"), None);
    assert_eq!(check_document(&web.document), Vec::new());
    assert_eq!(check_against_ir(&ir, &web.document), Vec::new());
    // IR 대조 검사는 숨김이 빠진 장식과 숨겨진 의미를 잡습니다.
    let mut exposed = web.document.clone();
    exposed.remove_attr(decoration, "aria-hidden");
    exposed.set_attr(visible, "aria-hidden", "true");
    assert_eq!(
        check_against_ir(&ir, &exposed),
        vec![
            WebA11yIssue::ExposedWithoutSemantics {
                element: decoration
            },
            WebA11yIssue::SemanticsHidden { element: visible },
        ]
    );
}

#[test]
fn mapping_uses_native_semantics_reading_order_and_hidden_subtrees() {
    let fixture = Fixture::new();
    let ir = fixture.ir(&fx::web_profile());
    let web = map_accessibility(&ir);
    let doc = &web.document;
    let of = |n: u8| {
        doc.elements()
            .iter()
            .position(|e| e.source == Some(sid(n)))
            .map(|i| doc.element(i as u32))
            .expect("element")
    };
    // Fragment(3)는 element가 없고 자식이 root에 붙습니다.
    assert!(doc.elements().iter().all(|e| e.source != Some(sid(3))));
    assert_eq!(of(4).parent, doc.roots().first().copied());
    // 이미지 이름 = 속성 값(alt), 장식 shape는 aria-hidden.
    assert_eq!(of(4).tag, "img");
    assert_eq!(of(4).attr("alt"), Some("프로필 사진"));
    assert_eq!(of(5).attr("aria-hidden"), Some("true"));
    // radio group·radio·초기 선택.
    assert_eq!(of(6).attr("role"), Some("radiogroup"));
    assert_eq!(of(6).attr("aria-label"), Some("크기"));
    assert_eq!(of(7).tag, "div");
    assert_eq!(of(7).attr("role"), None);
    assert_eq!((of(8).tag, of(8).attr("role")), ("button", Some("radio")));
    assert_eq!(of(8).attr("aria-checked"), Some("true"));
    assert_eq!(of(10).attr("aria-checked"), Some("false"));
    // text field, 비활성 초기값 false라 disabled 없음, dialog.
    assert_eq!((of(11).tag, of(11).attr("type")), ("input", Some("text")));
    assert_eq!(of(11).attr("aria-label"), Some("메모"));
    assert_eq!(of(12).attr("disabled"), None);
    assert_eq!(of(16).tag, "dialog");
    // `<dialog open>`은 non-modal이라 aria-modal을 쓰지 않습니다(노트).
    assert_eq!(of(16).attr("aria-modal"), None);
    // 읽기 순서(이미지 먼저)가 DOM 순서와 달라 root가 자식 전체를 aria-owns로 적습니다.
    let root = doc.element(doc.roots()[0]);
    let owns: Vec<&str> = root
        .attr("aria-owns")
        .expect("aria-owns")
        .split(' ')
        .collect();
    assert_eq!(owns.first().copied(), Some(element_id(sid(4)).as_str()));
    assert_eq!(owns.len(), 7);
    assert!(owns.iter().all(|id| doc.by_id(id).is_some()));
}

#[test]
fn disabled_and_listbox_mapping_follow_state_values() {
    // 저장 중(초기값 true)이면 native button에 disabled.
    let mut fixture = Fixture::new();
    for state in &mut fixture.states {
        if state.id == sid(fx::STATE_SAVING) {
            state.initial = stellar_core_types::typed_value::TypedValue::Boolean(true);
        }
    }
    let ir = fixture.ir(&fx::web_profile());
    let web = map_accessibility(&ir);
    let button = web
        .document
        .elements()
        .iter()
        .find(|e| e.source == Some(sid(12)))
        .expect("button");
    assert_eq!(button.attr("disabled"), Some(""));
    assert_eq!(check_document(&web.document), Vec::new());
}

/// 손으로 만든 문서 도우미.
struct Doc {
    doc: HtmlDocument,
}

impl Doc {
    fn new() -> Doc {
        Doc {
            doc: HtmlDocument::new(),
        }
    }

    fn root(&mut self, tag: &'static str, attrs: &[(&str, &str)]) -> u32 {
        let index = self.doc.add(tag, None);
        self.doc.push_root(index);
        for (name, value) in attrs {
            self.doc.set_attr(index, name, *value);
        }
        index
    }

    fn child(&mut self, parent: u32, tag: &'static str, attrs: &[(&str, &str)]) -> u32 {
        let index = self.doc.add(tag, None);
        self.doc.append(parent, index);
        for (name, value) in attrs {
            self.doc.set_attr(index, name, *value);
        }
        index
    }

    fn text(&mut self, parent: u32, text: &str) {
        self.doc.append_text(parent, text);
    }

    fn issues(&self) -> Vec<WebA11yIssue> {
        check_document(&self.doc)
    }
}

#[test]
fn static_check_detects_each_rule_violation() {
    // 각 규칙의 위반 fixture가 정확히 그 issue를 냅니다.
    let mut d = Doc::new();
    let root = d.root("div", &[("role", "toolbarx")]);
    let _ = root;
    assert_eq!(
        d.issues(),
        vec![WebA11yIssue::UnknownRole {
            element: 0,
            role: "toolbarx".to_string()
        }]
    );

    let mut d = Doc::new();
    let dialog = d.root("dialog", &[("role", "button"), ("aria-label", "x")]);
    assert_eq!(
        d.issues(),
        vec![WebA11yIssue::RoleNotAllowed {
            element: dialog,
            role: "button".to_string()
        }]
    );

    let mut d = Doc::new();
    let root = d.root(
        "div",
        &[("aria-label", "이름 금지"), ("aria-checked", "true")],
    );
    assert_eq!(
        d.issues(),
        vec![
            WebA11yIssue::AttributeNotAllowed {
                element: root,
                attribute: "aria-checked".to_string()
            },
            WebA11yIssue::AttributeNotAllowed {
                element: root,
                attribute: "aria-label".to_string()
            },
        ]
    );

    let mut d = Doc::new();
    let group = d.root("div", &[("role", "radiogroup"), ("aria-label", "g")]);
    let radio = d.child(
        group,
        "button",
        &[
            ("type", "submit"),
            ("role", "radio"),
            ("aria-checked", "yes"),
        ],
    );
    d.text(radio, "하나");
    assert_eq!(
        d.issues(),
        vec![
            WebA11yIssue::InvalidAttributeValue {
                element: radio,
                attribute: "aria-checked".to_string()
            },
            WebA11yIssue::InvalidAttributeValue {
                element: radio,
                attribute: "type".to_string()
            },
        ]
    );

    let mut d = Doc::new();
    let group = d.root("div", &[("role", "radiogroup"), ("aria-label", "g")]);
    let radio = d.child(group, "button", &[("type", "button"), ("role", "radio")]);
    d.text(radio, "하나");
    assert_eq!(
        d.issues(),
        vec![WebA11yIssue::MissingRequiredState {
            element: radio,
            attribute: "aria-checked"
        }]
    );

    let mut d = Doc::new();
    let button = d.root("button", &[("type", "button")]);
    assert_eq!(
        d.issues(),
        vec![WebA11yIssue::MissingAccessibleName {
            element: button,
            role: "button".to_string()
        }]
    );

    let mut d = Doc::new();
    let input = d.root("input", &[("type", "text"), ("aria-labelledby", "missing")]);
    assert_eq!(
        d.issues(),
        vec![
            WebA11yIssue::MissingAccessibleName {
                element: input,
                role: "textbox".to_string()
            },
            WebA11yIssue::BrokenIdReference {
                element: input,
                attribute: "aria-labelledby",
                id: "missing".to_string()
            },
        ]
    );

    let mut d = Doc::new();
    let root = d.root("div", &[("id", "a")]);
    let second = d.child(root, "span", &[("id", "a")]);
    assert_eq!(
        d.issues(),
        vec![WebA11yIssue::DuplicateId {
            element: second,
            id: "a".to_string()
        }]
    );

    let mut d = Doc::new();
    let root = d.root("div", &[]);
    let option = d.child(
        root,
        "div",
        &[("role", "option"), ("aria-selected", "false")],
    );
    d.text(option, "항목");
    let radio = d.child(
        root,
        "button",
        &[
            ("type", "button"),
            ("role", "radio"),
            ("aria-checked", "false"),
        ],
    );
    d.text(radio, "하나");
    assert_eq!(
        d.issues(),
        vec![
            WebA11yIssue::MissingRequiredContext {
                element: option,
                role: "option".to_string()
            },
            WebA11yIssue::MissingRequiredContext {
                element: radio,
                role: "radio".to_string()
            },
        ]
    );

    let mut d = Doc::new();
    let listbox = d.root("ul", &[("role", "listbox"), ("aria-label", "목록")]);
    let list = d.child(listbox, "div", &[("role", "list")]);
    let _ = list;
    let group = d.child(
        listbox,
        "div",
        &[("role", "radiogroup"), ("aria-label", "g")],
    );
    assert_eq!(
        d.issues(),
        vec![
            WebA11yIssue::MissingRequiredOwned {
                element: listbox,
                role: "listbox".to_string()
            },
            WebA11yIssue::MissingRequiredOwned {
                element: list,
                role: "list".to_string()
            },
            WebA11yIssue::MissingRequiredOwned {
                element: group,
                role: "radiogroup".to_string()
            },
        ]
    );

    let mut d = Doc::new();
    let outer = d.root("button", &[("type", "button"), ("aria-label", "바깥")]);
    let inner = d.child(outer, "input", &[("type", "text"), ("aria-label", "안쪽")]);
    assert_eq!(
        d.issues(),
        vec![WebA11yIssue::NestedInteractive {
            element: inner,
            ancestor: outer
        }]
    );

    let mut d = Doc::new();
    let root = d.root("div", &[("aria-hidden", "true")]);
    let button = d.child(
        root,
        "button",
        &[("type", "button"), ("aria-label", "숨김")],
    );
    assert_eq!(
        d.issues(),
        vec![WebA11yIssue::FocusableInHidden { element: button }]
    );

    let mut d = Doc::new();
    let dialog = d.root(
        "dialog",
        &[("aria-label", "알림"), ("aria-modal", "true"), ("open", "")],
    );
    assert_eq!(
        d.issues(),
        vec![WebA11yIssue::ModalOpenDialog { element: dialog }]
    );
    // `open` 없이 script가 `showModal()`로 여는 dialog의 aria-modal은 허용합니다.
    let mut d = Doc::new();
    d.root("dialog", &[("aria-label", "알림"), ("aria-modal", "true")]);
    assert_eq!(d.issues(), Vec::new());

    let mut d = Doc::new();
    let root = d.root("div", &[("role", "group"), ("aria-owns", "x")]);
    let other = d.child(root, "div", &[("role", "group"), ("aria-owns", "x")]);
    let target = d.child(other, "span", &[("id", "x")]);
    let _ = (root, other);
    assert_eq!(
        d.issues(),
        vec![WebA11yIssue::OwnedTwice { element: target }]
    );

    let mut d = Doc::new();
    let a = d.root("div", &[("role", "group"), ("id", "a"), ("aria-owns", "b")]);
    let b = d.child(
        a,
        "div",
        &[
            ("role", "radiogroup"),
            ("id", "b"),
            ("aria-owns", "a"),
            ("aria-label", "g"),
        ],
    );
    let radio = d.child(
        b,
        "button",
        &[
            ("type", "button"),
            ("role", "radio"),
            ("aria-checked", "true"),
        ],
    );
    d.text(radio, "하나");
    let issues = d.issues();
    assert!(
        issues.contains(&WebA11yIssue::OwnsCycle { element: a }),
        "{issues:?}"
    );
}

#[test]
fn static_check_accepts_native_and_labelledby_names() {
    let mut d = Doc::new();
    let root = d.root("div", &[("role", "group"), ("aria-labelledby", "t")]);
    let title = d.child(root, "span", &[("id", "t")]);
    d.text(title, "제목");
    let list = d.child(
        root,
        "ul",
        &[
            ("role", "listbox"),
            ("aria-labelledby", "t"),
            ("tabindex", "0"),
        ],
    );
    let item = d.child(list, "li", &[("role", "option"), ("aria-selected", "true")]);
    d.text(item, "사과");
    let image = d.child(root, "img", &[("alt", "로고")]);
    let decorative = d.child(root, "img", &[("alt", ""), ("aria-hidden", "true")]);
    let _ = (image, decorative);
    assert_eq!(d.issues(), Vec::new());
}

#[test]
fn selectable_list_maps_to_listbox_with_options() {
    use stellar_accessibility::node::{
        AccessibilityBinding, AccessibilityNode, AccessibilityRole, ActionKind, ActionTarget,
    };
    use stellar_accessibility::snapshot::AccessibilitySnapshot;
    use stellar_accessibility::tree::AccessibilityTree;
    // 6은 이름 있는 group, 7은 단일 선택 List, 8·10은 선택 가능한 항목, 9는 항목 텍스트.
    let mut fixture = Fixture::new();
    let mut tree = AccessibilityTree::new();
    let selection = sid(fx::STATE_SIZE);
    let item = |n: u8| {
        AccessibilityNode::new(sid(n), AccessibilityRole::ListItem)
            .with_state(
                StateKind::Selected,
                AccessibilityBinding::SelectionMatches {
                    selection,
                    item: sid(n),
                },
            )
            .with_action(ActionKind::Select, ActionTarget::State(selection))
    };
    for node in [
        AccessibilityNode::new(sid(1), AccessibilityRole::Generic),
        AccessibilityNode::new(sid(6), AccessibilityRole::Group)
            .with_label(AccessibilityBinding::text("크기")),
        AccessibilityNode::new(sid(7), AccessibilityRole::List)
            .with_label(AccessibilityBinding::text("크기 목록"))
            .with_state(StateKind::Focusable, AccessibilityBinding::flag(true)),
        item(8),
        AccessibilityNode::new(sid(9), AccessibilityRole::StaticText)
            .with_label(AccessibilityBinding::NodeText(sid(9))),
        item(10).with_label(AccessibilityBinding::text("크게")),
    ] {
        tree.insert(node).expect("insert");
    }
    tree.push_root(sid(1));
    tree.push_child(sid(1), sid(6));
    tree.push_child(sid(6), sid(7));
    tree.push_child(sid(7), sid(8));
    tree.push_child(sid(8), sid(9));
    tree.push_child(sid(7), sid(10));
    fixture.accessibility = AccessibilitySnapshot::from_tree(&tree).expect("snapshot");
    let ir = fixture.ir(&fx::web_profile());
    let web = map_accessibility(&ir);
    let doc = &web.document;
    let of = |n: u8| {
        doc.elements()
            .iter()
            .find(|e| e.source == Some(sid(n)))
            .expect("element")
    };
    assert_eq!((of(7).tag, of(7).attr("role")), ("ul", Some("listbox")));
    assert_eq!(of(7).attr("tabindex"), Some("0"));
    assert_eq!((of(8).tag, of(8).attr("role")), ("li", Some("option")));
    assert_eq!(of(8).attr("aria-selected"), Some("true"));
    assert_eq!(of(10).attr("aria-selected"), Some("false"));
    assert_eq!(of(6).attr("role"), Some("group"));
    assert_eq!(check_document(doc), Vec::new());
}

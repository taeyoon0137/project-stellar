//! 접근성 의미 → SwiftUI·AppKit API (XXII.3.3).
//!
//! node마다 의미를 dialect 중립 동작 목록으로 정한 뒤 dialect별로 씁니다.
//!
//! | 의미 | SwiftUI(`ViewModifier`) | AppKit(`NSView`) |
//! |---|---|---|
//! | 접근성 entry 없는 subtree(맨 위 view, Fragment는 view가 없어 건너뜀) | `.accessibilityHidden(true)` | `setAccessibilityElement(false)`, `setAccessibilityChildren([])` |
//! | group·radioGroup·list·dialog | `.accessibilityElement(children: .contain)` | `setAccessibilityElement(true)` + role |
//! | button·radio·listItem | `.accessibilityElement(children: .combine)` | `setAccessibilityElement(true)` + role, 이름이 없으면 하위 staticText 텍스트를 label로 |
//! | role | button·radio `.isButton`, staticText `.isStaticText`, image `.isImage` | `.group` `.button` `.textField` `.list` `.radioGroup` `.radioButton` `.staticText` `.image`, dialog는 `.group` + subrole `.dialog`(근사), listItem은 `.group`(근사) |
//! | textField | native `TextField`가 제공(노트) | `.textField` |
//! | label | `.accessibilityLabel(Text(verbatim:))`·`Text(상태)` | `setAccessibilityLabel` |
//! | 자기 텍스트 내용(staticText) | `Text` view가 제공 | `setAccessibilityValue` |
//! | value | `.accessibilityValue(Text(상태))` | `setAccessibilityValue` |
//! | disabled | `.disabled(식)` | `setAccessibilityEnabled(!식)` |
//! | selected·checked | `.accessibilityAddTraits(식 ? .isSelected : [])` | selected `setAccessibilitySelected`, checked `setAccessibilityValue(식 ? 1 : 0)` |
//! | pressed | `.isToggle`(iOS·tvOS 17, macOS 14, watchOS 10, visionOS 1 이상) + `.isSelected` | `setAccessibilityValue(식 ? 1 : 0)`(근사) |
//! | focused | 대응 없음(노트, `FocusState`는 XXII.3.1) | `setAccessibilityFocused` |
//! | modal | `.accessibilityAddTraits(.isModal)` | `setAccessibilityModal(true)` |
//! | multiselectable | 대응 없음(노트) | 대응 없음(노트) |
//! | activate·select | 기본 `.accessibilityAction { }` 하나로 합침 | `NSAccessibilityCustomAction`(근사) |
//! | dismiss | `.accessibilityAction(.escape) { }` | `NSAccessibilityCustomAction`(근사) |
//! | 읽기 순서(DOM과 다름) | 부모 `.contain` + 자식 `.accessibilitySortPriority`(큰 값 먼저) | 부모 `setAccessibilityChildren`(id → view 표) |
//! | focusable·editable·focus·setValue | native control이 제공 | native control이 제공 |
//!
//! 상태 binding은 상태 변수 이름(정리한 Swift 식별자)으로 읽습니다. SwiftUI는 `@Binding`, AppKit은 값 인자입니다.
//! 선택 일치는 `상태 == "<item StableId>"`입니다. semantic action은 `onAction("<action StableId>")`, 상태 쓰기는
//! SwiftUI는 binding 대입, AppKit은 `onStateChange("<상태 StableId>", "<값>")`입니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use stellar_accessibility::node::{
    AccessibilityBinding, AccessibilityRole, ActionKind, ActionTarget, StateKind,
};
use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;
use stellar_target_profile::capability::ElementKind;
use stellar_target_profile::spec::{ApplePlatform, AppleProfile};

use crate::apple::swift::{identifiers, string_literal};
use crate::apple::{AppleDialect, AppleNote, AppleNoteKind, AppleNoteSubject};
use crate::facet::Facet;
use crate::ir::snapshot::id_hex;
use crate::ir::{CodegenIr, IrAccessibilityEntry, ResolvedBinding};

/// 상태 변수의 Swift 타입.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SwiftType {
    /// `String`.
    String,
    /// `Bool`.
    Bool,
    /// `Double`.
    Double,
}

impl SwiftType {
    /// Swift 타입 이름.
    pub(crate) fn name(self) -> &'static str {
        match self {
            SwiftType::String => "String",
            SwiftType::Bool => "Bool",
            SwiftType::Double => "Double",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TextExpr {
    Literal(String),
    State(StableId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Flag {
    Const(bool),
    State(StableId),
    Matches(StableId, StableId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Stmt {
    Emit(StableId),
    SetState(StableId, StableId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Op {
    Hidden,
    Contain,
    Combine,
    Role(AccessibilityRole),
    Label(TextExpr),
    OwnText(String),
    Value(TextExpr),
    Disabled(Flag),
    Selected(Flag),
    Checked(Flag),
    Pressed(Flag),
    Focused(Flag),
    Modal,
    SortPriority(u32),
    Action(ActionKind, Vec<Stmt>),
    Children(Vec<StableId>),
}

/// 생성한 접근성 코드를 view에 붙이는 데 필요한 정보(node 하나).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AccessibilityHook {
    /// 매개변수로 받는 상태 변수(StableId 오름차순).
    pub states: Vec<StableId>,
    /// `onAction`을 받는지.
    pub action: bool,
    /// `onStateChange`를 받는지(AppKit).
    pub set_state: bool,
    /// `views` 표를 받는지(AppKit).
    pub children: bool,
}

/// 상태 변수 → (Swift 식별자, 타입). Swift로 표현할 수 없는 타입의 상태는 빠집니다. 접근성 코드와 view 코드가
/// 같은 이름을 씁니다.
pub(crate) type StateNames = BTreeMap<StableId, (String, SwiftType)>;

/// IR 상태 변수의 Swift 이름과 타입.
pub(crate) fn state_names(ir: &CodegenIr) -> StateNames {
    let typed: Vec<(StableId, &str, Option<SwiftType>)> = ir
        .states
        .iter()
        .map(|state| {
            let ty = match state.initial {
                TypedValue::String(_) => Some(SwiftType::String),
                TypedValue::Boolean(_) => Some(SwiftType::Bool),
                TypedValue::Number(_) => Some(SwiftType::Double),
                _ => None,
            };
            (state.id, state.name.as_str(), ty)
        })
        .collect();
    let names = identifiers(typed.iter().map(|(_, name, _)| *name), "state");
    typed
        .iter()
        .zip(names)
        .filter_map(|((id, _, ty), name)| ty.map(|ty| (*id, (name, ty))))
        .collect()
}

fn hook(ops: &[Op]) -> AccessibilityHook {
    let (states, action, set_state, children) = uses(ops);
    AccessibilityHook {
        states: states.into_iter().collect(),
        action,
        set_state,
        children,
    }
}

struct Ctx<'a> {
    ir: &'a CodegenIr,
    apple: AppleProfile,
    states: StateNames,
    notes: &'a mut Vec<AppleNote>,
}

fn toggle_available(apple: AppleProfile) -> bool {
    let major = apple.minimum_os.major();
    match apple.platform {
        ApplePlatform::Ios | ApplePlatform::IpadOs | ApplePlatform::TvOs => major >= 17,
        ApplePlatform::MacOs => major >= 14,
        ApplePlatform::WatchOs => major >= 10,
        ApplePlatform::VisionOs => major >= 1,
    }
}

impl Ctx<'_> {
    fn note(&mut self, node: StableId, kind: AppleNoteKind) {
        self.notes.push(AppleNote {
            subject: AppleNoteSubject::Node(node),
            kind,
        });
    }

    fn state_of_type(&self, id: StableId, expected: SwiftType) -> bool {
        self.states.get(&id).is_some_and(|(_, ty)| *ty == expected)
    }

    fn text(
        &mut self,
        node: StableId,
        binding: &AccessibilityBinding,
        facet: Facet,
    ) -> Option<TextExpr> {
        match self.ir.resolve_binding(binding) {
            ResolvedBinding::Constant(TypedValue::String(text)) => {
                Some(TextExpr::Literal(text.clone()))
            }
            ResolvedBinding::Property(property) => match &property.value {
                TypedValue::String(text) => Some(TextExpr::Literal(text.clone())),
                _ => {
                    self.note(node, AppleNoteKind::UnresolvedBinding(facet));
                    None
                }
            },
            ResolvedBinding::NodeText {
                text: Some(text), ..
            } => Some(TextExpr::Literal(text.to_string())),
            ResolvedBinding::State(state) if self.state_of_type(state.id, SwiftType::String) => {
                Some(TextExpr::State(state.id))
            }
            ResolvedBinding::ComponentProperty(_) => {
                self.note(node, AppleNoteKind::ComponentProperty(facet));
                None
            }
            _ => {
                self.note(node, AppleNoteKind::UnresolvedBinding(facet));
                None
            }
        }
    }

    fn flag(
        &mut self,
        node: StableId,
        binding: &AccessibilityBinding,
        facet: Facet,
    ) -> Option<Flag> {
        match self.ir.resolve_binding(binding) {
            ResolvedBinding::Constant(TypedValue::Boolean(value)) => Some(Flag::Const(*value)),
            ResolvedBinding::Property(property) => match property.value {
                TypedValue::Boolean(value) => Some(Flag::Const(value)),
                _ => {
                    self.note(node, AppleNoteKind::UnresolvedBinding(facet));
                    None
                }
            },
            ResolvedBinding::State(state) if self.state_of_type(state.id, SwiftType::Bool) => {
                Some(Flag::State(state.id))
            }
            ResolvedBinding::SelectionMatches { state, item }
                if self.state_of_type(state.id, SwiftType::String) =>
            {
                Some(Flag::Matches(state.id, item))
            }
            ResolvedBinding::ComponentProperty(_) => {
                self.note(node, AppleNoteKind::ComponentProperty(facet));
                None
            }
            _ => {
                self.note(node, AppleNoteKind::UnresolvedBinding(facet));
                None
            }
        }
    }

    /// entry 하나의 동작 목록(읽기 순서 동작 제외).
    fn entry_ops(&mut self, index: u32, entry: &IrAccessibilityEntry) -> Vec<Op> {
        let node = &self.ir.nodes[index as usize];
        let id = node.id;
        let semantics = &entry.semantics;
        let role = semantics.role;
        let mut ops = Vec::new();
        match role {
            AccessibilityRole::Group
            | AccessibilityRole::RadioGroup
            | AccessibilityRole::List
            | AccessibilityRole::Dialog => ops.push(Op::Contain),
            AccessibilityRole::Button | AccessibilityRole::Radio | AccessibilityRole::ListItem => {
                ops.push(Op::Combine)
            }
            _ => {}
        }
        ops.push(Op::Role(role));
        if let Some(label) = &semantics.label {
            let own_text = matches!(label, AccessibilityBinding::NodeText(target) if *target == id);
            if own_text && role == AccessibilityRole::StaticText {
                match &node.text {
                    Some(text) => ops.push(Op::OwnText(text.clone())),
                    None => self.note(id, AppleNoteKind::UnresolvedBinding(Facet::Label)),
                }
            } else if let Some(text) = self.text(id, label, Facet::Label) {
                ops.push(Op::Label(text));
            }
        }
        if let Some(value) = &semantics.value {
            if let Some(text) = self.text(id, value, Facet::Value) {
                ops.push(Op::Value(text));
            }
        }
        for (state, binding) in &semantics.states {
            let facet = Facet::State(*state);
            match state {
                StateKind::Focusable | StateKind::Editable => {}
                StateKind::Multiselectable => self.note(id, AppleNoteKind::NoEquivalent(facet)),
                _ => {
                    let Some(flag) = self.flag(id, binding, facet) else {
                        continue;
                    };
                    match state {
                        StateKind::Disabled => ops.push(Op::Disabled(flag)),
                        StateKind::Selected => ops.push(Op::Selected(flag)),
                        StateKind::Checked => ops.push(Op::Checked(flag)),
                        StateKind::Pressed => ops.push(Op::Pressed(flag)),
                        StateKind::Focused => ops.push(Op::Focused(flag)),
                        StateKind::Modal => {
                            if flag != Flag::Const(false) {
                                ops.push(Op::Modal);
                            }
                        }
                        StateKind::Focusable | StateKind::Editable | StateKind::Multiselectable => {
                        }
                    }
                }
            }
        }
        // 기본 동작(activate + select)과 dismiss.
        let mut default_body = Vec::new();
        for (action, target) in &semantics.actions {
            let facet = Facet::Action(*action);
            match (action, target) {
                (ActionKind::Activate, ActionTarget::SemanticAction(action_id)) => {
                    default_body.push(Stmt::Emit(*action_id))
                }
                (ActionKind::Select, ActionTarget::State(state)) => {
                    if self.state_of_type(*state, SwiftType::String) {
                        default_body.push(Stmt::SetState(*state, id));
                    } else {
                        self.note(id, AppleNoteKind::UnresolvedBinding(facet));
                    }
                }
                (ActionKind::Dismiss, ActionTarget::SemanticAction(action_id)) => {
                    ops.push(Op::Action(
                        ActionKind::Dismiss,
                        vec![Stmt::Emit(*action_id)],
                    ));
                }
                (ActionKind::Focus | ActionKind::SetValue, _) => {}
                _ => self.note(id, AppleNoteKind::NoEquivalent(facet)),
            }
        }
        if !default_body.is_empty() {
            ops.push(Op::Action(ActionKind::Activate, default_body));
        }
        ops
    }
}

/// node마다 동작 목록(IR node index 순).
fn plan(ctx: &mut Ctx<'_>) -> Vec<Vec<Op>> {
    let ir = ctx.ir;
    let has = ir.semantic_subtrees();
    let element_parents = ir.element_parents();
    let mut ops: Vec<Vec<Op>> = vec![Vec::new(); ir.nodes.len()];
    for (index, node) in ir.nodes.iter().enumerate() {
        match ir.semantics_of(index as u32) {
            Some(entry) => ops[index] = ctx.entry_ops(index as u32, entry),
            // Fragment는 view가 없어 modifier를 붙일 곳이 없습니다. 자식은 Fragment를 건너뛴 부모로 판정합니다.
            None if node.element == ElementKind::FRAGMENT => {}
            None => {
                let parent_has = element_parents[index].is_none_or(|p| has[p as usize]);
                if !has[index] && parent_has {
                    ops[index].push(Op::Hidden);
                }
            }
        }
    }
    // 읽기 순서.
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
    let child_lists = ir.accessibility_child_lists();
    for (entry_index, entry) in ir.accessibility.iter().enumerate() {
        let children: Vec<u32> = child_lists[entry_index]
            .iter()
            .map(|child| ir.accessibility[*child as usize].node)
            .collect();
        if children.is_empty() {
            continue;
        }
        let mut visual = children.clone();
        visual.sort_unstable();
        let reparented: Vec<u32> = children
            .iter()
            .copied()
            .filter(|child| !is_descendant(*child, entry.node))
            .collect();
        if visual == children && reparented.is_empty() {
            continue;
        }
        let parent = entry.node as usize;
        if !ops[parent]
            .iter()
            .any(|op| matches!(op, Op::Contain | Op::Combine))
        {
            ops[parent].insert(0, Op::Contain);
        }
        ops[parent].push(Op::Children(
            children.iter().map(|c| ir.nodes[*c as usize].id).collect(),
        ));
        let count = children.len() as u32;
        for (position, child) in children.iter().enumerate() {
            ops[*child as usize].push(Op::SortPriority(count - position as u32));
        }
        for child in reparented {
            let id = ir.nodes[child as usize].id;
            ctx.note(id, AppleNoteKind::Approximated(Facet::ReadingOrder));
        }
    }
    ops
}

fn flag_code(ctx: &Ctx<'_>, flag: &Flag) -> String {
    match flag {
        Flag::Const(value) => value.to_string(),
        Flag::State(state) => ctx.states[state].0.clone(),
        Flag::Matches(state, item) => format!(
            "{} == {}",
            ctx.states[state].0,
            string_literal(&item.to_string())
        ),
    }
}

fn text_code(ctx: &Ctx<'_>, text: &TextExpr, swiftui: bool) -> String {
    match (text, swiftui) {
        (TextExpr::Literal(value), true) => format!("Text(verbatim: {})", string_literal(value)),
        (TextExpr::Literal(value), false) => string_literal(value),
        (TextExpr::State(state), true) => format!("Text({})", ctx.states[state].0),
        (TextExpr::State(state), false) => ctx.states[state].0.clone(),
    }
}

/// 동작 목록이 읽는 상태 변수와 action 사용 여부.
fn uses(ops: &[Op]) -> (BTreeSet<StableId>, bool, bool, bool) {
    let mut states = BTreeSet::new();
    let mut action = false;
    let mut set_state = false;
    let mut children = false;
    let flag = |flag: &Flag, states: &mut BTreeSet<StableId>| match flag {
        Flag::State(s) | Flag::Matches(s, _) => {
            states.insert(*s);
        }
        Flag::Const(_) => {}
    };
    for op in ops {
        match op {
            Op::Label(TextExpr::State(s)) | Op::Value(TextExpr::State(s)) => {
                states.insert(*s);
            }
            Op::Disabled(f)
            | Op::Selected(f)
            | Op::Checked(f)
            | Op::Pressed(f)
            | Op::Focused(f) => flag(f, &mut states),
            Op::Action(_, body) => {
                for stmt in body {
                    match stmt {
                        Stmt::Emit(_) => action = true,
                        Stmt::SetState(s, _) => {
                            set_state = true;
                            states.insert(*s);
                        }
                    }
                }
            }
            Op::Children(_) => children = true,
            _ => {}
        }
    }
    (states, action, set_state, children)
}

fn swiftui_modifier(ctx: &mut Ctx<'_>, node: StableId, ops: &[Op], prefix: &str) -> String {
    let (states, action, _, _) = uses(ops);
    let mut lines: Vec<String> = Vec::new();
    for op in ops {
        match op {
            Op::Hidden => lines.push(".accessibilityHidden(true)".to_string()),
            Op::Contain => lines.push(".accessibilityElement(children: .contain)".to_string()),
            Op::Combine => lines.push(".accessibilityElement(children: .combine)".to_string()),
            Op::Role(role) => match role {
                AccessibilityRole::Button | AccessibilityRole::Radio => {
                    lines.push(".accessibilityAddTraits(.isButton)".to_string())
                }
                AccessibilityRole::StaticText => {
                    lines.push(".accessibilityAddTraits(.isStaticText)".to_string())
                }
                AccessibilityRole::Image => {
                    lines.push(".accessibilityAddTraits(.isImage)".to_string())
                }
                AccessibilityRole::TextField => {
                    ctx.note(node, AppleNoteKind::NativeControlRequired(Facet::Role))
                }
                AccessibilityRole::RadioGroup
                | AccessibilityRole::List
                | AccessibilityRole::Dialog => {
                    ctx.note(node, AppleNoteKind::Approximated(Facet::Role))
                }
                AccessibilityRole::Generic
                | AccessibilityRole::Group
                | AccessibilityRole::ListItem => {}
            },
            Op::Label(text) => lines.push(format!(
                ".accessibilityLabel({})",
                text_code(ctx, text, true)
            )),
            Op::OwnText(_) => {}
            Op::Value(text) => lines.push(format!(
                ".accessibilityValue({})",
                text_code(ctx, text, true)
            )),
            Op::Disabled(flag) => {
                if *flag != Flag::Const(false) {
                    lines.push(format!(".disabled({})", flag_code(ctx, flag)));
                }
            }
            Op::Selected(flag) | Op::Checked(flag) => match flag {
                Flag::Const(true) => lines.push(".accessibilityAddTraits(.isSelected)".to_string()),
                Flag::Const(false) => {}
                _ => lines.push(format!(
                    ".accessibilityAddTraits({} ? .isSelected : [])",
                    flag_code(ctx, flag)
                )),
            },
            Op::Pressed(flag) => {
                if toggle_available(ctx.apple) {
                    lines.push(".accessibilityAddTraits(.isToggle)".to_string());
                    match flag {
                        Flag::Const(true) => {
                            lines.push(".accessibilityAddTraits(.isSelected)".to_string())
                        }
                        Flag::Const(false) => {}
                        _ => lines.push(format!(
                            ".accessibilityAddTraits({} ? .isSelected : [])",
                            flag_code(ctx, flag)
                        )),
                    }
                } else {
                    ctx.note(
                        node,
                        AppleNoteKind::Unavailable(Facet::State(StateKind::Pressed)),
                    );
                }
            }
            Op::Focused(_) => ctx.note(
                node,
                AppleNoteKind::NoEquivalent(Facet::State(StateKind::Focused)),
            ),
            Op::Modal => lines.push(".accessibilityAddTraits(.isModal)".to_string()),
            Op::SortPriority(priority) => {
                lines.push(format!(".accessibilitySortPriority({priority})"))
            }
            Op::Action(kind, body) => {
                let head = match kind {
                    ActionKind::Dismiss => ".accessibilityAction(.escape) {".to_string(),
                    _ => ".accessibilityAction {".to_string(),
                };
                lines.push(head);
                for stmt in body {
                    lines.push(match stmt {
                        Stmt::Emit(action) => {
                            format!("    onAction({})", string_literal(&action.to_string()))
                        }
                        Stmt::SetState(state, value) => format!(
                            "    {} = {}",
                            ctx.states[state].0,
                            string_literal(&value.to_string())
                        ),
                    });
                }
                lines.push("}".to_string());
            }
            Op::Children(_) => {}
        }
    }
    if lines.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "struct {prefix}Accessibility{}: ViewModifier {{\n",
        id_hex(node)
    );
    for state in &states {
        let (name, ty) = &ctx.states[state];
        let _ = writeln!(out, "    @Binding var {name}: {}", ty.name());
    }
    if action {
        out.push_str("    let onAction: (String) -> Void\n");
    }
    if !states.is_empty() || action {
        out.push('\n');
    }
    out.push_str("    func body(content: Content) -> some View {\n        content\n");
    for line in lines {
        let _ = writeln!(out, "            {line}");
    }
    out.push_str("    }\n}\n");
    out
}

/// 접근성 하위 staticText의 텍스트(읽기 순서, 공백으로 연결). AppKit은 SwiftUI `.combine`처럼 자식 텍스트를 이름으로
/// 합치지 않으므로 이름 없는 button·radio·listItem에 씁니다.
fn descendant_text(ir: &CodegenIr, node: StableId) -> Option<String> {
    let index = ir.node_index(node)?;
    let entry_index = ir.nodes[index as usize].semantics? as usize;
    let depth = ir.accessibility[entry_index].depth;
    let texts: Vec<&str> = ir.accessibility[entry_index + 1..]
        .iter()
        .take_while(|entry| entry.depth > depth)
        .filter(|entry| entry.semantics.role == AccessibilityRole::StaticText)
        .filter_map(|entry| ir.nodes[entry.node as usize].text.as_deref())
        .collect();
    (!texts.is_empty()).then(|| texts.join(" "))
}

fn appkit_function(ctx: &mut Ctx<'_>, node: StableId, ops: &[Op]) -> String {
    let (states, action, set_state, children) = uses(ops);
    let mut lines: Vec<String> = Vec::new();
    let mut actions: Vec<String> = Vec::new();
    let combine = ops.iter().any(|op| matches!(op, Op::Combine));
    let labelled = ops.iter().any(|op| matches!(op, Op::Label(_)));
    let generic = ops
        .iter()
        .any(|op| matches!(op, Op::Role(AccessibilityRole::Generic)));
    for op in ops {
        match op {
            Op::Hidden => {
                lines.push("view.setAccessibilityElement(false)".to_string());
                lines.push("view.setAccessibilityChildren([])".to_string());
            }
            Op::Contain | Op::Combine => {}
            Op::Role(role) => {
                let role_name = match role {
                    AccessibilityRole::Generic => {
                        lines.push("view.setAccessibilityElement(false)".to_string());
                        continue;
                    }
                    AccessibilityRole::Group => "group",
                    AccessibilityRole::Button => "button",
                    AccessibilityRole::TextField => "textField",
                    AccessibilityRole::Dialog => {
                        ctx.note(node, AppleNoteKind::Approximated(Facet::Role));
                        "group"
                    }
                    AccessibilityRole::List => "list",
                    AccessibilityRole::ListItem => {
                        ctx.note(node, AppleNoteKind::Approximated(Facet::Role));
                        "group"
                    }
                    AccessibilityRole::RadioGroup => "radioGroup",
                    AccessibilityRole::Radio => "radioButton",
                    AccessibilityRole::StaticText => "staticText",
                    AccessibilityRole::Image => "image",
                };
                lines.push("view.setAccessibilityElement(true)".to_string());
                lines.push(format!("view.setAccessibilityRole(.{role_name})"));
                if *role == AccessibilityRole::Dialog {
                    lines.push("view.setAccessibilitySubrole(.dialog)".to_string());
                }
                if combine && !labelled {
                    if let Some(text) = descendant_text(ctx.ir, node) {
                        lines.push(format!(
                            "view.setAccessibilityLabel({})",
                            string_literal(&text)
                        ));
                    }
                }
            }
            Op::Label(text) => lines.push(format!(
                "view.setAccessibilityLabel({})",
                text_code(ctx, text, false)
            )),
            Op::OwnText(text) => lines.push(format!(
                "view.setAccessibilityValue({})",
                string_literal(text)
            )),
            Op::Value(text) => lines.push(format!(
                "view.setAccessibilityValue({})",
                text_code(ctx, text, false)
            )),
            Op::Disabled(flag) => lines.push(match flag {
                Flag::Const(value) => format!("view.setAccessibilityEnabled({})", !value),
                _ => format!("view.setAccessibilityEnabled(!({}))", flag_code(ctx, flag)),
            }),
            Op::Selected(flag) => lines.push(format!(
                "view.setAccessibilitySelected({})",
                flag_code(ctx, flag)
            )),
            Op::Checked(flag) => lines.push(format!(
                "view.setAccessibilityValue(({}) ? 1 : 0)",
                flag_code(ctx, flag)
            )),
            Op::Pressed(flag) => {
                ctx.note(
                    node,
                    AppleNoteKind::Approximated(Facet::State(StateKind::Pressed)),
                );
                lines.push(format!(
                    "view.setAccessibilityValue(({}) ? 1 : 0)",
                    flag_code(ctx, flag)
                ));
            }
            Op::Focused(flag) => lines.push(format!(
                "view.setAccessibilityFocused({})",
                flag_code(ctx, flag)
            )),
            Op::Modal => lines.push("view.setAccessibilityModal(true)".to_string()),
            Op::SortPriority(_) => {}
            Op::Action(kind, body) => {
                ctx.note(node, AppleNoteKind::Approximated(Facet::Action(*kind)));
                let mut action_code = format!(
                    "NSAccessibilityCustomAction(name: {}) {{\n",
                    string_literal(kind.name())
                );
                for stmt in body {
                    let _ = writeln!(
                        action_code,
                        "                {}",
                        match stmt {
                            Stmt::Emit(action) => {
                                format!("onAction({})", string_literal(&action.to_string()))
                            }
                            Stmt::SetState(state, value) => format!(
                                "onStateChange({}, {})",
                                string_literal(&state.to_string()),
                                string_literal(&value.to_string())
                            ),
                        }
                    );
                }
                action_code.push_str("                return true\n            }");
                actions.push(action_code);
            }
            Op::Children(ids) => {
                if generic {
                    // 요소가 아닌 view의 자식 순서는 보조 기술이 부모 요소 기준으로 다시 모을 수 있습니다.
                    ctx.note(node, AppleNoteKind::Approximated(Facet::ReadingOrder));
                }
                let list: Vec<String> = ids
                    .iter()
                    .map(|id| string_literal(&id.to_string()))
                    .collect();
                lines.push(format!(
                    "view.setAccessibilityChildren([{}].compactMap {{ views[$0] }})",
                    list.join(", ")
                ));
            }
        }
    }
    if !actions.is_empty() {
        let mut code = "view.setAccessibilityCustomActions([\n".to_string();
        for action_code in &actions {
            let _ = writeln!(code, "            {action_code},");
        }
        code.push_str("        ])");
        lines.push(code);
    }
    if lines.is_empty() {
        return String::new();
    }
    let mut params = vec!["to view: NSView".to_string()];
    for state in &states {
        let (name, ty) = &ctx.states[state];
        params.push(format!("{name}: {}", ty.name()));
    }
    if action {
        params.push("onAction: @escaping (String) -> Void".to_string());
    }
    if set_state {
        params.push("onStateChange: @escaping (String, String) -> Void".to_string());
    }
    if children {
        params.push("views: [String: NSView]".to_string());
    }
    let mut out = format!(
        "    static func apply{}({}) {{\n",
        id_hex(node),
        params.join(", ")
    );
    for line in lines {
        let _ = writeln!(out, "        {line}");
    }
    out.push_str("    }\n");
    out
}

/// 접근성 Swift 소스(SwiftUI modifier 목록 또는 AppKit 함수 enum)와 node별 hook(IR node index 순, 코드를 만들지
/// 않은 node는 `None`).
pub(crate) fn accessibility_source(
    ir: &CodegenIr,
    dialect: AppleDialect,
    apple: AppleProfile,
    prefix: &str,
    notes: &mut Vec<AppleNote>,
) -> (String, Vec<Option<AccessibilityHook>>) {
    let mut ctx = Ctx {
        ir,
        apple,
        states: state_names(ir),
        notes,
    };
    let ops = plan(&mut ctx);
    let mut hooks = vec![None; ir.nodes.len()];
    let mut out = String::new();
    match dialect {
        AppleDialect::SwiftUi => {
            for (index, node_ops) in ops.iter().enumerate() {
                let code = swiftui_modifier(&mut ctx, ir.nodes[index].id, node_ops, prefix);
                if !code.is_empty() {
                    hooks[index] = Some(hook(node_ops));
                    out.push('\n');
                    out.push_str(&code);
                }
            }
        }
        AppleDialect::AppKit => {
            let mut functions = Vec::new();
            for (index, node_ops) in ops.iter().enumerate() {
                let code = appkit_function(&mut ctx, ir.nodes[index].id, node_ops);
                if !code.is_empty() {
                    hooks[index] = Some(hook(node_ops));
                    functions.push(code);
                }
            }
            let _ = write!(out, "\n@MainActor\nenum {prefix}Accessibility {{\n");
            out.push_str(&functions.join("\n"));
            out.push_str("}\n");
        }
    }
    (out, hooks)
}

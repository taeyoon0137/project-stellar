//! 테스트 전용 fixture 도우미.

use std::fmt::Write as _;

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;

use crate::node::{AccessibilityBinding, ActionTarget};
use crate::tree::AccessibilityTree;

/// 마지막 바이트가 `n`인 결정론 StableId.
pub(crate) fn id(n: u8) -> StableId {
    let mut bytes = [0u8; 16];
    bytes[15] = n;
    StableId::from_bytes(bytes)
}

/// fixture id는 `#n`, 그 외는 전체 StableId.
pub(crate) fn short(id: StableId) -> String {
    let bytes = id.to_bytes();
    if bytes[..15].iter().all(|byte| *byte == 0) {
        format!("#{}", bytes[15])
    } else {
        id.to_string()
    }
}

fn binding(binding: &AccessibilityBinding) -> String {
    match binding {
        AccessibilityBinding::Constant(TypedValue::String(text)) => format!("{text:?}"),
        AccessibilityBinding::Constant(TypedValue::Boolean(value)) => value.to_string(),
        AccessibilityBinding::Constant(value) => format!("const:0x{:02X}", value.type_tag()),
        AccessibilityBinding::Property(path) => format!("path:{}", short(path.root())),
        AccessibilityBinding::ComponentProperty(id) => format!("prop:{}", short(*id)),
        AccessibilityBinding::State(id) => format!("state:{}", short(*id)),
        AccessibilityBinding::NodeText(id) => format!("text:{}", short(*id)),
        AccessibilityBinding::SelectionMatches { selection, item } => {
            format!("selection:{}=={}", short(*selection), short(*item))
        }
    }
}

fn target(target: &ActionTarget) -> String {
    match target {
        ActionTarget::None => String::new(),
        ActionTarget::SemanticAction(id) => format!("=action:{}", short(*id)),
        ActionTarget::State(id) => format!("=state:{}", short(*id)),
    }
}

/// 접근성 tree의 사람이 읽는 golden 텍스트(root부터 preorder, 들여쓰기 2칸).
pub(crate) fn outline(tree: &AccessibilityTree) -> String {
    let mut out = String::new();
    let mut stack: Vec<(StableId, usize)> = tree.roots().iter().rev().map(|r| (*r, 0)).collect();
    while let Some((source, depth)) = stack.pop() {
        let Some(node) = tree.node(source) else {
            continue;
        };
        let _ = write!(
            out,
            "{}{} {}",
            "  ".repeat(depth),
            node.role.name(),
            short(source)
        );
        if let Some(label) = &node.label {
            let _ = write!(out, " label={}", binding(label));
        }
        if let Some(value) = &node.value {
            let _ = write!(out, " value={}", binding(value));
        }
        if !node.states.is_empty() {
            let states: Vec<String> = node
                .states
                .iter()
                .map(|(state, value)| format!("{}={}", state.name(), binding(value)))
                .collect();
            let _ = write!(out, " state{{{}}}", states.join(","));
        }
        if !node.actions.is_empty() {
            let actions: Vec<String> = node
                .actions
                .iter()
                .map(|(action, value)| format!("{}{}", action.name(), target(value)))
                .collect();
            let _ = write!(out, " action{{{}}}", actions.join(","));
        }
        if let Some(order) = node.reading_order {
            let _ = write!(out, " order={order}");
        }
        out.push('\n');
        stack.extend(tree.children(source).iter().rev().map(|c| (*c, depth + 1)));
    }
    out
}

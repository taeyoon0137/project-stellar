//! 접근성 binding을 IR 안의 값·상태로 해석합니다 (XXII.1.1).
//!
//! adapter는 binding 종류마다 플랫폼 표현이 다릅니다. 정적 출력(HTML 등)은 초기값을, 상태를 가진 출력
//! (SwiftUI·Compose)은 상태 변수 참조를 씁니다. [`CodegenIr::resolve_binding`]은 그 둘을 모두 알려 주고,
//! [`CodegenIr::static_value`]는 초기값을 계산합니다.

use stellar_accessibility::node::AccessibilityBinding;
use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;

use super::{CodegenIr, IrProperty, IrState};

/// IR 안에서 해석한 binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedBinding<'a> {
    /// 상수.
    Constant(&'a TypedValue),
    /// IR 속성 값(token이 정했을 수 있음).
    Property(&'a IrProperty),
    /// IR node의 텍스트 내용(`text`가 없으면 내용을 모름).
    NodeText {
        /// IR node index.
        node: u32,
        /// 해석된 텍스트.
        text: Option<&'a str>,
    },
    /// 상태 변수 값.
    State(&'a IrState),
    /// 상태 변수 값이 item identity(item StableId 문자열)와 같은지.
    SelectionMatches {
        /// 선택 상태 변수.
        state: &'a IrState,
        /// item StableId.
        item: StableId,
    },
    /// 컴포넌트 Property(IR 밖에서 주입되는 값).
    ComponentProperty(StableId),
    /// IR에 없는 대상(제외된 node·값이 없는 속성·선언되지 않은 상태).
    Missing,
}

impl CodegenIr {
    /// binding을 IR 안의 대상으로 해석합니다.
    pub fn resolve_binding<'a>(&'a self, binding: &'a AccessibilityBinding) -> ResolvedBinding<'a> {
        match binding {
            AccessibilityBinding::Constant(value) => ResolvedBinding::Constant(value),
            AccessibilityBinding::Property(path) => self
                .property(path)
                .map_or(ResolvedBinding::Missing, ResolvedBinding::Property),
            AccessibilityBinding::NodeText(id) => match self.node_index(*id) {
                Some(node) => ResolvedBinding::NodeText {
                    node,
                    text: self.nodes[node as usize].text.as_deref(),
                },
                None => ResolvedBinding::Missing,
            },
            AccessibilityBinding::State(id) => self
                .state(*id)
                .map_or(ResolvedBinding::Missing, ResolvedBinding::State),
            AccessibilityBinding::SelectionMatches { selection, item } => {
                match self.state(*selection) {
                    Some(state) => ResolvedBinding::SelectionMatches { state, item: *item },
                    None => ResolvedBinding::Missing,
                }
            }
            AccessibilityBinding::ComponentProperty(id) => ResolvedBinding::ComponentProperty(*id),
        }
    }

    /// binding의 정적(초기) 값. 컴포넌트 Property·IR에 없는 대상·내용을 모르는 텍스트는 `None`입니다.
    ///
    /// 선택 일치는 상태 초기값이 item StableId 문자열과 같은지(List item identity NodeId 방식, IX.6.5)입니다.
    pub fn static_value(&self, binding: &AccessibilityBinding) -> Option<TypedValue> {
        match self.resolve_binding(binding) {
            ResolvedBinding::Constant(value) => Some(value.clone()),
            ResolvedBinding::Property(property) => Some(property.value.clone()),
            ResolvedBinding::NodeText { text, .. } => {
                text.map(|text| TypedValue::String(text.to_string()))
            }
            ResolvedBinding::State(state) => Some(state.initial.clone()),
            ResolvedBinding::SelectionMatches { state, item } => Some(TypedValue::Boolean(
                state.initial == TypedValue::String(item.to_string()),
            )),
            ResolvedBinding::ComponentProperty(_) | ResolvedBinding::Missing => None,
        }
    }

    /// 정적 Boolean 값(값이 Boolean이 아니거나 정적으로 모르면 `None`).
    pub fn static_flag(&self, binding: &AccessibilityBinding) -> Option<bool> {
        match self.static_value(binding)? {
            TypedValue::Boolean(value) => Some(value),
            _ => None,
        }
    }

    /// 정적 문자열 값(값이 String이 아니거나 정적으로 모르면 `None`).
    pub fn static_text(&self, binding: &AccessibilityBinding) -> Option<String> {
        match self.static_value(binding)? {
            TypedValue::String(value) => Some(value),
            _ => None,
        }
    }
}

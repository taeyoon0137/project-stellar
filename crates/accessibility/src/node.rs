//! 접근성 의미 node (IX.8.1).
//!
//! [`AccessibilityNode`]는 접근성 트리의 node 하나입니다(기술 아키텍처 §18: role·label·value·state·action·읽기
//! 순서). 시각 layer와 독립된 접근성 트리를 이루며, 각 node는 자신을 만든 시각 node 하나(**source StableId**)를
//! 가리킵니다.
//!
//! - **role**: [`AccessibilityRole`]. Target Adapter(Apple·Windows·Web·Android)가 플랫폼 역할로 변환합니다.
//! - **label binding / value binding**: [`AccessibilityBinding`]. 상수, 속성 경로(VIII.6.1 `ValueBinding`의
//!   target이 될 수 있는 [`PropertyPath`]), 컴포넌트 Property, 상태 변수, 다른 node의 텍스트 내용, 선택 상태
//!   일치를 가리킵니다. 값 계산은 binding evaluator가 맡고 이 모델은 출처만 기록합니다.
//! - **state**: [`StateKind`] → binding 희소 map. Boolean 값을 기대합니다.
//! - **action**: [`ActionKind`] → [`ActionTarget`] 희소 map. 실행 시 의미(semantic action) 또는 값을 쓰는 상태
//!   변수를 가리킵니다.
//! - **reading order**: 형제 사이의 명시 읽기 순서(`None`이면 트리 순서).
//!
//! canonical binary 형식(V.1 규칙: 다중 바이트 little-endian, `u32` LE 길이 프리픽스):
//!
//! ```text
//! node_bytes := version:u16 (= 1) ++ node
//! node       := source[16] ++ role:u8 ++ opt(label) ++ opt(value)
//!               ++ count:u32 ++ (state:u8 ++ binding)*      (state tag 오름차순, 중복 없음)
//!               ++ count:u32 ++ (action:u8 ++ target)*      (action tag 오름차순, 중복 없음)
//!               ++ opt_u32(reading_order)
//! opt(x)     := 0x00 | 0x01 ++ x
//! binding    := 0x01 ++ blob(TypedValue bytes)             상수
//!             | 0x02 ++ blob(PropertyPath canonical bytes) 속성 경로
//!             | 0x03 ++ id[16]                             컴포넌트 Property
//!             | 0x04 ++ id[16]                             상태 변수
//!             | 0x05 ++ id[16]                             node 텍스트 내용
//!             | 0x06 ++ selection[16] ++ item[16]          선택 상태 일치
//! target     := 0x00 | 0x01 ++ action[16] | 0x02 ++ state[16]
//! ```
//!
//! 디코더는 미지 tag·잘림·정렬 위반·잉여 bytes를 거부하고, 다시 인코딩한 bytes가 입력과 같을 때만 받아들입니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::property::{PropertyKey, PropertyPath};
use stellar_core_types::typed_value::TypedValue;

use crate::codec::{
    put_bytes, put_count, put_id, put_opt_u32, put_u16, put_u8, CodecError, Reader,
};

/// node binary 형식 버전.
pub const NODE_FORMAT_VERSION: u16 = 1;

/// 접근성 역할. tag 값은 저장 형식 계약이므로 바꾸거나 재사용하지 않습니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AccessibilityRole {
    /// 의미 없는 container(자식 묶음만 제공).
    Generic,
    /// 이름 있는 의미 그룹.
    Group,
    /// 버튼.
    Button,
    /// 편집 가능한 텍스트 필드.
    TextField,
    /// 대화 상자(모달).
    Dialog,
    /// 목록.
    List,
    /// 목록 항목.
    ListItem,
    /// 단일 선택 radio 그룹.
    RadioGroup,
    /// radio 항목.
    Radio,
    /// 정적 텍스트.
    StaticText,
    /// 이미지·그래픽.
    Image,
}

impl AccessibilityRole {
    /// 정의된 모든 역할(tag 오름차순).
    pub const ALL: [AccessibilityRole; 11] = [
        AccessibilityRole::Generic,
        AccessibilityRole::Group,
        AccessibilityRole::Button,
        AccessibilityRole::TextField,
        AccessibilityRole::Dialog,
        AccessibilityRole::List,
        AccessibilityRole::ListItem,
        AccessibilityRole::RadioGroup,
        AccessibilityRole::Radio,
        AccessibilityRole::StaticText,
        AccessibilityRole::Image,
    ];

    /// 안정 tag.
    pub const fn tag(self) -> u8 {
        match self {
            AccessibilityRole::Generic => 0x01,
            AccessibilityRole::Group => 0x02,
            AccessibilityRole::Button => 0x03,
            AccessibilityRole::TextField => 0x04,
            AccessibilityRole::Dialog => 0x05,
            AccessibilityRole::List => 0x06,
            AccessibilityRole::ListItem => 0x07,
            AccessibilityRole::RadioGroup => 0x08,
            AccessibilityRole::Radio => 0x09,
            AccessibilityRole::StaticText => 0x0A,
            AccessibilityRole::Image => 0x0B,
        }
    }

    /// tag에서 역할을 찾습니다.
    pub fn from_tag(tag: u8) -> Option<AccessibilityRole> {
        AccessibilityRole::ALL
            .into_iter()
            .find(|role| role.tag() == tag)
    }

    /// 안정 이름(진단·snapshot 텍스트용).
    pub const fn name(self) -> &'static str {
        match self {
            AccessibilityRole::Generic => "generic",
            AccessibilityRole::Group => "group",
            AccessibilityRole::Button => "button",
            AccessibilityRole::TextField => "textField",
            AccessibilityRole::Dialog => "dialog",
            AccessibilityRole::List => "list",
            AccessibilityRole::ListItem => "listItem",
            AccessibilityRole::RadioGroup => "radioGroup",
            AccessibilityRole::Radio => "radio",
            AccessibilityRole::StaticText => "staticText",
            AccessibilityRole::Image => "image",
        }
    }
}

impl fmt::Display for AccessibilityRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 접근성 상태 종류(Boolean). tag 값은 저장 형식 계약입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StateKind {
    /// keyboard focus를 받을 수 있음.
    Focusable,
    /// 현재 focus를 가짐.
    Focused,
    /// 비활성.
    Disabled,
    /// 눌림(toggle button).
    Pressed,
    /// 선택됨(목록 항목).
    Selected,
    /// 체크됨(radio).
    Checked,
    /// 값 편집 가능.
    Editable,
    /// 모달(배경 상호작용 차단).
    Modal,
    /// 여러 항목 선택 가능.
    Multiselectable,
}

impl StateKind {
    /// 정의된 모든 상태(tag 오름차순).
    pub const ALL: [StateKind; 9] = [
        StateKind::Focusable,
        StateKind::Focused,
        StateKind::Disabled,
        StateKind::Pressed,
        StateKind::Selected,
        StateKind::Checked,
        StateKind::Editable,
        StateKind::Modal,
        StateKind::Multiselectable,
    ];

    /// 안정 tag.
    pub const fn tag(self) -> u8 {
        match self {
            StateKind::Focusable => 0x01,
            StateKind::Focused => 0x02,
            StateKind::Disabled => 0x03,
            StateKind::Pressed => 0x04,
            StateKind::Selected => 0x05,
            StateKind::Checked => 0x06,
            StateKind::Editable => 0x07,
            StateKind::Modal => 0x08,
            StateKind::Multiselectable => 0x09,
        }
    }

    /// tag에서 상태를 찾습니다.
    pub fn from_tag(tag: u8) -> Option<StateKind> {
        StateKind::ALL.into_iter().find(|state| state.tag() == tag)
    }

    /// 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            StateKind::Focusable => "focusable",
            StateKind::Focused => "focused",
            StateKind::Disabled => "disabled",
            StateKind::Pressed => "pressed",
            StateKind::Selected => "selected",
            StateKind::Checked => "checked",
            StateKind::Editable => "editable",
            StateKind::Modal => "modal",
            StateKind::Multiselectable => "multiselectable",
        }
    }
}

impl fmt::Display for StateKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 보조 기술이 실행할 수 있는 동작 종류. tag 값은 저장 형식 계약입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ActionKind {
    /// 활성화(누르기).
    Activate,
    /// focus 이동.
    Focus,
    /// 값 설정(텍스트 편집).
    SetValue,
    /// 항목 선택.
    Select,
    /// 닫기.
    Dismiss,
}

impl ActionKind {
    /// 정의된 모든 동작(tag 오름차순).
    pub const ALL: [ActionKind; 5] = [
        ActionKind::Activate,
        ActionKind::Focus,
        ActionKind::SetValue,
        ActionKind::Select,
        ActionKind::Dismiss,
    ];

    /// 안정 tag.
    pub const fn tag(self) -> u8 {
        match self {
            ActionKind::Activate => 0x01,
            ActionKind::Focus => 0x02,
            ActionKind::SetValue => 0x03,
            ActionKind::Select => 0x04,
            ActionKind::Dismiss => 0x05,
        }
    }

    /// tag에서 동작을 찾습니다.
    pub fn from_tag(tag: u8) -> Option<ActionKind> {
        ActionKind::ALL
            .into_iter()
            .find(|action| action.tag() == tag)
    }

    /// 안정 이름.
    pub const fn name(self) -> &'static str {
        match self {
            ActionKind::Activate => "activate",
            ActionKind::Focus => "focus",
            ActionKind::SetValue => "setValue",
            ActionKind::Select => "select",
            ActionKind::Dismiss => "dismiss",
        }
    }
}

impl fmt::Display for ActionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// 동작 실행 대상.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ActionTarget {
    /// 별도 대상 없음(플랫폼 기본 동작, 예: focus 이동).
    None,
    /// semantic action(IX.5.3) StableId.
    SemanticAction(StableId),
    /// 값을 쓰는 상태 변수(IX.5.1) StableId.
    State(StableId),
}

const TARGET_NONE: u8 = 0x00;
const TARGET_SEMANTIC_ACTION: u8 = 0x01;
const TARGET_STATE: u8 = 0x02;

impl ActionTarget {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        match self {
            ActionTarget::None => put_u8(buf, TARGET_NONE),
            ActionTarget::SemanticAction(id) => {
                put_u8(buf, TARGET_SEMANTIC_ACTION);
                put_id(buf, *id);
            }
            ActionTarget::State(id) => {
                put_u8(buf, TARGET_STATE);
                put_id(buf, *id);
            }
        }
    }

    fn decode_from(reader: &mut Reader<'_>) -> Result<ActionTarget, CodecError> {
        Ok(match reader.u8()? {
            TARGET_NONE => ActionTarget::None,
            TARGET_SEMANTIC_ACTION => ActionTarget::SemanticAction(reader.id()?),
            TARGET_STATE => ActionTarget::State(reader.id()?),
            _ => return Err(CodecError::new("알 수 없는 action target tag")),
        })
    }
}

impl fmt::Display for ActionTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionTarget::None => f.write_str("none"),
            ActionTarget::SemanticAction(id) => write!(f, "action:{id}"),
            ActionTarget::State(id) => write!(f, "state:{id}"),
        }
    }
}

/// label·value·state 값의 출처.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AccessibilityBinding {
    /// 상수 값.
    Constant(TypedValue),
    /// 속성 경로의 현재 값(그 속성이 ValueBinding(VIII.6.1)으로 계산될 수 있음).
    Property(PropertyPath),
    /// 컴포넌트 Property(IX.4) StableId.
    ComponentProperty(StableId),
    /// 상태 변수(IX.5.1) StableId.
    State(StableId),
    /// source node의 텍스트 내용(이름을 내용에서 계산).
    NodeText(StableId),
    /// `selection` 상태 변수 값이 `item`의 identity와 같은지(선택·체크 상태).
    SelectionMatches {
        /// 선택된 item identity를 담는 상태 변수.
        selection: StableId,
        /// item source node.
        item: StableId,
    },
}

const BINDING_CONSTANT: u8 = 0x01;
const BINDING_PROPERTY: u8 = 0x02;
const BINDING_COMPONENT_PROPERTY: u8 = 0x03;
const BINDING_STATE: u8 = 0x04;
const BINDING_NODE_TEXT: u8 = 0x05;
const BINDING_SELECTION_MATCHES: u8 = 0x06;

impl AccessibilityBinding {
    /// 상수 문자열 binding.
    pub fn text(value: impl Into<String>) -> AccessibilityBinding {
        AccessibilityBinding::Constant(TypedValue::String(value.into()))
    }

    /// 상수 Boolean binding.
    pub fn flag(value: bool) -> AccessibilityBinding {
        AccessibilityBinding::Constant(TypedValue::Boolean(value))
    }

    pub(crate) fn encode_into(&self, buf: &mut Vec<u8>) {
        match self {
            AccessibilityBinding::Constant(value) => {
                put_u8(buf, BINDING_CONSTANT);
                put_bytes(buf, &value.to_bytes());
            }
            AccessibilityBinding::Property(path) => {
                put_u8(buf, BINDING_PROPERTY);
                put_bytes(buf, &path.to_canonical_bytes());
            }
            AccessibilityBinding::ComponentProperty(id) => {
                put_u8(buf, BINDING_COMPONENT_PROPERTY);
                put_id(buf, *id);
            }
            AccessibilityBinding::State(id) => {
                put_u8(buf, BINDING_STATE);
                put_id(buf, *id);
            }
            AccessibilityBinding::NodeText(id) => {
                put_u8(buf, BINDING_NODE_TEXT);
                put_id(buf, *id);
            }
            AccessibilityBinding::SelectionMatches { selection, item } => {
                put_u8(buf, BINDING_SELECTION_MATCHES);
                put_id(buf, *selection);
                put_id(buf, *item);
            }
        }
    }

    pub(crate) fn decode_from(reader: &mut Reader<'_>) -> Result<AccessibilityBinding, CodecError> {
        Ok(match reader.u8()? {
            BINDING_CONSTANT => AccessibilityBinding::Constant(
                TypedValue::from_bytes(reader.bytes()?)
                    .map_err(|_| CodecError::new("상수 TypedValue 형식 위반"))?,
            ),
            BINDING_PROPERTY => {
                AccessibilityBinding::Property(decode_property_path(reader.bytes()?)?)
            }
            BINDING_COMPONENT_PROPERTY => AccessibilityBinding::ComponentProperty(reader.id()?),
            BINDING_STATE => AccessibilityBinding::State(reader.id()?),
            BINDING_NODE_TEXT => AccessibilityBinding::NodeText(reader.id()?),
            BINDING_SELECTION_MATCHES => AccessibilityBinding::SelectionMatches {
                selection: reader.id()?,
                item: reader.id()?,
            },
            _ => return Err(CodecError::new("알 수 없는 binding tag")),
        })
    }
}

impl fmt::Display for AccessibilityBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccessibilityBinding::Constant(TypedValue::String(text)) => write!(f, "{text:?}"),
            AccessibilityBinding::Constant(TypedValue::Boolean(value)) => write!(f, "{value}"),
            AccessibilityBinding::Constant(value) => write!(f, "const:0x{:02X}", value.type_tag()),
            AccessibilityBinding::Property(path) => write!(f, "property:{}", path.root()),
            AccessibilityBinding::ComponentProperty(id) => write!(f, "componentProperty:{id}"),
            AccessibilityBinding::State(id) => write!(f, "state:{id}"),
            AccessibilityBinding::NodeText(id) => write!(f, "text:{id}"),
            AccessibilityBinding::SelectionMatches { selection, item } => {
                write!(f, "selection:{selection}=={item}")
            }
        }
    }
}

/// PropertyPath canonical bytes(III.2.2: root(16) ++ keys_len(u32 BE) ++ keys(u32 BE) ++ tail)를 읽습니다.
fn decode_property_path(bytes: &[u8]) -> Result<PropertyPath, CodecError> {
    const MALFORMED: CodecError = CodecError::new("PropertyPath 형식 위반");
    let mut pos = 0usize;
    let mut take = |n: usize| -> Result<&[u8], CodecError> {
        if bytes.len() - pos < n {
            return Err(MALFORMED);
        }
        let slice = &bytes[pos..pos + n];
        pos += n;
        Ok(slice)
    };
    let mut root = [0u8; 16];
    root.copy_from_slice(take(16)?);
    let be_u32 = |slice: &[u8]| u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]);
    let key_count = be_u32(take(4)?) as usize;
    if key_count.saturating_mul(4) > bytes.len() {
        return Err(MALFORMED);
    }
    let mut keys = Vec::with_capacity(key_count);
    for _ in 0..key_count {
        keys.push(PropertyKey(be_u32(take(4)?)));
    }
    let mut path = PropertyPath::with_keys(StableId::from_bytes(root), keys);
    match take(1)?[0] {
        0 => {}
        1 => path = path.with_index(be_u32(take(4)?)),
        2 => {
            let len = be_u32(take(4)?) as usize;
            let key = std::str::from_utf8(take(len)?).map_err(|_| MALFORMED)?;
            path = path.with_key_segment(key);
        }
        _ => return Err(MALFORMED),
    }
    if path.to_canonical_bytes() != bytes {
        return Err(MALFORMED);
    }
    Ok(path)
}

/// 접근성 트리의 node 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibilityNode {
    /// 이 node를 만든 시각 node StableId(정확히 하나).
    pub source: StableId,
    /// 역할.
    pub role: AccessibilityRole,
    /// 접근 가능한 이름(label) binding.
    pub label: Option<AccessibilityBinding>,
    /// 값 binding.
    pub value: Option<AccessibilityBinding>,
    /// 상태 → Boolean binding(희소, tag 오름차순).
    pub states: BTreeMap<StateKind, AccessibilityBinding>,
    /// 동작 → 실행 대상(희소, tag 오름차순).
    pub actions: BTreeMap<ActionKind, ActionTarget>,
    /// 형제 사이 명시 읽기 순서(`None`이면 트리 순서).
    pub reading_order: Option<u32>,
}

impl AccessibilityNode {
    /// label·value·state·action·읽기 순서가 없는 node.
    pub fn new(source: StableId, role: AccessibilityRole) -> AccessibilityNode {
        AccessibilityNode {
            source,
            role,
            label: None,
            value: None,
            states: BTreeMap::new(),
            actions: BTreeMap::new(),
            reading_order: None,
        }
    }

    /// label을 설정한 사본(빌더).
    pub fn with_label(mut self, label: AccessibilityBinding) -> AccessibilityNode {
        self.label = Some(label);
        self
    }

    /// value를 설정한 사본(빌더).
    pub fn with_value(mut self, value: AccessibilityBinding) -> AccessibilityNode {
        self.value = Some(value);
        self
    }

    /// 상태를 설정한 사본(빌더).
    pub fn with_state(
        mut self,
        state: StateKind,
        value: AccessibilityBinding,
    ) -> AccessibilityNode {
        self.states.insert(state, value);
        self
    }

    /// 동작을 설정한 사본(빌더).
    pub fn with_action(mut self, action: ActionKind, target: ActionTarget) -> AccessibilityNode {
        self.actions.insert(action, target);
        self
    }

    /// 읽기 순서를 설정한 사본(빌더).
    pub fn with_reading_order(mut self, order: u32) -> AccessibilityNode {
        self.reading_order = Some(order);
        self
    }

    /// 상태가 상수 `true`인지(binding된 상태는 정적으로 알 수 없으므로 `false`).
    pub fn is_state_constant_true(&self, state: StateKind) -> bool {
        matches!(
            self.states.get(&state),
            Some(AccessibilityBinding::Constant(TypedValue::Boolean(true)))
        )
    }

    /// version 프리픽스를 포함한 canonical bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        put_u16(&mut buf, NODE_FORMAT_VERSION);
        self.encode_into(&mut buf);
        buf
    }

    /// [`AccessibilityNode::to_bytes`]의 역.
    pub fn from_bytes(bytes: &[u8]) -> Result<AccessibilityNode, CodecError> {
        let mut reader = Reader::new(bytes);
        if reader.u16()? != NODE_FORMAT_VERSION {
            return Err(CodecError::new("지원하지 않는 node 형식 버전"));
        }
        let node = AccessibilityNode::decode_from(&mut reader)?;
        reader.expect_end()?;
        if node.to_bytes() != bytes {
            return Err(CodecError::new("canonical 형식이 아님"));
        }
        Ok(node)
    }

    pub(crate) fn encode_into(&self, buf: &mut Vec<u8>) {
        put_id(buf, self.source);
        self.encode_semantics(buf);
        put_opt_u32(buf, self.reading_order);
    }

    /// source·읽기 순서를 뺀 의미 본문(role ++ label ++ value ++ states ++ actions). snapshot(IX.8.6)과 공유합니다.
    pub(crate) fn encode_semantics(&self, buf: &mut Vec<u8>) {
        put_u8(buf, self.role.tag());
        encode_opt_binding(buf, self.label.as_ref());
        encode_opt_binding(buf, self.value.as_ref());
        put_count(buf, self.states.len());
        for (state, binding) in &self.states {
            put_u8(buf, state.tag());
            binding.encode_into(buf);
        }
        put_count(buf, self.actions.len());
        for (action, target) in &self.actions {
            put_u8(buf, action.tag());
            target.encode_into(buf);
        }
    }

    pub(crate) fn decode_from(reader: &mut Reader<'_>) -> Result<AccessibilityNode, CodecError> {
        let source = reader.id()?;
        let mut node = AccessibilityNode::decode_semantics(reader, source)?;
        node.reading_order = reader.opt_u32()?;
        Ok(node)
    }

    /// [`AccessibilityNode::encode_semantics`]의 역(읽기 순서는 `None`).
    pub(crate) fn decode_semantics(
        reader: &mut Reader<'_>,
        source: StableId,
    ) -> Result<AccessibilityNode, CodecError> {
        let role = AccessibilityRole::from_tag(reader.u8()?)
            .ok_or(CodecError::new("알 수 없는 role tag"))?;
        let label = decode_opt_binding(reader)?;
        let value = decode_opt_binding(reader)?;
        let mut states = BTreeMap::new();
        let state_count = reader.count(2)?;
        let mut previous: Option<u8> = None;
        for _ in 0..state_count {
            let tag = reader.u8()?;
            if previous.is_some_and(|p| p >= tag) {
                return Err(CodecError::new("state tag가 오름차순이 아님"));
            }
            previous = Some(tag);
            let state = StateKind::from_tag(tag).ok_or(CodecError::new("알 수 없는 state tag"))?;
            states.insert(state, AccessibilityBinding::decode_from(reader)?);
        }
        let mut actions = BTreeMap::new();
        let action_count = reader.count(2)?;
        let mut previous: Option<u8> = None;
        for _ in 0..action_count {
            let tag = reader.u8()?;
            if previous.is_some_and(|p| p >= tag) {
                return Err(CodecError::new("action tag가 오름차순이 아님"));
            }
            previous = Some(tag);
            let action =
                ActionKind::from_tag(tag).ok_or(CodecError::new("알 수 없는 action tag"))?;
            actions.insert(action, ActionTarget::decode_from(reader)?);
        }
        Ok(AccessibilityNode {
            source,
            role,
            label,
            value,
            states,
            actions,
            reading_order: None,
        })
    }
}

fn encode_opt_binding(buf: &mut Vec<u8>, binding: Option<&AccessibilityBinding>) {
    match binding {
        Some(binding) => {
            put_u8(buf, 1);
            binding.encode_into(buf);
        }
        None => put_u8(buf, 0),
    }
}

fn decode_opt_binding(reader: &mut Reader<'_>) -> Result<Option<AccessibilityBinding>, CodecError> {
    if reader.bool()? {
        Ok(Some(AccessibilityBinding::decode_from(reader)?))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::id;

    fn full_node() -> AccessibilityNode {
        AccessibilityNode::new(id(1), AccessibilityRole::Radio)
            .with_label(AccessibilityBinding::NodeText(id(2)))
            .with_value(AccessibilityBinding::Property(
                PropertyPath::with_keys(id(3), vec![PropertyKey(7)]).with_key_segment("ko"),
            ))
            .with_state(StateKind::Focusable, AccessibilityBinding::flag(true))
            .with_state(
                StateKind::Checked,
                AccessibilityBinding::SelectionMatches {
                    selection: id(4),
                    item: id(1),
                },
            )
            .with_state(
                StateKind::Disabled,
                AccessibilityBinding::ComponentProperty(id(5)),
            )
            .with_state(StateKind::Focused, AccessibilityBinding::State(id(6)))
            .with_action(ActionKind::Activate, ActionTarget::SemanticAction(id(7)))
            .with_action(ActionKind::Focus, ActionTarget::None)
            .with_action(ActionKind::Select, ActionTarget::State(id(4)))
            .with_reading_order(3)
    }

    #[test]
    fn node_round_trips_every_field() {
        let node = full_node();
        let bytes = node.to_bytes();
        assert_eq!(AccessibilityNode::from_bytes(&bytes), Ok(node.clone()));
        // 같은 node는 항상 같은 bytes(결정론).
        assert_eq!(full_node().to_bytes(), bytes);

        let minimal = AccessibilityNode::new(id(9), AccessibilityRole::Generic);
        assert_eq!(
            AccessibilityNode::from_bytes(&minimal.to_bytes()),
            Ok(minimal)
        );

        // 모든 role·state·action·binding 갈래가 round-trip합니다.
        for role in AccessibilityRole::ALL {
            let mut node = AccessibilityNode::new(id(1), role)
                .with_label(AccessibilityBinding::text("이름"))
                .with_value(AccessibilityBinding::Property(
                    PropertyPath::new(id(2)).with_index(4),
                ));
            for (index, state) in StateKind::ALL.into_iter().enumerate() {
                node = node.with_state(state, AccessibilityBinding::flag(index % 2 == 0));
            }
            for action in ActionKind::ALL {
                node = node.with_action(action, ActionTarget::None);
            }
            assert_eq!(AccessibilityNode::from_bytes(&node.to_bytes()), Ok(node));
            assert_eq!(AccessibilityRole::from_tag(role.tag()), Some(role));
        }
    }

    #[test]
    fn malformed_node_bytes_are_rejected() {
        let bytes = full_node().to_bytes();
        // 모든 잘린 prefix는 거부합니다.
        for len in 0..bytes.len() {
            assert!(
                AccessibilityNode::from_bytes(&bytes[..len]).is_err(),
                "len {len}"
            );
        }
        // 잉여 bytes.
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(
            AccessibilityNode::from_bytes(&trailing),
            Err(CodecError::new("trailing bytes"))
        );
        // 버전.
        let mut version = bytes.clone();
        version[0] = 2;
        assert_eq!(
            AccessibilityNode::from_bytes(&version),
            Err(CodecError::new("지원하지 않는 node 형식 버전"))
        );
        // 미지 role tag(version 2 + source 16 다음 바이트).
        let mut role = bytes.clone();
        role[18] = 0xEE;
        assert_eq!(
            AccessibilityNode::from_bytes(&role),
            Err(CodecError::new("알 수 없는 role tag"))
        );
    }

    #[test]
    fn unsorted_or_duplicate_state_tags_are_rejected() {
        let mut buf = Vec::new();
        put_u16(&mut buf, NODE_FORMAT_VERSION);
        put_id(&mut buf, id(1));
        put_u8(&mut buf, AccessibilityRole::Button.tag());
        put_u8(&mut buf, 0); // label 없음
        put_u8(&mut buf, 0); // value 없음
        put_count(&mut buf, 2);
        for state in [StateKind::Disabled, StateKind::Focusable] {
            put_u8(&mut buf, state.tag());
            AccessibilityBinding::flag(true).encode_into(&mut buf);
        }
        put_count(&mut buf, 0);
        put_opt_u32(&mut buf, None);
        assert_eq!(
            AccessibilityNode::from_bytes(&buf),
            Err(CodecError::new("state tag가 오름차순이 아님"))
        );
    }

    #[test]
    fn invalid_constant_and_property_path_payloads_are_rejected() {
        // Boolean payload 2는 TypedValue 형식 위반.
        let mut buf = Vec::new();
        put_u16(&mut buf, NODE_FORMAT_VERSION);
        put_id(&mut buf, id(1));
        put_u8(&mut buf, AccessibilityRole::Image.tag());
        put_u8(&mut buf, 1);
        put_u8(&mut buf, BINDING_CONSTANT);
        put_bytes(&mut buf, &[0x01, 0x02]);
        put_u8(&mut buf, 0);
        put_count(&mut buf, 0);
        put_count(&mut buf, 0);
        put_opt_u32(&mut buf, None);
        assert_eq!(
            AccessibilityNode::from_bytes(&buf),
            Err(CodecError::new("상수 TypedValue 형식 위반"))
        );

        // key 개수가 남은 bytes보다 큰 PropertyPath.
        let mut path = id(2).to_bytes().to_vec();
        path.extend_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(
            decode_property_path(&path),
            Err(CodecError::new("PropertyPath 형식 위반"))
        );
        let valid = PropertyPath::with_keys(id(2), vec![PropertyKey(1), PropertyKey(2)]);
        assert_eq!(decode_property_path(&valid.to_canonical_bytes()), Ok(valid));
    }

    #[test]
    fn constant_true_state_query() {
        let node = full_node();
        assert!(node.is_state_constant_true(StateKind::Focusable));
        // binding된 상태는 정적으로 참이 아닙니다.
        assert!(!node.is_state_constant_true(StateKind::Focused));
        assert!(!node.is_state_constant_true(StateKind::Modal));
    }
}

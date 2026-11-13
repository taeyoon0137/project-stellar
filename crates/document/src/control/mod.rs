//! Control contract (IX.6).
//!
//! Control은 node 종류가 아닙니다. Button·TextInput·Modal·List 같은 의미는 일반 **Frame에 부착하는 계약**으로
//! 표현합니다(기술 아키텍처 12.5: `Button = Frame + Action Semantic + Activate/Focus Contract`).
//!
//! - [`ControlContract`]는 [`SemanticContract`](무엇을 의미하는가: 종류와 의미 field)와
//!   [`InteractionContract`](어떻게 상호작용하는가: focus 가능 여부, 처리하는 이벤트)의 쌍입니다.
//! - 계약은 Frame payload와 **분리된** [`ControlContractTable`]에 frame StableId로 저장합니다. 계약을 떼면
//!   표에서 항목만 사라지고 Frame은 그대로 일반 Frame입니다.
//! - 계약 종류는 파일 스키마의 닫힌 enum이 아닙니다(12.5: 향후 추가·축소 가능). 종류는 numeric
//!   [`ContractKind`]이고 의미 field는 numeric [`ContractField`] → [`ContractValue`] 희소 map입니다. 모르는 종류·
//!   field도 그대로 보존(round-trip)하며, 종류별 의미 검사는 registry(IX.6.7)가 맡습니다.
//! - field 값은 상수, 컴포넌트 Property, 상태 변수(IX.5.1), semantic action(IX.5.3), node, 이벤트 종류를
//!   가리킵니다. 참조 대상 해석은 [`ContractScope`]가 제공합니다(보통 컴포넌트 정의).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::{TypedValue, ValueDecodeError};
use stellar_expressions::types::{ExpressionType, TypeSpec};

/// Button 계약 (IX.6.2).
pub mod button;

/// TextInput 계약 (IX.6.3).
pub mod text_input;

use crate::component::codec::{
    put_bool, put_bytes, put_count, put_id, put_u16, put_u8, Malformed, Reader,
};
use crate::component::definition::{ComponentEntry, ComponentPropertyId};
use crate::component::event::EventKind;
use crate::component::state_machine::{SemanticActionId, StateVariableId};
use crate::node_kind::NodeKind;

/// 계약 종류 numeric ID. 알려진 종류는 registry(IX.6.7)가 정의합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContractKind(pub u16);

/// 의미 field numeric ID(종류별 의미는 종류 모듈이 정의).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContractField(pub u16);

/// 의미 field 값.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractValue {
    /// 상수 값.
    Constant(TypedValue),
    /// 컴포넌트 Property(읽기·binding 대상).
    Property(ComponentPropertyId),
    /// 상태 변수(읽기·쓰기 가능한 runtime 값).
    State(StateVariableId),
    /// semantic action.
    Action(SemanticActionId),
    /// node.
    Node(StableId),
    /// 이벤트 종류.
    Event(EventKind),
}

/// [`ContractValue`]의 갈래(검사 메시지용).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ValueKind {
    /// 상수.
    Constant,
    /// Property.
    Property,
    /// 상태 변수.
    State,
    /// semantic action.
    Action,
    /// node.
    Node,
    /// 이벤트 종류.
    Event,
}

const VALUE_CONSTANT: u8 = 0x01;
const VALUE_PROPERTY: u8 = 0x02;
const VALUE_STATE: u8 = 0x03;
const VALUE_ACTION: u8 = 0x04;
const VALUE_NODE: u8 = 0x05;
const VALUE_EVENT: u8 = 0x06;

impl ContractValue {
    /// 값의 갈래.
    pub fn kind(&self) -> ValueKind {
        match self {
            ContractValue::Constant(_) => ValueKind::Constant,
            ContractValue::Property(_) => ValueKind::Property,
            ContractValue::State(_) => ValueKind::State,
            ContractValue::Action(_) => ValueKind::Action,
            ContractValue::Node(_) => ValueKind::Node,
            ContractValue::Event(_) => ValueKind::Event,
        }
    }

    /// 값 타입(상수·Property·상태 변수만). 참조 대상을 찾지 못하면 `None`.
    pub fn value_type(&self, scope: &dyn ContractScope) -> Option<TypeSpec> {
        match self {
            ContractValue::Constant(value) => Some(TypeSpec::required(ExpressionType::of(value))),
            ContractValue::Property(id) => scope.property_type(*id),
            ContractValue::State(id) => scope.state_type(*id),
            ContractValue::Action(_) | ContractValue::Node(_) | ContractValue::Event(_) => None,
        }
    }

    fn encode_into(&self, buf: &mut Vec<u8>) {
        match self {
            ContractValue::Constant(value) => {
                put_u8(buf, VALUE_CONSTANT);
                put_bytes(buf, &value.to_bytes());
            }
            ContractValue::Property(id) => {
                put_u8(buf, VALUE_PROPERTY);
                put_id(buf, id.stable_id());
            }
            ContractValue::State(id) => {
                put_u8(buf, VALUE_STATE);
                put_id(buf, id.stable_id());
            }
            ContractValue::Action(id) => {
                put_u8(buf, VALUE_ACTION);
                put_id(buf, id.stable_id());
            }
            ContractValue::Node(id) => {
                put_u8(buf, VALUE_NODE);
                put_id(buf, *id);
            }
            ContractValue::Event(kind) => {
                put_u8(buf, VALUE_EVENT);
                put_u8(buf, kind.tag());
            }
        }
    }

    fn decode_from(reader: &mut Reader<'_>) -> Result<ContractValue, ContractError> {
        Ok(match reader.u8()? {
            VALUE_CONSTANT => ContractValue::Constant(
                TypedValue::from_bytes(reader.bytes()?).map_err(ContractError::Value)?,
            ),
            VALUE_PROPERTY => ContractValue::Property(ComponentPropertyId::new(reader.id()?)),
            VALUE_STATE => ContractValue::State(StateVariableId::new(reader.id()?)),
            VALUE_ACTION => ContractValue::Action(SemanticActionId::new(reader.id()?)),
            VALUE_NODE => ContractValue::Node(reader.id()?),
            VALUE_EVENT => {
                ContractValue::Event(EventKind::from_tag(reader.u8()?).map_err(|_| {
                    ContractError::Malformed {
                        reason: "알 수 없는 이벤트 종류",
                    }
                })?)
            }
            _ => {
                return Err(ContractError::Malformed {
                    reason: "알 수 없는 contract 값 tag",
                })
            }
        })
    }
}

/// 의미 계약: 종류 + 의미 field 희소 map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticContract {
    kind: ContractKind,
    fields: BTreeMap<ContractField, ContractValue>,
}

impl SemanticContract {
    /// field 없는 의미 계약.
    pub fn new(kind: ContractKind) -> SemanticContract {
        SemanticContract {
            kind,
            fields: BTreeMap::new(),
        }
    }

    /// field를 설정한 사본(빌더).
    pub fn with(mut self, field: ContractField, value: ContractValue) -> SemanticContract {
        self.fields.insert(field, value);
        self
    }

    /// 종류.
    pub fn kind(&self) -> ContractKind {
        self.kind
    }

    /// field 값.
    pub fn field(&self, field: ContractField) -> Option<&ContractValue> {
        self.fields.get(&field)
    }

    /// field를 설정하고 이전 값을 반환합니다.
    pub fn set(&mut self, field: ContractField, value: ContractValue) -> Option<ContractValue> {
        self.fields.insert(field, value)
    }

    /// field를 지웁니다.
    pub fn remove(&mut self, field: ContractField) -> Option<ContractValue> {
        self.fields.remove(&field)
    }

    /// field(id 오름차순).
    pub fn fields(&self) -> impl Iterator<Item = (ContractField, &ContractValue)> + '_ {
        self.fields.iter().map(|(field, value)| (*field, value))
    }
}

/// 상호작용 계약: focus 가능 여부와 처리하는 이벤트 종류.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InteractionContract {
    /// keyboard focus를 받을 수 있는지.
    pub focusable: bool,
    /// 처리하는 이벤트 종류(오름차순).
    pub events: BTreeSet<EventKind>,
}

impl InteractionContract {
    /// focus 가능 여부와 이벤트 목록으로 만듭니다.
    pub fn new(
        focusable: bool,
        events: impl IntoIterator<Item = EventKind>,
    ) -> InteractionContract {
        InteractionContract {
            focusable,
            events: events.into_iter().collect(),
        }
    }

    /// 이벤트를 처리하는지.
    pub fn handles(&self, event: EventKind) -> bool {
        self.events.contains(&event)
    }
}

/// Frame에 부착하는 control 계약.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlContract {
    /// 의미 계약.
    pub semantic: SemanticContract,
    /// 상호작용 계약.
    pub interaction: InteractionContract,
}

impl ControlContract {
    /// 계약을 만듭니다.
    pub fn new(semantic: SemanticContract, interaction: InteractionContract) -> ControlContract {
        ControlContract {
            semantic,
            interaction,
        }
    }

    /// 종류.
    pub fn kind(&self) -> ContractKind {
        self.semantic.kind
    }
}

/// contract 표 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractError {
    /// Frame이 아닌 node에 부착하려 했습니다.
    NotAFrame {
        /// node.
        node: StableId,
        /// 실제 종류(없으면 node 없음).
        kind: Option<NodeKind>,
    },
    /// 이미 계약이 있습니다(한 Frame에 하나).
    AlreadyAttached {
        /// frame.
        frame: StableId,
    },
    /// 상수 값 bytes가 잘못됐습니다.
    Value(ValueDecodeError),
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for ContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContractError::NotAFrame { node, kind } => match kind {
                Some(kind) => write!(
                    f,
                    "control 계약은 Frame에만 부착합니다(node {node}는 {})",
                    kind.name()
                ),
                None => write!(f, "control 계약 대상 node {node}가 없습니다"),
            },
            ContractError::AlreadyAttached { frame } => {
                write!(f, "Frame {frame}에 이미 control 계약이 있습니다")
            }
            ContractError::Value(err) => write!(f, "contract 상수 값 오류: {err}"),
            ContractError::Malformed { reason } => {
                write!(f, "control contract binary 형식 위반: {reason}")
            }
        }
    }
}

impl std::error::Error for ContractError {}

impl From<Malformed> for ContractError {
    fn from(err: Malformed) -> ContractError {
        ContractError::Malformed { reason: err.0 }
    }
}

/// frame StableId → 계약. Frame payload와 분리된 저장소입니다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ControlContractTable {
    contracts: BTreeMap<StableId, ControlContract>,
}

impl ControlContractTable {
    /// 빈 표.
    pub fn new() -> ControlContractTable {
        ControlContractTable::default()
    }

    /// Frame에 계약을 부착합니다. `kind_of`는 대상 node 종류를 알려 줍니다(없으면 `None`).
    pub fn attach(
        &mut self,
        frame: StableId,
        kind_of: impl Fn(StableId) -> Option<NodeKind>,
        contract: ControlContract,
    ) -> Result<(), ContractError> {
        let kind = kind_of(frame);
        if kind != Some(NodeKind::Frame) {
            return Err(ContractError::NotAFrame { node: frame, kind });
        }
        if self.contracts.contains_key(&frame) {
            return Err(ContractError::AlreadyAttached { frame });
        }
        self.contracts.insert(frame, contract);
        Ok(())
    }

    /// 계약을 뗍니다. Frame은 일반 Frame으로 남습니다.
    pub fn detach(&mut self, frame: StableId) -> Option<ControlContract> {
        self.contracts.remove(&frame)
    }

    /// Frame의 계약.
    pub fn get(&self, frame: StableId) -> Option<&ControlContract> {
        self.contracts.get(&frame)
    }

    /// (frame, 계약) 목록(frame id 오름차순).
    pub fn iter(&self) -> impl Iterator<Item = (StableId, &ControlContract)> + '_ {
        self.contracts
            .iter()
            .map(|(frame, contract)| (*frame, contract))
    }

    /// 계약 수.
    pub fn len(&self) -> usize {
        self.contracts.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.contracts.is_empty()
    }

    /// canonical binary bytes(frame id 오름차순, field id 오름차순, 이벤트 tag 오름차순).
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        put_count(&mut buf, self.contracts.len());
        for (frame, contract) in &self.contracts {
            put_id(&mut buf, *frame);
            put_u16(&mut buf, contract.semantic.kind.0);
            put_count(&mut buf, contract.semantic.fields.len());
            for (field, value) in &contract.semantic.fields {
                put_u16(&mut buf, field.0);
                value.encode_into(&mut buf);
            }
            put_bool(&mut buf, contract.interaction.focusable);
            put_count(&mut buf, contract.interaction.events.len());
            for event in &contract.interaction.events {
                put_u8(&mut buf, event.tag());
            }
        }
        buf
    }

    /// binary bytes에서 디코딩합니다. 정렬 위반·중복·trailing bytes는 거부합니다. 모르는 종류·field는
    /// 보존합니다.
    pub fn decode(bytes: &[u8]) -> Result<ControlContractTable, ContractError> {
        let mut reader = Reader::new(bytes);
        let mut table = ControlContractTable::new();
        let count = reader.count()?;
        let mut previous_frame: Option<StableId> = None;
        for _ in 0..count {
            let frame = reader.id()?;
            if previous_frame.is_some_and(|p| p >= frame) {
                return Err(ContractError::Malformed {
                    reason: "frame id가 오름차순이 아님",
                });
            }
            previous_frame = Some(frame);
            let mut semantic = SemanticContract::new(ContractKind(reader.u16()?));
            let field_count = reader.count()?;
            let mut previous_field: Option<ContractField> = None;
            for _ in 0..field_count {
                let field = ContractField(reader.u16()?);
                if previous_field.is_some_and(|p| p >= field) {
                    return Err(ContractError::Malformed {
                        reason: "contract field가 오름차순이 아님",
                    });
                }
                previous_field = Some(field);
                semantic
                    .fields
                    .insert(field, ContractValue::decode_from(&mut reader)?);
            }
            let focusable = reader.bool()?;
            let event_count = reader.count()?;
            let mut events = BTreeSet::new();
            let mut previous_event: Option<EventKind> = None;
            for _ in 0..event_count {
                let event =
                    EventKind::from_tag(reader.u8()?).map_err(|_| ContractError::Malformed {
                        reason: "알 수 없는 이벤트 종류",
                    })?;
                if previous_event.is_some_and(|p| p >= event) {
                    return Err(ContractError::Malformed {
                        reason: "이벤트 목록이 오름차순이 아님",
                    });
                }
                previous_event = Some(event);
                events.insert(event);
            }
            table.contracts.insert(
                frame,
                ControlContract::new(semantic, InteractionContract { focusable, events }),
            );
        }
        reader.expect_end()?;
        Ok(table)
    }
}

/// 계약 field 참조 해석 환경.
pub trait ContractScope {
    /// Property 타입.
    fn property_type(&self, property: ComponentPropertyId) -> Option<TypeSpec>;
    /// 상태 변수 타입.
    fn state_type(&self, variable: StateVariableId) -> Option<TypeSpec>;
    /// semantic action이 선언됐는지.
    fn has_action(&self, action: SemanticActionId) -> bool;
    /// node 종류.
    fn node_kind(&self, node: StableId) -> Option<NodeKind>;
    /// `node`가 `ancestor` 자신이거나 그 자손인지.
    fn is_within(&self, node: StableId, ancestor: StableId) -> bool;
    /// `node`의 자식 목록(순서대로).
    fn children(&self, node: StableId) -> Vec<StableId>;
}

impl ContractScope for ComponentEntry {
    fn property_type(&self, property: ComponentPropertyId) -> Option<TypeSpec> {
        self.property(property).map(|p| p.value_type())
    }

    fn state_type(&self, variable: StateVariableId) -> Option<TypeSpec> {
        self.state_machine()
            .variable(variable)
            .map(|v| v.value_type())
    }

    fn has_action(&self, action: SemanticActionId) -> bool {
        self.state_machine().action(action).is_some()
    }

    fn node_kind(&self, node: StableId) -> Option<NodeKind> {
        self.tree().node(node).map(|n| n.kind)
    }

    fn is_within(&self, node: StableId, ancestor: StableId) -> bool {
        let tree = self.tree();
        let mut current = Some(node);
        let mut steps = 0usize;
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            steps += 1;
            if steps > tree.len() {
                return false;
            }
            current = tree.node(id).and_then(|n| n.parent);
        }
        false
    }

    fn children(&self, node: StableId) -> Vec<StableId> {
        self.tree().children(node).to_vec()
    }
}

/// 계약 검사 문제의 심각도.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// 저장·export 전에 고쳐야 하는 오류.
    Error,
    /// 작성 단계 진단(저장 가능).
    Diagnostic,
}

/// 계약 검사 문제.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractProblem {
    /// 필수 field가 없습니다.
    MissingRequired,
    /// field 값 갈래가 기대와 다릅니다(예: action 자리에 node).
    WrongValueKind {
        /// 허용 갈래.
        expected: Vec<ValueKind>,
        /// 실제 갈래.
        found: ValueKind,
    },
    /// field 값 타입이 기대와 다릅니다.
    TypeMismatch {
        /// 기대 값 타입(비-nullable 부분).
        expected: ExpressionType,
        /// 실제 타입(참조를 해석하지 못하면 `None`).
        found: Option<TypeSpec>,
    },
    /// 참조 대상(Property·상태 변수·action·node)이 없습니다.
    UnresolvedReference,
    /// 값을 써야 하는 field가 쓸 수 없는 값(상수·Property)을 가리킵니다.
    NotWritable,
    /// node field가 계약 Frame 밖을 가리킵니다.
    OutsideFrame,
    /// 처리해야 하는 이벤트를 상호작용 계약이 선언하지 않았습니다.
    MissingInteraction {
        /// 이벤트.
        event: EventKind,
    },
    /// focus를 받아야 하는 계약이 focus 불가능으로 선언됐습니다.
    NotFocusable,
    /// 종류가 모르는 field입니다.
    UnknownField,
    /// registry에 없는 계약 종류입니다.
    UnknownKind,
    /// 다른 계약을 가리켜야 하는 node가 그 계약을 갖지 않았거나 조건을 어깁니다.
    InvalidLinkedContract,
}

impl ContractProblem {
    /// 심각도. 누락·상호작용 문제·모르는 field/종류는 진단, 나머지는 오류입니다.
    pub fn severity(&self) -> Severity {
        match self {
            ContractProblem::MissingRequired
            | ContractProblem::MissingInteraction { .. }
            | ContractProblem::NotFocusable
            | ContractProblem::UnknownField
            | ContractProblem::UnknownKind => Severity::Diagnostic,
            ContractProblem::WrongValueKind { .. }
            | ContractProblem::TypeMismatch { .. }
            | ContractProblem::UnresolvedReference
            | ContractProblem::NotWritable
            | ContractProblem::OutsideFrame
            | ContractProblem::InvalidLinkedContract => Severity::Error,
        }
    }
}

/// 계약 검사 결과 한 건.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractIssue {
    /// 계약 Frame.
    pub frame: StableId,
    /// 관련 field(계약 전체 문제면 `None`).
    pub field: Option<ContractField>,
    /// 문제.
    pub problem: ContractProblem,
}

impl ContractIssue {
    /// 심각도.
    pub fn severity(&self) -> Severity {
        self.problem.severity()
    }
}

/// 종류 검사기가 쓰는 field 검사 도우미.
pub(crate) struct FieldChecker<'a> {
    pub(crate) frame: StableId,
    pub(crate) contract: &'a ControlContract,
    pub(crate) scope: &'a dyn ContractScope,
    pub(crate) issues: Vec<ContractIssue>,
}

impl<'a> FieldChecker<'a> {
    pub(crate) fn new(
        frame: StableId,
        contract: &'a ControlContract,
        scope: &'a dyn ContractScope,
    ) -> FieldChecker<'a> {
        FieldChecker {
            frame,
            contract,
            scope,
            issues: Vec::new(),
        }
    }

    pub(crate) fn push(&mut self, field: Option<ContractField>, problem: ContractProblem) {
        self.issues.push(ContractIssue {
            frame: self.frame,
            field,
            problem,
        });
    }

    /// field 값(없고 `required`면 MissingRequired).
    pub(crate) fn get(
        &mut self,
        field: ContractField,
        required: bool,
    ) -> Option<&'a ContractValue> {
        let value = self.contract.semantic.field(field);
        if value.is_none() && required {
            self.push(Some(field), ContractProblem::MissingRequired);
        }
        value
    }

    fn wrong_kind(&mut self, field: ContractField, expected: &[ValueKind], found: ValueKind) {
        self.push(
            Some(field),
            ContractProblem::WrongValueKind {
                expected: expected.to_vec(),
                found,
            },
        );
    }

    /// 값 field(상수·Property·상태): 비-nullable 부분 타입이 `expected`여야 합니다. `writable`이면 상태 변수만
    /// 허용합니다.
    pub(crate) fn value(
        &mut self,
        field: ContractField,
        required: bool,
        expected: ExpressionType,
        writable: bool,
    ) {
        let Some(value) = self.get(field, required) else {
            return;
        };
        let kind = value.kind();
        if !matches!(
            kind,
            ValueKind::Constant | ValueKind::Property | ValueKind::State
        ) {
            self.wrong_kind(
                field,
                &[ValueKind::Constant, ValueKind::Property, ValueKind::State],
                kind,
            );
            return;
        }
        let found = value.value_type(self.scope);
        match found {
            None => self.push(Some(field), ContractProblem::UnresolvedReference),
            Some(spec) if spec.value_type() != expected => self.push(
                Some(field),
                ContractProblem::TypeMismatch {
                    expected,
                    found: Some(spec),
                },
            ),
            Some(_) => {
                if writable && kind != ValueKind::State {
                    self.push(Some(field), ContractProblem::NotWritable);
                }
            }
        }
    }

    /// action field: 선언된 semantic action이어야 합니다.
    pub(crate) fn action(&mut self, field: ContractField, required: bool) {
        let Some(value) = self.get(field, required) else {
            return;
        };
        match value {
            ContractValue::Action(action) => {
                if !self.scope.has_action(*action) {
                    self.push(Some(field), ContractProblem::UnresolvedReference);
                }
            }
            other => self.wrong_kind(field, &[ValueKind::Action], other.kind()),
        }
    }

    /// 상호작용 계약이 `event`를 처리해야 합니다.
    pub(crate) fn requires_event(&mut self, event: EventKind) {
        if !self.contract.interaction.handles(event) {
            self.push(None, ContractProblem::MissingInteraction { event });
        }
    }

    /// 상호작용 계약이 focus 가능해야 합니다.
    pub(crate) fn requires_focus(&mut self) {
        if !self.contract.interaction.focusable {
            self.push(None, ContractProblem::NotFocusable);
        }
    }

    /// `known`에 없는 field를 UnknownField로 보고합니다.
    pub(crate) fn unknown_fields(&mut self, known: &[ContractField]) {
        let unknown: Vec<ContractField> = self
            .contract
            .semantic
            .fields()
            .map(|(field, _)| field)
            .filter(|field| !known.contains(field))
            .collect();
        for field in unknown {
            self.push(Some(field), ContractProblem::UnknownField);
        }
    }

    pub(crate) fn finish(self) -> Vec<ContractIssue> {
        self.issues
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! control contract 테스트 공용 fixture: Toggle 컴포넌트(상태 머신 포함) 정의 tree를 scope로 씁니다.

    use crate::component::definition::ComponentLibrary;
    use crate::component::state_machine::fixtures::*;

    /// Toggle(상태 머신 포함) library. 정의 tree: 300(Frame) ← 301(Frame knob), 302(Text).
    pub(crate) fn library() -> ComponentLibrary {
        let mut library = toggle_library();
        let machine = toggle_machine(&library);
        library.set_state_machine(toggle(), machine).unwrap();
        library
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::state_machine::fixtures::*;
    use crate::frame::FramePayload;

    fn sample() -> ControlContract {
        ControlContract::new(
            SemanticContract::new(ContractKind(0x7001))
                .with(ContractField(2), ContractValue::Node(stable(KNOB)))
                .with(
                    ContractField(1),
                    ContractValue::Constant(TypedValue::String("확인".into())),
                )
                .with(ContractField(3), ContractValue::State(var(IS_ON)))
                .with(ContractField(4), ContractValue::Event(EventKind::Close)),
            InteractionContract::new(true, [EventKind::Focus, EventKind::Activate]),
        )
    }

    #[test]
    fn contract_attaches_only_to_frames_and_removal_keeps_plain_frame() {
        // 검증 기준(IX.6.1): contract 제거 후 일반 Frame으로 유지.
        let library = fixtures::library();
        let entry = library.get(toggle()).unwrap();
        let kind_of = |id| entry.node_kind(id);
        let frame_payload = FramePayload::default();
        let frame_bytes_before = frame_payload.encode();

        let mut table = ControlContractTable::new();
        // Text node에는 부착할 수 없습니다(Control은 node 종류가 아님 — Frame 계약).
        assert_eq!(
            table.attach(stable(302), kind_of, sample()).unwrap_err(),
            ContractError::NotAFrame {
                node: stable(302),
                kind: Some(NodeKind::Text),
            }
        );
        assert!(matches!(
            table.attach(stable(9_999), kind_of, sample()),
            Err(ContractError::NotAFrame { kind: None, .. })
        ));
        table.attach(stable(KNOB), kind_of, sample()).unwrap();
        assert!(matches!(
            table.attach(stable(KNOB), kind_of, sample()),
            Err(ContractError::AlreadyAttached { .. })
        ));
        assert_eq!(
            table.get(stable(KNOB)).unwrap().kind(),
            ContractKind(0x7001)
        );
        // 떼면 표에서만 사라지고, Frame 종류·payload는 그대로입니다.
        let removed = table.detach(stable(KNOB)).unwrap();
        assert_eq!(removed, sample());
        assert!(table.is_empty());
        assert_eq!(entry.node_kind(stable(KNOB)), Some(NodeKind::Frame));
        assert_eq!(frame_payload.encode(), frame_bytes_before);
        assert_eq!(table.encode(), ControlContractTable::new().encode());
    }

    #[test]
    fn table_round_trips_unknown_kinds_and_rejects_malformed_bytes() {
        let library = fixtures::library();
        let entry = library.get(toggle()).unwrap();
        let mut table = ControlContractTable::new();
        table
            .attach(stable(KNOB), |id| entry.node_kind(id), sample())
            .unwrap();
        table
            .attach(
                stable(ROOT),
                |id| entry.node_kind(id),
                ControlContract::new(
                    SemanticContract::new(ContractKind(1)),
                    InteractionContract::default(),
                ),
            )
            .unwrap();
        let bytes = table.encode();
        let decoded = ControlContractTable::decode(&bytes).unwrap();
        assert_eq!(decoded, table, "모르는 종류(0x7001)·field도 보존");
        assert_eq!(decoded.encode(), bytes);
        for cut in [0, 3, 20, bytes.len() - 1] {
            assert!(
                ControlContractTable::decode(&bytes[..cut]).is_err(),
                "절단 {cut}"
            );
        }
        let mut extended = bytes.clone();
        extended.push(0);
        assert!(ControlContractTable::decode(&extended).is_err());
        // 모르는 값 tag.
        let mut bad = Vec::new();
        put_count(&mut bad, 1);
        put_id(&mut bad, stable(1));
        put_u16(&mut bad, 1);
        put_count(&mut bad, 1);
        put_u16(&mut bad, 1);
        put_u8(&mut bad, 0x7F);
        assert!(ControlContractTable::decode(&bad).is_err());
    }
}

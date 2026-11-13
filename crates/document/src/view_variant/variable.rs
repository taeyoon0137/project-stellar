//! View 변수 (IX.7.1).
//!
//! View 변수는 ViewDefinition(VI.1.2)이 소유하는 화면 상태 축입니다(예: `cartState: empty | populated`,
//! `device: compact | regular`). 변수 하나는 StableId, 이름, 타입, default 값, 허용 enum 값을 갖습니다.
//!
//! - 타입은 표현식 타입 체계(VIII.1.1)의 [`TypeSpec`]입니다. nullable 변수(`Optional<T>`)만 실제 null 값
//!   [`VariableValue::ExplicitNull`]을 가질 수 있습니다. Variant 조건의 "제한 없음"(`Any`, IX.7.2)과는 다른
//!   값입니다.
//! - enum 타입 변수는 **허용 enum 값 목록**(비어 있지 않음, 같은 enum 타입, 중복 없음)을 가져야 하고 default는 그
//!   목록 안이어야 합니다(invalid enum default 거부). enum이 아닌 변수는 허용 목록을 갖지 않습니다.
//! - [`ViewVariableSet`]은 한 View의 변수 모음이며 이름은 그 안에서 유일합니다.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::{EnumValue, TypedValue, ValueDecodeError};
use stellar_expressions::types::{ExpressionType, TypeSpec};

use crate::component::codec::{
    put_bytes, put_count, put_id, put_str, put_u32, put_u8, Malformed, Reader,
};
use crate::component::property::{validate_property_name, ComponentPropertyError};
use crate::component::typed_expr::put_type_spec;

/// View 변수 식별자.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ViewVariableId(StableId);

impl ViewVariableId {
    /// `StableId`로부터 만듭니다.
    pub const fn new(id: StableId) -> ViewVariableId {
        ViewVariableId(id)
    }

    /// 내부 `StableId`.
    pub const fn stable_id(&self) -> StableId {
        self.0
    }
}

/// View 변수 값. `ExplicitNull`은 nullable 변수의 실제 null 값입니다(조건의 `Any`와 다름).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VariableValue {
    /// 값.
    Value(TypedValue),
    /// 실제 null 값.
    ExplicitNull,
}

const VALUE_PRESENT: u8 = 0x01;
const VALUE_NULL: u8 = 0x02;

impl VariableValue {
    pub(crate) fn encode_into(&self, buf: &mut Vec<u8>) {
        match self {
            VariableValue::Value(value) => {
                put_u8(buf, VALUE_PRESENT);
                put_bytes(buf, &value.to_bytes());
            }
            VariableValue::ExplicitNull => put_u8(buf, VALUE_NULL),
        }
    }

    pub(crate) fn decode_from(reader: &mut Reader<'_>) -> Result<VariableValue, ViewVariableError> {
        match reader.u8()? {
            VALUE_PRESENT => Ok(VariableValue::Value(
                TypedValue::from_bytes(reader.bytes()?).map_err(ViewVariableError::Value)?,
            )),
            VALUE_NULL => Ok(VariableValue::ExplicitNull),
            _ => Err(ViewVariableError::Malformed {
                reason: "알 수 없는 변수 값 tag",
            }),
        }
    }
}

/// View 변수 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewVariableError {
    /// 이름이 잘못됐습니다.
    InvalidName {
        /// 원인.
        reason: &'static str,
    },
    /// 같은 이름의 변수가 있습니다.
    DuplicateName {
        /// 이름.
        name: String,
    },
    /// 같은 id의 변수가 있습니다.
    DuplicateId {
        /// 변수.
        variable: ViewVariableId,
    },
    /// 변수가 없습니다.
    NotFound {
        /// 변수.
        variable: ViewVariableId,
    },
    /// default 값이 타입과 맞지 않습니다(nullable이 아닌데 ExplicitNull 포함).
    DefaultTypeMismatch {
        /// 변수.
        variable: ViewVariableId,
        /// 선언 타입.
        expected: TypeSpec,
    },
    /// enum default가 허용 값 목록에 없습니다.
    DefaultNotAllowed {
        /// 변수.
        variable: ViewVariableId,
        /// default.
        default: EnumValue,
    },
    /// enum 변수의 허용 값 목록이 비었거나, 다른 enum 타입 값·중복을 포함합니다.
    InvalidAllowedValues {
        /// 변수.
        variable: ViewVariableId,
        /// 원인.
        reason: &'static str,
    },
    /// 값 bytes가 잘못됐습니다.
    Value(ValueDecodeError),
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for ViewVariableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ViewVariableError::InvalidName { reason } => {
                write!(f, "View 변수 이름이 잘못됐습니다: {reason}")
            }
            ViewVariableError::DuplicateName { name } => {
                write!(f, "같은 이름의 View 변수가 이미 있습니다: {name}")
            }
            ViewVariableError::DuplicateId { variable } => {
                write!(f, "View 변수 {}가 이미 있습니다", variable.stable_id())
            }
            ViewVariableError::NotFound { variable } => {
                write!(f, "View 변수 {}가 없습니다", variable.stable_id())
            }
            ViewVariableError::DefaultTypeMismatch { variable, expected } => write!(
                f,
                "View 변수 {} default가 타입 {expected}와 맞지 않습니다",
                variable.stable_id()
            ),
            ViewVariableError::DefaultNotAllowed { variable, default } => write!(
                f,
                "View 변수 {} default enum 값 {}가 허용 목록에 없습니다",
                variable.stable_id(),
                default.variant()
            ),
            ViewVariableError::InvalidAllowedValues { variable, reason } => write!(
                f,
                "View 변수 {} 허용 enum 값이 잘못됐습니다: {reason}",
                variable.stable_id()
            ),
            ViewVariableError::Value(err) => write!(f, "View 변수 값 오류: {err}"),
            ViewVariableError::Malformed { reason } => {
                write!(f, "View 변수 binary 형식 위반: {reason}")
            }
        }
    }
}

impl std::error::Error for ViewVariableError {}

impl From<Malformed> for ViewVariableError {
    fn from(err: Malformed) -> ViewVariableError {
        ViewVariableError::Malformed { reason: err.0 }
    }
}

/// View 변수 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewVariable {
    id: ViewVariableId,
    name: String,
    value_type: TypeSpec,
    default: VariableValue,
    allowed: BTreeSet<EnumValue>,
}

impl ViewVariable {
    /// 변수를 만듭니다. `allowed`는 enum 타입 변수의 허용 값입니다(그 외 타입은 비어 있어야 함).
    pub fn new(
        id: ViewVariableId,
        name: String,
        value_type: TypeSpec,
        default: VariableValue,
        allowed: impl IntoIterator<Item = EnumValue>,
    ) -> Result<ViewVariable, ViewVariableError> {
        validate_property_name(&name).map_err(|err| match err {
            ComponentPropertyError::InvalidName { reason } => {
                ViewVariableError::InvalidName { reason }
            }
            _ => ViewVariableError::InvalidName {
                reason: "이름 규칙 위반",
            },
        })?;
        let mut set = BTreeSet::new();
        for value in allowed {
            if !set.insert(value) {
                return Err(ViewVariableError::InvalidAllowedValues {
                    variable: id,
                    reason: "중복 값",
                });
            }
        }
        let variable = ViewVariable {
            id,
            name,
            value_type,
            default,
            allowed: set,
        };
        variable.check()?;
        Ok(variable)
    }

    fn check(&self) -> Result<(), ViewVariableError> {
        let id = self.id;
        match self.value_type.value_type() {
            ExpressionType::Enum(enum_type) => {
                if self.allowed.is_empty() {
                    return Err(ViewVariableError::InvalidAllowedValues {
                        variable: id,
                        reason: "enum 변수의 허용 값이 비었음",
                    });
                }
                if self
                    .allowed
                    .iter()
                    .any(|v| v.enum_type() != enum_type.raw())
                {
                    return Err(ViewVariableError::InvalidAllowedValues {
                        variable: id,
                        reason: "다른 enum 타입 값",
                    });
                }
            }
            _ => {
                if !self.allowed.is_empty() {
                    return Err(ViewVariableError::InvalidAllowedValues {
                        variable: id,
                        reason: "enum이 아닌 변수는 허용 enum 값을 가질 수 없음",
                    });
                }
            }
        }
        if !self.accepts(&self.default) {
            if let VariableValue::Value(TypedValue::Enum(e)) = &self.default {
                if self.value_type.value_type().matches(&TypedValue::Enum(*e)) {
                    return Err(ViewVariableError::DefaultNotAllowed {
                        variable: id,
                        default: *e,
                    });
                }
            }
            return Err(ViewVariableError::DefaultTypeMismatch {
                variable: id,
                expected: self.value_type,
            });
        }
        Ok(())
    }

    /// 값이 이 변수에 허용되는지(타입·nullable·허용 enum 값).
    pub fn accepts(&self, value: &VariableValue) -> bool {
        match value {
            VariableValue::ExplicitNull => self.value_type.is_optional(),
            VariableValue::Value(value) => {
                if !self.value_type.accepts(Some(value)) {
                    return false;
                }
                match value {
                    TypedValue::Enum(e) => self.allowed.contains(e),
                    _ => true,
                }
            }
        }
    }

    /// 식별자.
    pub fn id(&self) -> ViewVariableId {
        self.id
    }

    /// 이름(식별에 사용 금지).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 타입.
    pub fn value_type(&self) -> TypeSpec {
        self.value_type
    }

    /// default 값.
    pub fn default_value(&self) -> &VariableValue {
        &self.default
    }

    /// 허용 enum 값(오름차순).
    pub fn allowed_values(&self) -> impl Iterator<Item = EnumValue> + '_ {
        self.allowed.iter().copied()
    }

    pub(crate) fn encode_into(&self, buf: &mut Vec<u8>) {
        put_id(buf, self.id.stable_id());
        put_str(buf, &self.name);
        put_type_spec(buf, self.value_type);
        self.default.encode_into(buf);
        put_count(buf, self.allowed.len());
        for value in &self.allowed {
            put_u32(buf, value.enum_type());
            put_u32(buf, value.variant());
        }
    }

    pub(crate) fn decode_from(reader: &mut Reader<'_>) -> Result<ViewVariable, ViewVariableError> {
        let id = ViewVariableId::new(reader.id()?);
        let name = reader.string()?;
        let value_type = reader.type_spec()?;
        let default = VariableValue::decode_from(reader)?;
        let count = reader.count()?;
        let mut allowed = Vec::with_capacity(crate::component::codec::capacity_hint(count));
        for _ in 0..count {
            let value = EnumValue::new(reader.u32()?, reader.u32()?);
            if allowed
                .last()
                .is_some_and(|last: &EnumValue| *last >= value)
            {
                return Err(ViewVariableError::Malformed {
                    reason: "허용 enum 값이 오름차순이 아님",
                });
            }
            allowed.push(value);
        }
        ViewVariable::new(id, name, value_type, default, allowed)
    }
}

/// 한 View의 변수 모음(변수 id 오름차순).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewVariableSet {
    view: StableId,
    variables: BTreeMap<ViewVariableId, ViewVariable>,
}

impl ViewVariableSet {
    /// View의 빈 변수 모음.
    pub fn new(view: StableId) -> ViewVariableSet {
        ViewVariableSet {
            view,
            variables: BTreeMap::new(),
        }
    }

    /// 소유 View.
    pub fn view(&self) -> StableId {
        self.view
    }

    /// 변수를 추가합니다. id·이름 중복은 거부합니다.
    pub fn insert(&mut self, variable: ViewVariable) -> Result<(), ViewVariableError> {
        if self.variables.contains_key(&variable.id) {
            return Err(ViewVariableError::DuplicateId {
                variable: variable.id,
            });
        }
        if self.variables.values().any(|v| v.name == variable.name) {
            return Err(ViewVariableError::DuplicateName {
                name: variable.name.clone(),
            });
        }
        self.variables.insert(variable.id, variable);
        Ok(())
    }

    /// 변수를 교체합니다(같은 id). 이름 중복은 거부하고 이전 값을 돌려줍니다.
    pub fn replace(&mut self, variable: ViewVariable) -> Result<ViewVariable, ViewVariableError> {
        if !self.variables.contains_key(&variable.id) {
            return Err(ViewVariableError::NotFound {
                variable: variable.id,
            });
        }
        if self
            .variables
            .values()
            .any(|v| v.id != variable.id && v.name == variable.name)
        {
            return Err(ViewVariableError::DuplicateName {
                name: variable.name.clone(),
            });
        }
        self.variables
            .insert(variable.id, variable.clone())
            .ok_or(ViewVariableError::NotFound {
                variable: variable.id,
            })
    }

    /// 변수를 제거합니다.
    pub fn remove(&mut self, variable: ViewVariableId) -> Result<ViewVariable, ViewVariableError> {
        self.variables
            .remove(&variable)
            .ok_or(ViewVariableError::NotFound { variable })
    }

    /// 변수.
    pub fn get(&self, variable: ViewVariableId) -> Option<&ViewVariable> {
        self.variables.get(&variable)
    }

    /// 변수(id 오름차순).
    pub fn iter(&self) -> impl Iterator<Item = &ViewVariable> + '_ {
        self.variables.values()
    }

    /// 변수 수.
    pub fn len(&self) -> usize {
        self.variables.len()
    }

    /// 비어 있는지.
    pub fn is_empty(&self) -> bool {
        self.variables.is_empty()
    }

    /// canonical binary bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        put_id(&mut buf, self.view);
        put_count(&mut buf, self.variables.len());
        for variable in self.variables.values() {
            variable.encode_into(&mut buf);
        }
        buf
    }

    /// binary bytes에서 디코딩하고 생성 규칙으로 재검증합니다.
    pub fn decode(bytes: &[u8]) -> Result<ViewVariableSet, ViewVariableError> {
        let mut reader = Reader::new(bytes);
        let mut set = ViewVariableSet::new(reader.id()?);
        let count = reader.count()?;
        let mut previous: Option<ViewVariableId> = None;
        for _ in 0..count {
            let variable = ViewVariable::decode_from(&mut reader)?;
            if previous.is_some_and(|p| p >= variable.id) {
                return Err(ViewVariableError::Malformed {
                    reason: "View 변수가 id 오름차순이 아님",
                });
            }
            previous = Some(variable.id);
            set.insert(variable)?;
        }
        reader.expect_end()?;
        Ok(set)
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! View 변수 테스트 fixture: `device: compact | regular`, `payment: idle | processing | error | completed`
    //! (nullable), `dark: Boolean`.

    use super::*;
    use stellar_expressions::types::EnumTypeId;

    pub(crate) fn stable(ts: u64) -> StableId {
        StableId::new_v7(ts, 0x0abc, 0x0123_4567_89ab)
    }

    pub(crate) const DEVICE_ENUM: u32 = 0x0D01;
    pub(crate) const PAYMENT_ENUM: u32 = 0x0D02;
    pub(crate) const DEVICE: u64 = 7_001;
    pub(crate) const PAYMENT: u64 = 7_002;
    pub(crate) const DARK: u64 = 7_003;
    pub(crate) const VIEW: u64 = 7_000;

    pub(crate) fn vid(ts: u64) -> ViewVariableId {
        ViewVariableId::new(stable(ts))
    }

    pub(crate) fn device(variant: u32) -> VariableValue {
        VariableValue::Value(TypedValue::Enum(EnumValue::new(DEVICE_ENUM, variant)))
    }

    pub(crate) fn payment(variant: u32) -> VariableValue {
        VariableValue::Value(TypedValue::Enum(EnumValue::new(PAYMENT_ENUM, variant)))
    }

    /// compact(0)·regular(1).
    pub(crate) fn device_variable() -> ViewVariable {
        ViewVariable::new(
            vid(DEVICE),
            "device".into(),
            TypeSpec::required(ExpressionType::Enum(EnumTypeId::new(DEVICE_ENUM))),
            device(1),
            [
                EnumValue::new(DEVICE_ENUM, 0),
                EnumValue::new(DEVICE_ENUM, 1),
            ],
        )
        .unwrap()
    }

    /// idle(0)·processing(1)·error(2)·completed(3), nullable.
    pub(crate) fn payment_variable() -> ViewVariable {
        ViewVariable::new(
            vid(PAYMENT),
            "paymentState".into(),
            TypeSpec::optional(ExpressionType::Enum(EnumTypeId::new(PAYMENT_ENUM))),
            payment(0),
            (0..4).map(|v| EnumValue::new(PAYMENT_ENUM, v)),
        )
        .unwrap()
    }

    pub(crate) fn dark_variable() -> ViewVariable {
        ViewVariable::new(
            vid(DARK),
            "dark".into(),
            TypeSpec::required(ExpressionType::Boolean),
            VariableValue::Value(TypedValue::Boolean(false)),
            [],
        )
        .unwrap()
    }

    pub(crate) fn variable_set() -> ViewVariableSet {
        let mut set = ViewVariableSet::new(stable(VIEW));
        set.insert(device_variable()).unwrap();
        set.insert(payment_variable()).unwrap();
        set.insert(dark_variable()).unwrap();
        set
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use stellar_expressions::types::EnumTypeId;

    fn device_type() -> TypeSpec {
        TypeSpec::required(ExpressionType::Enum(EnumTypeId::new(DEVICE_ENUM)))
    }

    #[test]
    fn invalid_enum_default_is_rejected() {
        // 검증 기준(IX.7.1): invalid enum default 거부.
        let allowed = [
            EnumValue::new(DEVICE_ENUM, 0),
            EnumValue::new(DEVICE_ENUM, 1),
        ];
        // 허용 목록 밖 variant(2).
        assert_eq!(
            ViewVariable::new(
                vid(DEVICE),
                "device".into(),
                device_type(),
                device(2),
                allowed
            )
            .unwrap_err(),
            ViewVariableError::DefaultNotAllowed {
                variable: vid(DEVICE),
                default: EnumValue::new(DEVICE_ENUM, 2),
            }
        );
        // 다른 enum 타입 default.
        assert!(matches!(
            ViewVariable::new(
                vid(DEVICE),
                "device".into(),
                device_type(),
                payment(0),
                allowed
            ),
            Err(ViewVariableError::DefaultTypeMismatch { .. })
        ));
        // nullable이 아닌 변수의 ExplicitNull default.
        assert!(matches!(
            ViewVariable::new(
                vid(DEVICE),
                "device".into(),
                device_type(),
                VariableValue::ExplicitNull,
                allowed
            ),
            Err(ViewVariableError::DefaultTypeMismatch { .. })
        ));
        // 허용 목록이 비었거나, 다른 enum 타입 값·중복을 포함.
        for (values, reason) in [
            (vec![], "enum 변수의 허용 값이 비었음"),
            (
                vec![
                    EnumValue::new(DEVICE_ENUM, 1),
                    EnumValue::new(PAYMENT_ENUM, 0),
                ],
                "다른 enum 타입 값",
            ),
            (
                vec![
                    EnumValue::new(DEVICE_ENUM, 1),
                    EnumValue::new(DEVICE_ENUM, 1),
                ],
                "중복 값",
            ),
        ] {
            assert_eq!(
                ViewVariable::new(
                    vid(DEVICE),
                    "device".into(),
                    device_type(),
                    device(1),
                    values
                )
                .unwrap_err(),
                ViewVariableError::InvalidAllowedValues {
                    variable: vid(DEVICE),
                    reason,
                }
            );
        }
        // enum이 아닌 변수는 허용 enum 값을 가질 수 없습니다.
        assert!(matches!(
            ViewVariable::new(
                vid(DARK),
                "dark".into(),
                TypeSpec::required(ExpressionType::Boolean),
                VariableValue::Value(TypedValue::Boolean(true)),
                allowed
            ),
            Err(ViewVariableError::InvalidAllowedValues { .. })
        ));
        // 올바른 변수.
        let ok = device_variable();
        assert!(ok.accepts(&device(0)));
        assert!(!ok.accepts(&device(5)));
        assert!(!ok.accepts(&VariableValue::ExplicitNull));
        assert!(payment_variable().accepts(&VariableValue::ExplicitNull));
    }

    #[test]
    fn variable_set_round_trips_and_rejects_duplicates() {
        let set = variable_set();
        let bytes = set.encode();
        let decoded = ViewVariableSet::decode(&bytes).unwrap();
        assert_eq!(decoded, set);
        assert_eq!(decoded.encode(), bytes);
        for cut in [0, 16, 20, bytes.len() - 1] {
            assert!(
                ViewVariableSet::decode(&bytes[..cut]).is_err(),
                "절단 {cut}"
            );
        }
        let mut dup = variable_set();
        assert!(matches!(
            dup.insert(device_variable()),
            Err(ViewVariableError::DuplicateId { .. })
        ));
        let renamed = ViewVariable::new(
            vid(7_009),
            "device".into(),
            TypeSpec::required(ExpressionType::Boolean),
            VariableValue::Value(TypedValue::Boolean(true)),
            [],
        )
        .unwrap();
        assert!(matches!(
            dup.insert(renamed),
            Err(ViewVariableError::DuplicateName { .. })
        ));
        assert_eq!(dup.len(), 3);
    }
}

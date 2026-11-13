//! VariantCondition (IX.7.2).
//!
//! Variant 조건은 변수별로 [`ConditionTerm::Any`](제한 없음) 또는 [`ConditionTerm::Equals`]만 저장합니다
//! (기술 아키텍처 16장). 문서의 `device = null` 표기는 실제 null 값이 아니라 **제한 없음(wildcard)** 입니다. 실제
//! null 값을 조건으로 쓰려면 `Equals(ExplicitNull)`이며, 둘은 저장 tag가 다릅니다:
//!
//! | term | tag |
//! |---|---|
//! | `Any` | 0x00 |
//! | `Equals(Value(v))` | 0x01 + 값 bytes |
//! | `Equals(ExplicitNull)` | 0x02 |
//!
//! 조건에 언급하지 않은 변수는 `Any`입니다. 구체성(specificity)은 `Equals` term 수입니다(IX.7.4).

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::typed_value::TypedValue;

use super::variable::{VariableValue, ViewVariableError, ViewVariableId, ViewVariableSet};
use crate::component::codec::{put_bytes, put_count, put_id, put_u8, Malformed, Reader};

/// 변수 하나에 대한 조건.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ConditionTerm {
    /// 제한 없음(wildcard).
    Any,
    /// 값이 같아야 함(실제 null은 `Equals(ExplicitNull)`).
    Equals(VariableValue),
}

const TERM_ANY: u8 = 0x00;
const TERM_EQUALS_VALUE: u8 = 0x01;
const TERM_EQUALS_NULL: u8 = 0x02;

impl ConditionTerm {
    /// 저장 tag.
    pub const fn tag(&self) -> u8 {
        match self {
            ConditionTerm::Any => TERM_ANY,
            ConditionTerm::Equals(VariableValue::Value(_)) => TERM_EQUALS_VALUE,
            ConditionTerm::Equals(VariableValue::ExplicitNull) => TERM_EQUALS_NULL,
        }
    }

    /// 값이 이 term을 만족하는지.
    pub fn matches(&self, value: &VariableValue) -> bool {
        match self {
            ConditionTerm::Any => true,
            ConditionTerm::Equals(expected) => expected == value,
        }
    }
}

/// 조건 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionError {
    /// View에 없는 변수를 조건으로 씁니다(삭제된 변수 — orphan, IX.7.8).
    UnknownVariable {
        /// 변수.
        variable: ViewVariableId,
    },
    /// 조건 값이 변수에 허용되지 않습니다(타입·허용 enum·nullable).
    ValueNotAllowed {
        /// 변수.
        variable: ViewVariableId,
    },
    /// 변수 값 bytes 오류.
    Variable(ViewVariableError),
    /// 입력이 부족하거나 형식이 아닙니다.
    Malformed {
        /// 원인.
        reason: &'static str,
    },
}

impl fmt::Display for ConditionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConditionError::UnknownVariable { variable } => {
                write!(f, "조건 변수 {}가 View에 없습니다", variable.stable_id())
            }
            ConditionError::ValueNotAllowed { variable } => write!(
                f,
                "조건 값이 View 변수 {}에 허용되지 않습니다",
                variable.stable_id()
            ),
            ConditionError::Variable(err) => write!(f, "{err}"),
            ConditionError::Malformed { reason } => {
                write!(f, "VariantCondition binary 형식 위반: {reason}")
            }
        }
    }
}

impl std::error::Error for ConditionError {}

impl From<Malformed> for ConditionError {
    fn from(err: Malformed) -> ConditionError {
        ConditionError::Malformed { reason: err.0 }
    }
}

/// 변수 값 할당(preview·캔버스 배치의 현재 화면 상태). 없는 변수는 default입니다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Assignment {
    values: BTreeMap<ViewVariableId, VariableValue>,
}

impl Assignment {
    /// 모든 변수가 default인 할당.
    pub fn new() -> Assignment {
        Assignment::default()
    }

    /// 변수 값을 지정한 사본(빌더).
    pub fn with(mut self, variable: ViewVariableId, value: VariableValue) -> Assignment {
        self.values.insert(variable, value);
        self
    }

    /// 변수의 현재 값(지정이 없으면 `variables`의 default, 변수도 없으면 `None`).
    pub fn value<'a>(
        &'a self,
        variable: ViewVariableId,
        variables: &'a ViewVariableSet,
    ) -> Option<&'a VariableValue> {
        self.values
            .get(&variable)
            .or_else(|| variables.get(variable).map(|v| v.default_value()))
    }

    /// 지정 값이 모두 View 변수에 허용되는지.
    pub fn validate(&self, variables: &ViewVariableSet) -> Result<(), ConditionError> {
        for (variable, value) in &self.values {
            let record = variables
                .get(*variable)
                .ok_or(ConditionError::UnknownVariable {
                    variable: *variable,
                })?;
            if !record.accepts(value) {
                return Err(ConditionError::ValueNotAllowed {
                    variable: *variable,
                });
            }
        }
        Ok(())
    }
}

/// Variant 조건: 변수 → term(언급 안 한 변수는 `Any`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VariantCondition {
    terms: BTreeMap<ViewVariableId, ConditionTerm>,
}

impl VariantCondition {
    /// 모든 변수가 `Any`인 조건(base에 항상 적용).
    pub fn new() -> VariantCondition {
        VariantCondition::default()
    }

    /// term을 지정한 사본(빌더).
    pub fn with(mut self, variable: ViewVariableId, term: ConditionTerm) -> VariantCondition {
        self.terms.insert(variable, term);
        self
    }

    /// `variable = value` term을 지정한 사본(빌더).
    pub fn equals(self, variable: ViewVariableId, value: VariableValue) -> VariantCondition {
        self.with(variable, ConditionTerm::Equals(value))
    }

    /// 변수의 term(언급하지 않았으면 `Any`).
    pub fn term(&self, variable: ViewVariableId) -> &ConditionTerm {
        self.terms.get(&variable).unwrap_or(&ConditionTerm::Any)
    }

    /// 저장된 term(변수 id 오름차순).
    pub fn terms(&self) -> impl Iterator<Item = (ViewVariableId, &ConditionTerm)> + '_ {
        self.terms.iter().map(|(variable, term)| (*variable, term))
    }

    /// 조건이 참조하는 변수(Any 포함).
    pub fn variables(&self) -> impl Iterator<Item = ViewVariableId> + '_ {
        self.terms.keys().copied()
    }

    /// 구체성: `Equals` term 수.
    pub fn specificity(&self) -> usize {
        self.terms
            .values()
            .filter(|t| matches!(t, ConditionTerm::Equals(_)))
            .count()
    }

    /// 할당이 조건을 만족하는지. View에 없는 변수의 `Equals` term은 만족하지 않습니다(orphan 조건은 적용 안 함).
    pub fn matches(&self, assignment: &Assignment, variables: &ViewVariableSet) -> bool {
        self.terms.iter().all(|(variable, term)| match term {
            ConditionTerm::Any => true,
            ConditionTerm::Equals(_) => assignment
                .value(*variable, variables)
                .is_some_and(|value| term.matches(value)),
        })
    }

    /// 조건이 View 변수에 대해 유효한지(변수 존재, `Equals` 값 허용).
    pub fn validate(&self, variables: &ViewVariableSet) -> Result<(), ConditionError> {
        for (variable, term) in &self.terms {
            let record = variables
                .get(*variable)
                .ok_or(ConditionError::UnknownVariable {
                    variable: *variable,
                })?;
            if let ConditionTerm::Equals(value) = term {
                if !record.accepts(value) {
                    return Err(ConditionError::ValueNotAllowed {
                        variable: *variable,
                    });
                }
            }
        }
        Ok(())
    }

    pub(crate) fn encode_into(&self, buf: &mut Vec<u8>) {
        put_count(buf, self.terms.len());
        for (variable, term) in &self.terms {
            put_id(buf, variable.stable_id());
            put_u8(buf, term.tag());
            if let ConditionTerm::Equals(VariableValue::Value(value)) = term {
                put_bytes(buf, &value.to_bytes());
            }
        }
    }

    pub(crate) fn decode_from(reader: &mut Reader<'_>) -> Result<VariantCondition, ConditionError> {
        let count = reader.count()?;
        let mut condition = VariantCondition::new();
        let mut previous: Option<ViewVariableId> = None;
        for _ in 0..count {
            let variable = ViewVariableId::new(reader.id()?);
            if previous.is_some_and(|p| p >= variable) {
                return Err(ConditionError::Malformed {
                    reason: "조건 변수가 id 오름차순이 아님",
                });
            }
            previous = Some(variable);
            let term = match reader.u8()? {
                TERM_ANY => ConditionTerm::Any,
                TERM_EQUALS_VALUE => ConditionTerm::Equals(VariableValue::Value(
                    TypedValue::from_bytes(reader.bytes()?)
                        .map_err(|e| ConditionError::Variable(ViewVariableError::Value(e)))?,
                )),
                TERM_EQUALS_NULL => ConditionTerm::Equals(VariableValue::ExplicitNull),
                _ => {
                    return Err(ConditionError::Malformed {
                        reason: "알 수 없는 조건 term tag",
                    })
                }
            };
            condition.terms.insert(variable, term);
        }
        Ok(condition)
    }

    /// canonical binary bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode_into(&mut buf);
        buf
    }

    /// binary bytes에서 디코딩합니다(형식만 검사, 변수 검증은 [`VariantCondition::validate`]).
    pub fn decode(bytes: &[u8]) -> Result<VariantCondition, ConditionError> {
        let mut reader = Reader::new(bytes);
        let condition = VariantCondition::decode_from(&mut reader)?;
        reader.expect_end()?;
        Ok(condition)
    }
}

#[cfg(test)]
mod tests {
    use super::super::variable::fixtures::*;
    use super::*;

    #[test]
    fn explicit_null_and_any_are_distinct() {
        // 검증 기준(IX.7.2): ExplicitNull/Any fixture 구분.
        let set = variable_set();
        let any = VariantCondition::new().with(vid(PAYMENT), ConditionTerm::Any);
        let null = VariantCondition::new().equals(vid(PAYMENT), VariableValue::ExplicitNull);
        // 저장 tag가 다릅니다.
        assert_ne!(any.encode(), null.encode());
        assert_eq!(any.term(vid(PAYMENT)).tag(), 0x00);
        assert_eq!(null.term(vid(PAYMENT)).tag(), 0x02);
        // Any는 null·error 모두 만족, Equals(ExplicitNull)은 실제 null만.
        let at_null = Assignment::new().with(vid(PAYMENT), VariableValue::ExplicitNull);
        let at_error = Assignment::new().with(vid(PAYMENT), payment(2));
        assert!(any.matches(&at_null, &set) && any.matches(&at_error, &set));
        assert!(null.matches(&at_null, &set));
        assert!(!null.matches(&at_error, &set));
        // 구체성: Any는 0, Equals는 1.
        assert_eq!(any.specificity(), 0);
        assert_eq!(null.specificity(), 1);
        // nullable이 아닌 변수(device)에 Equals(ExplicitNull)은 허용되지 않습니다.
        assert_eq!(
            VariantCondition::new()
                .equals(vid(DEVICE), VariableValue::ExplicitNull)
                .validate(&set)
                .unwrap_err(),
            ConditionError::ValueNotAllowed {
                variable: vid(DEVICE)
            }
        );
        null.validate(&set).unwrap();
        any.validate(&set).unwrap();
        // 언급하지 않은 변수는 Any.
        assert_eq!(
            VariantCondition::new().term(vid(DEVICE)),
            &ConditionTerm::Any
        );
    }

    #[test]
    fn conditions_round_trip_and_validate_against_variables() {
        let set = variable_set();
        let condition = VariantCondition::new()
            .equals(vid(PAYMENT), payment(2))
            .equals(vid(DEVICE), device(0))
            .with(vid(DARK), ConditionTerm::Any);
        let bytes = condition.encode();
        assert_eq!(VariantCondition::decode(&bytes).unwrap(), condition);
        assert!(VariantCondition::decode(&bytes[..bytes.len() - 1]).is_err());
        condition.validate(&set).unwrap();
        // 없는 변수·허용 밖 enum 값.
        assert!(matches!(
            VariantCondition::new()
                .equals(vid(7_999), device(0))
                .validate(&set),
            Err(ConditionError::UnknownVariable { .. })
        ));
        assert!(matches!(
            VariantCondition::new()
                .equals(vid(DEVICE), device(7))
                .validate(&set),
            Err(ConditionError::ValueNotAllowed { .. })
        ));
        // 할당 default: device 기본 regular(1)이므로 compact(0) 조건은 불만족.
        assert!(!condition.matches(&Assignment::new(), &set));
        let assignment = Assignment::new()
            .with(vid(DEVICE), device(0))
            .with(vid(PAYMENT), payment(2));
        assert!(condition.matches(&assignment, &set));
        assignment.validate(&set).unwrap();
        assert!(Assignment::new()
            .with(vid(DEVICE), VariableValue::ExplicitNull)
            .validate(&set)
            .is_err());
    }
}

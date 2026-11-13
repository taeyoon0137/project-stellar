//! 표현식 평가 공통 타입과 값 의미 (VIII.4.1).
//!
//! bytecode evaluator([`crate::vm`])와 AST 기준 evaluator([`crate::ast_eval`])가 같은 오류 타입·참조 입력·
//! 값 의미 함수를 공유합니다. 두 evaluator가 이 모듈의 함수만으로 값을 계산하므로 결과가 같아야 합니다.
//!
//! 평가 값은 `Option<TypedValue>`입니다. `None`은 `Optional<T>`의 값 없음(null)입니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_core_types::typed_value::TypedValue;

use crate::ast::{ExprNodeId, ReferenceTarget};
use crate::types::TypeSpec;

/// 참조 대상의 현재 값 제공자.
///
/// 반환값: `None` = 모르는 대상, `Some(None)` = 값 없음(null), `Some(Some(v))` = 값.
pub trait ReferenceValues {
    /// 참조 대상의 현재 값.
    fn reference_value(&self, target: &ReferenceTarget) -> Option<Option<TypedValue>>;
}

impl ReferenceValues for BTreeMap<ReferenceTarget, Option<TypedValue>> {
    fn reference_value(&self, target: &ReferenceTarget) -> Option<Option<TypedValue>> {
        self.get(target).cloned()
    }
}

/// 평가 오류 종류.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EvalErrorKind {
    /// 참조 대상 값이 입력에 없습니다.
    MissingReference(ReferenceTarget),
    /// 참조 값이 type 검사 때의 타입과 다릅니다(값 없음이 허용되지 않는 경우 포함).
    ReferenceTypeMismatch {
        /// 참조 대상.
        target: ReferenceTarget,
        /// 기대 타입.
        expected: TypeSpec,
    },
    /// 실행 중 값의 타입이 연산 규칙과 맞지 않습니다(검증된 program에서는 발생하지 않는 방어 오류).
    OperandTypeMismatch,
    /// program 내부 불변식 위반(stack 부족·잘못된 index). 검증된 program에서는 발생하지 않습니다.
    MalformedProgram,
    /// AST 기준 evaluator에 type 오류가 있는 AST가 주어졌습니다.
    IllTyped,
    /// 이 단계에서 아직 평가 의미가 없는 node 종류입니다(연산·호출 의미는 VIII.4.2·VIII.4.4).
    UnsupportedNode,
}

impl fmt::Display for EvalErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalErrorKind::MissingReference(target) => write!(f, "참조 값이 없습니다: {target}"),
            EvalErrorKind::ReferenceTypeMismatch { target, expected } => {
                write!(f, "참조 {target}의 값이 {expected} 타입이 아닙니다")
            }
            EvalErrorKind::OperandTypeMismatch => f.write_str("연산 값의 타입이 규칙과 다릅니다"),
            EvalErrorKind::MalformedProgram => f.write_str("program 내부 불변식 위반"),
            EvalErrorKind::IllTyped => f.write_str("type 오류가 있는 AST는 평가할 수 없습니다"),
            EvalErrorKind::UnsupportedNode => f.write_str("아직 평가할 수 없는 node 종류입니다"),
        }
    }
}

/// 평가 오류 (VIII.4.1). 원인 AST node를 함께 기록합니다.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EvalError {
    /// 오류가 난 AST node(알 수 있으면).
    pub node: Option<ExprNodeId>,
    /// 오류 종류.
    pub kind: EvalErrorKind,
}

impl EvalError {
    /// 오류를 만듭니다.
    pub const fn new(node: Option<ExprNodeId>, kind: EvalErrorKind) -> EvalError {
        EvalError { node, kind }
    }
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.node {
            Some(node) => write!(f, "node {node}: {}", self.kind),
            None => write!(f, "{}", self.kind),
        }
    }
}

impl std::error::Error for EvalError {}

/// 참조 값을 읽고 type 검사 때의 타입과 맞는지 확인합니다.
pub fn load_reference(
    target: &ReferenceTarget,
    expected: TypeSpec,
    values: &dyn ReferenceValues,
) -> Result<Option<TypedValue>, EvalErrorKind> {
    let value = values
        .reference_value(target)
        .ok_or_else(|| EvalErrorKind::MissingReference(target.clone()))?;
    if expected.accepts(value.as_ref()) {
        Ok(value)
    } else {
        Err(EvalErrorKind::ReferenceTypeMismatch {
            target: target.clone(),
            expected,
        })
    }
}

/// `if` 선택 의미: 조건이 `true`면 참 branch 값.
pub fn select_if(
    condition: &Option<TypedValue>,
    then_value: Option<TypedValue>,
    else_value: Option<TypedValue>,
) -> Result<Option<TypedValue>, EvalErrorKind> {
    match condition {
        Some(TypedValue::Boolean(true)) => Ok(then_value),
        Some(TypedValue::Boolean(false)) => Ok(else_value),
        _ => Err(EvalErrorKind::OperandTypeMismatch),
    }
}

/// `match` arm 선택 의미: 대상과 같은 첫 pattern의 index. 없으면 `None`(기본 branch).
pub fn match_arm(scrutinee: &Option<TypedValue>, patterns: &[TypedValue]) -> Option<usize> {
    let scrutinee = scrutinee.as_ref()?;
    patterns.iter().position(|pattern| pattern == scrutinee)
}

#[cfg(test)]
mod tests {
    use stellar_core_types::ids::StableId;
    use stellar_core_types::reference::TokenRef;
    use stellar_core_types::value::FiniteF32;

    use super::*;
    use crate::types::ExpressionType;

    #[test]
    fn reference_loading_validates_type_and_null() {
        let token = ReferenceTarget::Token(TokenRef::new(StableId::from_bytes([1; 16])));
        let number = TypedValue::Number(FiniteF32::ONE);
        let mut values: BTreeMap<ReferenceTarget, Option<TypedValue>> = BTreeMap::new();
        let required = TypeSpec::required(ExpressionType::Number);
        let optional = TypeSpec::optional(ExpressionType::Number);
        assert_eq!(
            load_reference(&token, required, &values),
            Err(EvalErrorKind::MissingReference(token.clone()))
        );
        values.insert(token.clone(), Some(number.clone()));
        assert_eq!(load_reference(&token, required, &values), Ok(Some(number)));
        values.insert(token.clone(), None);
        assert_eq!(load_reference(&token, optional, &values), Ok(None));
        assert_eq!(
            load_reference(&token, required, &values),
            Err(EvalErrorKind::ReferenceTypeMismatch {
                target: token.clone(),
                expected: required
            })
        );
        values.insert(token.clone(), Some(TypedValue::Boolean(true)));
        assert!(load_reference(&token, optional, &values).is_err());
    }

    #[test]
    fn selection_semantics() {
        let a = Some(TypedValue::Boolean(true));
        let b = Some(TypedValue::Boolean(false));
        assert_eq!(select_if(&a, a.clone(), b.clone()), Ok(a.clone()));
        assert_eq!(select_if(&b, a.clone(), b.clone()), Ok(b.clone()));
        assert_eq!(
            select_if(&None, a.clone(), b.clone()),
            Err(EvalErrorKind::OperandTypeMismatch)
        );
        let patterns = [TypedValue::Boolean(false), TypedValue::Boolean(true)];
        assert_eq!(match_arm(&a, &patterns), Some(1));
        assert_eq!(match_arm(&None, &patterns), None);
    }
}

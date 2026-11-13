//! TextInput 계약 (IX.6.3).
//!
//! `TextInput = Frame + Editable Value + Text Input/IME/Focus Contract`(기술 아키텍처 12.5).
//!
//! | field | id | 값 | 필수 |
//! |---|---|---|---|
//! | value | 1 | 편집 가능한 String — 상태 변수만(입력이 값을 씀) | 예 |
//! | placeholder | 2 | String(상수·Property·상태) | 아니오 |
//! | focused | 3 | Boolean(상수·Property·상태) | 아니오 |
//! | imeInput | 4 | Boolean(상수·Property·상태) — IME 조합 입력 허용 | 아니오(없으면 허용) |
//!
//! 편집 값 binding의 타입이 String이 아니면 **오류**입니다. 상수·Property는 입력이 쓸 수 없으므로
//! `NotWritable` 오류입니다. 상호작용 계약은 focus 가능하고 `Input`을 처리해야 합니다(아니면 진단).

use stellar_core_types::ids::StableId;
use stellar_expressions::types::ExpressionType;

use super::{
    ContractField, ContractIssue, ContractKind, ContractProblem, ContractScope, ContractValue,
    ControlContract, FieldChecker, InteractionContract, SemanticContract,
};
use crate::component::event::EventKind;
use crate::component::state_machine::StateVariableId;

/// TextInput 계약 종류.
pub const TEXT_INPUT: ContractKind = ContractKind(0x0002);

/// TextInput field.
pub mod fields {
    use super::ContractField;

    /// 편집 값(String 상태 변수).
    pub const VALUE: ContractField = ContractField(1);
    /// placeholder(String).
    pub const PLACEHOLDER: ContractField = ContractField(2);
    /// focus 상태(Boolean).
    pub const FOCUSED: ContractField = ContractField(3);
    /// IME 조합 입력 허용(Boolean).
    pub const IME_INPUT: ContractField = ContractField(4);

    /// 알려진 field 전체.
    pub const ALL: [ContractField; 4] = [VALUE, PLACEHOLDER, FOCUSED, IME_INPUT];
}

/// 기본 TextInput 계약: 편집 값 상태 변수, focus 가능, Input·Focus·Blur·Key 처리.
pub fn text_input_contract(value: StateVariableId) -> ControlContract {
    ControlContract::new(
        SemanticContract::new(TEXT_INPUT).with(fields::VALUE, ContractValue::State(value)),
        InteractionContract::new(
            true,
            [
                EventKind::Input,
                EventKind::Focus,
                EventKind::Blur,
                EventKind::Key,
            ],
        ),
    )
}

/// TextInput 계약을 검사합니다.
pub fn validate_text_input(
    frame: StableId,
    contract: &ControlContract,
    scope: &dyn ContractScope,
) -> Vec<ContractIssue> {
    let mut checker = FieldChecker::new(frame, contract, scope);
    if contract.kind() != TEXT_INPUT {
        checker.push(None, ContractProblem::UnknownKind);
        return checker.finish();
    }
    checker.value(fields::VALUE, true, ExpressionType::String, true);
    checker.value(fields::PLACEHOLDER, false, ExpressionType::String, false);
    checker.value(fields::FOCUSED, false, ExpressionType::Boolean, false);
    checker.value(fields::IME_INPUT, false, ExpressionType::Boolean, false);
    checker.requires_focus();
    checker.requires_event(EventKind::Input);
    checker.unknown_fields(&fields::ALL);
    checker.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::state_machine::fixtures::*;
    use crate::control::fixtures::library;
    use crate::control::Severity;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_expressions::types::TypeSpec;

    fn check(contract: &ControlContract) -> Vec<ContractIssue> {
        let library = library();
        validate_text_input(stable(KNOB), contract, library.get(toggle()).unwrap())
    }

    #[test]
    fn string_state_binding_is_valid() {
        let mut contract = text_input_contract(var(TEXT));
        contract.semantic.set(
            fields::PLACEHOLDER,
            ContractValue::Constant(TypedValue::String("이름".into())),
        );
        contract.semantic.set(
            fields::IME_INPUT,
            ContractValue::Constant(TypedValue::Boolean(true)),
        );
        assert_eq!(check(&contract), vec![]);
    }

    #[test]
    fn non_string_editable_binding_is_an_error() {
        // 검증 기준(IX.6.3): editable binding type이 string이 아니면 오류.
        let issues = check(&text_input_contract(var(IS_ON)));
        assert_eq!(
            issues,
            vec![ContractIssue {
                frame: stable(KNOB),
                field: Some(fields::VALUE),
                problem: ContractProblem::TypeMismatch {
                    expected: ExpressionType::String,
                    found: Some(TypeSpec::required(ExpressionType::Boolean)),
                },
            }]
        );
        assert_eq!(issues[0].severity(), Severity::Error);
        // String이어도 상수·Property는 입력이 쓸 수 없습니다.
        let mut constant = text_input_contract(var(TEXT));
        constant.semantic.set(
            fields::VALUE,
            ContractValue::Constant(TypedValue::String("고정".into())),
        );
        assert_eq!(check(&constant)[0].problem, ContractProblem::NotWritable);
        let mut property = text_input_contract(var(TEXT));
        property
            .semantic
            .set(fields::VALUE, ContractValue::Property(prop(CAPTION)));
        assert_eq!(check(&property)[0].problem, ContractProblem::NotWritable);
        // 없는 상태 변수.
        assert_eq!(
            check(&text_input_contract(var(999)))[0].problem,
            ContractProblem::UnresolvedReference
        );
    }

    #[test]
    fn missing_value_and_interaction_are_diagnosed() {
        let mut contract = text_input_contract(var(TEXT));
        contract.semantic.remove(fields::VALUE);
        contract.interaction = InteractionContract::new(false, []);
        let problems: Vec<_> = check(&contract).into_iter().map(|i| i.problem).collect();
        assert_eq!(
            problems,
            vec![
                ContractProblem::MissingRequired,
                ContractProblem::NotFocusable,
                ContractProblem::MissingInteraction {
                    event: EventKind::Input
                },
            ]
        );
    }
}

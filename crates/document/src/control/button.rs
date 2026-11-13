//! Button 계약 (IX.6.2).
//!
//! `Button = Frame + Action Semantic + Activate/Focus Contract`(기술 아키텍처 12.5).
//!
//! | field | id | 값 | 필수 |
//! |---|---|---|---|
//! | activate | 1 | semantic action(활성화 시 의미) | 예(없으면 진단) |
//! | focused | 2 | Boolean(상수·Property·상태) | 아니오 |
//! | disabled | 3 | Boolean(상수·Property·상태) | 아니오 |
//! | pressed | 4 | Boolean(상수·Property·상태) | 아니오 |
//!
//! 상호작용 계약은 focus 가능하고 `Activate`를 처리해야 합니다(아니면 진단).

use stellar_core_types::ids::StableId;
use stellar_expressions::types::ExpressionType;

use super::{
    ContractField, ContractIssue, ContractKind, ContractProblem, ContractScope, ContractValue,
    ControlContract, FieldChecker, InteractionContract, SemanticContract,
};
use crate::component::event::EventKind;
use crate::component::state_machine::SemanticActionId;

/// Button 계약 종류.
pub const BUTTON: ContractKind = ContractKind(0x0001);

/// Button field.
pub mod fields {
    use super::ContractField;

    /// 활성화 시 의미(semantic action).
    pub const ACTIVATE: ContractField = ContractField(1);
    /// focus 상태(Boolean).
    pub const FOCUSED: ContractField = ContractField(2);
    /// 비활성 상태(Boolean).
    pub const DISABLED: ContractField = ContractField(3);
    /// 눌림 상태(Boolean).
    pub const PRESSED: ContractField = ContractField(4);

    /// 알려진 field 전체.
    pub const ALL: [ContractField; 4] = [ACTIVATE, FOCUSED, DISABLED, PRESSED];
}

/// 기본 Button 계약: `activate` action, focus 가능, Activate·Focus·Blur 처리.
pub fn button_contract(activate: SemanticActionId) -> ControlContract {
    ControlContract::new(
        SemanticContract::new(BUTTON).with(fields::ACTIVATE, ContractValue::Action(activate)),
        InteractionContract::new(
            true,
            [EventKind::Activate, EventKind::Focus, EventKind::Blur],
        ),
    )
}

/// Button 계약을 검사합니다.
pub fn validate_button(
    frame: StableId,
    contract: &ControlContract,
    scope: &dyn ContractScope,
) -> Vec<ContractIssue> {
    let mut checker = FieldChecker::new(frame, contract, scope);
    if contract.kind() != BUTTON {
        checker.push(None, ContractProblem::UnknownKind);
        return checker.finish();
    }
    checker.action(fields::ACTIVATE, true);
    for field in [fields::FOCUSED, fields::DISABLED, fields::PRESSED] {
        checker.value(field, false, ExpressionType::Boolean, false);
    }
    checker.requires_focus();
    checker.requires_event(EventKind::Activate);
    checker.unknown_fields(&fields::ALL);
    checker.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::state_machine::fixtures::*;
    use crate::control::fixtures::library;
    use crate::control::{Severity, ValueKind};
    use stellar_core_types::typed_value::TypedValue;

    fn check(contract: &ControlContract) -> Vec<ContractIssue> {
        let library = library();
        validate_button(stable(KNOB), contract, library.get(toggle()).unwrap())
    }

    #[test]
    fn complete_button_has_no_issues() {
        let mut contract = button_contract(action_id(TOGGLED));
        contract
            .semantic
            .set(fields::PRESSED, ContractValue::State(var(IS_ON)));
        contract.semantic.set(
            fields::DISABLED,
            ContractValue::Property(prop(INITIALLY_ON)),
        );
        contract.semantic.set(
            fields::FOCUSED,
            ContractValue::Constant(TypedValue::Boolean(false)),
        );
        assert_eq!(check(&contract), vec![]);
    }

    #[test]
    fn missing_required_action_is_diagnosed() {
        // 검증 기준(IX.6.2): 필수 action 누락 diagnostic.
        let mut contract = button_contract(action_id(TOGGLED));
        contract.semantic.remove(fields::ACTIVATE);
        let issues = check(&contract);
        assert_eq!(
            issues,
            vec![ContractIssue {
                frame: stable(KNOB),
                field: Some(fields::ACTIVATE),
                problem: ContractProblem::MissingRequired,
            }]
        );
        assert_eq!(issues[0].severity(), Severity::Diagnostic);
    }

    #[test]
    fn wrong_values_and_interaction_are_reported() {
        let mut contract = button_contract(action_id(998));
        contract
            .semantic
            .set(fields::PRESSED, ContractValue::State(var(TEXT)));
        contract
            .semantic
            .set(fields::DISABLED, ContractValue::Node(stable(ROOT)));
        contract.interaction = InteractionContract::new(false, [EventKind::Focus]);
        let issues = check(&contract);
        let problems: Vec<_> = issues
            .iter()
            .map(|i| (i.field, i.problem.clone()))
            .collect();
        assert_eq!(
            problems,
            vec![
                (Some(fields::ACTIVATE), ContractProblem::UnresolvedReference),
                (
                    Some(fields::DISABLED),
                    ContractProblem::WrongValueKind {
                        expected: vec![ValueKind::Constant, ValueKind::Property, ValueKind::State],
                        found: ValueKind::Node,
                    }
                ),
                (
                    Some(fields::PRESSED),
                    ContractProblem::TypeMismatch {
                        expected: ExpressionType::Boolean,
                        found: Some(stellar_expressions::types::TypeSpec::required(
                            ExpressionType::String
                        )),
                    }
                ),
                (None, ContractProblem::NotFocusable),
                (
                    None,
                    ContractProblem::MissingInteraction {
                        event: EventKind::Activate
                    }
                ),
            ]
        );
        // 다른 종류의 계약은 Button 검사 대상이 아닙니다.
        let mut other = button_contract(action_id(TOGGLED));
        other.semantic = SemanticContract::new(ContractKind(0x7777));
        assert_eq!(check(&other)[0].problem, ContractProblem::UnknownKind);
    }
}

//! Modal 계약 (IX.6.4).
//!
//! `Modal = Frame + Overlay + Focus Scope + Dismiss Contract`(기술 아키텍처 12.5).
//!
//! | field | id | 값 | 필수 |
//! |---|---|---|---|
//! | overlay | 1 | node(계약 Frame 자신 또는 자손) — 배경을 가리는 overlay | 아니오 |
//! | focusScopeRoot | 2 | node(계약 Frame 자신 또는 자손) — focus가 갇히는 범위의 root | 예(없으면 진단) |
//! | dismiss | 3 | semantic action — 닫힘의 의미 | 아니오 |
//! | open | 4 | Boolean(상수·Property·상태) — 열림 상태 | 아니오 |
//!
//! focus scope root·overlay가 계약 Frame 밖 node면 `OutsideFrame` 오류입니다. 상호작용 계약은 `Close`(dismiss)를
//! 처리해야 합니다(아니면 진단).

use stellar_core_types::ids::StableId;
use stellar_expressions::types::ExpressionType;

use super::{
    ContractField, ContractIssue, ContractKind, ContractProblem, ContractScope, ContractValue,
    ControlContract, FieldChecker, InteractionContract, SemanticContract,
};
use crate::component::event::EventKind;

/// Modal 계약 종류.
pub const MODAL: ContractKind = ContractKind(0x0003);

/// Modal field.
pub mod fields {
    use super::ContractField;

    /// overlay node.
    pub const OVERLAY: ContractField = ContractField(1);
    /// focus scope root node.
    pub const FOCUS_SCOPE_ROOT: ContractField = ContractField(2);
    /// 닫힘의 의미(semantic action).
    pub const DISMISS: ContractField = ContractField(3);
    /// 열림 상태(Boolean).
    pub const OPEN: ContractField = ContractField(4);

    /// 알려진 field 전체.
    pub const ALL: [ContractField; 4] = [OVERLAY, FOCUS_SCOPE_ROOT, DISMISS, OPEN];
}

/// 기본 Modal 계약: focus scope root node, Open·Close·Key 처리(focus 자체는 scope 안 control이 받음).
pub fn modal_contract(focus_scope_root: StableId) -> ControlContract {
    ControlContract::new(
        SemanticContract::new(MODAL).with(
            fields::FOCUS_SCOPE_ROOT,
            ContractValue::Node(focus_scope_root),
        ),
        InteractionContract::new(false, [EventKind::Open, EventKind::Close, EventKind::Key]),
    )
}

/// Modal 계약을 검사합니다.
pub fn validate_modal(
    frame: StableId,
    contract: &ControlContract,
    scope: &dyn ContractScope,
) -> Vec<ContractIssue> {
    let mut checker = FieldChecker::new(frame, contract, scope);
    if contract.kind() != MODAL {
        checker.push(None, ContractProblem::UnknownKind);
        return checker.finish();
    }
    checker.node(fields::OVERLAY, false, true);
    checker.node(fields::FOCUS_SCOPE_ROOT, true, true);
    checker.action(fields::DISMISS, false);
    checker.value(fields::OPEN, false, ExpressionType::Boolean, false);
    checker.requires_event(EventKind::Close);
    checker.unknown_fields(&fields::ALL);
    checker.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::state_machine::fixtures::*;
    use crate::control::fixtures::library;
    use crate::control::Severity;

    /// 계약 Frame = Toggle root(300), 자손 knob(301).
    fn check(contract: &ControlContract) -> Vec<ContractIssue> {
        let library = library();
        validate_modal(stable(ROOT), contract, library.get(toggle()).unwrap())
    }

    #[test]
    fn complete_modal_has_no_issues() {
        let mut contract = modal_contract(stable(KNOB));
        contract
            .semantic
            .set(fields::OVERLAY, ContractValue::Node(stable(ROOT)));
        contract
            .semantic
            .set(fields::DISMISS, ContractValue::Action(action_id(TOGGLED)));
        contract
            .semantic
            .set(fields::OPEN, ContractValue::State(var(IS_ON)));
        assert_eq!(check(&contract), vec![]);
    }

    #[test]
    fn missing_focus_scope_root_is_diagnosed() {
        // 검증 기준(IX.6.4): focus scope root 누락 diagnostic.
        let mut contract = modal_contract(stable(KNOB));
        contract.semantic.remove(fields::FOCUS_SCOPE_ROOT);
        let issues = check(&contract);
        assert_eq!(
            issues,
            vec![ContractIssue {
                frame: stable(ROOT),
                field: Some(fields::FOCUS_SCOPE_ROOT),
                problem: ContractProblem::MissingRequired,
            }]
        );
        assert_eq!(issues[0].severity(), Severity::Diagnostic);
    }

    #[test]
    fn scope_outside_frame_and_missing_dismiss_handling_are_reported() {
        let library = library();
        let entry = library.get(toggle()).unwrap();
        // 계약 Frame = knob(301): root(300)는 Frame 밖입니다.
        let mut contract = modal_contract(stable(ROOT));
        contract
            .semantic
            .set(fields::OVERLAY, ContractValue::Node(stable(9_999)));
        contract.interaction = InteractionContract::new(false, [EventKind::Open]);
        let problems: Vec<_> = validate_modal(stable(KNOB), &contract, entry)
            .into_iter()
            .map(|i| (i.field, i.problem))
            .collect();
        assert_eq!(
            problems,
            vec![
                (Some(fields::OVERLAY), ContractProblem::UnresolvedReference),
                (
                    Some(fields::FOCUS_SCOPE_ROOT),
                    ContractProblem::OutsideFrame
                ),
                (
                    None,
                    ContractProblem::MissingInteraction {
                        event: EventKind::Close
                    }
                ),
            ]
        );
    }
}

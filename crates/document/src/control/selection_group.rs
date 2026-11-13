//! SingleSelectionGroup 계약 (IX.6.6).
//!
//! Radio UI는 별도 RadioButton node를 만들지 않고 **List + Button**을 합성해 표현합니다(기술 아키텍처 12.5).
//! 그룹 Frame은 단일 선택 List와 그 item인 Button들을 묶고, 한 번에 하나만 선택된다는 의미를 더합니다.
//!
//! | field | id | 값 | 필수 |
//! |---|---|---|---|
//! | list | 1 | node(그룹 Frame 자신 또는 자손) — Single 선택 List 계약을 가진 Frame | 예(없으면 진단) |
//! | selected | 2 | String 상태 변수 — 선택된 item identity | 아니오 |
//!
//! 검사: list node는 List 계약이어야 하고 선택 방식이 Single이어야 하며, 그 collection의 자식(item)은 모두 Button
//! 계약을 가져야 합니다(아니면 `InvalidLinkedContract` 오류). runtime 선택 상태는
//! [`validate_single_selection`]이 둘 이상 선택을 거부합니다.

use std::fmt;

use stellar_core_types::ids::StableId;
use stellar_expressions::types::ExpressionType;

use super::button::BUTTON;
use super::list::{self, SelectionMode, LIST};
use super::{
    ContractField, ContractIssue, ContractKind, ContractProblem, ContractScope, ContractValue,
    ControlContract, ControlContractTable, FieldChecker, InteractionContract, SemanticContract,
};
use crate::component::event::EventKind;

/// SingleSelectionGroup 계약 종류.
pub const SINGLE_SELECTION_GROUP: ContractKind = ContractKind(0x0005);

/// SingleSelectionGroup field.
pub mod fields {
    use super::ContractField;

    /// 단일 선택 List Frame.
    pub const LIST: ContractField = ContractField(1);
    /// 선택된 item identity(String 상태 변수).
    pub const SELECTED: ContractField = ContractField(2);

    /// 알려진 field 전체.
    pub const ALL: [ContractField; 2] = [LIST, SELECTED];
}

/// 기본 SingleSelectionGroup 계약: list Frame, SelectionChange 처리(focus는 List·Button이 받음).
pub fn single_selection_group_contract(list: StableId) -> ControlContract {
    ControlContract::new(
        SemanticContract::new(SINGLE_SELECTION_GROUP).with(fields::LIST, ContractValue::Node(list)),
        InteractionContract::new(false, [EventKind::SelectionChange]),
    )
}

/// SingleSelectionGroup 계약을 검사합니다. 연결된 List·Button 계약은 `table`에서 찾습니다.
pub fn validate_single_selection_group(
    frame: StableId,
    contract: &ControlContract,
    table: &ControlContractTable,
    scope: &dyn ContractScope,
) -> Vec<ContractIssue> {
    let mut checker = FieldChecker::new(frame, contract, scope);
    if contract.kind() != SINGLE_SELECTION_GROUP {
        checker.push(None, ContractProblem::UnknownKind);
        return checker.finish();
    }
    if let Some(list_frame) = checker.node(fields::LIST, true, true) {
        match table.get(list_frame) {
            Some(linked)
                if linked.kind() == LIST
                    && list::selection_mode(linked) == SelectionMode::Single =>
            {
                let collection = match linked.semantic.field(list::fields::COLLECTION) {
                    Some(ContractValue::Node(node)) => Some(*node),
                    _ => None,
                };
                let items = collection.map(|c| scope.children(c)).unwrap_or_default();
                if items
                    .iter()
                    .any(|item| table.get(*item).map(ControlContract::kind) != Some(BUTTON))
                {
                    checker.push(Some(fields::LIST), ContractProblem::InvalidLinkedContract);
                }
            }
            _ => checker.push(Some(fields::LIST), ContractProblem::InvalidLinkedContract),
        }
    }
    checker.value(fields::SELECTED, false, ExpressionType::String, true);
    checker.requires_event(EventKind::SelectionChange);
    checker.unknown_fields(&fields::ALL);
    checker.finish()
}

/// runtime 선택 상태 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionStateError {
    /// 단일 선택 그룹에 둘 이상이 선택됐습니다.
    MultipleSelected {
        /// 그룹 Frame.
        group: StableId,
        /// 선택된 identity 목록.
        selected: Vec<String>,
    },
}

impl fmt::Display for SelectionStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SelectionStateError::MultipleSelected { group, selected } => write!(
                f,
                "단일 선택 그룹 {group}에 {}개가 선택됐습니다",
                selected.len()
            ),
        }
    }
}

impl std::error::Error for SelectionStateError {}

/// runtime 선택 상태가 단일 선택 규칙(0개 또는 1개)을 지키는지 검사합니다.
pub fn validate_single_selection(
    group: StableId,
    selected: &[String],
) -> Result<(), SelectionStateError> {
    if selected.len() > 1 {
        return Err(SelectionStateError::MultipleSelected {
            group,
            selected: selected.to_vec(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::state_machine::fixtures::*;
    use crate::control::button::button_contract;
    use crate::control::fixtures::library;
    use crate::control::list::list_contract;
    use crate::node_kind::NodeKind;

    /// 그룹·List Frame = root(300), collection = root → item = 301(Frame knob), 302(Text).
    /// `item_buttons`면 knob에만 Button 계약을 붙입니다.
    fn table(item_buttons: bool, mode: SelectionMode) -> ControlContractTable {
        let library = library();
        let entry = library.get(toggle()).unwrap();
        let kind_of = |id| crate::control::ContractScope::node_kind(entry, id);
        let mut table = ControlContractTable::new();
        table
            .attach(stable(ROOT), kind_of, list_contract(stable(ROOT), mode))
            .unwrap();
        if item_buttons {
            table
                .attach(stable(KNOB), kind_of, button_contract(action_id(TOGGLED)))
                .unwrap();
        }
        table
    }

    #[test]
    fn more_than_one_selected_runtime_state_is_rejected() {
        // 검증 기준(IX.6.6): 한 번에 둘 이상 selected인 runtime state 거부.
        let selected: Vec<String> = vec!["a".into(), "b".into()];
        assert_eq!(
            validate_single_selection(stable(ROOT), &selected).unwrap_err(),
            SelectionStateError::MultipleSelected {
                group: stable(ROOT),
                selected: selected.clone(),
            }
        );
        validate_single_selection(stable(ROOT), &selected[..1]).unwrap();
        validate_single_selection(stable(ROOT), &[]).unwrap();
    }

    #[test]
    fn radio_is_list_plus_button_without_a_new_node_kind() {
        // RadioButton node 종류는 없습니다(합성으로 표현).
        assert!(NodeKind::ALL.iter().all(|k| !k.name().contains("radio")));
        let library = library();
        let entry = library.get(toggle()).unwrap();
        let mut group = single_selection_group_contract(stable(ROOT));
        group
            .semantic
            .set(fields::SELECTED, ContractValue::State(var(TEXT)));
        // root의 item 중 302(Text)는 Button 계약이 없으므로 knob만 Button인 표는 연결 오류입니다.
        let partial = table(true, SelectionMode::Single);
        assert_eq!(
            validate_single_selection_group(stable(ROOT), &group, &partial, entry)
                .into_iter()
                .map(|i| i.problem)
                .collect::<Vec<_>>(),
            vec![ContractProblem::InvalidLinkedContract]
        );
        // List가 Multiple 선택이면 연결 오류.
        let multiple = table(true, SelectionMode::Multiple);
        assert_eq!(
            validate_single_selection_group(stable(ROOT), &group, &multiple, entry)[0].problem,
            ContractProblem::InvalidLinkedContract
        );
        // List 계약이 없는 node.
        let mut no_list = ControlContractTable::new();
        no_list
            .attach(
                stable(KNOB),
                |id| crate::control::ContractScope::node_kind(entry, id),
                button_contract(action_id(TOGGLED)),
            )
            .unwrap();
        assert_eq!(
            validate_single_selection_group(stable(ROOT), &group, &no_list, entry)[0].problem,
            ContractProblem::InvalidLinkedContract
        );
    }

    #[test]
    fn complete_group_has_no_issues() {
        // collection을 knob(301, 자식 없음)으로 두면 item이 없어 Button 조건을 만족합니다.
        let library = library();
        let entry = library.get(toggle()).unwrap();
        let kind_of = |id| crate::control::ContractScope::node_kind(entry, id);
        let mut table = ControlContractTable::new();
        table
            .attach(
                stable(ROOT),
                kind_of,
                list_contract(stable(KNOB), SelectionMode::Single),
            )
            .unwrap();
        let mut group = single_selection_group_contract(stable(ROOT));
        group
            .semantic
            .set(fields::SELECTED, ContractValue::State(var(TEXT)));
        assert_eq!(
            validate_single_selection_group(stable(ROOT), &group, &table, entry),
            vec![]
        );
        // 누락 진단.
        group.semantic.remove(fields::LIST);
        assert_eq!(
            validate_single_selection_group(stable(ROOT), &group, &table, entry)[0].problem,
            ContractProblem::MissingRequired
        );
    }
}

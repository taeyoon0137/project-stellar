//! 외부 변경 겹침·분할 test (XIX.7.1, XIX.7.3).

use super::*;
use crate::command::test_fixtures::{sid, small_model};
use crate::command::Command;
use crate::id::{CommandId, CommandOrigin};
use crate::lock::lease::Lease;
use crate::request::CommandRequest;
use crate::state::DocumentState;

fn table(model: &DocumentModel) -> LeaseTable {
    let mut table = LeaseTable::new();
    table
        .insert(Lease {
            id: LeaseId(sid(900)),
            subtrees: vec![SubtreeLock::capture(model, sid(13)).unwrap()],
            externals: Vec::new(),
        })
        .unwrap();
    table
}

fn renamed(model: &DocumentModel, node: u64, name: &str) -> DocumentModel {
    let mut state = DocumentState::new(
        DocumentModel::from_records(model.to_records().unwrap())
            .unwrap()
            .0,
    );
    state
        .execute(&CommandRequest::at(
            &state,
            Command::rename_node(sid(node), name),
        ))
        .unwrap();
    state.into_model()
}

#[test]
fn overlap_is_lease_subtree_or_moved_ancestor() {
    let model = small_model();
    let table = table(&model);
    let none = overlapping_leases(&table, &model, &model, [ExternalTarget::Node(sid(12))]);
    assert!(none.is_empty());
    let inside = overlapping_leases(&table, &model, &model, [ExternalTarget::Node(sid(13))]);
    assert_eq!(inside[&LeaseId(sid(900))], BTreeSet::from([sid(13)]));
    // 11은 lease root 13의 진조상입니다: 속성 변경은 겹치지 않고 위치 변경은 겹칩니다.
    assert!(overlapping_leases(&table, &model, &model, [ExternalTarget::Node(sid(11))]).is_empty());
    assert!(
        !overlapping_leases(&table, &model, &model, [ExternalTarget::Moved(sid(11))]).is_empty()
    );
    assert!(!overlapping_leases(
        &table,
        &model,
        &model,
        [ExternalTarget::View {
            root_frame: sid(10)
        }]
    )
    .is_empty());
}

#[test]
fn partition_keeps_local_agent_changes_and_applies_independent_external_changes() {
    let base = small_model();
    let base_records = base.to_records().unwrap();
    // 창: 에이전트가 13 이름을 바꿈(저장 전).
    let current = renamed(&base, 13, "agent");
    // 외부: 12 이름을 바꿈.
    let incoming = renamed(&base, 12, "external").to_records().unwrap();
    let partition = partition_external(&base_records, &current, &incoming).unwrap();
    assert_eq!(partition.applicable.changes.len(), 1);
    assert!(partition.divergent.is_empty());
    assert_eq!(partition.local_only, vec![RecordTarget::Node(sid(13))]);

    let mut state = DocumentState::new(current);
    let tx = partition
        .applicable
        .clone()
        .into_transaction(CommandId::from_stable_id(sid(7000)), state.revision());
    assert_eq!(tx.origin, CommandOrigin::System);
    state.execute_transaction(&tx).unwrap();
    assert_eq!(
        state.model().record(sid(12)).unwrap().display_name,
        "external"
    );
    assert_eq!(state.model().record(sid(13)).unwrap().display_name, "agent");

    // 외부도 13을 바꾸면 덮어쓰지 않고 reconciliation 대상입니다.
    let both = renamed(&renamed(&base, 12, "external"), 13, "external")
        .to_records()
        .unwrap();
    let current = renamed(&base, 13, "agent");
    let partition = partition_external(&base_records, &current, &both).unwrap();
    assert_eq!(partition.divergent, vec![RecordTarget::Node(sid(13))]);
    assert_eq!(partition.applicable.changes.len(), 1);
}

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
    let mut copy = DocumentModel::from_records(model.to_records().unwrap())
        .unwrap()
        .0;
    copy.replace_context(model.context().clone());
    let mut state = DocumentState::new(copy);
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
    // 창: 에이전트가 13 이름을 바꿈(저장 전).
    let current = renamed(&base, 13, "agent");
    // 외부: 12 이름을 바꿈.
    let incoming = renamed(&base, 12, "external");
    let partition = partition_external(&base, &current, &incoming).unwrap();
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
    let both = renamed(&renamed(&base, 12, "external"), 13, "external");
    let current = renamed(&base, 13, "agent");
    let partition = partition_external(&base, &current, &both).unwrap();
    assert_eq!(partition.divergent, vec![RecordTarget::Node(sid(13))]);
    assert_eq!(partition.applicable.changes.len(), 1);
}

/// `model`에 key 하나(`key`)와 컴포넌트 `component`의 requirement를 더한 사본.
fn with_requirement(model: &DocumentModel, key: u64, component: u64) -> DocumentModel {
    use stellar_document::context::fixtures::{literal, rgb};
    use stellar_document::context::key::{ContextKey, ContextScope};
    use stellar_document::expressions::types::{ExpressionType, TypeSpec};
    let mut set = model.context().clone();
    if set.key(sid(key)).is_none() {
        set.insert_key(
            ContextKey::new(
                sid(key),
                format!("k{key}"),
                ContextScope::Document(sid(1)),
                TypeSpec::required(ExpressionType::Color),
                literal(key + 1, rgb(1.0, 1.0, 1.0)),
            )
            .unwrap(),
        )
        .unwrap();
    }
    set.add_requirement(sid(component), sid(key));
    let mut out = DocumentModel::from_records(model.to_records().unwrap())
        .unwrap()
        .0;
    out.replace_context(set);
    out
}

#[test]
fn partition_compares_the_context_too() {
    // 검증 기준(ADR-0123 결정 3): 외부 Context 변경도 분할 대상입니다. 창이 Context를 바꾸지 않았으면 바로 적용하고,
    // 둘 다 바꿨으면 덮어쓰지 않고 reconciliation으로 넘깁니다.
    let base = small_model();
    let current = renamed(&base, 13, "agent");
    let incoming = with_requirement(&base, 900, 950);
    let partition = partition_external(&base, &current, &incoming).unwrap();
    assert_eq!(partition.applicable.changes.len(), 1);
    assert!(partition.divergent.is_empty());
    let mut state = DocumentState::new(current);
    let tx = partition
        .applicable
        .clone()
        .into_transaction(CommandId::from_stable_id(sid(7001)), state.revision());
    state.execute_transaction(&tx).unwrap();
    assert_eq!(
        state.model().context().live(state.model()),
        incoming.context().live(&incoming)
    );
    assert_eq!(state.model().record(sid(13)).unwrap().display_name, "agent");

    // 창도 Context를 바꿨으면 Context 전체가 reconciliation 대상입니다.
    let current = with_requirement(&base, 901, 951);
    let partition = partition_external(&base, &current, &incoming).unwrap();
    assert!(partition.applicable.is_empty());
    assert_eq!(partition.divergent, vec![RecordTarget::Context]);
    assert!(partition.local_only.is_empty());
}

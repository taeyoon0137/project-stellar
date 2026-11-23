//! 에이전트 작업 그룹·되돌리기 history test (XIX.6.2, XIX.6.4, XIX.6.8).

use std::collections::BTreeSet;

use super::*;
use crate::history::test_fixtures::{sid, small_state, tx};
use stellar_commands::agent_batch::{AgentBatchId, AgentBatchRecord};
use stellar_commands::agent_revert::plan_revert;
use stellar_commands::command::Command;
use stellar_commands::id::{AgentProvenance, CommandId, CommandOrigin, WindowSessionId};
use stellar_commands::lock::lease::{Lease, LeaseId, SubtreeLock};
use stellar_commands::request::CommandRequest;

const LEASE: u64 = 900;

fn agent_batch(doc: &mut DocumentState, seq: u32, commands: Vec<Command>) -> AgentBatchRecord {
    let forward = CommandTransaction {
        id: CommandId::from_stable_id(sid(1000 + u64::from(seq))),
        origin: CommandOrigin::FutureAgent(AgentProvenance::new(sid(LEASE))),
        requests: commands
            .into_iter()
            .map(|c| CommandRequest::new(doc.revision(), c))
            .collect(),
    };
    let applied = doc.execute_transaction(&forward).unwrap();
    AgentBatchRecord::capture(
        AgentBatchId {
            session: sid(800),
            seq,
        },
        forward,
        applied,
        doc.model(),
    )
}

fn name(doc: &DocumentState, node: u64) -> String {
    doc.model().record(sid(node)).unwrap().display_name
}

fn window() -> WindowHistory {
    WindowHistory::new(WindowSessionId::from_stable_id(sid(77)))
}

#[test]
fn completed_group_is_one_undoable_entry_without_reexecution() {
    let mut doc = small_state();
    let baseline = doc.model().to_records().unwrap();
    let mut window = window();
    let mut labels = AgentEntryLabels::new();
    let first = agent_batch(&mut doc, 1, vec![Command::rename_node(sid(11), "a")]);
    let second = agent_batch(&mut doc, 2, vec![Command::rename_node(sid(12), "b")]);
    let revision = doc.revision();
    let group = AgentGroup::from_batches(
        CommandId::from_stable_id(sid(2000)),
        AgentProvenance::new(sid(LEASE)),
        &[&first, &second],
    )
    .unwrap();
    let sequence = WindowAgentHistory {
        window: &mut window,
        doc: &mut doc,
        labels: &mut labels,
    }
    .confirm_group(group, AgentGroupOutcome::Completed)
    .unwrap();
    assert_eq!(doc.revision(), revision, "확정은 다시 실행하지 않습니다");
    assert_eq!(window.history().len(), 1);
    let entry = &window.history().entries()[0];
    assert_eq!(entry.sequence.get(), sequence);
    assert_eq!(
        entry.origin,
        CommandOrigin::FutureAgent(AgentProvenance::new(sid(LEASE)))
    );
    assert_eq!(labels.action_name(entry), "에이전트 편집");

    window.undo(&mut doc).unwrap().unwrap();
    assert_eq!(doc.model().to_records().unwrap(), baseline);
    window.redo(&mut doc).unwrap().unwrap();
    assert_eq!(name(&doc, 11), "a");
    assert_eq!(name(&doc, 12), "b");
}

#[test]
fn interrupted_group_is_labelled() {
    let mut doc = small_state();
    let mut window = window();
    let mut labels = AgentEntryLabels::new();
    let batch = agent_batch(&mut doc, 1, vec![Command::rename_node(sid(11), "a")]);
    let group = AgentGroup::from_batches(
        CommandId::from_stable_id(sid(2000)),
        AgentProvenance::new(sid(LEASE)),
        &[&batch],
    )
    .unwrap();
    WindowAgentHistory {
        window: &mut window,
        doc: &mut doc,
        labels: &mut labels,
    }
    .confirm_group(group, AgentGroupOutcome::Interrupted)
    .unwrap();
    let entry = &window.history().entries()[0];
    assert_eq!(labels.action_name(entry), "에이전트 편집(중단됨)");
    assert_eq!(name(&doc, 11), "a", "적용 batch 유지");
}

#[test]
fn revert_entry_is_undoable_and_reapplies_agent_change() {
    let mut doc = small_state();
    let mut window = window();
    let mut labels = AgentEntryLabels::new();
    let batch = agent_batch(&mut doc, 1, vec![Command::rename_node(sid(11), "a")]);
    let plan = plan_revert(&doc, &[&batch], &BTreeSet::new()).unwrap();
    let transaction = plan
        .transaction(CommandId::from_stable_id(sid(3000)))
        .unwrap();
    let applied = WindowAgentHistory {
        window: &mut window,
        doc: &mut doc,
        labels: &mut labels,
    }
    .apply_revert(&transaction, &LeaseTable::new())
    .unwrap();
    assert_eq!(name(&doc, 11), "n11");
    assert!(!applied.changes.is_empty());
    let entry = &window.history().entries()[0];
    assert_eq!(entry.origin, CommandOrigin::User);
    assert_eq!(labels.action_name(entry), "에이전트 변경 되돌리기");
    window.undo(&mut doc).unwrap().unwrap();
    assert_eq!(name(&doc, 11), "a", "되돌리기 Undo 후 재적용 일치");
    window.redo(&mut doc).unwrap().unwrap();
    assert_eq!(name(&doc, 11), "n11");
}

#[test]
fn revert_overlapping_other_lease_or_stale_is_rejected_without_effect() {
    let mut doc = small_state();
    let mut window = window();
    let mut labels = AgentEntryLabels::new();
    let batch = agent_batch(&mut doc, 1, vec![Command::rename_node(sid(13), "a")]);
    let plan = plan_revert(&doc, &[&batch], &BTreeSet::new()).unwrap();
    let transaction = plan
        .transaction(CommandId::from_stable_id(sid(3000)))
        .unwrap();
    let mut table = LeaseTable::new();
    table
        .insert(Lease {
            id: LeaseId(sid(901)),
            subtrees: vec![SubtreeLock::capture(doc.model(), sid(11)).unwrap()],
            externals: Vec::new(),
        })
        .unwrap();
    let revision = doc.revision();
    let error = WindowAgentHistory {
        window: &mut window,
        doc: &mut doc,
        labels: &mut labels,
    }
    .apply_revert(&transaction, &table)
    .unwrap_err();
    assert!(matches!(error, AgentSinkError::Lease(_)), "{error:?}");
    assert_eq!(doc.revision(), revision);
    assert_eq!(name(&doc, 13), "a");
    assert!(window.history().is_empty());
    assert!(labels.is_empty());

    let later = tx(&doc, 50, Command::rename_node(sid(12), "x"));
    window.record_document(&mut doc, &later, None).unwrap();
    let error = WindowAgentHistory {
        window: &mut window,
        doc: &mut doc,
        labels: &mut labels,
    }
    .apply_revert(&transaction, &LeaseTable::new())
    .unwrap_err();
    assert_eq!(error, AgentSinkError::Stale);
    assert_eq!(window.history().len(), 1);
}

#[test]
fn session_revert_failing_in_the_middle_has_no_effect() {
    let mut doc = small_state();
    let mut window = window();
    let mut labels = AgentEntryLabels::new();
    let first = agent_batch(&mut doc, 1, vec![Command::rename_node(sid(11), "a")]);
    let second = agent_batch(&mut doc, 2, vec![Command::rename_node(sid(12), "b")]);
    let plan = plan_revert(&doc, &[&first, &second], &BTreeSet::new()).unwrap();
    let mut transaction = plan
        .transaction(CommandId::from_stable_id(sid(3000)))
        .unwrap();
    assert_eq!(transaction.requests.len(), 2);
    // 두 역 command 사이에 적용할 수 없는 command를 끼워 중간 실패를 만듭니다.
    transaction.requests.insert(
        1,
        CommandRequest::new(doc.revision(), Command::rename_node(sid(999), "없음")),
    );
    let before = doc.model().to_records().unwrap();
    let revision = doc.revision();
    let error = WindowAgentHistory {
        window: &mut window,
        doc: &mut doc,
        labels: &mut labels,
    }
    .apply_revert(&transaction, &LeaseTable::new())
    .unwrap_err();
    assert!(matches!(error, AgentSinkError::Transaction(_)), "{error:?}");
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.model().to_records().unwrap(), before, "효과 0");
    assert!(window.history().is_empty());
}

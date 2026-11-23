//! 출처 색인과 표시 상태 전이 test (XIX.6.7, XIX.6.10).

use std::collections::BTreeSet;

use super::*;
use crate::attached::edit::apply_approved;
use crate::attached::revert::{apply_revert, preview_revert};
use crate::provenance_fixture::*;
use stellar_ops::commands::agent_batch::record_key;
use stellar_ops::commands::agent_sink::{AgentGroup, AgentGroupOutcome, AgentHistorySink};
use stellar_ops::commands::change::node_keys;
use stellar_ops::commands::command::Command;
use stellar_ops::commands::document::record_codec::encode_node_record;
use stellar_ops::commands::id::AgentProvenance;
use stellar_ops::commands::lock::lease::{Lease, LeaseTable, SubtreeLock};
use stellar_ops::commands::node_ops::InsertNode;
use stellar_ops::commands::state::DocumentState;
use stellar_ops::validate::Definitions;

const L: u64 = 900;
const S: u64 = 700;
const NAME: &str = "버튼 정리 세션";

fn table(state: &DocumentState) -> LeaseTable {
    let mut table = LeaseTable::new();
    table
        .insert(Lease {
            id: lease_id(L),
            subtrees: vec![SubtreeLock::capture(state.model(), sid(13)).unwrap()],
            externals: Vec::new(),
        })
        .unwrap();
    table
}

fn batch(
    provenance: &mut AgentChangeProvenance,
    state: &mut DocumentState,
    n: u64,
    commands: Vec<Command>,
) -> AgentBatchId {
    let tx = proposal(state, lease_id(L), n, commands);
    let applied = apply_approved(state, &tx, &table(state)).unwrap();
    provenance
        .record_batch(session(S), NAME, lease_id(L), tx, applied, state.model())
        .unwrap()
}

fn user(
    history: &mut TestHistory,
    n: u64,
    command: Command,
) -> stellar_ops::commands::change::ChangeSet {
    let tx = CommandTransaction::new(id(n), CommandOrigin::User)
        .with_command(history.doc.revision(), command);
    let applied = history.doc.execute_transaction(&tx).unwrap();
    applied.changes
}

fn saved_bytes(state: &DocumentState) -> Vec<u8> {
    let records = state.model().to_records().unwrap();
    let mut out = records.definition.encode();
    for node in &records.nodes {
        out.extend(encode_node_record(node));
    }
    out
}

fn no_definitions<'a>() -> Definitions<'a> {
    Definitions::Failed("정의 없음")
}

fn confirm(provenance: &mut AgentChangeProvenance, history: &mut TestHistory) -> u64 {
    let records = provenance.in_progress(session(S));
    let group =
        AgentGroup::from_batches(id(2000), AgentProvenance::new(lease_id(L).0), &records).unwrap();
    let sequence = history
        .confirm_group(group, AgentGroupOutcome::Completed)
        .unwrap();
    provenance.settle(session(S), BatchOutcome::Completed, Some(sequence));
    sequence
}

#[test]
fn batch_maps_to_exact_stable_ids_and_property_paths() {
    let mut state = state();
    let mut provenance = AgentChangeProvenance::new();
    let first = batch(
        &mut provenance,
        &mut state,
        100,
        vec![
            Command::rename_node(sid(13), "a"),
            Command::set_visibility(sid(14), false),
        ],
    );
    let second = batch(
        &mut provenance,
        &mut state,
        101,
        vec![Command::InsertNode(InsertNode::single(frame(
            20,
            Some(14),
            0x10,
        )))],
    );
    assert_eq!(first.seq, 1);
    assert_eq!(second.seq, 2);
    let entry = provenance.get(first).unwrap();
    assert_eq!(entry.session, session(S));
    assert_eq!(entry.session_name, NAME);
    assert_eq!(entry.lease, lease_id(L));
    assert_eq!(entry.history_sequence, None);
    assert_eq!(entry.outcome, BatchOutcome::InProgress);
    assert_eq!(
        entry.changed_properties(),
        vec![
            PropertyPath::with_keys(sid(13), vec![node_keys::DISPLAY_NAME]),
            PropertyPath::with_keys(sid(14), vec![node_keys::VISIBLE]),
        ]
    );
    assert_eq!(entry.changed_nodes(), BTreeSet::from([sid(13), sid(14)]));
    assert_eq!(entry.record.fingerprints.len(), 2);
    let entry = provenance.get(second).unwrap();
    assert_eq!(entry.record.created, BTreeSet::from([sid(20)]));
    assert!(entry.record.fingerprints.contains_key(&record_key(sid(20))));
    assert!(entry.record.deleted.is_empty());
    assert_eq!(
        provenance.shown_nodes(),
        BTreeSet::from([sid(13), sid(14), sid(20)])
    );
}

#[test]
fn provenance_is_not_in_saved_bytes_and_is_dropped_with_window() {
    let mut state = state();
    let mut provenance = AgentChangeProvenance::new();
    batch(
        &mut provenance,
        &mut state,
        100,
        vec![Command::rename_node(sid(13), "a")],
    );
    let bytes = saved_bytes(&state);
    let contains = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    assert!(!contains(NAME.as_bytes()), "세션 표시 이름 0");
    assert!(!contains(&sid(S).to_bytes()), "세션 id 0");
    assert!(!contains(&lease_id(L).0.to_bytes()), "lease id 0");
    assert_eq!(provenance.close_window(), 1);
    assert!(provenance.is_empty(), "창 닫기 후 색인 0");
}

#[test]
fn keep_clears_display_without_touching_document_or_history() {
    let mut history = TestHistory::new(state());
    let mut provenance = AgentChangeProvenance::new();
    let first = batch(
        &mut provenance,
        &mut history.doc,
        100,
        vec![Command::rename_node(sid(13), "a")],
    );
    confirm(&mut provenance, &mut history);
    let revision = history.doc.revision();
    let bytes = saved_bytes(&history.doc);
    let entries = history.entries.len();
    let kept = provenance
        .keep(
            ProvenanceScope::Batch(first),
            UserUiAction::from_user_interface(),
        )
        .unwrap();
    assert_eq!(kept, 1);
    assert_eq!(provenance.get(first).unwrap().display(), DisplayState::Kept);
    assert_eq!(history.doc.revision(), revision);
    assert_eq!(saved_bytes(&history.doc), bytes);
    assert_eq!(history.entries.len(), entries);
    assert!(provenance.shown_nodes().is_empty());
    assert!(matches!(
        provenance.keep(
            ProvenanceScope::Batch(first),
            UserUiAction::from_user_interface()
        ),
        Err(ProvenanceError::NotShown(_, DisplayState::Kept))
    ));
}

#[test]
fn later_change_undo_redo_and_revert_transitions() {
    let mut history = TestHistory::new(state());
    let mut provenance = AgentChangeProvenance::new();
    let first = batch(
        &mut provenance,
        &mut history.doc,
        100,
        vec![Command::rename_node(sid(13), "a")],
    );
    let group = confirm(&mut provenance, &mut history);
    let display = |p: &AgentChangeProvenance| p.get(first).unwrap().display();

    // 다른 속성 변경은 상태를 바꾸지 않습니다.
    let changes = user(&mut history, 200, Command::set_visibility(sid(13), false));
    provenance.observe(&changes, CommandOrigin::User, history.doc.model());
    assert_eq!(display(&provenance), DisplayState::PendingReview);
    // 같은 속성을 사용자가 바꾸면 "이후 변경됨", 그 변경을 되돌리면 "검토 대기".
    let changes = user(&mut history, 201, Command::rename_node(sid(13), "user"));
    provenance.observe(&changes, CommandOrigin::User, history.doc.model());
    assert_eq!(display(&provenance), DisplayState::ChangedLater);
    let changes = user(&mut history, 202, Command::rename_node(sid(13), "a"));
    provenance.observe(&changes, CommandOrigin::User, history.doc.model());
    assert_eq!(display(&provenance), DisplayState::PendingReview);
    // 같은 세션의 이후 batch는 "이후 변경됨"이 아닙니다.
    let own = CommandOrigin::FutureAgent(AgentProvenance::new(lease_id(L).0));
    provenance.observe(&changes, own, history.doc.model());
    assert_eq!(display(&provenance), DisplayState::PendingReview);

    // 그룹 entry Undo → 실행 취소됨, Redo → 이전 상태.
    let (sequence, changes, entry) = history.undo();
    assert_eq!(sequence, group);
    provenance.on_history(sequence, HistoryDirection::Undo);
    provenance.observe(&changes, CommandOrigin::User, history.doc.model());
    assert_eq!(display(&provenance), DisplayState::Undone);
    let changes = history.redo(entry);
    provenance.on_history(sequence, HistoryDirection::Redo);
    provenance.observe(&changes, CommandOrigin::User, history.doc.model());
    assert_eq!(display(&provenance), DisplayState::PendingReview);

    // 되돌리기 확정 → 되돌림, 그 Undo → 이전 상태, Redo → 되돌림.
    let table = LeaseTable::new();
    let preview = preview_revert(
        &history.doc,
        &provenance,
        ProvenanceScope::Batch(first),
        &BTreeSet::new(),
        &table,
        no_definitions,
    )
    .unwrap();
    let applied = apply_revert(
        &preview,
        &mut history,
        &table,
        &mut provenance,
        id(3000),
        UserUiAction::from_user_interface(),
    )
    .unwrap();
    assert_eq!(display(&provenance), DisplayState::Reverted);
    assert_eq!(name(&history.doc, 13), "n13");
    let (sequence, _, entry) = history.undo();
    assert_eq!(sequence, applied.sequence);
    provenance.on_history(sequence, HistoryDirection::Undo);
    assert_eq!(display(&provenance), DisplayState::PendingReview);
    assert_eq!(name(&history.doc, 13), "a");
    history.redo(entry);
    provenance.on_history(sequence, HistoryDirection::Redo);
    assert_eq!(display(&provenance), DisplayState::Reverted);
    // 되돌린 batch는 다시 되돌리거나 유지할 수 없습니다.
    assert!(matches!(
        provenance.revertible(ProvenanceScope::Batch(first)),
        Err(ProvenanceError::NotShown(_, DisplayState::Reverted))
    ));
}

#[test]
fn mcp_catalog_has_no_provenance_or_revert_write_path() {
    for tool in crate::agent_protocol::catalog::tools::all_tools() {
        let name = tool.name.to_ascii_lowercase();
        for word in ["provenance", "revert", "keep", "rollback", "undo"] {
            assert!(!name.contains(word), "{} exposes {word}", tool.name);
        }
    }
}

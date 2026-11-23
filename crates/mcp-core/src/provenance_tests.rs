//! 출처 색인과 표시 상태 전이 test (XIX.6.7, XIX.6.10).

use std::collections::BTreeSet;

use super::*;
use crate::attached::edit::apply_approved;
use crate::provenance_fixture::*;
use stellar_ops::commands::agent_batch::record_key;
use stellar_ops::commands::change::node_keys;
use stellar_ops::commands::command::Command;
use stellar_ops::commands::document::record_codec::encode_node_record;
use stellar_ops::commands::lock::lease::{Lease, LeaseTable, SubtreeLock};
use stellar_ops::commands::node_ops::InsertNode;
use stellar_ops::commands::state::DocumentState;

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

fn saved_bytes(state: &DocumentState) -> Vec<u8> {
    let records = state.model().to_records().unwrap();
    let mut out = records.definition.encode();
    for node in &records.nodes {
        out.extend(encode_node_record(node));
    }
    out
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

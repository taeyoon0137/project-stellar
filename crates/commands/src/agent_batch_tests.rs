//! 에이전트 batch 기록 test (XIX.6.1, XIX.6.7).

use super::*;
use crate::command::test_fixtures::{frame, sid, small_model};
use crate::command::Command;
use crate::id::{AgentProvenance, CommandId, CommandOrigin};
use crate::node_ops::{DeleteSubtree, InsertNode};
use crate::request::CommandRequest;
use crate::state::DocumentState;
use stellar_core_types::property::PropertyPath;

fn agent_tx(state: &DocumentState, id: u64, commands: Vec<Command>) -> CommandTransaction {
    CommandTransaction {
        id: CommandId::from_stable_id(sid(id)),
        origin: CommandOrigin::FutureAgent(AgentProvenance::new(sid(900))),
        requests: commands
            .into_iter()
            .map(|c| CommandRequest::new(state.revision(), c))
            .collect(),
    }
}

fn batch_id(seq: u32) -> AgentBatchId {
    AgentBatchId {
        session: sid(800),
        seq,
    }
}

fn apply(state: &mut DocumentState, seq: u32, commands: Vec<Command>) -> AgentBatchRecord {
    let tx = agent_tx(state, 1000 + u64::from(seq), commands);
    let applied = state.execute_transaction(&tx).unwrap();
    AgentBatchRecord::capture(batch_id(seq), tx, applied, state.model())
}

#[test]
fn capture_keeps_inverse_without_document_copy_and_inverse_restores_records() {
    let mut state = DocumentState::new(small_model());
    let baseline = state.model().to_records().unwrap();
    let mut commands = Vec::new();
    for i in 0..1000u32 {
        let node = [11, 12, 13][i as usize % 3];
        commands.push(match i % 2 {
            0 => Command::rename_node(sid(node), format!("이름 {i}")),
            _ => Command::set_visibility(sid(node), i % 4 == 1),
        });
    }
    let record = apply(&mut state, 1, commands);
    assert_eq!(record.forward.requests.len(), 1000);
    assert_eq!(record.inverse.requests.len(), 1000);
    state
        .execute_transaction(&record.inverse.rebased(state.revision()))
        .unwrap();
    assert_eq!(state.model().to_records().unwrap(), baseline);
}

#[test]
fn property_changes_are_keys_with_post_apply_fingerprints() {
    let mut state = DocumentState::new(small_model());
    let record = apply(&mut state, 1, vec![Command::rename_node(sid(11), "a")]);
    let key = PropertyPath::with_keys(sid(11), vec![node_keys::DISPLAY_NAME]);
    assert_eq!(record.fingerprints.len(), 1, "{:?}", record.fingerprints);
    assert!(record.fingerprints.contains_key(&key));
    assert!(record.diverged(state.model()).is_empty());
    // 다른 속성 변경은 이 key 지문을 바꾸지 않습니다.
    state
        .execute(&CommandRequest::at(
            &state,
            Command::set_visibility(sid(11), false),
        ))
        .unwrap();
    assert!(record.diverged(state.model()).is_empty());
    state
        .execute(&CommandRequest::at(
            &state,
            Command::rename_node(sid(11), "b"),
        ))
        .unwrap();
    assert_eq!(record.diverged(state.model()), vec![key]);
}

#[test]
fn created_and_deleted_nodes_have_record_keys() {
    let mut state = DocumentState::new(small_model());
    let record = apply(
        &mut state,
        1,
        vec![
            Command::InsertNode(InsertNode::single(frame(20, Some(12), 0x10))),
            Command::DeleteSubtree(DeleteSubtree { root: sid(13) }),
        ],
    );
    assert_eq!(record.created, BTreeSet::from([sid(20)]));
    assert_eq!(record.deleted, BTreeSet::from([sid(13)]));
    assert!(matches!(
        record.fingerprints[&record_key(sid(20))],
        ValueFingerprint::Present(_)
    ));
    assert_eq!(
        record.fingerprints[&record_key(sid(13))],
        ValueFingerprint::Absent
    );
    // 자식 목록만 바뀐 부모도 레코드 key이고, 지문은 레코드(자식 목록 제외) 기준입니다.
    assert!(record.fingerprints.contains_key(&record_key(sid(12))));
    assert!(record.diverged(state.model()).is_empty());
}

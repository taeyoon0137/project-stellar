//! 되돌리기 계획 test (XIX.6.8, XIX.6.9).

use super::*;
use crate::agent_batch::AgentBatchRecord;
use crate::command::test_fixtures::{frame, sid, small_model};
use crate::id::AgentProvenance;
use crate::node_ops::InsertNode;
use crate::state::DocumentState;

fn batch_id(seq: u32) -> AgentBatchId {
    AgentBatchId {
        session: sid(800),
        seq,
    }
}

fn apply(state: &mut DocumentState, seq: u32, commands: Vec<Command>) -> AgentBatchRecord {
    let tx = CommandTransaction {
        id: CommandId::from_stable_id(sid(1000 + u64::from(seq))),
        origin: CommandOrigin::FutureAgent(AgentProvenance::new(sid(900))),
        requests: commands
            .into_iter()
            .map(|c| CommandRequest::new(state.revision(), c))
            .collect(),
    };
    let applied = state.execute_transaction(&tx).unwrap();
    AgentBatchRecord::capture(batch_id(seq), tx, applied, state.model())
}

fn user(state: &mut DocumentState, command: Command) {
    state.execute(&CommandRequest::at(state, command)).unwrap();
}

fn run(state: &mut DocumentState, plan: &RevertPlan) {
    let tx = plan
        .transaction(CommandId::from_stable_id(sid(5000)))
        .expect("commands");
    assert_eq!(tx.origin, CommandOrigin::User);
    state.execute_transaction(&tx).unwrap();
}

fn name(state: &DocumentState, node: u64) -> String {
    state.model().record(sid(node)).unwrap().display_name
}

#[test]
fn non_conflicting_batch_revert_restores_baseline_without_touching_state() {
    let mut state = DocumentState::new(small_model());
    let baseline = state.model().to_records().unwrap();
    let batch = apply(
        &mut state,
        1,
        vec![
            Command::rename_node(sid(11), "agent"),
            Command::set_visibility(sid(12), false),
        ],
    );
    let revision = state.revision();
    let plan = plan_revert(&state, &[&batch], &BTreeSet::new()).unwrap();
    assert_eq!(state.revision(), revision, "계획은 문서를 바꾸지 않습니다");
    assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);
    assert_eq!(plan.restored.len(), 2);
    run(&mut state, &plan);
    assert_eq!(state.model().to_records().unwrap(), baseline);
}

#[test]
fn later_user_edit_is_conflict_kept_by_default_and_restored_when_chosen() {
    let mut state = DocumentState::new(small_model());
    let batch = apply(
        &mut state,
        1,
        vec![
            Command::rename_node(sid(11), "agent"),
            Command::set_visibility(sid(12), false),
        ],
    );
    user(&mut state, Command::rename_node(sid(11), "user"));
    let plan = plan_revert(&state, &[&batch], &BTreeSet::new()).unwrap();
    let key = PropertyPath::with_keys(sid(11), vec![node_keys::DISPLAY_NAME]);
    assert_eq!(plan.conflicts.len(), 1);
    assert_eq!(plan.conflicts[0].key.as_ref(), Some(&key));
    assert_eq!(plan.conflicts[0].kind, ConflictKind::ChangedLater);
    assert!(!plan.conflicts[0].included);
    let mut kept = DocumentState::with_revision(
        DocumentModel::from_records(state.model().to_records().unwrap())
            .unwrap()
            .0,
        state.revision(),
    );
    run(&mut kept, &plan);
    assert_eq!(name(&kept, 11), "user", "사용자 편집 기본 보존");
    assert!(
        kept.model().record(sid(12)).unwrap().visible,
        "비충돌 속성 복원"
    );

    let include = BTreeSet::from([RevertItem {
        batch: batch.id,
        key,
    }]);
    let plan = plan_revert(&state, &[&batch], &include).unwrap();
    assert!(plan.conflicts[0].included);
    run(&mut state, &plan);
    assert_eq!(name(&state, 11), "n11");
}

#[test]
fn created_node_with_later_child_is_structural_conflict_kept() {
    let mut state = DocumentState::new(small_model());
    let batch = apply(
        &mut state,
        1,
        vec![
            Command::InsertNode(InsertNode::single(frame(20, Some(12), 0x10))),
            Command::rename_node(sid(11), "agent"),
        ],
    );
    user(
        &mut state,
        Command::InsertNode(InsertNode::single(frame(21, Some(20), 0x10))),
    );
    let plan = plan_revert(&state, &[&batch], &BTreeSet::new()).unwrap();
    assert!(plan.conflicts.iter().any(|c| c.kind
        == ConflictKind::ChildAddedLater {
            parent: sid(20),
            child: sid(21)
        }));
    run(&mut state, &plan);
    assert!(
        state.model().contains(sid(20)),
        "구조 충돌 기본값은 현재 유지"
    );
    assert!(state.model().contains(sid(21)));
    assert_eq!(name(&state, 11), "n11");
}

#[test]
fn deleted_target_node_is_node_deleted_conflict() {
    let mut state = DocumentState::new(small_model());
    let batch = apply(&mut state, 1, vec![Command::rename_node(sid(13), "agent")]);
    user(
        &mut state,
        Command::DeleteSubtree(crate::node_ops::DeleteSubtree { root: sid(13) }),
    );
    let plan = plan_revert(&state, &[&batch], &BTreeSet::new()).unwrap();
    assert!(plan.is_empty());
    assert!(plan
        .conflicts
        .iter()
        .any(|c| c.kind == ConflictKind::NodeDeleted { node: sid(13) }));
}

#[test]
fn session_revert_of_ten_batches_restores_baseline_in_one_transaction() {
    let mut state = DocumentState::new(small_model());
    let baseline = state.model().to_records().unwrap();
    let mut batches = Vec::new();
    for seq in 1..=10u32 {
        let node = [11, 12, 13][seq as usize % 3];
        batches.push(apply(
            &mut state,
            seq,
            vec![Command::rename_node(sid(node), format!("agent {seq}"))],
        ));
    }
    let refs: Vec<&AgentBatchRecord> = batches.iter().collect();
    let plan = plan_revert(&state, &refs, &BTreeSet::new()).unwrap();
    assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);
    assert_eq!(plan.batches.first(), Some(&batch_id(10)));
    run(&mut state, &plan);
    assert_eq!(state.model().to_records().unwrap(), baseline);

    // 변경 단위로 앞 batch만 되돌리면 뒤 batch가 같은 속성을 바꿔 충돌입니다.
    let mut state = DocumentState::new(small_model());
    let first = apply(&mut state, 1, vec![Command::rename_node(sid(11), "a")]);
    let _second = apply(&mut state, 2, vec![Command::rename_node(sid(11), "b")]);
    let plan = plan_revert(&state, &[&first], &BTreeSet::new()).unwrap();
    assert_eq!(plan.conflicts[0].kind, ConflictKind::ChangedLater);
    assert!(plan.is_empty());
}

#[test]
fn failing_apply_after_state_change_has_no_effect() {
    let mut state = DocumentState::new(small_model());
    let batch = apply(&mut state, 1, vec![Command::rename_node(sid(11), "agent")]);
    let plan = plan_revert(&state, &[&batch], &BTreeSet::new()).unwrap();
    user(&mut state, Command::rename_node(sid(12), "later"));
    let before = state.model().to_records().unwrap();
    let revision = state.revision();
    let tx = plan
        .transaction(CommandId::from_stable_id(sid(5000)))
        .unwrap();
    assert!(
        state.execute_transaction(&tx).is_err(),
        "낡은 base revision 거부"
    );
    assert_eq!(state.revision(), revision);
    assert_eq!(state.model().to_records().unwrap(), before);
}

//! 되돌리기 미리보기·적용 test (XIX.6.8, XIX.6.9).

use super::*;
use crate::attached::edit::apply_approved;
use crate::provenance_fixture::*;
use stellar_ops::commands::agent_revert::ConflictKind;
use stellar_ops::commands::change::node_keys;
use stellar_ops::commands::command::Command;
use stellar_ops::commands::id::CommandOrigin;
use stellar_ops::commands::lock::lease::{Lease, LeaseId, SubtreeLock};
use stellar_ops::commands::transaction::CommandTransaction;
use stellar_ops::core_types::property::PropertyPath;

use crate::permission::AgentSessionId;

const L: u64 = 900;
const S: u64 = 700;

fn no_definitions<'a>() -> Definitions<'a> {
    Definitions::Failed("정의 없음")
}

fn lease_table(state: &DocumentState, lease: LeaseId, root: u64) -> LeaseTable {
    let mut table = LeaseTable::new();
    table
        .insert(Lease {
            id: lease,
            subtrees: vec![SubtreeLock::capture(state.model(), sid(root)).unwrap()],
            externals: Vec::new(),
        })
        .unwrap();
    table
}

fn agent_batch(
    provenance: &mut AgentChangeProvenance,
    state: &mut DocumentState,
    n: u64,
    commands: Vec<Command>,
) -> AgentBatchId {
    let tx = proposal(state, lease_id(L), n, commands);
    let applied = apply_approved(state, &tx, &lease_table(state, lease_id(L), 11)).unwrap();
    provenance
        .record_batch(
            AgentSessionId(sid(S)),
            "세션",
            lease_id(L),
            tx,
            applied,
            state.model(),
        )
        .unwrap()
}

fn user(history: &mut TestHistory, n: u64, command: Command) {
    let tx = CommandTransaction::new(id(n), CommandOrigin::User)
        .with_command(history.doc.revision(), command);
    history.doc.execute_transaction(&tx).unwrap();
}

fn ui() -> UserUiAction {
    UserUiAction::from_user_interface()
}

#[test]
fn change_revert_restores_non_conflicting_and_keeps_later_user_edit() {
    let mut history = TestHistory::new(state());
    let mut provenance = AgentChangeProvenance::new();
    let batch = agent_batch(
        &mut provenance,
        &mut history.doc,
        100,
        vec![
            Command::rename_node(sid(13), "agent"),
            Command::set_visibility(sid(14), false),
        ],
    );
    user(&mut history, 200, Command::rename_node(sid(13), "user"));
    let revision = history.doc.revision();
    let table = LeaseTable::new();
    let preview = preview_revert(
        &history.doc,
        &provenance,
        ProvenanceScope::Batch(batch),
        &BTreeSet::new(),
        &table,
        no_definitions,
    )
    .unwrap();
    assert_eq!(
        history.doc.revision(),
        revision,
        "미리보기 전후 revision 동일"
    );
    let inner = preview.preview.as_ref().unwrap();
    assert_eq!(inner.revision, revision);
    assert_eq!(inner.diff.entries.len(), 1, "visibility 하나만 복원");
    assert_eq!(preview.plan.conflicts.len(), 1);
    assert_eq!(
        preview.plan.conflicts[0].key,
        Some(PropertyPath::with_keys(
            sid(13),
            vec![node_keys::DISPLAY_NAME]
        ))
    );
    let applied = apply_revert(
        &preview,
        &mut history,
        &table,
        &mut provenance,
        id(3000),
        ui(),
    )
    .unwrap();
    assert!(!applied.changes.is_empty(), "autosave 입력 change set");
    assert_eq!(name(&history.doc, 13), "user", "이후 사용자 편집 기본 보존");
    assert!(
        history.doc.model().record(sid(14)).unwrap().visible,
        "baseline 일치"
    );
    assert_eq!(history.entries.len(), 1);
    assert_eq!(history.entries[0].kind, "revert");
    assert_eq!(history.entries[0].origin, CommandOrigin::User);
}

#[test]
fn revert_overlapping_active_lease_is_shown_blocked_and_rejected() {
    let mut history = TestHistory::new(state());
    let mut provenance = AgentChangeProvenance::new();
    let batch = agent_batch(
        &mut provenance,
        &mut history.doc,
        100,
        vec![Command::rename_node(sid(13), "agent")],
    );
    let other = lease_table(&history.doc, lease_id(901), 13);
    let revision = history.doc.revision();
    let preview = preview_revert(
        &history.doc,
        &provenance,
        ProvenanceScope::Batch(batch),
        &BTreeSet::new(),
        &other,
        no_definitions,
    )
    .unwrap();
    assert!(preview.blocked_by_lease());
    let error = apply_revert(
        &preview,
        &mut history,
        &other,
        &mut provenance,
        id(3000),
        ui(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        RevertFlowError::Apply(AgentSinkError::Lease(_))
    ));
    assert_eq!(history.doc.revision(), revision);
    assert_eq!(name(&history.doc, 13), "agent");
    assert!(history.entries.is_empty());
}

#[test]
fn session_revert_of_ten_batches_is_one_entry_with_exact_conflicts() {
    let mut history = TestHistory::new(state());
    let mut provenance = AgentChangeProvenance::new();
    for n in 0..8u64 {
        let node = [11, 13, 14][n as usize % 3];
        agent_batch(
            &mut provenance,
            &mut history.doc,
            100 + n,
            vec![Command::rename_node(sid(node), format!("agent {n}"))],
        );
    }
    agent_batch(
        &mut provenance,
        &mut history.doc,
        120,
        vec![Command::set_visibility(sid(14), false)],
    );
    agent_batch(
        &mut provenance,
        &mut history.doc,
        121,
        vec![Command::set_visibility(sid(13), false)],
    );
    assert_eq!(provenance.len(), 10);
    // 사용자가 11 이름을 나중에 바꿉니다(충돌).
    user(&mut history, 200, Command::rename_node(sid(11), "user"));
    let table = LeaseTable::new();
    let preview = preview_revert(
        &history.doc,
        &provenance,
        ProvenanceScope::Session(AgentSessionId(sid(S))),
        &BTreeSet::new(),
        &table,
        no_definitions,
    )
    .unwrap();
    let conflict_keys: Vec<_> = preview
        .plan
        .conflicts
        .iter()
        .map(|c| (c.batch.seq, c.key.clone(), c.kind.clone()))
        .collect();
    let display = PropertyPath::with_keys(sid(11), vec![node_keys::DISPLAY_NAME]);
    // 11 이름을 바꾼 batch는 순번 1, 4, 7입니다. 역순으로 보면 7이 사용자 값과 달라 제외되고, 7이 남으므로 4와 1도
    // 현재 값(사용자 값)과 달라 충돌입니다.
    assert_eq!(
        conflict_keys,
        vec![
            (7, Some(display.clone()), ConflictKind::ChangedLater),
            (4, Some(display.clone()), ConflictKind::ChangedLater),
            (1, Some(display), ConflictKind::ChangedLater),
        ]
    );
    let applied = apply_revert(
        &preview,
        &mut history,
        &table,
        &mut provenance,
        id(3000),
        ui(),
    )
    .unwrap();
    assert_eq!(history.entries.len(), 1, "history entry 1");
    assert_eq!(applied.sequence, history.entries[0].sequence);
    assert_eq!(name(&history.doc, 11), "user");
    assert_eq!(name(&history.doc, 13), "n13");
    assert_eq!(name(&history.doc, 14), "n14");
    assert!(history.doc.model().record(sid(13)).unwrap().visible);
    assert!(history.doc.model().record(sid(14)).unwrap().visible);
}

#[test]
fn failure_at_apply_time_has_no_effect() {
    let mut history = TestHistory::new(state());
    let mut provenance = AgentChangeProvenance::new();
    for n in 0..3u64 {
        agent_batch(
            &mut provenance,
            &mut history.doc,
            100 + n,
            vec![Command::rename_node(sid(13), format!("agent {n}"))],
        );
    }
    let table = LeaseTable::new();
    let scope = ProvenanceScope::Session(AgentSessionId(sid(S)));
    let preview = preview_revert(
        &history.doc,
        &provenance,
        scope,
        &BTreeSet::new(),
        &table,
        no_definitions,
    )
    .unwrap();
    // 미리보기 뒤 문서가 바뀌면 적용하지 않습니다.
    user(&mut history, 200, Command::rename_node(sid(12), "later"));
    let revision = history.doc.revision();
    let error = apply_revert(
        &preview,
        &mut history,
        &table,
        &mut provenance,
        id(3000),
        ui(),
    )
    .unwrap_err();
    assert_eq!(error, RevertFlowError::Apply(AgentSinkError::Stale));
    assert_eq!(history.doc.revision(), revision);
    assert_eq!(name(&history.doc, 13), "agent 2");
    // history가 거부하면 효과 0.
    let preview = preview_revert(
        &history.doc,
        &provenance,
        scope,
        &BTreeSet::new(),
        &table,
        no_definitions,
    )
    .unwrap();
    history.refuse = true;
    assert!(apply_revert(
        &preview,
        &mut history,
        &table,
        &mut provenance,
        id(3000),
        ui()
    )
    .is_err());
    assert_eq!(history.doc.revision(), revision);
    assert_eq!(name(&history.doc, 13), "agent 2");
    assert!(provenance
        .batches()
        .iter()
        .all(|b| b.display() == crate::provenance::DisplayState::PendingReview));
}

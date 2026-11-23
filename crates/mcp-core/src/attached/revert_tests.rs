//! 되돌리기 미리보기·적용 test (XIX.6.8, XIX.6.9).

use super::*;
use crate::attached::edit::apply_approved;
use crate::provenance_fixture::*;
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

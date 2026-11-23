//! 에이전트 세션 완료·중지·종료 절차 test (XIX.6.1~XIX.6.4, XIX.6.6).

use super::*;
use crate::provenance_fixture::*;
use stellar_ops::commands::command::Command;
use stellar_ops::commands::lock::lease::ViolationReason;
use stellar_ops::diff::diff_records;

use crate::provenance_fixture::world::{World, L, S};

#[test]
fn thousand_command_batch_inverse_restores_baseline_semantic_snapshot() {
    let mut world = World::new(SessionOrigin::ExternalClient);
    let baseline = world.history.doc.model().to_records().unwrap();
    let commands: Vec<Command> = (0..1000u32)
        .map(|i| match i % 2 {
            0 => Command::rename_node(sid([13, 14][i as usize % 4 / 2]), format!("이름 {i}")),
            _ => Command::set_visibility(sid(14), i % 4 == 1),
        })
        .collect();
    let batch = world.apply(100, commands);
    let record = &world.provenance.get(batch).unwrap().record;
    assert_eq!(record.inverse.requests.len(), 1000);
    let changed = world.history.doc.model().to_records().unwrap();
    assert!(!diff_records(&[&baseline], &[&changed]).unwrap().is_empty());
    let inverse = record.inverse.rebased(world.history.doc.revision());
    world.history.doc.execute_transaction(&inverse).unwrap();
    let restored = world.history.doc.model().to_records().unwrap();
    assert!(diff_records(&[&baseline], &[&restored]).unwrap().is_empty());
}

#[test]
fn complete_confirms_one_entry_and_releases_lease_for_user_edit() {
    let mut world = World::new(SessionOrigin::ExternalClient);
    world.apply(100, vec![Command::rename_node(sid(13), "a")]);
    world.apply(101, vec![Command::rename_node(sid(14), "b")]);
    let revision = world.history.doc.revision();
    let completed = {
        let (mut ctx, sessions, history) = world.ctx();
        sessions
            .complete(session(S), &mut ctx, history, id(3000))
            .unwrap()
    };
    assert_eq!(completed.batches, 2);
    assert_eq!(world.history.entries.len(), 1, "history entry 1");
    assert_eq!(world.history.entries[0].kind, "completed");
    assert_eq!(
        world.history.entries[0].origin,
        CommandOrigin::FutureAgent(AgentProvenance::new(lease_id(L).0))
    );
    assert_eq!(world.history.doc.revision(), revision);
    assert!(world.table().is_empty(), "lease 해제");
    assert_eq!(world.sessions.lease(session(S)), None);
    assert!(
        world.sessions.is_active(session(S)),
        "세션은 다음 편집을 받음"
    );
    // 완료 후 사용자 편집 가능.
    let user = CommandTransaction::new(id(3100), CommandOrigin::User).with_command(
        world.history.doc.revision(),
        Command::rename_node(sid(13), "user"),
    );
    let table = world.table();
    stellar_ops::commands::lock::lease::execute_leased(&mut world.history.doc, &user, &table)
        .unwrap();
    assert_eq!(name(&world.history.doc, 13), "user");
    let batch = &world.provenance.batches()[0];
    assert_eq!(batch.outcome, BatchOutcome::Completed);
    assert_eq!(
        batch.history_sequence,
        Some(completed.history_sequence.unwrap())
    );
}

#[test]
fn user_edit_inside_lease_is_rejected_before_completion() {
    let mut world = World::new(SessionOrigin::ExternalClient);
    let user = CommandTransaction::new(id(3100), CommandOrigin::User).with_command(
        world.history.doc.revision(),
        Command::rename_node(sid(13), "user"),
    );
    let table = world.table();
    let error =
        stellar_ops::commands::lock::lease::execute_leased(&mut world.history.doc, &user, &table)
            .unwrap_err();
    assert!(matches!(
        error,
        stellar_ops::commands::lock::lease::LeasedExecutionError::Lease(v)
            if v.reason == ViolationReason::Leased(lease_id(L))
    ));
}

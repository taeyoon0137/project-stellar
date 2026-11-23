//! 에이전트 세션 완료·중지·종료 절차 test (XIX.6.1~XIX.6.4, XIX.6.6).

use super::*;
use crate::permission::Confirmation;
use crate::provenance_fixture::*;
use stellar_ops::commands::command::Command;
use stellar_ops::commands::lock::lease::{LeaseTable, ViolationReason};
use stellar_ops::diff::diff_records;
use stellar_ops::validate::Definitions;

use crate::provenance_fixture::world::{World, L, S};

fn no_definitions<'a>() -> Definitions<'a> {
    Definitions::Failed("정의 없음")
}

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

#[test]
fn kill_fixture_keeps_batches_and_clears_lease_permission_and_pending() {
    let mut world = World::new(SessionOrigin::ExternalClient);
    world.apply(100, vec![Command::rename_node(sid(13), "a")]);
    let pending = world
        .permissions
        .request(
            &key(S),
            crate::agent_protocol::permission::Permission::Apply,
        )
        .unwrap();
    let tx = proposal(
        &world.history.doc,
        lease_id(L),
        101,
        vec![Command::rename_node(sid(14), "x")],
    );
    world.sessions.queue_proposal(session(S), tx).unwrap();
    let revision = world.history.doc.revision();

    let report = world.end(EndReason::ProcessExited, 6_000);

    assert_eq!(name(&world.history.doc, 13), "a", "적용 batch 유지");
    assert_eq!(name(&world.history.doc, 14), "n14", "미적용 제안 폐기");
    assert_eq!(world.history.doc.revision(), revision, "자동 rollback 없음");
    assert!(
        world.provenance.shown_nodes().contains(&sid(13)),
        "출처 표시 존재"
    );
    assert!(world.table().is_empty(), "lease 0");
    assert_eq!(world.permissions.connection(&key(S)), None, "permission 0");
    let Confirmation::Pending(request) = pending else {
        panic!("pending");
    };
    assert_eq!(
        report.rejected_requests,
        vec![(
            crate::agent_protocol::permission::Permission::Apply,
            request
        )]
    );
    assert_eq!(report.discarded_proposals, 1);
    assert_eq!(
        world.sessions.pending_proposals(session(S)),
        0,
        "대기 요청 0"
    );
    assert_eq!(world.history.entries.len(), 1, "history entry 1");
    assert_eq!(world.history.entries[0].kind, "interrupted");
    assert_eq!(
        report.history_sequence,
        Some(world.history.entries[0].sequence)
    );
    assert_eq!(
        world.provenance.batches()[0].outcome,
        BatchOutcome::Interrupted
    );
    assert_eq!(
        world.sessions.end_reason(session(S)),
        Some(EndReason::ProcessExited)
    );
    // 끝난 세션은 다시 시작·재개하지 않습니다.
    assert!(matches!(
        world.sessions.start(
            session(S),
            key(S),
            SessionOrigin::ExternalClient,
            "x",
            7_000
        ),
        Err(SessionError::Ended(_, EndReason::ProcessExited))
    ));
}

#[test]
fn end_without_batches_creates_no_entry_and_still_releases() {
    let mut world = World::new(SessionOrigin::ExternalClient);
    let report = world.end(EndReason::TrustRevoked, 2_000);
    assert!(world.history.entries.is_empty());
    assert_eq!(report.history_sequence, None);
    assert_eq!(report.released_lease, Some(lease_id(L)));
    assert!(world.table().is_empty());
}

#[test]
fn history_refusal_still_releases_lease_and_permission() {
    let mut world = World::new(SessionOrigin::ExternalClient);
    world.apply(100, vec![Command::rename_node(sid(13), "a")]);
    world.history.refuse = true;
    let report = world.end(EndReason::AbnormalExit, 2_000);
    assert!(report.history_error.is_some());
    assert!(world.table().is_empty());
    assert_eq!(world.permissions.connection(&key(S)), None);
    assert_eq!(
        world.provenance.batches()[0].outcome,
        BatchOutcome::InProgress,
        "확정 못 한 batch는 진행 중으로 남음"
    );
    assert_eq!(name(&world.history.doc, 13), "a");
    // 끝나지 않은 세션은 받지 않습니다(진행 중 편집은 complete·end 경로).
    let mut active = World::new(SessionOrigin::ExternalClient);
    assert!(matches!(
        active.sessions.confirm_interrupted(
            session(S),
            &mut active.provenance,
            &mut active.history,
            id(5000)
        ),
        Err(SessionError::NotEnded(_))
    ));
    // 다음 확정 기회에 남은 batch를 "중단됨" entry 하나로 확정합니다.
    world.history.refuse = false;
    let revision = world.history.doc.revision();
    let (sequence, count) = world
        .sessions
        .confirm_interrupted(
            session(S),
            &mut world.provenance,
            &mut world.history,
            id(5000),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(world.history.entries.len(), 1);
    assert_eq!(world.history.entries[0].kind, "interrupted");
    assert_eq!(sequence, Some(world.history.entries[0].sequence));
    assert_eq!(world.history.doc.revision(), revision, "다시 실행하지 않음");
    assert_eq!(
        world.provenance.batches()[0].outcome,
        BatchOutcome::Interrupted
    );
    // 남은 batch가 없으면 entry를 만들지 않습니다.
    assert_eq!(
        world
            .sessions
            .confirm_interrupted(
                session(S),
                &mut world.provenance,
                &mut world.history,
                id(5001)
            )
            .unwrap(),
        (None, 0)
    );
    assert_eq!(world.history.entries.len(), 1);
}

#[test]
fn external_client_disconnect_ends_but_app_started_waits_for_grace() {
    let mut external = World::new(SessionOrigin::ExternalClient);
    external.apply(100, vec![Command::rename_node(sid(13), "a")]);
    let report = {
        let (mut ctx, sessions, history) = external.ctx();
        sessions
            .on_bridge_disconnected(session(S), &mut ctx, history, id(4000), 3_000)
            .unwrap()
    };
    let report = report.expect("외부 클라이언트는 연결 종료가 곧 종료");
    assert_eq!(report.reason, EndReason::McpDisconnected);
    assert!(external.table().is_empty());

    let mut app = World::new(SessionOrigin::AppStarted);
    app.apply(100, vec![Command::rename_node(sid(13), "a")]);
    let kept = {
        let (mut ctx, sessions, history) = app.ctx();
        sessions
            .on_bridge_disconnected(session(S), &mut ctx, history, id(4000), 3_000)
            .unwrap()
    };
    assert!(kept.is_none(), "유예 안 세션 유지");
    assert!(app.sessions.is_active(session(S)));
    assert_eq!(app.table().len(), 1, "재연결 중 lease 유지");
    // 유예 초과(AgentSession이 판정) 뒤 같은 종료 결과.
    let report = app.end(EndReason::ReconnectGraceExpired, 30_000);
    assert_eq!(report.interrupted_batches, 1);
    assert!(app.table().is_empty());
    assert_eq!(app.permissions.connection(&key(S)), None);
    assert_eq!(app.history.entries[0].kind, "interrupted");
    assert_eq!(name(&app.history.doc, 13), "a");
}

#[test]
fn cancel_keeps_batches_and_cancel_with_revert_opens_preview_without_change() {
    let mut world = World::new(SessionOrigin::ExternalClient);
    world.apply(100, vec![Command::rename_node(sid(13), "a")]);
    world.apply(101, vec![Command::rename_node(sid(14), "b")]);
    let revision = world.history.doc.revision();
    let (report, preview) = {
        let (mut ctx, sessions, history) = world.ctx();
        sessions
            .cancel_and_preview_revert(
                session(S),
                &mut ctx,
                history,
                id(4000),
                5_000,
                &LeaseTable::new(),
                no_definitions,
            )
            .unwrap()
    };
    assert_eq!(report.reason, EndReason::UserStopped);
    assert_eq!(report.interrupted_batches, 2);
    assert!(world.table().is_empty(), "lease 0");
    assert_eq!(world.history.entries.len(), 1, "중단 history entry 1");
    let preview = preview.expect("세션 되돌리기 미리보기");
    assert_eq!(
        world.history.doc.revision(),
        revision,
        "미리보기 승인 전 revision 변화 0"
    );
    assert_eq!(name(&world.history.doc, 13), "a");
    assert_eq!(preview.plan.restored.len(), 2);
    assert!(!preview.blocked_by_lease());

    // 승인하면 비충돌 속성이 baseline과 같아집니다.
    let table = LeaseTable::new();
    crate::attached::revert::apply_revert(
        &preview,
        &mut world.history,
        &table,
        &mut world.provenance,
        id(4100),
        crate::provenance::UserUiAction::from_user_interface(),
    )
    .unwrap();
    assert_eq!(name(&world.history.doc, 13), "n13");
    assert_eq!(name(&world.history.doc, 14), "n14");
}

#[test]
fn interrupted_diagnostic_has_only_session_duration_and_code() {
    let mut world = World::new(SessionOrigin::ExternalClient);
    world.apply(100, vec![Command::rename_node(sid(13), "비밀 이름")]);
    let report = world.end(EndReason::ProcessExited, 4_500);
    let InterruptedAgentTask {
        session: diagnostic_session,
        duration_ms,
        error_code,
    } = report.diagnostic;
    assert_eq!(diagnostic_session, session(S));
    assert_eq!(duration_ms, 3_500);
    assert_eq!(error_code, "process-exited");
    let text = report.diagnostic.to_string();
    for forbidden in [
        "비밀",
        "디자인 정리",
        "/project",
        &sid(13).to_string(),
        &sid(1).to_string(),
    ] {
        assert!(!text.contains(forbidden), "{text} contains {forbidden}");
    }
}

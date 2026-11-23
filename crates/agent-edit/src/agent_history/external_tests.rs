//! 외부 변경 batch 겹침 판정과 반영 test (XIX.7.1~XIX.7.3).

use super::*;
use crate::agent_history::SessionOrigin;
use crate::provenance::DisplayState;
use crate::provenance_fixture::world::{World, L, S};
use crate::provenance_fixture::*;
use stellar_ops::commands::agent_external::RecordTarget;
use stellar_ops::commands::command::Command;
use stellar_ops::commands::id::CommandOrigin;
use stellar_ops::commands::request::CommandRequest;

/// 원본 문서에 이름 변경을 적용한 외부 원본.
fn external(renames: &[(u64, &str)]) -> DocumentRecords {
    let mut state = state();
    for (node, name) in renames {
        state
            .execute(&CommandRequest::at(
                &state,
                Command::rename_node(sid(*node), *name),
            ))
            .unwrap();
    }
    state.model().to_records().unwrap()
}

fn respond(
    world: &mut World,
    base: &DocumentRecords,
    incoming: &DocumentRecords,
) -> ExternalDecision {
    let table = world.table();
    let (mut ctx, sessions, history) = world.ctx();
    respond_to_external(
        sessions,
        &mut ctx,
        history,
        base,
        incoming,
        &table,
        id(4000),
        9_000,
    )
    .unwrap()
}

#[test]
fn external_batch_semantic_diff_and_lease_overlap() {
    let world = World::new(SessionOrigin::ExternalClient);
    let base = state().model().to_records().unwrap();
    let table = world.table();
    let outside = analyze_external(
        world.history.doc.model(),
        &base,
        &external(&[(12, "x")]),
        &table,
    )
    .unwrap();
    assert_eq!(outside.diff.entries.len(), 1);
    assert!(!outside.overlaps_any(), "non-overlap");
    let inside = analyze_external(
        world.history.doc.model(),
        &base,
        &external(&[(14, "x")]),
        &table,
    )
    .unwrap();
    assert_eq!(
        inside.overlaps.get(&lease_id(L)),
        Some(&BTreeSet::from([sid(14)])),
        "lease root 아래 자손도 subtree 안"
    );
}

#[test]
fn non_overlapping_external_change_is_applied_keeping_agent_work() {
    let mut world = World::new(SessionOrigin::ExternalClient);
    let base = state().model().to_records().unwrap();
    world.apply(100, vec![Command::rename_node(sid(13), "agent")]);
    let decision = respond(&mut world, &base, &external(&[(12, "external")]));
    assert!(decision.ended.is_empty());
    assert!(world.sessions.is_active(session(S)), "세션 유지");
    assert_eq!(world.table().len(), 1, "lease 유지");
    // 반영은 창 session 경로가 합니다. 여기서는 판정 근거의 분할(창에서 안 바뀐 레코드)을 그 경로 대신 적용합니다.
    let tx = decision
        .analysis
        .partition
        .applicable
        .clone()
        .into_transaction(id(4001), world.history.doc.revision());
    assert_eq!(tx.origin, CommandOrigin::System);
    let applied = world.history.doc.execute_transaction(&tx).unwrap();
    world
        .provenance
        .observe(&applied.changes, tx.origin, world.history.doc.model());
    assert_eq!(name(&world.history.doc, 12), "external");
    assert_eq!(name(&world.history.doc, 13), "agent", "agent 작업 유지");
    assert_eq!(
        world.provenance.batches()[0].display(),
        DisplayState::PendingReview
    );
    assert!(decision.analysis.partition.divergent.is_empty());
}

#[test]
fn overlapping_external_change_ends_session_without_deleting_agent_change() {
    let mut world = World::new(SessionOrigin::ExternalClient);
    let base = state().model().to_records().unwrap();
    world.apply(100, vec![Command::rename_node(sid(13), "agent")]);
    let revision = world.history.doc.revision();
    let decision = respond(
        &mut world,
        &base,
        &external(&[(12, "external"), (13, "external")]),
    );
    assert_eq!(decision.ended.len(), 1);
    assert_eq!(
        decision.ended[0].reason,
        crate::agent_history::EndReason::LeaseConflict
    );
    assert!(world.table().is_empty(), "lease 0");
    assert!(!world.sessions.is_active(session(S)));
    assert_eq!(world.history.doc.revision(), revision, "자동 rollback 없음");
    assert_eq!(
        name(&world.history.doc, 13),
        "agent",
        "agent 변경 자동 삭제 0"
    );
    assert_eq!(world.history.entries.len(), 1);
    assert_eq!(world.history.entries[0].kind, "interrupted");
    assert!(
        world.provenance.shown_nodes().contains(&sid(13)),
        "출처 표시 유지"
    );
    // 겹친 속성은 semantic conflict 후보, 겹친 레코드는 reconciliation 입력입니다(판정 근거, 적용은 창 session).
    assert_eq!(
        decision.analysis.partition.divergent,
        vec![RecordTarget::Node(sid(13))]
    );
    let conflicts = &decision.analysis.conflicts;
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(conflicts[0].object, sid(13));
    assert!(conflicts[0].path.is_some());
    assert_eq!(name(&world.history.doc, 13), "agent");
}

#[test]
fn page_and_view_rename_is_not_lease_overlap() {
    // lease root의 진조상(페이지·View) 이름 변경은 사용자 편집 판정과 같이 겹침이 아닙니다(ADR-0100 결정 8).
    let world = World::new(SessionOrigin::ExternalClient);
    let base = state().model().to_records().unwrap();
    let mut incoming = base.clone();
    incoming.definition.pages[0].rename("외부 페이지".to_string());
    incoming.definition.pages[0].views[0].rename("외부 View".to_string());
    let analysis =
        analyze_external(world.history.doc.model(), &base, &incoming, &world.table()).unwrap();
    assert_eq!(analysis.diff.entries.len(), 2, "{:?}", analysis.diff);
    assert!(!analysis.overlaps_any(), "{:?}", analysis.overlaps);
}

#[test]
fn view_root_frame_swap_or_removal_targets_both_roots() {
    let base = state().model().to_records().unwrap();
    let mut swapped = base.clone();
    swapped.definition.pages[0].views[0].root_frame = sid(12);
    assert_eq!(
        changed_view_roots(sid(3), [&base.definition, &swapped.definition]),
        BTreeSet::from([sid(10), sid(12)])
    );
    let mut removed = base.clone();
    removed.definition.pages[0].views.clear();
    assert_eq!(
        changed_view_roots(sid(2), [&base.definition, &removed.definition]),
        BTreeSet::from([sid(10)])
    );
    let mut renamed = base.clone();
    renamed.definition.pages[0].views[0].rename("x".to_string());
    assert!(changed_view_roots(sid(3), [&base.definition, &renamed.definition]).is_empty());
}

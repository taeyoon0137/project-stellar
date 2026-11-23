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

/// 레코드만 있는(Context 없는) 원본.
fn source(records: DocumentRecords) -> DocumentSource {
    DocumentSource {
        records,
        context: Default::default(),
    }
}

/// 창이 마지막으로 맞춘 원본.
fn base_source() -> DocumentSource {
    DocumentSource::of_model(state().model()).unwrap()
}

/// 원본 문서에 이름 변경을 적용한 외부 원본.
fn external(renames: &[(u64, &str)]) -> DocumentSource {
    let mut state = state();
    for (node, name) in renames {
        state
            .execute(&CommandRequest::at(
                &state,
                Command::rename_node(sid(*node), *name),
            ))
            .unwrap();
    }
    DocumentSource::of_model(state.model()).unwrap()
}

fn respond(
    world: &mut World,
    base: &DocumentSource,
    incoming: &DocumentSource,
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
    let base = base_source();
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
    let base = base_source();
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
    let base = base_source();
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
    let analysis = analyze_external(
        world.history.doc.model(),
        &source(base),
        &source(incoming),
        &world.table(),
    )
    .unwrap();
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

/// 원본 문서에 key `theme`(900)와 provider `provider`의 provision(910), 컴포넌트 950 requirement를 둔 Context.
fn context_source(provider: Option<u64>, requirement: bool) -> DocumentSource {
    use stellar_ops::commands::document::context::fixtures::{literal, rgb};
    use stellar_ops::commands::document::context::key::{ContextKey, ContextScope};
    use stellar_ops::commands::document::context::provision::{ContextOwner, ContextProvision};
    use stellar_ops::commands::document::expressions::types::{ExpressionType, TypeSpec};
    let mut document = base_source();
    let set = &mut document.context;
    set.insert_key(
        ContextKey::new(
            sid(900),
            "theme",
            ContextScope::Document(sid(1)),
            TypeSpec::required(ExpressionType::Color),
            literal(901, rgb(1.0, 1.0, 1.0)),
        )
        .unwrap(),
    )
    .unwrap();
    if let Some(provider) = provider {
        set.insert_provision(
            ContextProvision::new(
                sid(910),
                ContextOwner::View(sid(3)),
                sid(provider),
                sid(900),
                literal(911, rgb(0.0, 0.0, 1.0)),
            )
            .unwrap(),
        )
        .unwrap();
    }
    if requirement {
        set.add_requirement(sid(950), sid(900));
    }
    document
}

#[test]
fn external_context_changes_are_diffed_partitioned_and_checked_for_lease_overlap() {
    // 검증 기준(ADR-0123 결정 3): 외부 변경 분석이 Context도 비교합니다. provision은 provider node 변경이라 lease
    // subtree 안이면 겹치고, key·requirement는 겹치지 않습니다.
    let world = World::new(SessionOrigin::ExternalClient);
    let base = base_source();
    let table = world.table();
    let current = world.history.doc.model();
    let outside =
        analyze_external(current, &base, &context_source(Some(12), true), &table).unwrap();
    let kinds: Vec<&str> = outside.diff.entries.iter().map(|e| e.kind).collect();
    assert_eq!(outside.diff.entries.len(), 3, "{kinds:?}");
    assert!(!outside.overlaps_any(), "{:?}", outside.overlaps);
    assert_eq!(outside.partition.applicable.changes.len(), 1);
    assert!(outside.partition.divergent.is_empty());
    let inside =
        analyze_external(current, &base, &context_source(Some(14), false), &table).unwrap();
    assert_eq!(
        inside.overlaps.get(&lease_id(L)),
        Some(&BTreeSet::from([sid(14)])),
        "lease subtree 안 provider의 provision"
    );
}

/// `document`의 key 900 이름을 `name`으로 바꾼 Context 사본.
fn key_renamed(document: &DocumentSource, name: &str) -> DocumentSource {
    use stellar_ops::commands::document::context::key::ContextKey;
    let mut out = document.clone();
    let key = out.context.key(sid(900)).unwrap().clone();
    let _ = out.context.remove_key(sid(900));
    out.context
        .insert_key(
            ContextKey::new(
                key.id(),
                name,
                key.scope(),
                key.value_type(),
                key.default_expression().ast().clone(),
            )
            .unwrap(),
        )
        .unwrap();
    out
}

#[test]
fn context_changed_on_both_sides_goes_to_reconciliation_with_a_conflict_candidate() {
    let mut world = World::new(SessionOrigin::ExternalClient);
    let base = context_source(None, false);
    // 창: key 이름을 brand로 바꾼 Context(저장 전).
    let current = key_renamed(&base, "brand").into_model().unwrap();
    world.history.doc = stellar_ops::commands::state::DocumentState::new(current);
    // 외부: 같은 key 이름을 accent로 바꾸고 requirement를 더함.
    let mut incoming = key_renamed(&base, "accent");
    incoming.context.add_requirement(sid(950), sid(900));
    let decision = respond(&mut world, &base, &incoming);
    assert!(decision.apply.is_none(), "Context를 덮어쓰지 않음");
    assert_eq!(decision.divergent, vec![RecordTarget::Context]);
    let conflicts = &decision.analysis.conflicts;
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(conflicts[0].object, sid(900));
}

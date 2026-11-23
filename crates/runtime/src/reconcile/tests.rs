//! 열린 창 외부 변경 reconciliation 시험(XIII.3.4): 겹치지 않는 변경 자동 적용, 겹침 conflict, base 없음.

use std::path::Path;

use stellar_commands::id::CommandId;
use stellar_core_types::ids::StableId;
use stellar_ops::apply::{apply, ApplyMode, ApplyRequest, CommandInput};
use stellar_ops::lock::LockPolicy;
use stellar_ops::snapshot::open_snapshot;
use stellar_project::window_session::{StaleResolution, StaleSaveChoice, WriterProvenance};
use stellar_storage::chunk::path::ChunkObjectPath;
use stellar_storage::generation::loader::load_current_generation;

use super::ReconcileBlock;
use crate::window_session::test_fixtures::*;
use crate::window_session::{ExternalSync, ProjectWindowSession, StaleResolved};

fn command(n: u64) -> CommandId {
    CommandId::from_stable_id(StableId::new_v7(80_000 + n, 0x0e0e, n))
}

/// `stellar apply`(headless ops)로 node 이름을 바꿉니다. 외부 writer입니다.
fn cli_rename(project: &TempDir, locks: &TempDir, node: u64, name: &str) {
    let request = ApplyRequest {
        base_revision: open_snapshot(project.path()).unwrap().revision(),
        document: sid(1),
        commands: vec![CommandInput::Rename {
            node: sid(node),
            name: name.to_string(),
        }],
    };
    apply(
        project.path(),
        &request,
        ApplyMode::Write,
        &LockPolicy::Required {
            dir: locks.path().to_path_buf(),
        },
    )
    .unwrap();
}

fn name_of(session: &ProjectWindowSession, node: u64) -> String {
    session
        .document()
        .unwrap()
        .model()
        .record(sid(node))
        .unwrap()
        .display_name
}

fn disk_name(root: &Path, node: u64) -> String {
    open_snapshot(root)
        .unwrap()
        .document_records(sid(1))
        .unwrap()
        .nodes
        .into_iter()
        .find(|n| n.id == sid(node))
        .unwrap()
        .display_name
}

/// 창을 열고 node 12 이름을 바꿉니다(미저장).
fn dirty_window(project: &TempDir) -> ProjectWindowSession {
    let mut window_session =
        ProjectWindowSession::load(identity(project.path()), window(2)).unwrap();
    window_session
        .apply(
            &rename(window_session.document().unwrap(), 2, 12, "창 편집"),
            0,
        )
        .unwrap();
    window_session
}

#[test]
fn merged_result_is_one_undo_step_and_saves_both_changes() {
    let project = saved_project("reconcile-undo");
    let locks = TempDir::new("reconcile-undo-locks");
    let mut session = dirty_window(&project);
    let entries = session.history().history().len();
    cli_rename(&project, &locks, 11, "CLI 편집");
    assert!(matches!(
        session.sync_external(command(1)),
        ExternalSync::Applied { .. }
    ));
    assert_eq!(
        session.history().history().len(),
        entries + 1,
        "반영 항목 하나"
    );
    assert_eq!(name_of(&session, 11), "CLI 편집");
    assert_eq!(name_of(&session, 12), "창 편집");
    // 저장하면 디스크는 두 변경을 모두 담습니다(외부 변경을 덮어쓰지 않음).
    let dir = locks.path().to_path_buf();
    assert!(session.save_with_lock_dir(move || Ok(dir)).unwrap().written);
    assert_eq!(disk_name(project.path(), 11), "CLI 편집");
    assert_eq!(disk_name(project.path(), 12), "창 편집");
    // Undo 한 번이 병합 반영 전체를 되돌립니다(로컬 편집만 남음).
    assert!(session.undo().unwrap());
    assert_eq!(name_of(&session, 11), "n11");
    assert_eq!(name_of(&session, 12), "창 편집");
}

#[test]
fn identical_change_on_both_sides_leaves_the_window_clean() {
    let project = saved_project("reconcile-same");
    let locks = TempDir::new("reconcile-same-locks");
    let mut session = dirty_window(&project);
    cli_rename(&project, &locks, 12, "창 편집");
    assert!(matches!(
        session.sync_external(command(1)),
        ExternalSync::Applied { .. }
    ));
    assert!(!session.has_unsaved_changes(), "결과가 디스크와 같음");
    assert_eq!(session.stale_state().unwrap(), None);
}

#[test]
fn overlapping_change_keeps_the_document_and_offers_the_two_choices() {
    let project = saved_project("reconcile-overlap");
    let locks = TempDir::new("reconcile-overlap-locks");
    let mut session = dirty_window(&project);
    cli_rename(&project, &locks, 12, "CLI 편집");
    let before = session.history().history().len();
    assert!(matches!(
        session.sync_external(command(1)),
        ExternalSync::Stale {
            writer: WriterProvenance::External,
            resolution: StaleResolution::Choose,
        }
    ));
    assert_eq!(name_of(&session, 12), "창 편집", "문서 그대로");
    assert_eq!(session.history().history().len(), before);
    let conflict = session.external_conflict().unwrap();
    assert_eq!(conflict.reason, ReconcileBlock::Conflicts);
    assert_eq!(conflict.conflicts.len(), 1);
    assert_eq!(conflict.conflicts[0].object, sid(12));
    assert_eq!(
        session.stale_state().unwrap(),
        Some((WriterProvenance::External, StaleResolution::Choose))
    );
    // 다른 외부 변경이 또 오면 새 관찰값이라 다시 reconciliation을 기다립니다.
    cli_rename(&project, &locks, 11, "또 CLI");
    assert_eq!(
        session.stale_state().unwrap(),
        Some((
            WriterProvenance::External,
            StaleResolution::ReconciliationPending
        ))
    );
    assert!(session.external_conflict().is_none());
    assert!(matches!(
        session.sync_external(command(2)),
        ExternalSync::Stale {
            resolution: StaleResolution::Choose,
            ..
        }
    ));
    // 두 선택지 가운데 UseLatestDisk는 디스크를 받습니다.
    assert!(matches!(
        session.resolve_stale(StaleSaveChoice::UseLatestDisk),
        Ok(StaleResolved::Reloaded { .. })
    ));
    assert_eq!(name_of(&session, 12), "CLI 편집");
    assert_eq!(name_of(&session, 11), "또 CLI");
    assert!(session.external_conflict().is_none());
}

#[test]
fn use_current_window_after_a_conflict_writes_the_window_document() {
    let project = saved_project("reconcile-overlap-current");
    let locks = TempDir::new("reconcile-overlap-current-locks");
    let mut session = dirty_window(&project);
    cli_rename(&project, &locks, 12, "CLI 편집");
    session.sync_external(command(1));
    assert!(matches!(
        session.resolve_stale(StaleSaveChoice::UseCurrentWindow),
        Ok(StaleResolved::Saved(_))
    ));
    assert_eq!(disk_name(project.path(), 12), "창 편집");
    assert!(!session.has_unsaved_changes());
}

#[test]
fn missing_base_generation_is_not_merged() {
    let project = saved_project("reconcile-no-base");
    let locks = TempDir::new("reconcile-no-base-locks");
    let mut session = dirty_window(&project);
    let base = load_current_generation(project.path(), project_id()).unwrap();
    cli_rename(&project, &locks, 11, "CLI 편집");
    // base generation에만 있던 chunk를 지웁니다(GC 등으로 base를 더 읽을 수 없음).
    let current = load_current_generation(project.path(), project_id()).unwrap();
    for pointer in &base.manifest.chunk_pointers {
        if !current.manifest.chunk_pointers.contains(pointer) {
            let path = ChunkObjectPath::for_hash(&pointer.content_hash).to_relative_path();
            std::fs::remove_file(project.path().join(path)).unwrap();
        }
    }
    assert!(matches!(
        session.sync_external(command(1)),
        ExternalSync::Stale {
            resolution: StaleResolution::Choose,
            ..
        }
    ));
    assert!(matches!(
        session.external_conflict().unwrap().reason,
        ReconcileBlock::BaseUnavailable(_)
    ));
    assert_eq!(name_of(&session, 11), "n11", "문서 그대로");
    assert_eq!(name_of(&session, 12), "창 편집");
}

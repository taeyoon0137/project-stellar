//! 두 창 stale 저장의 두 선택(XII.7.1·XII.7.5·XII.7.6)과 외부 변경 반영(UX.4.1·UX.4.4) 시험.
//!
//! XVI.11.6 시나리오(같은 프로젝트 두 창 → 첫 창 저장 → 두 번째 창 저장 stale → 두 선택)를 셸 없이 코어로
//! 실행합니다.

use super::test_fixtures::*;
use super::*;
use std::path::Path;
use stellar_commands::document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_ops::apply::{apply, ApplyMode, ApplyRequest, CommandInput};
use stellar_ops::lock::LockPolicy;
use stellar_ops::snapshot::open_snapshot;
use stellar_storage::generation::current::write_current_pointer;

fn json(session: &ProjectWindowSession) -> String {
    model_snapshot_json(session.document().unwrap().model()).unwrap()
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

fn current_bytes(root: &Path) -> Vec<u8> {
    std::fs::read(stellar_storage::generation::current::current_pointer_path(
        root,
    ))
    .unwrap()
}

/// 디스크 current에서 새로 연 창(디스크 내용 확인용).
fn reopened(project: &TempDir) -> ProjectWindowSession {
    ProjectWindowSession::load(identity(project.path()), window(99)).unwrap()
}

fn command(n: u64) -> CommandId {
    CommandId::from_stable_id(StableId::new_v7(70_000 + n, 0x0e0e, n))
}

/// 잠금 디렉터리를 `locks`로 고정한 저장.
fn save(session: &mut ProjectWindowSession, locks: &TempDir) -> SaveOutcome {
    let dir = locks.path().to_path_buf();
    session.save_with_lock_dir(move || Ok(dir)).unwrap()
}

/// `stellar apply`(headless ops)로 node 이름을 바꿉니다. 외부 writer입니다.
fn cli_rename(project: &TempDir, locks: &TempDir, node: u64, name: &str) {
    let snapshot = open_snapshot(project.path()).unwrap();
    let request = ApplyRequest {
        base_revision: snapshot.revision(),
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

/// 두 창을 열고 A는 11, B는 12의 이름을 바꾼 뒤 A만 저장합니다.
fn two_windows(tag: &str) -> (TempDir, TempDir, ProjectWindowSession, ProjectWindowSession) {
    let project = saved_project_with(tag, &[&sample_document(), &other_document()]);
    let locks = TempDir::new(&format!("{tag}-locks"));
    let mut a = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
    let mut b = ProjectWindowSession::load(identity(project.path()), window(2)).unwrap();
    a.apply(&rename(a.document().unwrap(), 1, 11, "A 편집"), 0)
        .unwrap();
    b.apply(&rename(b.document().unwrap(), 2, 12, "B 편집"), 0)
        .unwrap();
    assert!(save(&mut a, &locks).written);
    (project, locks, a, b)
}

#[test]
fn second_window_keeps_its_base_and_its_save_is_stale() {
    // XII.7.1: 각 창은 load 당시 base generation·hash를 유지하고 save 전에 디스크와 비교합니다.
    let (project, locks, a, mut b) = two_windows("stale-base");
    let base_b = b.loaded_generation().unwrap().manifest_hash;
    assert_eq!(a.generation_number(), 2);
    assert_eq!(b.generation_number(), 1, "B의 base는 열 때의 generation 1");
    assert_ne!(a.loaded_generation().unwrap().manifest_hash, base_b);
    // XII.2.4: A의 편집·저장은 B 메모리 문서에 나타나지 않습니다.
    assert_eq!(name_of(&b, 11), "n11");
    assert_eq!(
        b.stale_state().unwrap(),
        Some((WriterProvenance::AppWindow, StaleResolution::Choose))
    );
    let before = current_bytes(project.path());
    let dir = locks.path().to_path_buf();
    assert!(matches!(
        b.save_with_lock_dir(move || Ok(dir)),
        Err(WindowSessionSaveError::Stale(_))
    ));
    assert_eq!(
        current_bytes(project.path()),
        before,
        "stale 저장은 쓰지 않음"
    );
    assert_eq!(b.loaded_generation().unwrap().manifest_hash, base_b);
    assert!(b.has_unsaved_changes(), "편집은 메모리에 남음");
    assert_eq!(name_of(&b, 12), "B 편집");
    assert!(a.stale_state().unwrap().is_none(), "A는 디스크와 같음");
}

#[test]
fn use_current_window_writes_the_window_snapshot_over_the_latest_disk() {
    // XII.7.5: 선택 후 디스크가 창 snapshot과 같고, history는 남습니다.
    let (project, locks, mut a, mut b) = two_windows("use-current");
    let dir = locks.path().to_path_buf();
    let resolved = b
        .resolve_stale_with_lock_dir(StaleSaveChoice::UseCurrentWindow, move || Ok(dir))
        .unwrap();
    assert!(matches!(
        resolved,
        StaleResolved::Saved(SaveOutcome {
            generation: 3,
            written: true,
            ..
        })
    ));
    let disk = reopened(&project);
    assert_eq!(
        json(&disk),
        json(&b),
        "디스크 문서 = 창 snapshot(A 편집은 덮임)"
    );
    assert_eq!(name_of(&disk, 11), "n11");
    assert_eq!(name_of(&disk, 12), "B 편집");
    assert!(!b.has_unsaved_changes());
    assert!(b.stale_state().unwrap().is_none());
    assert_eq!(
        b.history().history().len(),
        1,
        "UseCurrentWindow는 history를 유지"
    );
    // 다른 root 문서는 최신 generation 것을 그대로 잇습니다.
    let snapshot = open_snapshot(project.path()).unwrap();
    assert_eq!(snapshot.documents().len(), 2);
    assert_eq!(snapshot.document_records(sid(500)).unwrap(), {
        let mut records = other_document().to_records().unwrap();
        records.sort_canonical();
        records
    });
    // 이제 A가 stale이고(앱 창 writer) 두 선택지를 받습니다.
    assert_eq!(
        a.stale_state().unwrap(),
        Some((WriterProvenance::AppWindow, StaleResolution::Choose))
    );
    // 다시 고를 것이 없으면 NotStale입니다.
    assert!(matches!(
        b.resolve_stale(StaleSaveChoice::UseCurrentWindow),
        Err(StaleResolveError::NotStale)
    ));
    a.undo().unwrap();
}

#[test]
fn use_current_window_rechecks_the_base_just_before_commit() {
    // ADR-0083 결정 1: 최신 generation을 읽은 뒤 commit 전에 다른 저장이 끝나면 다시 stale입니다.
    let (project, locks, mut a, mut b) = two_windows("use-current-race");
    a.apply(&rename(a.document().unwrap(), 3, 11, "A 두 번째"), 0)
        .unwrap();
    let dir = locks.path().to_path_buf();
    let a_dir = dir.clone();
    let mut racing = Some(&mut a);
    let result = b.resolve_stale_with_lock_dir(StaleSaveChoice::UseCurrentWindow, move || {
        // B가 최신 generation(2)을 읽은 뒤, 잠금을 잡기 전에 A가 generation 3을 씁니다.
        let a = racing.take().unwrap();
        assert!(a.save_with_lock_dir(move || Ok(a_dir)).unwrap().written);
        Ok(dir)
    });
    assert!(matches!(
        result,
        Err(StaleResolveError::Save(WindowSessionSaveError::Stale(_)))
    ));
    let disk = reopened(&project);
    assert_eq!(name_of(&disk, 11), "A 두 번째", "B는 아무것도 쓰지 않음");
    assert_eq!(name_of(&disk, 12), "n12");
    assert_eq!(b.generation_number(), 1, "B base는 그대로");
    assert!(b.has_unsaved_changes());
    assert!(b.stale_state().unwrap().is_some(), "다시 stale");
}

#[test]
fn use_current_window_does_not_overwrite_an_external_save_after_the_judgement() {
    // ADR-0083 결정 1·2: 두 선택지는 앱 창 writer로 판정한 generation 위에서만 씁니다. 판정 뒤 CLI가 저장했으면
    // 그 generation을 덮어쓰지 않고 다시 stale이며, 다시 판정하면 reconciliation 대기입니다.
    let (project, locks, _a, mut b) = two_windows("use-current-external-after-judgement");
    let judged = read_current_pointer(project.path()).unwrap();
    assert_eq!(
        b.stale_state().unwrap(),
        Some((WriterProvenance::AppWindow, StaleResolution::Choose))
    );
    cli_rename(&project, &locks, 11, "CLI 편집");
    let before = current_bytes(project.path());
    let dir = locks.path().to_path_buf();
    assert!(matches!(
        b.save_over_latest_with_lock_dir(judged, move || Ok(dir)),
        Err(WindowSessionSaveError::Stale(_))
    ));
    assert_eq!(
        current_bytes(project.path()),
        before,
        "CLI 저장을 덮지 않음"
    );
    assert_eq!(name_of(&reopened(&project), 11), "CLI 편집");
    assert!(b.has_unsaved_changes());
    assert_eq!(b.generation_number(), 1, "B base는 그대로");
    assert!(matches!(
        b.resolve_stale(StaleSaveChoice::UseCurrentWindow),
        Err(StaleResolveError::ReconciliationRequired)
    ));
}

#[test]
fn use_latest_disk_discards_local_edits_and_history() {
    // XII.7.6: reload 후 local changes·history 없음, 문서 = 최신 디스크.
    let (project, _locks, _a, mut b) = two_windows("use-latest");
    b.select(Selection::new([sid(11)], None)).unwrap();
    let revision_before = b.revision();
    let resolved = b.resolve_stale(StaleSaveChoice::UseLatestDisk).unwrap();
    assert_eq!(
        resolved,
        StaleResolved::Reloaded {
            generation: 2,
            discarded_history_entries: 2,
        }
    );
    let disk = reopened(&project);
    assert_eq!(json(&b), json(&disk));
    assert_eq!(name_of(&b, 11), "A 편집");
    assert_eq!(name_of(&b, 12), "n12", "로컬 편집은 버림");
    assert!(!b.has_unsaved_changes());
    assert!(b.history().history().is_empty());
    assert_eq!(b.history().history().redo_len(), 0);
    assert!(
        b.view().selection().is_empty(),
        "선택도 history와 함께 버림"
    );
    assert!(
        b.revision() > revision_before,
        "revision은 앞으로만 감(캐시 무효화)"
    );
    assert_eq!(b.tabs().len(), 1, "열린 탭 유지");
    assert!(b.stale_state().unwrap().is_none());
}

#[test]
fn use_latest_disk_keeps_memory_edits_when_the_latest_generation_does_not_load() {
    // ADR-0083 결정 1: 최신 snapshot을 완전히 로드하지 못하면 메모리 편집·history를 유지합니다.
    let (project, _locks, _a, mut b) = two_windows("use-latest-broken");
    let before = json(&b);
    write_current_pointer(
        project.path(),
        &ContentHash::compute(b"manifest that does not exist"),
    )
    .unwrap();
    assert!(b.reload_latest_disk().is_err());
    assert_eq!(json(&b), before);
    assert_eq!(b.history().history().len(), 1);
    assert!(b.has_unsaved_changes());
    assert_eq!(b.generation_number(), 1);
}

#[test]
fn external_writer_over_dirty_window_is_reconciled_when_changes_do_not_overlap() {
    // XIII.3.4(ADR-0110 결정 1): 외부 writer(CLI)와 dirty가 겹치면 두 선택지 대신 semantic reconciliation입니다.
    // 서로 다른 node 변경은 자동 병합되고, 결과는 새 base 위의 미저장 편집입니다. 판정만으로는 쓰지 않습니다.
    let project = saved_project("external-dirty");
    let locks = TempDir::new("external-dirty-locks");
    let mut b = ProjectWindowSession::load(identity(project.path()), window(2)).unwrap();
    b.apply(&rename(b.document().unwrap(), 2, 12, "B 편집"), 0)
        .unwrap();
    cli_rename(&project, &locks, 11, "CLI 편집");
    assert_eq!(
        b.stale_state().unwrap(),
        Some((
            WriterProvenance::External,
            StaleResolution::ReconciliationPending
        ))
    );
    let before = current_bytes(project.path());
    for choice in StaleSaveChoice::ALL {
        assert!(matches!(
            b.resolve_stale(choice),
            Err(StaleResolveError::ReconciliationRequired)
        ));
    }
    assert!(matches!(
        b.sync_external(command(1)),
        ExternalSync::Applied {
            generation: 2,
            changes: Some(_)
        }
    ));
    assert_eq!(current_bytes(project.path()), before, "반영은 쓰지 않음");
    assert_eq!(name_of(&b, 12), "B 편집");
    assert_eq!(name_of(&b, 11), "CLI 편집");
    assert!(b.has_unsaved_changes(), "결과는 디스크와 달라 미저장 편집");
    assert_eq!(b.stale_state().unwrap(), None, "새 base 위라 stale 아님");
    assert!(b.external_conflict().is_none());
}

#[test]
fn clean_window_applies_a_cli_apply_as_one_undoable_command() {
    // UX.4.1: clean 창은 외부 CLI `stellar apply`를 ApplyExternalSnapshot 하나로 반영하고 Undo 한 번으로 되돌립니다.
    let project = saved_project("external-clean");
    let locks = TempDir::new("external-clean-locks");
    let mut b = ProjectWindowSession::load(identity(project.path()), window(2)).unwrap();
    b.select(Selection::new([sid(11), sid(12)], Some(sid(12))))
        .unwrap();
    let original = json(&b);
    cli_rename(&project, &locks, 11, "CLI 편집");

    let ExternalSync::Applied {
        generation,
        changes: Some(changes),
    } = b.sync_external(command(1))
    else {
        panic!("clean 창은 반영해야 함");
    };
    assert_eq!(generation, 2);
    assert_eq!(changes.changed_nodes(), vec![sid(11)], "cache 무효화 입력");
    assert_eq!(name_of(&b, 11), "CLI 편집");
    assert!(!b.has_unsaved_changes(), "반영 결과는 디스크와 같음");
    assert_eq!(b.view().selection().ids(), &[sid(11), sid(12)], "선택 보존");
    let entry = b.history().history().entries().last().unwrap();
    assert_eq!(entry.action_name(), "외부 변경 반영");

    // 자기 저장·같은 generation은 다시 반영하지 않습니다(hash 비교).
    assert!(matches!(
        b.sync_external(command(2)),
        ExternalSync::Unchanged
    ));
    // 새로 연 창도 같은 의미입니다(그 열기가 원본을 cache에 맞춰 쓰므로 위 확인 뒤에 엽니다, ADR-0088).
    assert_eq!(json(&b), json(&reopened(&project)));

    // Undo 한 번이 외부 변경 전체를 되돌립니다. 결과는 디스크와 다르므로 dirty입니다.
    assert!(b.undo().unwrap());
    assert_eq!(json(&b), original);
    assert!(b.has_unsaved_changes());
    assert!(b.redo().unwrap());
    assert_eq!(name_of(&b, 11), "CLI 편집");
    assert_eq!(
        json(&b),
        json(&reopened(&project)),
        "Redo 결과는 디스크와 같은 내용"
    );
}

#[test]
fn own_save_is_not_applied_again() {
    let project = saved_project("external-own");
    let locks = TempDir::new("external-own-locks");
    let mut b = ProjectWindowSession::load(identity(project.path()), window(2)).unwrap();
    b.apply(&rename(b.document().unwrap(), 2, 12, "B 편집"), 0)
        .unwrap();
    assert!(save(&mut b, &locks).written);
    let history = b.history().history().len();
    assert!(matches!(
        b.sync_external(command(1)),
        ExternalSync::Unchanged
    ));
    assert_eq!(b.history().history().len(), history);
}

#[test]
fn broken_external_generation_is_rejected_without_changes() {
    // UX.4.4: 깨진 외부 generation은 반영 0이고 문서·base·history를 바꾸지 않습니다.
    let project = saved_project("external-broken");
    let mut b = ProjectWindowSession::load(identity(project.path()), window(2)).unwrap();
    let before = json(&b);
    write_current_pointer(project.path(), &ContentHash::compute(b"no such manifest")).unwrap();
    assert!(matches!(
        b.sync_external(command(1)),
        ExternalSync::Rejected(ExternalRejection::Load(_))
    ));
    assert_eq!(json(&b), before);
    assert_eq!(b.generation_number(), 1);
    assert!(b.history().history().is_empty());
}

#[test]
fn external_change_waits_for_the_gesture_to_end() {
    // UX.4.1: 제스처 중에는 반영하지 않고, 끝난 뒤 반영합니다.
    let project = saved_project("external-gesture");
    let locks = TempDir::new("external-gesture-locks");
    let mut b = ProjectWindowSession::load(identity(project.path()), window(2)).unwrap();
    cli_rename(&project, &locks, 11, "CLI 편집");
    b.history.begin_pointer_gesture(1).unwrap();
    assert!(matches!(
        b.sync_external(command(1)),
        ExternalSync::Deferred
    ));
    assert_eq!(name_of(&b, 11), "n11");
    b.history
        .cancel_preview(&mut b.document.as_mut().unwrap().state)
        .unwrap();
    assert!(matches!(
        b.sync_external(command(1)),
        ExternalSync::Applied {
            changes: Some(_),
            ..
        }
    ));
    assert_eq!(name_of(&b, 11), "CLI 편집");
}

#[test]
fn worker_write_on_an_outdated_base_is_discarded_without_switching_the_pointer() {
    // XIII.1.4: 직렬화·쓰기는 worker에서 하고, base가 오래된 결과는 current pointer로 전환하지 않습니다.
    let project = saved_project("worker-stale-discard");
    let locks = TempDir::new("worker-stale-discard-locks");
    let mut w = ProjectWindowSession::load(identity(project.path()), window(5)).unwrap();
    w.apply(&rename(w.document().unwrap(), 1, 11, "첫 편집"), 0)
        .unwrap();
    let first = w.capture_save().unwrap().unwrap();
    // 같은 base(generation 1) 위의 둘째 캡처. 첫 쓰기가 먼저 끝나면 이 쓰기의 base는 오래됩니다.
    w.apply(&rename(w.document().unwrap(), 2, 12, "둘째 편집"), 0)
        .unwrap();
    let second = w.capture_save().unwrap().unwrap();
    let dir = locks.path().to_path_buf();
    let (written, timings) = std::thread::spawn(move || first.write_timed(move || Ok(dir)))
        .join()
        .unwrap();
    assert!(timings.write > std::time::Duration::ZERO);
    let outcome = w.apply_save(written.unwrap());
    assert!(outcome.written);
    let after_first = current_bytes(project.path());
    let dir = locks.path().to_path_buf();
    let stale = std::thread::spawn(move || second.write(move || Ok(dir)))
        .join()
        .unwrap();
    assert!(matches!(stale, Err(WindowSessionSaveError::Stale(_))));
    assert_eq!(
        current_bytes(project.path()),
        after_first,
        "오래된 base 결과는 current pointer를 바꾸지 않음"
    );
    // 둘째 편집은 dirty로 남아 다음 저장이 새 base 위에서 씁니다.
    assert!(w.has_unsaved_changes());
    assert!(save(&mut w, &locks).written);
    assert_eq!(name_of(&reopened(&project), 12), "둘째 편집");
}

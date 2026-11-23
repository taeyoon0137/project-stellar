//! 앱 runtime의 stale 알림·선택(XII.7.7), 외부 변경 감시 반영(UX.4.1·UX.4.4), 닫기·Git UI flush(XIII.1.6·XIV.2.8),
//! 자동 저장 trace(XIII.1.7) 시험.

use super::tests::next_solicited;
use super::*;
use crate::app_window_tests::{open_project, open_window};
use crate::window_session::test_fixtures::{rename, saved_project, sid, TempDir};
use crate::window_session::SaveOutcome;
use stellar_ops::apply::{apply, ApplyMode, ApplyRequest, CommandInput};
use stellar_ops::lock::LockPolicy;
use stellar_ops::snapshot::open_snapshot;
use stellar_project::autosave::trace::validate_trace_line;
use stellar_project::autosave::FlushPolicy;
use stellar_storage::generation::current::{current_pointer_path, write_current_pointer};
use stellar_storage::hash::ContentHash;

fn policy(debounce_ms: u64, max_delay_ms: u64) -> AutosavePolicy {
    AutosavePolicy {
        debounce: Duration::from_millis(debounce_ms),
        max_delay: Duration::from_millis(max_delay_ms),
        app_deactivate_flush: FlushPolicy {
            timeout: Duration::from_millis(500),
        },
        window_close_flush: FlushPolicy {
            timeout: Duration::from_millis(5_000),
        },
    }
}

fn edit(app: &AppRuntime, session: WindowSessionHandle, command: u64, node: u64, name: &str) {
    let name = name.to_string();
    let request = app
        .submit(move |ctx, request| {
            let window = window_mut(ctx, session)?;
            let tx = rename(window.document().unwrap(), command, node, &name);
            window.apply(&tx, 0).unwrap();
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
        .unwrap();
    assert!(matches!(
        next_solicited(app),
        RuntimeEvent::WindowViewChanged { request: got, .. } if got == request
    ));
}

fn name_of(app: &AppRuntime, session: WindowSessionHandle, node: u64) -> String {
    app.job(move |ctx: &mut DocumentContext| {
        ctx.app_sessions().windows[&session]
            .session
            .document()
            .unwrap()
            .model()
            .record(sid(node))
            .unwrap()
            .display_name
    })
    .unwrap()
    .wait()
    .unwrap()
}

fn generation(app: &AppRuntime, session: WindowSessionHandle) -> u64 {
    app.state().window(session).unwrap().generation
}

fn dirty(app: &AppRuntime, session: WindowSessionHandle) -> bool {
    app.state().window(session).unwrap().has_unsaved_changes
}

fn status(app: &AppRuntime, session: WindowSessionHandle) -> AutosaveStatus {
    app.state().window(session).unwrap().autosave
}

fn current(project: &TempDir) -> Vec<u8> {
    std::fs::read(current_pointer_path(project.path())).unwrap()
}

/// `until`이 참이 될 때까지 최대 10초 기다립니다.
fn wait_for(mut until: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if until() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    until()
}

fn alerts(app: &AppRuntime) -> Vec<StaleAlert> {
    app.take_stale_alerts().unwrap().wait().unwrap()
}

/// 외부 writer(`stellar apply`)로 node 이름을 바꿉니다.
fn cli_rename(project: &TempDir, locks: &TempDir, node: u64, name: &str) {
    let snapshot = open_snapshot(project.path()).unwrap();
    apply(
        project.path(),
        &ApplyRequest {
            base_revision: snapshot.revision(),
            document: sid(1),
            commands: vec![CommandInput::Rename {
                node: sid(node),
                name: name.to_string(),
            }],
        },
        ApplyMode::Write,
        &LockPolicy::Required {
            dir: locks.path().to_path_buf(),
        },
    )
    .unwrap();
}

/// 두 창: A가 자동 저장으로 먼저 쓰고, B의 자동 저장이 stale로 멈춘 상태.
///
/// 두 창 모두 generation 1 위에서 편집한 뒤 flush합니다. flush는 창 순서대로 캡처하고 writer는 받은 순서대로
/// 쓰므로 A가 먼저 씁니다(clean 창은 다른 창의 저장을 외부 변경으로 받아들이므로 B는 A가 쓰기 전에 편집).
fn stale_pair(
    tag: &str,
) -> (
    TempDir,
    AppRuntime,
    WindowSessionHandle,
    WindowSessionHandle,
) {
    let project = saved_project(tag);
    let mut app = AppRuntime::start().unwrap();
    app.enable_autosave_with(policy(60_000, 60_000));
    let owner = open_project(&app, project.path());
    let (a, _, _) = open_window(&app, owner);
    let (b, _, _) = open_window(&app, owner);
    edit(&app, a, 700, 11, "A");
    edit(&app, b, 800, 12, "B");
    app.flush_autosave().unwrap();
    assert!(wait_for(|| generation(&app, a) == 2));
    assert!(wait_for(|| status(&app, b) == AutosaveStatus::Stale));
    (project, app, a, b)
}

#[test]
fn stale_autosave_requests_one_alert_and_writes_nothing_before_a_choice() {
    // XII.7.7: 반복 알림 없음, 선택 전 write 없음.
    let (project, app, _a, b) = stale_pair("pipeline-stale-once");
    let before = current(&project);
    assert_eq!(
        alerts(&app),
        vec![StaleAlert {
            session: b,
            resolution: StaleResolution::Choose,
        }]
    );
    for n in 0..3 {
        edit(&app, b, 801 + n, 12, "B 계속");
        std::thread::sleep(Duration::from_millis(120));
        assert!(alerts(&app).is_empty(), "같은 stale 구간 알림은 한 번");
    }
    app.flush_autosave().unwrap();
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(current(&project), before, "선택 전에는 쓰지 않음");
    assert_eq!(status(&app, b), AutosaveStatus::Stale);
    assert!(dirty(&app, b));

    // 선택: 이 창 버전으로 저장. 디스크는 B 창과 같아지고 자동 저장이 다시 동작합니다.
    let resolved = app
        .resolve_stale(b, StaleSaveChoice::UseCurrentWindow)
        .unwrap()
        .wait()
        .unwrap()
        .unwrap();
    assert!(matches!(
        resolved,
        StaleResolved::Saved(SaveOutcome {
            generation: 3,
            written: true,
            ..
        })
    ));
    assert!(wait_for(|| status(&app, b) == AutosaveStatus::Idle));
    assert!(!dirty(&app, b));
    assert!(alerts(&app).is_empty());
    let snapshot = open_snapshot(project.path()).unwrap();
    let records = snapshot.document_records(sid(1)).unwrap();
    let name = |id| {
        records
            .nodes
            .iter()
            .find(|n| n.id == sid(id))
            .unwrap()
            .display_name
            .clone()
    };
    assert_eq!(name(12), "B 계속");
    assert_eq!(name(11), "n11", "UseCurrentWindow는 창 snapshot 전체를 씀");
    let _ = &project;
    edit(&app, b, 900, 12, "다시 자동 저장");
    app.flush_autosave().unwrap();
    assert!(wait_for(|| generation(&app, b) == 4));
    app.shutdown().unwrap();
}

#[test]
fn use_latest_disk_from_the_runtime_discards_local_edits() {
    let (_project, app, _a, b) = stale_pair("pipeline-use-latest");
    assert_eq!(alerts(&app).len(), 1);
    let resolved = app
        .resolve_stale(b, StaleSaveChoice::UseLatestDisk)
        .unwrap()
        .wait()
        .unwrap()
        .unwrap();
    assert!(matches!(
        resolved,
        StaleResolved::Reloaded { generation: 2, .. }
    ));
    assert_eq!(name_of(&app, b, 11), "A");
    assert_eq!(name_of(&app, b, 12), "n12");
    assert!(!dirty(&app, b));
    assert!(wait_for(|| status(&app, b) == AutosaveStatus::Idle));
    app.shutdown().unwrap();
}

#[test]
fn watcher_applies_a_cli_apply_to_a_clean_window_as_one_undoable_command() {
    // UX.4.1: watcher → command, 제스처 뒤 반영, 자기 저장 무시, Undo 한 번.
    let project = saved_project("pipeline-watch");
    let locks = TempDir::new("pipeline-watch-locks");
    let mut app = AppRuntime::start().unwrap();
    app.enable_autosave_with(policy(60_000, 60_000));
    let owner = open_project(&app, project.path());
    let (a, _, _) = open_window(&app, owner);
    cli_rename(&project, &locks, 11, "CLI 편집");
    assert!(
        wait_for(|| generation(&app, a) == 2),
        "감시 thread가 외부 generation을 반영"
    );
    assert_eq!(name_of(&app, a, 11), "CLI 편집");
    assert!(!dirty(&app, a));
    // Undo 한 번으로 외부 변경 전체가 되돌아갑니다(결과는 디스크와 달라 dirty).
    let request = app.undo(a).unwrap();
    assert!(matches!(
        next_solicited(&app),
        RuntimeEvent::HistoryReplayed { request: got, applied: true, .. } if got == request
    ));
    assert_eq!(name_of(&app, a, 11), "n11");
    assert!(dirty(&app, a));
    app.shutdown().unwrap();
}

#[test]
fn watcher_applies_an_external_text_source_edit_and_rebuilds_the_cache() {
    // V.10.6·V.10.7(ADR-0088): 원본 파일 event가 반영 job을 만들고, clean 창은 Undo 가능한 항목 하나로 받으며 cache
    // generation을 원본에 맞춥니다. 자기 저장 event는 걸러집니다.
    let project = saved_project("pipeline-watch-source");
    let mut app = AppRuntime::start().unwrap();
    app.enable_autosave_with(policy(60_000, 60_000));
    let owner = open_project(&app, project.path());
    let (a, _, _) = open_window(&app, owner);
    assert!(
        stellar_project::document_io::session::read_source(project.path())
            .unwrap()
            .is_some(),
        "열기가 원본으로 옮김"
    );
    // 열기 뒤 자기 원본 쓰기 event가 가라앉을 때까지 기다립니다.
    std::thread::sleep(Duration::from_millis(200));
    let mut text = stellar_project::external_change::source::TextSourceSession::open(
        project.path(),
        crate::window_session::test_fixtures::window(77),
    )
    .unwrap();
    let tx = rename(text.document(), 900, 11, "Git pull");
    text.apply(&tx).unwrap();
    text.save().unwrap();
    assert!(
        wait_for(|| name_of(&app, a, 11) == "Git pull"),
        "감시 thread가 외부 원본을 반영"
    );
    assert!(!dirty(&app, a));
    // cache 맞춤 쓰기는 자동 저장 writer가 합니다(debounce 없이 flush).
    app.flush_autosave().unwrap();
    assert!(wait_for(|| generation(&app, a) == 2), "cache를 원본에 맞춤");
    let request = app.undo(a).unwrap();
    assert!(matches!(
        next_solicited(&app),
        RuntimeEvent::HistoryReplayed { request: got, applied: true, .. } if got == request
    ));
    assert_eq!(name_of(&app, a, 11), "n11");
    assert!(dirty(&app, a));
    app.shutdown().unwrap();
}

#[test]
fn external_change_over_a_dirty_window_is_reconciled_and_autosave_resumes() {
    // XIII.3.4(ADR-0110 결정 2): 외부 writer와 dirty가 겹쳐도 겹치지 않는 변경이면 자동 병합하고, stale 알림 없이
    // 자동 저장이 병합 결과를 씁니다.
    let project = saved_project("pipeline-reconcile");
    let locks = TempDir::new("pipeline-reconcile-locks");
    let mut app = AppRuntime::start().unwrap();
    app.enable_autosave_with(policy(60_000, 60_000));
    let owner = open_project(&app, project.path());
    let (a, _, _) = open_window(&app, owner);
    edit(&app, a, 700, 12, "로컬");
    cli_rename(&project, &locks, 11, "CLI 편집");
    app.sync_external_changes(owner).unwrap().wait().unwrap();
    assert_eq!(alerts(&app), Vec::new(), "두 선택지 alert 0");
    assert_eq!(name_of(&app, a, 12), "로컬", "편집 보존");
    assert_eq!(name_of(&app, a, 11), "CLI 편집", "외부 변경 반영");
    assert!(dirty(&app, a));
    app.flush_autosave().unwrap();
    assert!(wait_for(|| !dirty(&app, a)), "자동 저장 재개");
    let saved = open_snapshot(project.path())
        .unwrap()
        .document_records(sid(1))
        .unwrap();
    let saved_name = |node: u64| {
        saved
            .nodes
            .iter()
            .find(|n| n.id == sid(node))
            .unwrap()
            .display_name
            .clone()
    };
    assert_eq!(saved_name(11), "CLI 편집");
    assert_eq!(saved_name(12), "로컬");
    app.shutdown().unwrap();
}

#[test]
fn autosave_writer_meeting_an_external_change_first_reconciles_and_saves_both() {
    // XIII.3.4(ADR-0110 결정 2): 감시 반영보다 자동 저장 writer가 먼저 외부 writer stale을 만나도, 쓰기 결과를 반영하는
    // 자리에서 병합하고 stale을 풀어 자동 저장이 병합 결과를 씁니다(두 선택지 alert 0).
    let project = saved_project("pipeline-reconcile-writer");
    let locks = TempDir::new("pipeline-reconcile-writer-locks");
    let mut app = AppRuntime::start().unwrap();
    app.enable_autosave_with(policy(60_000, 60_000));
    let owner = open_project(&app, project.path());
    let (a, _, _) = open_window(&app, owner);
    edit(&app, a, 701, 12, "로컬");
    cli_rename(&project, &locks, 11, "CLI 편집");
    app.flush_autosave().unwrap();
    let saved_name = |node: u64| {
        open_snapshot(project.path())
            .unwrap()
            .document_records(sid(1))
            .unwrap()
            .nodes
            .into_iter()
            .find(|n| n.id == sid(node))
            .unwrap()
            .display_name
    };
    assert!(
        wait_for(|| name_of(&app, a, 11) == "CLI 편집"),
        "쓰기 결과 반영 자리에서 병합"
    );
    let traces = app.autosave_traces(a).unwrap().wait().unwrap();
    assert_eq!(
        traces.first().map(|t| t.result),
        Some(AutosaveTraceResult::Stale),
        "첫 쓰기는 외부 writer stale"
    );
    assert_eq!(name_of(&app, a, 12), "로컬", "편집 보존");
    assert_eq!(alerts(&app), Vec::new(), "두 선택지 alert 0");
    app.flush_autosave().unwrap();
    assert!(wait_for(|| !dirty(&app, a)), "자동 저장 재개");
    assert_eq!(saved_name(11), "CLI 편집");
    assert_eq!(saved_name(12), "로컬");
    app.shutdown().unwrap();
}

#[test]
fn overlapping_external_change_leaves_the_two_choices_without_writing() {
    // XIII.3.4: 같은 속성을 외부 writer와 창이 다르게 바꾸면 자동 적용하지 않고 두 선택지 상태로 둡니다.
    let project = saved_project("pipeline-reconcile-overlap");
    let locks = TempDir::new("pipeline-reconcile-overlap-locks");
    let mut app = AppRuntime::start().unwrap();
    app.enable_autosave_with(policy(60_000, 60_000));
    let owner = open_project(&app, project.path());
    let (a, _, _) = open_window(&app, owner);
    edit(&app, a, 700, 12, "로컬");
    cli_rename(&project, &locks, 12, "CLI 편집");
    app.sync_external_changes(owner).unwrap().wait().unwrap();
    assert_eq!(
        alerts(&app),
        vec![StaleAlert {
            session: a,
            resolution: StaleResolution::Choose,
        }]
    );
    let before = current(&project);
    app.flush_autosave().unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(current(&project), before);
    assert_eq!(status(&app, a), AutosaveStatus::Stale);
    assert_eq!(name_of(&app, a, 12), "로컬", "편집 보존");
    // 두 선택지 가운데 UseCurrentWindow가 이 창 문서를 씁니다.
    assert!(matches!(
        app.resolve_stale(a, StaleSaveChoice::UseCurrentWindow)
            .unwrap()
            .wait()
            .unwrap(),
        Ok(StaleResolved::Saved(_))
    ));
    assert_ne!(current(&project), before);
    app.shutdown().unwrap();
}

#[test]
fn broken_external_generation_reports_a_diagnostic_and_applies_nothing() {
    // UX.4.4(코어): 깨진 외부 변경은 반영 0, 진단 보관.
    let project = saved_project("pipeline-broken");
    let app = AppRuntime::start().unwrap();
    let owner = open_project(&app, project.path());
    let (a, _, _) = open_window(&app, owner);
    write_current_pointer(project.path(), &ContentHash::compute(b"missing manifest")).unwrap();
    app.sync_external_changes(owner).unwrap().wait().unwrap();
    let status = app.external_status(a).unwrap().wait().unwrap().unwrap();
    assert!(status.rejection.is_some(), "{status:?}");
    assert_eq!(status.stale, None);
    assert_eq!(generation(&app, a), 1);
    assert_eq!(name_of(&app, a, 11), "n11");
    app.shutdown().unwrap();
}

/// `.stellar/journal`을 파일로 막은 상태. [`BlockedJournal::restore`]가 되돌립니다.
pub(crate) struct BlockedJournal {
    journal: std::path::PathBuf,
    backup: Option<std::path::PathBuf>,
}

impl BlockedJournal {
    pub(crate) fn restore(self) {
        std::fs::remove_file(&self.journal).unwrap();
        if let Some(backup) = &self.backup {
            std::fs::rename(backup, &self.journal).unwrap();
        }
    }
}

/// 텍스트 원본 저장 journal 자리를 파일로 막아 원본 쓰기가 실패하게 합니다.
pub(crate) fn block_journal(root: &std::path::Path) -> BlockedJournal {
    let journal = root.join(".stellar").join("journal");
    let backup = root.join("journal-backup");
    let moved = std::fs::rename(&journal, &backup).is_ok();
    std::fs::create_dir_all(journal.parent().unwrap()).unwrap();
    std::fs::write(&journal, b"blocked").unwrap();
    BlockedJournal {
        journal,
        backup: moved.then_some(backup),
    }
}

#[test]
fn close_flush_returns_a_decision_state_and_never_discards_edits() {
    // XIII.1.6: timeout·failure fixture.
    let project = saved_project("pipeline-close");
    let app = AppRuntime::start().unwrap();
    let owner = open_project(&app, project.path());
    let (a, _, _) = open_window(&app, owner);
    assert_eq!(
        app.flush_for_close(a, Duration::from_secs(5)),
        FlushOutcome::Clean
    );

    // 실패: 텍스트 원본 저장 journal(.stellar/journal)을 파일로 막습니다(원본이 Git 원본, ADR-0088).
    edit(&app, a, 700, 11, "닫기 전 편집");
    let blocked = block_journal(project.path());
    let outcome = app.flush_for_close(a, Duration::from_secs(5));
    assert!(
        matches!(&outcome, FlushOutcome::Blocked(FlushBlock::Failed(code)) if code == "source"),
        "{outcome:?}"
    );
    assert!(!outcome.may_close());
    assert!(dirty(&app, a), "실패해도 편집은 메모리에 남음");
    blocked.restore();

    // 시간 초과: document thread가 바쁘면 제한 시간 안에 끝나지 않습니다. 넘긴 flush는 취소하지 않고 끝까지
    // 실행되어 편집을 저장합니다(버리지 않음).
    let _busy = app
        .job(|_: &mut DocumentContext| std::thread::sleep(Duration::from_millis(400)))
        .unwrap();
    assert_eq!(
        app.flush_for_close(a, Duration::from_millis(30)),
        FlushOutcome::Blocked(FlushBlock::TimedOut)
    );
    assert!(wait_for(|| generation(&app, a) == 2));
    assert_eq!(name_of(&app, a, 11), "닫기 전 편집");

    // 정상 저장.
    edit(&app, a, 701, 11, "다시");
    assert_eq!(
        app.flush_for_close(a, Duration::from_secs(5)),
        FlushOutcome::Saved { generation: 3 }
    );
    let outcomes = app.flush_for_deactivate(Duration::from_secs(5));
    assert_eq!(outcomes, vec![(a, FlushOutcome::Clean)]);
    app.shutdown().unwrap();
}

#[test]
fn close_flush_of_a_stale_window_returns_the_two_choices() {
    let (_project, app, _a, b) = stale_pair("pipeline-close-stale");
    assert_eq!(
        app.flush_for_close(b, Duration::from_secs(5)),
        FlushOutcome::Blocked(FlushBlock::Stale(StaleResolution::Choose))
    );
    assert!(dirty(&app, b));
    app.shutdown().unwrap();
}

#[test]
fn git_ui_preflight_lets_the_caller_choose_save_or_disk() {
    // XIV.2.8: save/continue-with-disk 선택 fixture. 고르기 전에는 쓰지 않습니다.
    let project = saved_project("pipeline-git-ui");
    let mut app = AppRuntime::start().unwrap();
    app.enable_autosave_with(policy(60_000, 60_000));
    let owner = open_project(&app, project.path());
    let (a, _, _) = open_window(&app, owner);
    assert_eq!(
        app.git_ui_preflight(a).unwrap().wait().unwrap(),
        Some(GitUiPreflight::Clean)
    );
    edit(&app, a, 700, 11, "Git 전 편집");
    let before = current(&project);
    let revision = app.state().window(a).unwrap().revision;
    assert_eq!(
        app.git_ui_preflight(a).unwrap().wait().unwrap(),
        Some(GitUiPreflight::PendingEdits { revision })
    );
    assert_eq!(current(&project), before, "확인만으로는 쓰지 않음");

    // 디스크로 계속: 쓰지 않고 메모리 편집 유지.
    assert_eq!(
        app.prepare_git_ui(a, GitUiChoice::ContinueWithDisk, Duration::from_secs(5)),
        GitUiPreflight::PendingEdits { revision }
    );
    assert_eq!(current(&project), before);
    assert!(dirty(&app, a));

    // 먼저 저장: 디스크가 메모리와 같아짐.
    assert_eq!(
        app.prepare_git_ui(a, GitUiChoice::SaveFirst, Duration::from_secs(5)),
        GitUiPreflight::Clean
    );
    assert_ne!(current(&project), before);
    assert!(!dirty(&app, a));
    assert_eq!(generation(&app, a), 2);
    app.shutdown().unwrap();
}

#[test]
fn autosave_traces_record_timings_and_results_without_paths_or_names() {
    // XIII.1.7: trace schema 통과, 경로·이름 없음.
    let (project, app, a, b) = stale_pair("pipeline-trace");
    let traces = app.autosave_traces(a).unwrap().wait().unwrap();
    let saved = traces.last().unwrap();
    assert_eq!(saved.result, AutosaveTraceResult::Saved);
    assert!(saved.dirty_revision.0 > 0);
    assert!(saved.write > Duration::ZERO);
    let stale = app.autosave_traces(b).unwrap().wait().unwrap();
    assert_eq!(stale.last().unwrap().result, AutosaveTraceResult::Stale);
    let root = project.path().to_string_lossy().into_owned();
    for trace in traces.iter().chain(&stale) {
        let line = trace.to_json_line();
        assert_eq!(validate_trace_line(&line), Ok(()), "{line}");
        assert!(!line.contains(&root));
        assert!(!line.contains("n11") && !line.contains("문서"));
    }
    app.shutdown().unwrap();
}

#[test]
fn stale_alert_without_autosave_is_taken_once_and_keeps_the_stale_state() {
    // XII.7.7: 자동 저장을 켜지 않은 runtime도 알림은 stale 구간마다 한 번이고, 알림을 꺼내도 stale 상태는
    // 선택 전까지 남아 Git UI 확인·외부 상태가 계속 stale을 봅니다.
    let project = saved_project("pipeline-alert-no-autosave");
    let app = AppRuntime::start().unwrap();
    let owner = open_project(&app, project.path());
    let (a, _, _) = open_window(&app, owner);
    let (b, _, _) = open_window(&app, owner);
    edit(&app, a, 700, 11, "A");
    edit(&app, b, 800, 12, "B");
    assert!(matches!(
        app.flush_for_close(a, Duration::from_secs(5)),
        FlushOutcome::Saved { generation: 2 }
    ));
    app.sync_external_changes(owner).unwrap().wait().unwrap();
    let stale = Some(StaleResolution::Choose);
    assert_eq!(
        alerts(&app),
        vec![StaleAlert {
            session: b,
            resolution: StaleResolution::Choose
        }]
    );
    assert!(alerts(&app).is_empty(), "같은 stale 구간 알림은 한 번");
    let status = app.external_status(b).unwrap().wait().unwrap().unwrap();
    assert_eq!(status.stale, stale, "알림을 꺼내도 stale은 남음");
    assert_eq!(
        app.git_ui_preflight(b).unwrap().wait().unwrap(),
        Some(GitUiPreflight::Blocked(FlushBlock::Stale(
            StaleResolution::Choose
        )))
    );
    // 선택으로 풀리면 다음 stale은 다시 알립니다.
    assert!(app
        .resolve_stale(b, StaleSaveChoice::UseLatestDisk)
        .unwrap()
        .wait()
        .unwrap()
        .is_ok());
    assert_eq!(
        app.external_status(b)
            .unwrap()
            .wait()
            .unwrap()
            .unwrap()
            .stale,
        None
    );
    edit(&app, a, 701, 11, "A 다시");
    edit(&app, b, 801, 12, "B 다시");
    assert!(matches!(
        app.flush_for_close(a, Duration::from_secs(5)),
        FlushOutcome::Saved { generation: 3 }
    ));
    app.sync_external_changes(owner).unwrap().wait().unwrap();
    assert_eq!(alerts(&app).len(), 1, "새 stale 구간은 다시 알림");
    app.shutdown().unwrap();
}

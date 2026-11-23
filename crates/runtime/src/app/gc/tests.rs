//! 유휴 cache GC 일정 시험(ADR-0138 결정 2).

use super::*;
use crate::app_window_tests::{next_solicited, open_project, open_window};
use crate::window_session::test_fixtures::{identity, rename, saved_project, TempDir};
use stellar_storage::gc::retention::RetentionPolicy;

const IDLE: Duration = Duration::from_millis(300);

/// 시험 설정: 짧은 idle, 최근 1세대만 보존·유예 없음, 시험 잠금 폴더, 보관소 정리 없음.
fn config(locks: &Path) -> IdleGcConfig {
    let locks = locks.to_path_buf();
    IdleGcConfig {
        idle: IDLE,
        poll: Duration::from_millis(20),
        retention: RetentionPolicy {
            keep_recent_generations: 1,
            grace_period_seconds: 0,
        },
        lock_dir: Arc::new(move || Ok(locks.clone())),
        store: None,
    }
}

fn manifests(root: &Path) -> usize {
    std::fs::read_dir(root.join(".stellar/cache/chunks/generations"))
        .map(|entries| entries.count())
        .unwrap_or(0)
}

/// `until`이 참이 될 때까지 최대 5초 기다립니다.
fn wait_for(mut until: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if until() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    until()
}

fn collected(gc: &IdleGc) -> Vec<GcReport> {
    gc.runs()
        .into_iter()
        .filter_map(|run| match run.outcome {
            IdleGcOutcome::Collected(report) => Some(report),
            _ => None,
        })
        .collect()
}

fn edit(app: &AppRuntime, session: WindowSessionHandle, command: u64, name: &str) {
    let name = name.to_string();
    let request = app
        .submit(move |ctx, request| {
            let window = window_mut(ctx, session)?;
            let tx = rename(window.document().unwrap(), command, 11, &name);
            window.apply(&tx, 0).unwrap();
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
        .unwrap();
    assert!(matches!(
        next_solicited(app),
        RuntimeEvent::WindowViewChanged { request: got, .. } if got == request
    ));
}

fn save(app: &AppRuntime, session: WindowSessionHandle) {
    let request = app.save_now(session).unwrap();
    assert!(matches!(
        next_solicited(app),
        RuntimeEvent::WindowSessionSaved { request: got, written: true, .. } if got == request
    ));
}

#[test]
fn idle_gc_waits_for_idle_then_collects_old_generations_once_per_save() {
    let project = saved_project("idle-gc-collect");
    let locks = TempDir::new("idle-gc-collect-locks");
    let app = AppRuntime::start().unwrap();
    let (a, _, _) = open_window(&app, open_project(&app, project.path()));
    for (n, name) in [(700, "하나"), (701, "둘"), (702, "셋")] {
        edit(&app, a, n, name);
        save(&app, a);
    }
    let before = manifests(project.path());
    assert!(before >= 3, "세대 {before}");
    let gc = app.start_idle_gc_with(config(locks.path()));
    // 연 직후(마지막 활동 뒤 idle 전)는 정리하지 않습니다.
    std::thread::sleep(IDLE / 3);
    assert!(gc.runs().is_empty(), "idle 전 정리");
    assert!(wait_for(|| !collected(&gc).is_empty()));
    let reports = collected(&gc);
    assert_eq!(reports.len(), 1);
    assert!(reports[0].deleted > 0, "{:?}", reports[0]);
    assert_eq!(manifests(project.path()), 1, "current만 남음");
    // current가 그대로면 다시 정리하지 않습니다.
    std::thread::sleep(IDLE * 2);
    assert_eq!(collected(&gc).len(), 1);
    // 정리 뒤에도 저장은 잠금 충돌 없이 이어지고, 새 저장 뒤 다시 idle이 지나면 한 번 더 정리합니다.
    edit(&app, a, 703, "넷");
    save(&app, a);
    assert!(wait_for(|| collected(&gc).len() == 2));
    assert_eq!(manifests(project.path()), 1);
    drop(gc);
    app.shutdown().unwrap();
}

#[test]
fn idle_gc_does_not_run_while_edits_are_unsaved() {
    let project = saved_project("idle-gc-dirty");
    let locks = TempDir::new("idle-gc-dirty-locks");
    let app = AppRuntime::start().unwrap();
    let (a, _, _) = open_window(&app, open_project(&app, project.path()));
    edit(&app, a, 700, "하나");
    save(&app, a);
    // 저장하지 않은 편집이 남은 창은 바쁜 창입니다(자동 저장이 곧 쓸 수 있음).
    edit(&app, a, 701, "저장 전");
    let before = manifests(project.path());
    let gc = app.start_idle_gc_with(config(locks.path()));
    std::thread::sleep(IDLE * 3);
    assert!(gc.runs().is_empty(), "편집 중 정리: {:?}", gc.runs());
    assert_eq!(manifests(project.path()), before);
    // 저장하면 idle 뒤 정리합니다.
    save(&app, a);
    assert!(wait_for(|| !collected(&gc).is_empty()));
    drop(gc);
    app.shutdown().unwrap();
}

#[test]
fn idle_gc_skips_while_another_writer_holds_the_save_lock() {
    let project = saved_project("idle-gc-lock");
    let locks = TempDir::new("idle-gc-lock-locks");
    let app = AppRuntime::start().unwrap();
    let (a, _, _) = open_window(&app, open_project(&app, project.path()));
    edit(&app, a, 700, "하나");
    save(&app, a);
    let before = manifests(project.path());
    // 다른 프로세스(CLI)의 저장처럼 같은 프로젝트 잠금을 쥡니다.
    let held = SaveLock::for_identity(locks.path(), &identity(project.path())).unwrap();
    let gc = app.start_idle_gc_with(config(locks.path()));
    assert!(wait_for(|| gc
        .runs()
        .iter()
        .any(|run| run.outcome == IdleGcOutcome::LockBusy)));
    assert!(collected(&gc).is_empty());
    assert_eq!(manifests(project.path()), before, "잠금 중 삭제");
    // 잠금이 풀리면 다음 관찰에서 정리합니다.
    drop(held);
    assert!(wait_for(|| !collected(&gc).is_empty()));
    assert_eq!(manifests(project.path()), 1);
    drop(gc);
    app.shutdown().unwrap();
}

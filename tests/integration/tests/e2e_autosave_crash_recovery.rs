//! XVI.11.5 종단 시나리오: 자동 저장을 켠 앱이 편집 중 비정상 종료 → 다시 실행 → 복구 선택 → Git 작업 트리 확인.
//!
//! 판정 기준(계획 XVI.11.5): 미저장 변경이 복구되고, 복구한 창의 history는 비어 있습니다.
//!
//! 비정상 종료는 실제 프로세스 중단입니다(ADR-0111 결정 6). 이 테스트 binary를 자식 프로세스로 다시 실행해 자식이
//! 앱 runtime(자동 저장 + 복구 체크포인트)을 돌리다 `std::process::abort()`로 끝납니다. 정상 종료 정리(창 닫기·runtime
//! drop)는 한 번도 실행되지 않습니다. 부모가 같은 사용자 데이터 폴더(테스트 HOME)로 앱을 다시 실행합니다.
//!
//! - 결정적 경로: 자동 저장 debounce를 길게 두고 앱 비활성화 flush로 첫 편집을 자동 저장 writer가 쓰게 한 뒤, 둘째
//!   편집은 복구 체크포인트(debounce 1초)만 남기고 중단합니다. 다시 실행하면 디스크는 첫 편집까지이고 복구 후보가
//!   둘째 편집을 돌려줍니다.
//! - 자동 저장 쓰기 중 중단: flush 직후 바로 중단합니다. 쓰기가 끝났는지는 시점에 달렸으므로, 어느 쪽이든 프로젝트가
//!   열리고 검증을 통과하며 둘째 편집이 디스크나 복구본 한쪽에 남는지(잃지 않는지)를 봅니다. 두 모드 모두 체크포인트가
//!   남은 뒤에 중단합니다.

mod e2e_common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use e2e_common::{
    git, porcelain, reopen_json, test_home, tree, trusted_git, wait_until, App, TempDir,
    INHERITED_HOME,
};
use stellar_core_types::ids::StableId;
use stellar_core_types::value::{Rect, Size2, Vec2};
use stellar_ops::query::project_structure;
use stellar_ops::snapshot::open_snapshot;
use stellar_ops::validate::validate_project;
use stellar_project::autosave::AutosavePolicy;
use stellar_project::identity::ProjectIdentity;
use stellar_project::recovery::RecoverySnapshotStore;
use stellar_project::recovery_payload::RecoveryPayload;
use stellar_runtime::app::recovery::RecoveryAction;

const CHILD_ROOT: &str = "STELLAR_E2E_CRASH_ROOT";
const CHILD_OUT: &str = "STELLAR_E2E_CRASH_OUT";
const CHILD_MODE: &str = "STELLAR_E2E_CRASH_MODE";

/// 자동 저장 정책: debounce가 길어 flush 없이는 시험 안에 저절로 쓰지 않습니다.
fn long_debounce() -> AutosavePolicy {
    AutosavePolicy::from_budgets_yaml(
        "autosave:\n  debounce_ms: 60000\n  max_delay_ms: 120000\n  app_deactivate_flush_timeout_ms: 2000\n  window_close_flush_timeout_ms: 10000\n",
    )
    .expect("자동 저장 정책")
}

fn rect(x: f32, y: f32) -> Rect {
    Rect::new(Vec2::new(x, y).unwrap(), Size2::new(80.0, 60.0).unwrap())
}

fn view_root(root: &Path) -> StableId {
    project_structure(&open_snapshot(root).unwrap()).documents[0].pages[0].views[0].root_frame
}

/// 자식 프로세스: 앱을 돌리다 중단합니다. 부모가 환경 변수로 부를 때만 동작하고, 일반 시험 실행에서는 바로 끝납니다.
#[test]
fn crash_child_process() {
    let (Some(root), Some(out), Some(mode)) = (
        std::env::var_os(CHILD_ROOT),
        std::env::var_os(CHILD_OUT),
        std::env::var(CHILD_MODE).ok(),
    ) else {
        return;
    };
    let root = PathBuf::from(root);
    let parent = view_root(&root);
    let mut app = App::start_with_recovery();
    app.rt.enable_autosave_with(long_debounce());
    let project = app.open_project(&root);
    let (window, _, generation) = app.open_window(project);

    // 첫 편집: 앱 비활성화 flush로 자동 저장 writer가 씁니다.
    let saved = app.insert_frame(window, parent, rect(10.0, 10.0));
    app.rt.flush_autosave().unwrap();
    assert!(
        wait_until(Duration::from_secs(30), || {
            let w = app.window(window);
            !w.has_unsaved_changes && w.generation > generation
        }),
        "자동 저장이 첫 편집을 쓰지 않음"
    );

    // 둘째 편집: 저장되지 않은 채 복구 체크포인트만 남깁니다.
    let unsaved = app.insert_frame(window, parent, rect(200.0, 10.0));
    let lines = format!(
        "{saved}\n{unsaved}\n{}\n",
        app.window(window).window_id.stable_id()
    );
    std::fs::write(&out, lines).unwrap();
    // 살아 있는 창의 체크포인트는 그 창이 잠그므로 복구 후보 조회가 건너뜁니다. 저장소 파일을 직접 봅니다.
    let identity = ProjectIdentity::new(e2e_common::project_id_of(&root), &root).unwrap();
    let store = RecoverySnapshotStore::new(
        &stellar_project::trust_store::os_user_data_base()
            .unwrap()
            .join(stellar_ops::lock::APP_DATA_DIR),
    );
    let window_id = app.window(window).window_id;
    let revision = app.window(window).revision.0;
    assert!(
        wait_until(Duration::from_secs(30), || {
            store
                .read_snapshot(&identity, window_id)
                .ok()
                .flatten()
                .and_then(|bytes| RecoveryPayload::decode(&bytes).ok())
                .is_some_and(|payload| payload.revision == revision)
        }),
        "복구 체크포인트가 남지 않음"
    );
    assert!(app.window(window).has_unsaved_changes, "둘째 편집은 미저장");
    match mode.as_str() {
        "checkpoint" => {}
        // 자동 저장 쓰기를 접수한 직후 중단합니다(쓰기 완료 여부는 시점에 달림).
        "writing" => app.rt.flush_autosave().unwrap(),
        other => panic!("알 수 없는 mode {other}"),
    }
    std::process::abort();
}

/// 자식을 실행해 비정상 종료를 확인합니다. 반환: (자동 저장된 Frame, 미저장 Frame, 중단한 창 id).
fn crash(root: &Path, mode: &str, work: &Path) -> (StableId, StableId, StableId) {
    let out = work.join(format!("crash-{mode}.txt"));
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crash_child_process",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(INHERITED_HOME, test_home())
        .env(CHILD_ROOT, root)
        .env(CHILD_OUT, &out)
        .env(CHILD_MODE, mode)
        .status()
        .expect("자식 프로세스 실행");
    assert!(!status.success(), "자식은 abort로 끝나야 함: {status:?}");
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(6), "SIGABRT(비정상 종료): {status:?}");
    }
    let text = std::fs::read_to_string(&out).expect("자식 결과 파일(중단 전 기록)");
    let ids: Vec<StableId> = text
        .lines()
        .map(|line| line.parse().expect("StableId"))
        .collect();
    (ids[0], ids[1], ids[2])
}

/// 새 프로젝트(앱 생성 경로, 최초 commit까지).
fn new_project(dir: &TempDir) -> PathBuf {
    let root = dir.path.join("복구 프로젝트");
    let app = App::start();
    assert_eq!(app.create_project(&root), 0);
    app.shutdown();
    root
}

#[test]
fn crash_after_checkpoint_relaunch_restore_and_working_tree() {
    let Some(gate) = trusted_git() else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };
    let dir = TempDir::new("crash-checkpoint");
    let root = new_project(&dir);
    let head = git(&gate, &root, &["rev-parse", "HEAD"]);
    let (saved, unsaved, crashed) = crash(&root, "checkpoint", &dir.path);

    // 디스크는 자동 저장한 첫 편집까지입니다.
    let disk = reopen_json(&root);
    assert!(disk.contains(&saved.to_string()), "자동 저장 결과");
    assert!(
        !disk.contains(&unsaved.to_string()),
        "미저장 편집은 디스크에 없음"
    );
    assert_eq!(validate_project(&root).unwrap().error_count(), 0);

    // 다시 실행: 복구 후보는 중단한 창의 체크포인트입니다(기준 세대 = 디스크 current).
    let app = App::start_with_recovery();
    let project = app.open_project(&root);
    let (window, _, _) = app.open_window(project);
    let candidate = app
        .rt
        .state()
        .project(project)
        .unwrap()
        .recovery_candidate()
        .unwrap()
        .expect("복구 후보");
    assert_eq!(candidate.window.stable_id(), crashed);
    let before = tree(&root);
    let request = app
        .rt
        .recover_checkpoint(window, candidate.clone(), RecoveryAction::Restore)
        .unwrap();
    app.ok(request, "복구");

    // 복구는 이 창 메모리에만 적용됩니다: 미저장 변경 복구, history 비어 있음, 디스크 bytes 그대로.
    let w = app.window(window);
    assert!(w.has_unsaved_changes, "복구한 미저장 변경");
    assert_eq!((w.undo_depth, w.redo_depth), (0, 0), "history는 비어 있음");
    assert_eq!(tree(&root), before, "복구 직후 디스크 bytes 그대로");
    let rows = app
        .rt
        .sidebar_page(
            window,
            w.active_view.unwrap(),
            w.revision,
            Some(view_root(&root)),
            None,
        )
        .unwrap()
        .wait()
        .unwrap()
        .expect("레이어 page");
    let ids: Vec<StableId> = rows.rows.iter().map(|r| r.id).collect();
    assert!(ids.contains(&saved) && ids.contains(&unsaved), "{ids:?}");

    // Git 작업 트리: 자동 저장과 그 뒤 저장은 작업 트리 변경일 뿐 HEAD·index는 그대로입니다.
    let status_before_save = porcelain(&gate, &root);
    assert!(
        status_before_save.iter().any(|l| l.contains("design/")),
        "{status_before_save:?}"
    );
    assert_eq!(git(&gate, &root, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&gate, &root, &["diff", "--cached", "--name-only"]), "");
    let (_, written) = app.save(window);
    assert!(written, "복구 내용은 일반 저장으로 디스크에 씀");
    let disk = reopen_json(&root);
    assert!(disk.contains(&saved.to_string()) && disk.contains(&unsaved.to_string()));
    assert_eq!(git(&gate, &root, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&gate, &root, &["diff", "--cached", "--name-only"]), "");
    assert!(
        git(&gate, &root, &["diff", "--name-only"]).contains("design/current"),
        "저장은 작업 트리 변경"
    );
    // 저장 뒤 복구본은 정리되어 다음 실행에 다시 제안하지 않습니다.
    assert!(
        wait_until(Duration::from_secs(30), || app
            .rt
            .state()
            .project(project)
            .unwrap()
            .recovery_candidate()
            .unwrap()
            .is_none()),
        "저장 뒤 복구 후보 없음"
    );
    app.close_window(window);
    app.shutdown();
    assert_eq!(validate_project(&root).unwrap().error_count(), 0);
}

#[test]
fn crash_while_autosave_is_writing_loses_nothing() {
    let Some(_gate) = trusted_git() else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };
    let dir = TempDir::new("crash-writing");
    let root = new_project(&dir);
    let (saved, unsaved, crashed) = crash(&root, "writing", &dir.path);

    // 중단된 저장 거래가 있어도 다시 열 수 있고 검증을 통과합니다(저장 journal 복구, ADR-0078).
    let app = App::start_with_recovery();
    let project = app.open_project(&root);
    let (window, _, _) = app.open_window(project);
    assert_eq!(validate_project(&root).unwrap().error_count(), 0);
    let disk = reopen_json(&root);
    assert!(
        disk.contains(&saved.to_string()),
        "먼저 자동 저장한 편집은 남음"
    );
    let candidate = app
        .rt
        .state()
        .project(project)
        .unwrap()
        .recovery_candidate()
        .unwrap();
    if disk.contains(&unsaved.to_string()) {
        // 쓰기가 끝난 뒤 중단: 디스크가 최신이므로 낡은 복구본은 제안하지 않습니다.
        assert!(candidate.is_none(), "디스크가 최신이면 복구 제안 없음");
    } else {
        // 쓰기를 마치기 전에 중단: 체크포인트가 복구 후보로 남아 되살립니다.
        let candidate = candidate.expect("쓰기 전 중단이면 복구 후보가 있어야 함");
        assert_eq!(candidate.window.stable_id(), crashed);
        let request = app
            .rt
            .recover_checkpoint(window, candidate, RecoveryAction::Restore)
            .unwrap();
        app.ok(request, "복구");
        let w = app.window(window);
        assert!(w.has_unsaved_changes);
        assert_eq!(w.undo_depth, 0, "history는 비어 있음");
        assert!(app.save(window).1);
        assert!(reopen_json(&root).contains(&unsaved.to_string()));
    }
    app.close_window(window);
    app.shutdown();
}

//! XVI.11.6 종단 시나리오: 같은 프로젝트를 두 창에서 열고 서로 다른 편집 → 첫 창 Save Now → 두 번째 창 Save Now가
//! stale로 거부되고 알림이 한 번 나옴 → 두 선택지(현재 창 유지·최신 디스크 사용) 각각의 정책 결과.
//!
//! 앱 UI 없이 셸과 같은 앱 runtime 진입점만 씁니다(ADR-0111). 판정 기준은 XII.7.5·XII.7.6과 ADR-0083 결정 1입니다.
//!
//! - 현재 창 유지(UseCurrentWindow): 최신 디스크 위에 두 번째 창 snapshot을 그대로 씁니다. 디스크 문서는 두 번째 창
//!   문서와 같고(첫 창 편집은 덮임) 두 번째 창 history는 남습니다.
//! - 최신 디스크 사용(UseLatestDisk): 두 번째 창의 미저장 편집과 history를 버리고 디스크(첫 창 저장)를 다시 읽습니다.
//!   디스크 bytes는 바뀌지 않습니다.
//! - 어느 쪽도 Git index·HEAD를 바꾸지 않습니다. 저장 결과는 작업 트리 변경으로만 보입니다.

mod e2e_common;

use e2e_common::{headless_json, reopen_json, tree, trusted_git, App, TempDir};
use stellar_core_types::ids::StableId;
use stellar_core_types::value::{Rect, Size2, Vec2};
use stellar_ops::query::project_structure;
use stellar_ops::snapshot::open_snapshot;
use stellar_project::window_session::{StaleResolution, StaleSaveChoice, WriterProvenance};
use stellar_runtime::app::{RequestFailure, StaleAlert, WindowSessionHandle};
use stellar_runtime::window_session::StaleResolved;

/// 두 창 시나리오의 공통 상태.
struct TwoWindows {
    _dir: TempDir,
    root: std::path::PathBuf,
    app: App,
    a: WindowSessionHandle,
    b: WindowSessionHandle,
    document: StableId,
    /// 첫 창이 넣은 Frame.
    a_frame: StableId,
    /// 두 번째 창이 넣은 Text.
    b_text: StableId,
    /// 첫 창 저장 직후 디스크 문서.
    after_a_save: String,
    /// 첫 창 저장 직후 프로젝트 파일 전체.
    after_a_tree: std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
    head: String,
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(Vec2::new(x, y).unwrap(), Size2::new(w, h).unwrap())
}

/// 새 프로젝트 → 두 창 → A는 Frame, B는 Text 편집 → A Save Now → B Save Now stale 확인까지.
fn two_windows(tag: &str) -> Option<TwoWindows> {
    let git = trusted_git()?;
    let dir = TempDir::new(tag);
    let root = dir.path.join("두 창 프로젝트");
    let app = App::start();
    assert_eq!(app.create_project(&root), 0, "최초 commit까지 성공");
    let head = e2e_common::git(&git, &root, &["rev-parse", "HEAD"]);
    let structure = project_structure(&open_snapshot(&root).unwrap());
    let view_root = structure.documents[0].pages[0].views[0].root_frame;

    let project = app.open_project(&root);
    let (a, document, generation_a) = app.open_window(project);
    let (b, document_b, generation_b) = app.open_window(project);
    assert_ne!(a, b);
    assert_eq!(document, document_b, "같은 작업 문서");
    assert_eq!(generation_a, generation_b, "같은 base generation");

    // 서로 다른 편집. 창마다 문서·history가 독립입니다(XII.2.4).
    let a_frame = app.insert_frame(a, view_root, rect(40.0, 40.0, 200.0, 120.0));
    let b_text = app.insert_text(b, view_root, Vec2::new(24.0, 300.0).unwrap(), "두 번째 창");
    assert_eq!(app.window(a).undo_depth, 1);
    assert_eq!(app.window(b).undo_depth, 1);
    assert!(app.window(a).has_unsaved_changes && app.window(b).has_unsaved_changes);

    // 첫 창 저장.
    let (generation, written) = app.save(a);
    assert!(written);
    assert!(generation > generation_a, "첫 창 저장은 새 generation");
    let after_a_save = reopen_json(&root);
    assert!(after_a_save.contains(&a_frame.to_string()));
    assert!(!after_a_save.contains(&b_text.to_string()));
    let after_a_tree = tree(&root);

    // 두 번째 창 저장: 다른 창이 먼저 저장했으므로 아무것도 쓰지 않고 stale입니다(XII.7.1).
    let stale = app.try_save(b).expect_err("두 번째 창 저장은 stale");
    assert_eq!(
        stale.failure,
        RequestFailure::SaveStale,
        "{}",
        stale.message
    );
    assert_eq!(
        tree(&root),
        after_a_tree,
        "stale 저장은 디스크를 바꾸지 않음"
    );
    assert!(app.window(b).has_unsaved_changes, "편집은 메모리에 남음");
    assert_eq!(app.window(b).undo_depth, 1);

    Some(TwoWindows {
        _dir: dir,
        root,
        app,
        a,
        b,
        document,
        a_frame,
        b_text,
        after_a_save,
        after_a_tree,
        head,
    })
}

/// stale 알림(두 선택지 시트 요청)이 두 번째 창에 한 번만 나오는지. 디스크 변화 반영을 한 번 돌린 뒤 꺼냅니다(감시 thread가
/// current pointer 변화마다 하는 일과 같음).
fn assert_one_choice_alert(w: &TwoWindows) {
    let project = w.app.window(w.b).project;
    w.app
        .rt
        .sync_external_changes(project)
        .unwrap()
        .wait()
        .unwrap();
    let alerts = w.app.rt.take_stale_alerts().unwrap().wait().unwrap();
    assert_eq!(
        alerts,
        vec![StaleAlert {
            session: w.b,
            resolution: StaleResolution::Choose,
        }],
        "같은 앱의 다른 창이 쓴 generation이므로 두 선택지"
    );
    assert_eq!(
        StaleResolution::for_writer(WriterProvenance::AppWindow),
        StaleResolution::Choose
    );
    // 첫 창은 디스크와 같으므로 알림이 없습니다(위 목록이 두 번째 창 하나뿐인 것과 같은 판정을 이름으로 남김).
    assert!(!alerts.iter().any(|alert| alert.session == w.a));
    // 같은 stale 구간에서 디스크 변화 반영을 다시 돌려도 알림은 다시 나오지 않습니다.
    w.app
        .rt
        .sync_external_changes(project)
        .unwrap()
        .wait()
        .unwrap();
    let again = w.app.rt.take_stale_alerts().unwrap().wait().unwrap();
    assert!(again.is_empty(), "한 stale 구간의 알림은 한 번: {again:?}");
}

fn assert_git_untouched(w: &TwoWindows) {
    let git = trusted_git().unwrap();
    assert_eq!(
        e2e_common::git(&git, &w.root, &["rev-parse", "HEAD"]),
        w.head,
        "HEAD 불변"
    );
    assert_eq!(
        e2e_common::git(&git, &w.root, &["diff", "--cached", "--name-only"]),
        "",
        "index 불변(저장은 작업 트리만 바꿈)"
    );
    let status = e2e_common::porcelain(&git, &w.root);
    assert!(
        status.iter().any(|line| line.contains("design/")),
        "저장 결과는 작업 트리 변경: {status:?}"
    );
}

#[test]
fn second_window_keeps_its_snapshot_with_use_current_window() {
    let Some(w) = two_windows("use-current") else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };
    assert_one_choice_alert(&w);
    let resolved = w
        .app
        .rt
        .resolve_stale(w.b, StaleSaveChoice::UseCurrentWindow)
        .unwrap()
        .wait()
        .unwrap()
        .expect("현재 창 유지");
    let StaleResolved::Saved(outcome) = resolved else {
        panic!("저장이 아님: {resolved:?}");
    };
    assert!(outcome.wrote_anything());
    let b = w.app.window(w.b);
    assert!(!b.has_unsaved_changes, "저장 뒤 clean");
    assert_eq!(b.undo_depth, 1, "history 유지");
    assert_eq!(b.generation, outcome.generation);

    // 디스크 문서 = 두 번째 창 snapshot. 첫 창의 Frame은 덮였고 두 번째 창의 Text가 있습니다.
    let disk = reopen_json(&w.root);
    assert!(disk.contains(&w.b_text.to_string()));
    assert!(!disk.contains(&w.a_frame.to_string()), "첫 창 편집은 덮임");
    assert_ne!(disk, w.after_a_save);
    // CLI·MCP가 읽는 cache generation도 같은 문서입니다.
    assert_eq!(headless_json(&w.root, w.document), disk);
    // 두 번째 창에서 한 번 더 저장해도 쓸 것이 없습니다(디스크 = 창).
    assert!(!w.app.save(w.b).1);
    assert_git_untouched(&w);
    w.app.shutdown();
}

#[test]
fn second_window_discards_its_edits_with_use_latest_disk() {
    let Some(w) = two_windows("use-latest") else {
        eprintln!("시스템 Git 없음: 건너뜀");
        return;
    };
    assert_one_choice_alert(&w);
    let resolved = w
        .app
        .rt
        .resolve_stale(w.b, StaleSaveChoice::UseLatestDisk)
        .unwrap()
        .wait()
        .unwrap()
        .expect("최신 디스크 사용");
    let StaleResolved::Reloaded {
        generation,
        discarded_history_entries,
    } = resolved
    else {
        panic!("다시 불러오기가 아님: {resolved:?}");
    };
    assert_eq!(discarded_history_entries, 1, "두 번째 창 history를 버림");
    let b = w.app.window(w.b);
    assert_eq!(b.generation, generation);
    assert_eq!((b.undo_depth, b.redo_depth), (0, 0), "history 비움");
    assert!(!b.has_unsaved_changes);
    assert_eq!(tree(&w.root), w.after_a_tree, "디스크 bytes 그대로");
    let disk = reopen_json(&w.root);
    assert_eq!(disk, w.after_a_save, "디스크는 첫 창 저장 그대로");
    assert!(
        !disk.contains(&w.b_text.to_string()),
        "두 번째 창 편집은 버림"
    );
    // 다시 읽은 두 번째 창은 디스크와 같으므로 저장할 것이 없고 stale도 아닙니다.
    assert_eq!(w.app.save(w.b), (generation, false));
    assert!(w
        .app
        .rt
        .take_stale_alerts()
        .unwrap()
        .wait()
        .unwrap()
        .is_empty());
    assert_git_untouched(&w);
    // 다시 읽은 창에서 이어 편집하고 저장하면 일반 저장입니다.
    let more = w.app.insert_frame(
        w.b,
        open_snapshot(&w.root)
            .map(|s| project_structure(&s).documents[0].pages[0].views[0].root_frame)
            .unwrap(),
        rect(10.0, 10.0, 20.0, 20.0),
    );
    let (next, written) = w.app.save(w.b);
    assert!(written && next > generation);
    let disk = reopen_json(&w.root);
    assert!(disk.contains(&more.to_string()) && disk.contains(&w.a_frame.to_string()));
    w.app.shutdown();
}

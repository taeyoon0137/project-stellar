//! 복구 체크포인트 수명 시험: 앱 데이터에만 기록(XIII.8.1), 자동 저장과 snapshot 공유(XIII.8.2), 복구 선택 뒤
//! 디스크 불변(XIII.8.4), 정상 종료 뒤 정리(XIII.8.7).

use super::*;
use crate::app_window_tests::{open_project, open_window};
use crate::window_session::test_fixtures::{identity, rename, saved_project, sid, window, TempDir};
use std::time::Instant;

/// 프로젝트 폴더 전체(파일 경로 → bytes).
fn tree(root: &std::path::Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &std::path::Path, out: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(root, &mut out);
    out
}

/// 요청 없는 표시 변경 event(글꼴 적재 등)를 건너뛴 다음 완료 event.
fn next_event(app: &AppRuntime) -> RuntimeEvent {
    loop {
        let event = crate::app_window_tests::next_event(app);
        if event.request() != RequestId(0) {
            return event;
        }
    }
}

fn wait_for(mut until: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if until() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    until()
}

/// 복구본 저장소를 `store`로 바꾼 runtime.
fn app_with_store(store: &TempDir) -> AppRuntime {
    let mut app = AppRuntime::start().unwrap();
    app.recovery_root = Some(store.path().to_path_buf());
    app
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
        next_event(app),
        RuntimeEvent::WindowViewChanged { request: got, .. } if got == request
    ));
}

fn snapshot_count(store: &TempDir, project: &TempDir) -> usize {
    RecoverySnapshotStore::new(store.path())
        .list_sessions(&identity(project.path()))
        .map_or(0, |sessions| sessions.len())
}

#[test]
fn checkpoint_is_debounced_into_app_data_and_never_into_the_project_folder() {
    // XIII.8.1: dirty revision이 바뀌면 debounce 뒤 OS app data에 기록하고 프로젝트 폴더는 그대로입니다.
    let project = saved_project("checkpoint-app-data");
    let store = TempDir::new("checkpoint-app-data-store");
    let app = app_with_store(&store);
    let (a, _, _) = open_window(&app, open_project(&app, project.path()));
    let before = tree(project.path());
    edit(&app, a, 500, "복구 대상");
    // debounce(1초) 안에는 쓰지 않습니다.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(snapshot_count(&store, &project), 0, "debounce 전 기록");
    assert!(wait_for(|| snapshot_count(&store, &project) == 1));
    assert_eq!(tree(project.path()), before, "프로젝트 Git 폴더 변화 없음");
    app.shutdown().unwrap();
}

#[test]
fn autosave_and_checkpoint_share_one_document_serialization() {
    // XIII.8.2: 같은 revision의 불변 snapshot을 공유하고 문서 직렬화는 한 번입니다.
    let project = saved_project("checkpoint-shared");
    let store_dir = TempDir::new("checkpoint-shared-store");
    let locks = TempDir::new("checkpoint-shared-locks");
    let store = RecoverySnapshotStore::new(store_dir.path());
    let id = identity(project.path());
    let owner = window(51);
    let mut w = ProjectWindowSession::load(id.clone(), owner).unwrap();
    w.apply(&rename(w.document().unwrap(), 510, 11, "공유"), 0)
        .unwrap();
    let save = w.capture_save().unwrap().unwrap();
    let checkpoint = Capture::from_window(&w).unwrap();
    assert!(Arc::ptr_eq(save.snapshot(), &checkpoint.snapshot));
    let shared = Arc::clone(&checkpoint.snapshot);
    assert_eq!(shared.revision(), w.revision());
    assert_eq!(shared.serialization_count(), 0);
    fs::create_dir_all(store.project_dir(&id)).unwrap();
    checkpoint.write(&store, &id, owner).unwrap();
    let dir = locks.path().to_path_buf();
    let written = save.write(move || Ok(dir)).unwrap();
    assert_eq!(shared.serialization_count(), 1, "중복 직렬화 없음");
    w.apply_save(written);
    assert!(!w.has_unsaved_changes());
    // 새 revision은 새 snapshot입니다.
    w.apply(&rename(w.document().unwrap(), 511, 11, "다음"), 0)
        .unwrap();
    let next = w.shared_snapshot().unwrap().unwrap();
    assert!(!Arc::ptr_eq(&next, &shared));
}

#[test]
fn restored_checkpoint_lives_in_memory_and_leaves_disk_bytes_unchanged() {
    // XIII.8.4: 복구 선택은 새 in-memory 상태이고 디스크 current를 바로 덮어쓰지 않습니다. 다음 저장이 일반
    // working tree 변경으로 기록합니다.
    let project = saved_project("checkpoint-restore");
    let store_dir = TempDir::new("checkpoint-restore-store");
    let store = RecoverySnapshotStore::new(store_dir.path());
    let id = identity(project.path());
    let crashed = window(61);
    // 비정상 종료한 창이 남긴 복구본.
    let mut lost = ProjectWindowSession::load(id.clone(), crashed).unwrap();
    lost.apply(&rename(lost.document().unwrap(), 610, 11, "복구할 편집"), 0)
        .unwrap();
    fs::create_dir_all(store.project_dir(&id)).unwrap();
    Capture::from_window(&lost)
        .unwrap()
        .write(&store, &id, crashed)
        .unwrap();
    drop(lost);

    let app = AppRuntime::start().unwrap();
    let owner = open_project(&app, project.path());
    let (a, _, _) = open_window(&app, owner);
    let state = app.state();
    let summary = state.project(owner).unwrap();
    let candidate = summary
        .recovery_candidate_in(store_dir.path())
        .unwrap()
        .expect("V.9.3 규칙에 맞는 복구본");
    assert_eq!(candidate.window, crashed);
    let ready = summary
        .prepare_recovery_in(store_dir.path(), &candidate)
        .unwrap();
    let before = tree(project.path());
    app.sender
        .submit(move |ctx| apply_recovery(ctx, a, ready).map_err(|e| e.message))
        .unwrap()
        .wait()
        .unwrap()
        .unwrap();
    assert_eq!(tree(project.path()), before, "복구 직후 disk bytes 미변경");
    let restored = app
        .sender
        .submit(move |ctx| {
            let session = &ctx.app_sessions().windows[&a].session;
            (
                session
                    .document()
                    .unwrap()
                    .model()
                    .record(sid(11))
                    .unwrap()
                    .display_name,
                session.has_unsaved_changes(),
                session.history().history().len(),
            )
        })
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(restored, ("복구할 편집".to_string(), true, 0));
    // 다음 저장이 일반 저장 경로로 새 generation을 씁니다.
    let request = app.save_now(a).unwrap();
    assert!(matches!(
        next_event(&app),
        RuntimeEvent::WindowSessionSaved { request: got, generation: 2, written: true, .. }
            if got == request
    ));
    assert_ne!(tree(project.path()), before);
    app.shutdown().unwrap();
}

#[test]
fn normal_save_and_close_remove_the_checkpoint_and_offer_no_prompt() {
    // XIII.8.7: 정상 종료 뒤 복구본을 지우고, 남은 낡은 복구본은 다음 시작에서 stale로 제안하지 않습니다.
    let project = saved_project("checkpoint-close");
    let store = TempDir::new("checkpoint-close-store");
    let app = app_with_store(&store);
    let owner = open_project(&app, project.path());
    let (a, _, _) = open_window(&app, owner);
    edit(&app, a, 700, "저장 전");
    assert!(wait_for(|| snapshot_count(&store, &project) == 1));
    // 정상 저장 → clean → 복구본 삭제.
    let request = app.save_now(a).unwrap();
    assert!(matches!(
        next_event(&app),
        RuntimeEvent::WindowSessionSaved { request: got, .. } if got == request
    ));
    assert!(wait_for(|| snapshot_count(&store, &project) == 0));
    // 창 닫기(정상 종료)도 남기지 않습니다.
    edit(&app, a, 701, "닫기 전");
    assert!(wait_for(|| snapshot_count(&store, &project) == 1));
    let request = app.close_window_session(a).unwrap();
    let closed = next_event(&app);
    assert!(
        matches!(
            closed,
            RuntimeEvent::WindowSessionClosed { request: got, .. } if got == request
        ),
        "{closed:?}"
    );
    assert!(wait_for(|| snapshot_count(&store, &project) == 0));
    let state = app.state();
    assert_eq!(
        state
            .project(owner)
            .unwrap()
            .recovery_candidate_in(store.path())
            .unwrap(),
        None,
        "normal close 후 prompt 없음"
    );
    app.shutdown().unwrap();

    // 삭제에 실패해 남은 복구본(저장 뒤 기준 세대가 낡음)은 다음 시작에서 제안하지 않습니다.
    let id = identity(project.path());
    let stale_owner = window(71);
    let mut old = ProjectWindowSession::load(id.clone(), stale_owner).unwrap();
    old.apply(&rename(old.document().unwrap(), 710, 11, "낡은 편집"), 0)
        .unwrap();
    let recovery = RecoverySnapshotStore::new(store.path());
    fs::create_dir_all(recovery.project_dir(&id)).unwrap();
    let mut payload = Capture::from_window(&old).unwrap();
    payload.payload.base_manifest_hash = Some(ContentHash::compute(b"older generation"));
    payload.write(&recovery, &id, stale_owner).unwrap();
    let next = AppRuntime::start().unwrap();
    let owner = open_project(&next, project.path());
    let state = next.state();
    assert_eq!(
        state
            .project(owner)
            .unwrap()
            .recovery_candidate_in(store.path())
            .unwrap(),
        None,
        "stale 복구본은 무시"
    );
    next.shutdown().unwrap();
}

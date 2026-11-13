//! 셸 경계용 앱 runtime 통합 테스트: 프로젝트 열기 error mapping, 문서 열기·닫기, 완료 event와
//! 상태 snapshot, outbox 상한, UI thread 무대기.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use stellar_core_types::ids::StableId;
use stellar_core_types::revision::DocumentRevision;
use stellar_project::scaffold::write_default_scaffold;
use stellar_project::trust::TrustStatus;
use stellar_renderer_core::thread_role::{bind_thread_role, ThreadRole};
use stellar_runtime::app::{
    AppRuntime, DocumentSessionId, EventPoll, ProjectSessionId, RequestError, RequestFailure,
    RequestId, RuntimeEvent,
};
use stellar_runtime::command_queue::QueueCapacity;

const PROJECT_ID: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";

/// 테스트용 임시 디렉터리(drop 시 정리).
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "stellar-app-runtime-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// `stellar.yaml` + `.git` 디렉터리를 가진 프로젝트 fixture(git은 실행하지 않음).
fn project_fixture(name: &str) -> TempDir {
    let dir = TempDir::new("project");
    write_default_scaffold(dir.path(), name, PROJECT_ID).unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    dir
}

fn doc_id(seed: u64) -> StableId {
    StableId::new_v7(seed, 0x0d0c, seed)
}

/// 다음 완료 event를 최대 5초 기다립니다.
fn next_event(app: &AppRuntime) -> RuntimeEvent {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(event) = app.pop_event() {
            return event;
        }
        assert!(Instant::now() < deadline, "완료 event가 오지 않았습니다");
        std::thread::yield_now();
    }
}

fn expect_failure(app: &AppRuntime, request: RequestId, expected: RequestFailure) -> String {
    match next_event(app) {
        RuntimeEvent::RequestFailed {
            request: got,
            failure,
            message,
        } => {
            assert_eq!(got, request);
            assert_eq!(failure, expected, "{message}");
            message
        }
        other => panic!("실패 event가 아님: {other:?}"),
    }
}

fn open_project(app: &AppRuntime, root: &Path) -> ProjectSessionId {
    let request = app.open_project(root).unwrap();
    match next_event(app) {
        RuntimeEvent::ProjectOpened {
            request: got,
            project,
            ..
        } => {
            assert_eq!(got, request);
            project
        }
        other => panic!("프로젝트 열기 실패: {other:?}"),
    }
}

fn open_document(
    app: &AppRuntime,
    project: ProjectSessionId,
    id: StableId,
) -> (DocumentSessionId, bool) {
    let request = app.open_document(project, id).unwrap();
    match next_event(app) {
        RuntimeEvent::DocumentOpened {
            request: got,
            project: owner,
            document,
            document_id,
            already_open,
        } => {
            assert_eq!(got, request);
            assert_eq!(owner, project);
            assert_eq!(document_id, id);
            (document, already_open)
        }
        other => panic!("문서 열기 실패: {other:?}"),
    }
}

#[test]
fn opens_a_valid_project_and_publishes_state_before_the_event() {
    let fixture = project_fixture("디자인 시스템");
    let app = AppRuntime::start().unwrap();
    assert_eq!(app.state().generation, 0);
    let request = app.open_project(fixture.path()).unwrap();
    assert_eq!(request, RequestId(1));
    let event = next_event(&app);
    let RuntimeEvent::ProjectOpened {
        project,
        project_id,
        name,
        root,
        ..
    } = &event
    else {
        panic!("프로젝트 열기 실패: {event:?}");
    };
    assert_eq!(name, "디자인 시스템");
    assert_eq!(project_id.to_string(), PROJECT_ID);
    assert_eq!(
        Path::new(root),
        std::path::absolute(fixture.path()).unwrap()
    );
    // event를 받은 시점에 snapshot은 이미 결과를 담고 있습니다.
    let state = app.state();
    assert_eq!(state.generation, 1);
    let summary = state.project(*project).expect("snapshot에 프로젝트");
    assert_eq!(summary.name, "디자인 시스템");
    assert_eq!(summary.project_id, *project_id);
    assert_eq!(summary.trust, TrustStatus::NotTrusted);
    assert!(state.documents.is_empty());
    // event payload는 바이트로도 같은 내용입니다.
    assert_eq!(
        RuntimeEvent::decode(event.kind(), &event.encode()).unwrap(),
        event
    );
    assert_eq!(app.outstanding(), 0);
    app.shutdown().unwrap();
}

#[test]
fn invalid_project_folders_map_to_fixed_failures() {
    // XII.6.2 "invalid repo 안내 error mapping": 각 실패가 고정 코드와 안내 메시지로 돌아옵니다.
    let app = AppRuntime::start().unwrap();

    let empty = TempDir::new("no-repo");
    let request = app.open_project(empty.path()).unwrap();
    let message = expect_failure(&app, request, RequestFailure::NotARepository);
    assert!(message.contains(".git"));

    let git_only = TempDir::new("git-only");
    std::fs::create_dir(git_only.path().join(".git")).unwrap();
    let request = app.open_project(git_only.path()).unwrap();
    let message = expect_failure(&app, request, RequestFailure::NotAStellarProject);
    assert!(message.contains("자동 변환"));
    // 자동 변환 없음: 매니페스트를 만들지 않습니다.
    assert!(!git_only.path().join("stellar.yaml").exists());

    let corrupt = project_fixture("x");
    std::fs::write(corrupt.path().join("stellar.yaml"), "project: [unclosed\n").unwrap();
    let request = app.open_project(corrupt.path()).unwrap();
    expect_failure(&app, request, RequestFailure::ManifestCorrupt);

    let schema = project_fixture("x");
    std::fs::write(schema.path().join("stellar.yaml"), "schema_version: 1\n").unwrap();
    let request = app.open_project(schema.path()).unwrap();
    expect_failure(&app, request, RequestFailure::ManifestInvalid);

    let bad_id = TempDir::new("bad-id");
    write_default_scaffold(bad_id.path(), "x", "not-a-uuid").unwrap();
    std::fs::create_dir(bad_id.path().join(".git")).unwrap();
    let request = app.open_project(bad_id.path()).unwrap();
    expect_failure(&app, request, RequestFailure::InvalidProjectId);

    #[cfg(unix)]
    {
        let target = project_fixture("외부");
        let linked = TempDir::new("symlink");
        std::fs::create_dir(linked.path().join(".git")).unwrap();
        std::os::unix::fs::symlink(
            target.path().join("stellar.yaml"),
            linked.path().join("stellar.yaml"),
        )
        .unwrap();
        let request = app.open_project(linked.path()).unwrap();
        expect_failure(&app, request, RequestFailure::UntrustedSymlink);
    }

    // 실패한 요청은 상태를 바꾸지 않으므로 snapshot을 게시하지 않습니다.
    assert_eq!(app.state().generation, 0);
    assert!(app.state().projects.is_empty());
    app.shutdown().unwrap();
}

#[test]
fn path_rules_are_checked_without_a_request() {
    let app = AppRuntime::start().unwrap();
    for (path, needle) in [
        (Path::new(""), "빈"),
        (Path::new("relative/project"), "절대"),
        (Path::new("/tmp/a\0b"), "NUL"),
    ] {
        match app.open_project(path) {
            Err(RequestError::InvalidPath { reason }) => assert!(reason.contains(needle)),
            other => panic!("{path:?}: {other:?}"),
        }
    }
    assert_eq!(app.outstanding(), 0);
    assert!(app.pop_event().is_none());
    app.shutdown().unwrap();
}

#[test]
fn documents_open_and_close_under_a_project_session() {
    let first = project_fixture("A");
    let second = project_fixture("B");
    let app = AppRuntime::start().unwrap();
    let a = open_project(&app, first.path());
    let b = open_project(&app, second.path());
    assert_ne!(a, b);

    let (doc1, again) = open_document(&app, a, doc_id(1));
    assert!(!again);
    let (doc2, _) = open_document(&app, a, doc_id(2));
    // 같은 session에서 다시 열면 기존 문서 session입니다.
    assert_eq!(open_document(&app, a, doc_id(1)), (doc1, true));
    // 다른 프로젝트 session이 연 문서는 거부합니다.
    let request = app.open_document(b, doc_id(1)).unwrap();
    expect_failure(&app, request, RequestFailure::DocumentInOtherProject);
    let (doc3, _) = open_document(&app, b, doc_id(3));

    let state = app.state();
    let documents: Vec<_> = state
        .documents
        .iter()
        .map(|d| {
            (
                d.document,
                d.project,
                d.document_id,
                d.revision,
                d.node_count,
            )
        })
        .collect();
    assert_eq!(
        documents,
        [
            (doc1, a, doc_id(1), DocumentRevision(0), 0),
            (doc2, a, doc_id(2), DocumentRevision(0), 0),
            (doc3, b, doc_id(3), DocumentRevision(0), 0),
        ]
    );

    // 문서 닫기 → 같은 id를 다시 열면 새 session입니다.
    let request = app.close_document(doc2).unwrap();
    assert_eq!(
        next_event(&app),
        RuntimeEvent::DocumentClosed {
            request,
            document: doc2
        }
    );
    let request = app.close_document(doc2).unwrap();
    expect_failure(&app, request, RequestFailure::UnknownDocument);
    let (reopened, again) = open_document(&app, a, doc_id(2));
    assert!(!again);
    assert_ne!(reopened, doc2);

    // 프로젝트를 닫으면 그 아래 문서도 닫힙니다.
    let request = app.close_project(a).unwrap();
    assert_eq!(
        next_event(&app),
        RuntimeEvent::ProjectClosed {
            request,
            project: a,
            closed_documents: 2,
        }
    );
    let state = app.state();
    assert!(state.project(a).is_none());
    assert!(state.document(doc1).is_none());
    assert!(state.document(doc3).is_some());
    let request = app.open_document(a, doc_id(9)).unwrap();
    expect_failure(&app, request, RequestFailure::UnknownProject);
    // 닫힌 프로젝트의 문서는 이제 다른 session이 열 수 있습니다.
    open_document(&app, b, doc_id(1));
    app.shutdown().unwrap();
}

#[test]
fn outbox_is_bounded_without_losing_accepted_completions() {
    let fixture = TempDir::new("bounded");
    let app = AppRuntime::start_with(QueueCapacity::default(), 2).unwrap();
    let first = app.open_project(fixture.path()).unwrap();
    let second = app.open_project(fixture.path()).unwrap();
    // 완료 event를 꺼내지 않은 요청이 상한(2)이면 새 요청을 거부합니다.
    assert_eq!(
        app.open_project(fixture.path()),
        Err(RequestError::QueueFull)
    );
    assert_eq!(app.outstanding(), 2);
    // poll: 용량 미지정·부족이면 꺼내지 않고 길이만 알립니다.
    let deadline = Instant::now() + Duration::from_secs(5);
    let (kind, len) = loop {
        match app.poll_event(None) {
            EventPoll::Pending { kind, len } => break (kind, len),
            EventPoll::Empty => {
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            other => panic!("꺼내면 안 됨: {other:?}"),
        }
    };
    assert_eq!(
        app.poll_event(Some(len - 1)),
        EventPoll::Pending { kind, len }
    );
    let EventPoll::Taken {
        kind: taken_kind,
        payload,
    } = app.poll_event(Some(len))
    else {
        panic!("꺼내지 못함");
    };
    assert_eq!(taken_kind, kind);
    let event = RuntimeEvent::decode(kind, &payload).unwrap();
    assert_eq!(event.request(), first);
    // 하나를 꺼내면 다시 접수합니다. 접수했던 완료 event는 모두 남아 있습니다.
    let third = app.open_project(fixture.path()).unwrap();
    assert_eq!(next_event(&app).request(), second);
    assert_eq!(next_event(&app).request(), third);
    assert_eq!(app.poll_event(Some(1024)), EventPoll::Empty);
    assert_eq!(app.outstanding(), 0);
    app.shutdown().unwrap();
}

#[test]
fn ui_thread_requests_and_reads_never_wait() {
    // UI thread(ThreadRole::Ui)에서 요청·snapshot 읽기·event poll이 document thread를 기다리지
    // 않고 바로 돌아옵니다.
    let fixture = project_fixture("UI");
    let app = std::sync::Arc::new(AppRuntime::start().unwrap());
    let ui_app = std::sync::Arc::clone(&app);
    let root = fixture.path().to_path_buf();
    let ui = std::thread::spawn(move || {
        let _ui = bind_thread_role(ThreadRole::Ui).unwrap();
        let request = ui_app.open_project(&root).unwrap();
        let _ = ui_app.state();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(event) = ui_app.pop_event() {
                assert_eq!(event.request(), request);
                return event;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
    });
    let event = ui.join().unwrap();
    assert!(matches!(event, RuntimeEvent::ProjectOpened { .. }));
    std::sync::Arc::try_unwrap(app).unwrap().shutdown().unwrap();
}

#[test]
fn shutdown_runs_requests_accepted_before_it() {
    let app = AppRuntime::start().unwrap();
    assert_eq!(app.shutdown().unwrap().jobs_run, 0);
    let app = AppRuntime::start().unwrap();
    let fixture = TempDir::new("stop");
    app.open_project(fixture.path()).unwrap();
    app.close_document(DocumentSessionId(1)).unwrap();
    assert_eq!(app.shutdown().unwrap().jobs_run, 2);
}

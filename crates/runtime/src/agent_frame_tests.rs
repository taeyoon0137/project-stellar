//! frame 앞단 test (XXIX.3.4, ADR-0124 결정 3).

use std::os::unix::net::UnixStream;
use std::time::Duration;

use stellar_agent_edit::lease::LeaseState;
use stellar_agent_edit::permission::ConnectionApproval;
use stellar_agent_edit::provenance::UserUiAction;
use stellar_commands::codec::encode_command;
use stellar_commands::command::Command;
use stellar_core_types::ids::StableId;

use super::*;
use crate::agent_edit::{project_revision, WindowAgentEdit};
use crate::app::AppRuntime;
use crate::app_window_tests::{open_project, open_window};
use crate::window_session::test_fixtures::{saved_project, sid, TempDir};

const WAIT: Duration = Duration::from_secs(5);

struct App {
    _project: TempDir,
    _locks: TempDir,
    app: AppRuntime,
    window: WindowSessionHandle,
}

fn app(tag: &str) -> App {
    let project = saved_project(tag);
    let locks = TempDir::new(&format!("{tag}-locks"));
    let app = AppRuntime::start().unwrap();
    let owner = open_project(&app, project.path());
    let (window, _, _) = open_window(&app, owner);
    let dir = locks.path().to_path_buf();
    let mut ticket = app
        .with_agent_edit(window, move |agent| {
            *agent.agent = WindowAgentEdit::with_lock_dir(dir);
        })
        .unwrap();
    ticket.wait_timeout(WAIT).unwrap().unwrap().unwrap();
    App {
        _project: project,
        _locks: locks,
        app,
        window,
    }
}

fn job<T: Send + 'static>(
    f: &App,
    job: impl FnOnce(&mut crate::agent_edit::AgentWindow<'_>) -> T + Send + 'static,
) -> T {
    let mut ticket = f.app.with_agent_edit(f.window, job).unwrap();
    ticket.wait_timeout(WAIT).unwrap().unwrap().unwrap()
}

fn wait(mut ticket: AgentTicket) -> Result<AgentReply, AgentIoError> {
    ticket
        .wait_timeout(WAIT)
        .unwrap()
        .unwrap()
        .map_err(AgentIoError::from)?
}

fn begin(request: u64) -> Frame {
    Frame::BeginEdit {
        request,
        document: sid(1),
        targets: vec![sid(11)],
        externals: Vec::new(),
        description: "이름 정리".to_string(),
    }
}

fn rename_frame(request: u64, base: &str, name: &str) -> Frame {
    let command = Command::rename_node(sid(11), name.to_string());
    Frame::Propose {
        request,
        base_revision: base.to_string(),
        document: sid(1),
        commands: vec![(command.kind().name().to_string(), encode_command(&command))],
    }
}

fn node_name(f: &App, node: u64) -> String {
    job(f, move |w| {
        w.session()
            .document()
            .unwrap()
            .model()
            .record(sid(node))
            .unwrap()
            .display_name
    })
}

#[test]
fn frames_become_agent_requests_through_the_host() {
    let f = app("agent-frame-flow");
    let mut front = AgentFrontEnd::new(AppAgentHost::new(f.app.agent_io()));
    let conn = AgentConnection(1);
    let (session, started) = front.connect_external(conn, f.window, "외부").unwrap();
    assert_eq!(
        wait(started),
        Ok(AgentReply::Started(ConnectionApproval::Pending))
    );
    // 첫 연결 승인 전 lease 요청은 거부되고, 앞단은 lease를 기억하지 않습니다.
    assert_eq!(
        front.handle(conn, begin(1), WAIT),
        Err(HostRefusal::new(
            ResultKind::NotTrusted,
            "connection-not-approved"
        ))
    );
    assert_eq!(front.lease(conn), None);
    job(&f, move |w| {
        let key = w.permission_key(session);
        w.agent
            .permissions_mut(UserUiAction::from_user_interface())
            .respond_connection(&key, true)
            .unwrap();
    });
    // lease 없이 보낸 제안은 앞단이 거부합니다.
    assert_eq!(
        front.handle(conn, rename_frame(2, "g0", "x"), WAIT),
        Err(HostRefusal::new(ResultKind::NotFound, "no-edit"))
    );
    assert_eq!(
        front.handle(conn, begin(3), WAIT),
        Ok(AgentReply::LeaseRequested(LeaseState::AwaitingApproval))
    );
    let lease = front.lease(conn).expect("앱이 만든 lease");
    job(&f, move |w| {
        w.approve_lease(lease, UserUiAction::from_user_interface())
            .unwrap()
    });
    // 기준 revision은 창이 열어 둔 저장 generation과 같은 표기여야 합니다.
    let base = job(&f, |w| project_revision(w.session()).to_string());
    assert_eq!(
        front
            .handle(conn, rename_frame(4, "g0", "x"), WAIT)
            .map_err(|r| r.kind),
        Err(ResultKind::Stale)
    );
    assert_eq!(
        front
            .handle(conn, rename_frame(5, "기준 아님", "x"), WAIT)
            .map_err(|r| r.code),
        Err("invalid-base-revision")
    );
    let queued = front.handle(conn, rename_frame(6, &base, "프레임"), WAIT);
    assert!(matches!(queued, Ok(AgentReply::Queued(_))), "{queued:?}");
    assert_eq!(
        node_name(&f, 11),
        "n11",
        "승인 전 제안은 문서를 바꾸지 않음"
    );
    // 브리지가 보낸 응답 frame·모르는 전역 resource 종류는 거부합니다.
    assert_eq!(
        front
            .handle(
                conn,
                Frame::Reply {
                    request: 7,
                    result: "ok".into(),
                    message: String::new()
                },
                WAIT
            )
            .map_err(|r| r.code),
        Err("reply-from-bridge")
    );
    let unknown_kind = Frame::BeginEdit {
        request: 8,
        document: sid(1),
        targets: vec![sid(12)],
        externals: vec![("font".to_string(), StableId::new_v7(5, 5, 5))],
        description: String::new(),
    };
    assert_eq!(
        front.convert(conn, unknown_kind).map(|_| ()),
        Err(HostRefusal::new(
            ResultKind::InvalidArguments,
            "unknown-external-kind"
        ))
    );
    // 완료하면 lease를 지웁니다.
    let completed = front.handle(conn, Frame::Complete { request: 9 }, WAIT);
    assert!(
        matches!(completed, Ok(AgentReply::Completed(_))),
        "{completed:?}"
    );
    assert_eq!(front.lease(conn), None);
    // 연결 종료: 외부 클라이언트 세션은 종료 절차로 끝납니다.
    let ended = wait(front.disconnected(conn).unwrap());
    assert!(matches!(ended, Ok(AgentReply::Ended(Some(_)))), "{ended:?}");
    assert!(!job(&f, move |w| w.agent().sessions().is_active(session)));
}

#[test]
fn serve_reads_frames_and_writes_replies_until_the_bridge_closes() {
    let f = app("agent-frame-serve");
    let mut front = AgentFrontEnd::new(AppAgentHost::new(f.app.agent_io()));
    let conn = AgentConnection(2);
    let (session, started) = front.connect_external(conn, f.window, "외부").unwrap();
    wait(started).unwrap();
    job(&f, move |w| {
        let key = w.permission_key(session);
        w.agent
            .permissions_mut(UserUiAction::from_user_interface())
            .respond_connection(&key, true)
            .unwrap();
    });
    let (mut app_side, mut bridge) = UnixStream::pair().unwrap();
    let client = std::thread::spawn(move || {
        write_frame(&mut bridge, &begin(41)).unwrap();
        let first = read_frame(&mut bridge).unwrap();
        // 형식이 틀린 frame을 보내면 앱이 연결 처리를 멈춥니다(fail closed).
        std::io::Write::write_all(&mut bridge, b"XX garbage frame bytes").unwrap();
        bridge.shutdown(std::net::Shutdown::Write).unwrap();
        first
    });
    let end = front.serve(conn, &mut app_side, WAIT);
    assert_eq!(end, FrameError::BadMagic);
    assert_eq!(
        client.join().unwrap(),
        Frame::Reply {
            request: 41,
            result: "ok".to_string(),
            message: "lease-requested".to_string(),
        }
    );
    assert!(front.host().bound(conn).is_none(), "연결 결합 해제");
    // 연결 종료 요청은 job으로 들어갑니다. 끝날 때까지 기다립니다.
    let deadline = std::time::Instant::now() + WAIT;
    while job(&f, move |w| w.agent().sessions().is_active(session)) {
        assert!(std::time::Instant::now() < deadline, "세션 종료");
        std::thread::yield_now();
    }
    assert!(job(&f, |w| w.agent().lease_table().unwrap().is_empty()));
}

#[test]
fn hosted_bridge_bind_and_disconnect_reach_the_agent_session() {
    use stellar_agent_host::session::{
        AgentSession, AgentSessionId as HostSessionId, SessionState, UserStartRequest,
        DEFAULT_RECONNECT_GRACE_MS,
    };
    use stellar_agent_host::start::{FirstConnectionApproval, StartedSession};
    let f = app("agent-frame-hosted");
    let session = job(&f, |w| {
        let started = StartedSession {
            session: AgentSession::start_by_user(UserStartRequest {
                session: HostSessionId(21),
                mode: SessionMode::ReviewThenApply,
                now_ms: 0,
                reconnect_grace_ms: DEFAULT_RECONNECT_GRACE_MS,
            }),
            connection: FirstConnectionApproval {
                project: crate::agent::project_key(w.session().identity()),
                window: w.session().window(),
                session: HostSessionId(21),
                profile_id: "test-double".to_string(),
                mode: SessionMode::ReviewThenApply,
                approved_at_ms: 0,
            },
        };
        w.start_hosted(started, "내장", UserUiAction::from_user_interface(), 0)
            .unwrap()
    });
    let mut front = AgentFrontEnd::new(AppAgentHost::new(f.app.agent_io()));
    let conn = AgentConnection(5);
    let bound = front
        .bind_hosted(conn, f.window, session, SessionMode::ReviewThenApply)
        .unwrap();
    assert_eq!(wait(bound), Ok(AgentReply::Bound));
    let state = |f: &App| {
        job(f, |w| {
            let host = w.agent().hosted().host_ids()[0];
            w.agent().hosted().session(host).unwrap().state()
        })
    };
    assert_eq!(state(&f), SessionState::Running);
    // 내장 세션은 첫 연결 승인이 시작 확인으로 기록되어 바로 lease를 요청할 수 있습니다.
    assert_eq!(
        front.handle(conn, begin(1), WAIT),
        Ok(AgentReply::LeaseRequested(LeaseState::AwaitingApproval))
    );
    // 연결이 끊겨도 앱 시작 세션은 유지되고 재연결 유예에 들어갑니다.
    assert_eq!(
        wait(front.disconnected(conn).unwrap()),
        Ok(AgentReply::Ended(None))
    );
    assert!(matches!(state(&f), SessionState::Reconnecting { .. }));
    assert!(job(&f, move |w| w.agent().sessions().is_active(session)));
}

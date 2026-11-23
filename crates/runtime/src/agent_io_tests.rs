//! 에이전트 I/O 스레드 코어 test (XXIX.3.4, ADR-0112 결정 5).

use std::sync::mpsc;
use std::time::Duration;

use stellar_agent_edit::agent_history::SessionOrigin;
use stellar_agent_edit::lease::{LeaseState, LeaseTargets};
use stellar_agent_edit::permission::{AgentSessionId, ConnectionApproval};
use stellar_agent_edit::provenance::UserUiAction;
use stellar_commands::id::CommandId;
use stellar_commands::lock::lease::LeaseId;
use stellar_core_types::ids::StableId;
use stellar_core_types::revision::DocumentRevision;
use stellar_ops::apply::CommandInput;
use stellar_ops::revision::ProjectRevision;

use super::*;
use crate::agent_edit::{HistoryNames, WindowAgentEdit};
use crate::app::{AppRuntime, RuntimeEvent};
use crate::app_command::AppCommand;
use crate::app_window_tests::{next_solicited, open_project, open_window};
use crate::window_session::test_fixtures::{saved_project, sid, TempDir};

const WAIT: Duration = Duration::from_secs(5);

fn session() -> AgentSessionId {
    AgentSessionId(sid(700))
}

fn lease() -> LeaseId {
    LeaseId(sid(900))
}

fn command_id(n: u64) -> CommandId {
    CommandId::from_stable_id(StableId::new_v7(n, 0x0b0b, n))
}

fn rename(document: StableId, node: u64, name: &str) -> ApplyRequest {
    ApplyRequest {
        base_revision: ProjectRevision::Unsaved,
        document,
        commands: vec![CommandInput::Rename {
            node: sid(node),
            name: name.into(),
        }],
    }
}

fn reply(mut ticket: AgentTicket) -> Result<AgentReply, AgentIoError> {
    ticket
        .wait_timeout(WAIT)
        .unwrap()
        .expect("시간 안에 처리")
        .map_err(AgentIoError::from)?
}

struct App {
    _project: TempDir,
    _locks: TempDir,
    app: AppRuntime,
    window: WindowSessionHandle,
}

/// 창 하나를 연 runtime. 에이전트 lease 조정 디렉터리는 임시 디렉터리입니다.
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

/// document thread를 `release`가 올 때까지 막는 job을 넣습니다.
fn block_document_thread(app: &AppRuntime) -> mpsc::Sender<()> {
    let (release, wait) = mpsc::channel::<()>();
    let (started, running) = mpsc::channel::<()>();
    app.job_publisher()
        .try_submit(SubmitOptions::command(), move |_ctx| {
            started.send(()).unwrap();
            let _ = wait.recv_timeout(WAIT);
        })
        .unwrap();
    running.recv_timeout(WAIT).unwrap();
    release
}

fn revision(app: &App) -> DocumentRevision {
    let mut ticket = app
        .app
        .with_agent_edit(app.window, |agent| agent.session().revision())
        .unwrap();
    ticket.wait_timeout(WAIT).unwrap().unwrap().unwrap()
}

#[test]
fn io_thread_queues_validated_requests_without_writing_the_document() {
    // 검증 기준(XXIX.3.4 코어): I/O 스레드는 문서를 쓰지 않고, 검증한 요청만 document thread에 넣습니다.
    let f = app("agent-io-flow");
    let thread = AgentIoThread::spawn(f.app.agent_io(), DEFAULT_INBOX_CAPACITY).unwrap();
    let send = |request: AgentRequest| {
        let ticket = thread
            .send(f.window, request)
            .unwrap()
            .recv_timeout(WAIT)
            .unwrap()
            .unwrap();
        reply(ticket)
    };
    assert_eq!(
        send(AgentRequest::Start {
            session: session(),
            origin: SessionOrigin::ExternalClient,
            name: "외부".into(),
        }),
        Ok(AgentReply::Started(ConnectionApproval::Pending))
    );
    assert_eq!(
        send(AgentRequest::BeginEdit {
            session: session(),
            lease: lease(),
            targets: LeaseTargets {
                document: sid(1),
                targets: vec![sid(11)],
                externals: Vec::new(),
            },
        }),
        Ok(AgentReply::LeaseRequested(LeaseState::AwaitingApproval))
    );
    // 사용자가 lease를 승인합니다(앱 UI 경로).
    let mut approved = f
        .app
        .with_agent_edit(f.window, |agent| {
            agent.approve_lease(lease(), UserUiAction::from_user_interface())
        })
        .unwrap();
    assert_eq!(
        approved.wait_timeout(WAIT).unwrap().unwrap().unwrap(),
        Ok(LeaseState::Granted)
    );
    // 닫힌 enum으로 바꿀 수 없는 proposal은 I/O 스레드에서 거부되고 document thread에 들어가지 않습니다.
    let empty = ApplyRequest {
        base_revision: ProjectRevision::Unsaved,
        document: sid(1),
        commands: Vec::new(),
    };
    let rejected = thread
        .send(
            f.window,
            AgentRequest::Propose {
                session: session(),
                lease: lease(),
                id: command_id(1),
                request: empty,
            },
        )
        .unwrap()
        .recv_timeout(WAIT)
        .unwrap();
    assert!(matches!(rejected, Err(AgentIoError::Invalid(_))));
    // 다른 문서의 proposal은 document thread가 거부합니다.
    assert!(matches!(
        send(AgentRequest::Propose {
            session: session(),
            lease: lease(),
            id: command_id(2),
            request: rename(sid(77), 11, "x"),
        }),
        Err(AgentIoError::Invalid(_))
    ));
    // 올바른 proposal은 승인 대기로 들어가고 문서는 그대로입니다.
    let before = revision(&f);
    let queued = send(AgentRequest::Propose {
        session: session(),
        lease: lease(),
        id: command_id(3),
        request: rename(sid(1), 11, "에이전트"),
    });
    let Ok(AgentReply::Queued(proposal)) = queued else {
        panic!("{queued:?}");
    };
    assert_eq!(revision(&f), before, "I/O 경로는 문서를 바꾸지 않습니다");
    // 사용자가 승인하면 그때 적용됩니다. 완료 요청은 그룹 항목 하나로 확정합니다.
    let mut applied = f
        .app
        .with_agent_edit(f.window, move |agent| {
            agent.apply_proposal(session(), proposal, UserUiAction::from_user_interface())
        })
        .unwrap();
    assert!(applied
        .wait_timeout(WAIT)
        .unwrap()
        .unwrap()
        .unwrap()
        .is_ok());
    assert!(revision(&f) > before);
    assert!(matches!(
        send(AgentRequest::Complete {
            session: session(),
            entry_id: command_id(4),
        }),
        Ok(AgentReply::Completed(_))
    ));
    let mut names = f.app.agent_history_names(f.window).unwrap();
    assert_eq!(
        names.wait_timeout(WAIT).unwrap().unwrap().unwrap(),
        HistoryNames {
            undo: Some("에이전트 편집"),
            redo: None
        }
    );
    // 앱 Undo 명령은 출처 표시 전이를 거칩니다.
    let request = f.app.perform(f.window, AppCommand::Undo).unwrap();
    assert!(matches!(
        next_solicited(&f.app),
        RuntimeEvent::HistoryReplayed { request: got, applied: true, .. } if got == request
    ));
    let mut shown = f
        .app
        .with_agent_edit(f.window, |agent| {
            agent
                .agent()
                .provenance()
                .batches()
                .iter()
                .map(|b| b.display())
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(
        shown.wait_timeout(WAIT).unwrap().unwrap().unwrap(),
        vec![stellar_agent_edit::provenance::DisplayState::Undone]
    );
}

#[test]
fn in_flight_limit_returns_busy_without_waiting() {
    // 검증 기준(XXIX.3.4 코어): 넘치면 busy를 바로 돌려줍니다. 이미 접수한 요청은 버리지 않습니다.
    let f = app("agent-io-busy");
    let io = f.app.agent_io_with_limit(1);
    let release = block_document_thread(&f.app);
    let start = AgentRequest::Start {
        session: session(),
        origin: SessionOrigin::ExternalClient,
        name: "외부".into(),
    };
    let first = io.submit(f.window, start.clone()).unwrap();
    assert_eq!(io.in_flight(), 1);
    assert_eq!(io.submit(f.window, start).unwrap_err(), AgentIoError::Busy);
    release.send(()).unwrap();
    assert_eq!(
        reply(first),
        Ok(AgentReply::Started(ConnectionApproval::Pending))
    );
    assert_eq!(io.in_flight(), 0, "처리한 요청은 자리를 돌려줍니다");
    // 모르는 창은 document thread에서 거부합니다.
    let unknown = io
        .submit(
            WindowSessionHandle(999),
            AgentRequest::Complete {
                session: session(),
                entry_id: command_id(9),
            },
        )
        .unwrap();
    assert_eq!(
        reply(unknown),
        Err(AgentIoError::UnknownWindow(WindowSessionHandle(999)))
    );
}

#[test]
fn agent_requests_run_after_user_input() {
    // 검증 기준(XXIX.3.4 코어): 에이전트 요청은 사용자 입력보다 낮은 priority입니다.
    let f = app("agent-io-priority");
    let io = f.app.agent_io();
    let release = block_document_thread(&f.app);
    let agent = io
        .submit(
            f.window,
            AgentRequest::Start {
                session: session(),
                origin: SessionOrigin::ExternalClient,
                name: "외부".into(),
            },
        )
        .unwrap();
    let window = f.window;
    let mut input = f
        .app
        .job_publisher()
        .try_submit(SubmitOptions::input(), move |ctx| {
            ctx.app_sessions_mut().windows[&window]
                .agent
                .sessions()
                .is_active(session())
        })
        .unwrap();
    release.send(()).unwrap();
    assert_eq!(
        input.wait_timeout(WAIT).unwrap(),
        Some(false),
        "입력 job이 먼저 실행돼 세션이 아직 없습니다"
    );
    assert!(reply(agent).is_ok());
}

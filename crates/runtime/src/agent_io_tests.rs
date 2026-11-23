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

fn rename(base: ProjectRevision, document: StableId, node: u64, name: &str) -> ApplyRequest {
    ApplyRequest {
        base_revision: base,
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

/// 창이 열어 둔 저장 generation revision(에이전트가 읽은 기준).
fn base(app: &App) -> ProjectRevision {
    let mut ticket = app
        .app
        .with_agent_edit(app.window, |agent| {
            crate::agent_edit::project_revision(agent.session())
        })
        .unwrap();
    ticket.wait_timeout(WAIT).unwrap().unwrap().unwrap()
}

/// 사용자가 첫 연결을 승인합니다(앱 UI 경로).
fn approve_connection(app: &App, session: AgentSessionId) {
    let mut ticket = app
        .app
        .with_agent_edit(app.window, move |agent| {
            let key = agent.permission_key(session);
            agent
                .agent
                .permissions_mut(UserUiAction::from_user_interface())
                .respond_connection(&key, true)
        })
        .unwrap();
    assert_eq!(
        ticket.wait_timeout(WAIT).unwrap().unwrap().unwrap(),
        Ok(ConnectionApproval::Approved)
    );
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
    // 첫 연결 승인 전에는 lease 요청을 받지 않습니다(XIX.3.3, ADR-0124 결정 3).
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
        Err(AgentIoError::ConnectionNotApproved(
            ConnectionApproval::Pending
        ))
    );
    approve_connection(&f, session());
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
            request: rename(base(&f), sid(77), 11, "x"),
        }),
        Err(AgentIoError::Invalid(_))
    ));
    // 창이 열어 둔 저장 generation과 다른 기준의 proposal은 대기로 두지 않고 Stale로 거부합니다.
    let saved = base(&f);
    assert_ne!(
        saved,
        ProjectRevision::Unsaved,
        "저장된 fixture 프로젝트입니다"
    );
    let stale = send(AgentRequest::Propose {
        session: session(),
        lease: lease(),
        id: command_id(5),
        request: rename(ProjectRevision::Unsaved, sid(1), 11, "x"),
    });
    let Err(AgentIoError::Invalid(error)) = stale else {
        panic!("{stale:?}");
    };
    assert_eq!(
        error,
        stellar_agent_edit::attached::edit::AttachedEditError::StaleBase {
            expected: ProjectRevision::Unsaved,
            found: saved,
        }
    );
    assert_eq!(
        error.kind(),
        stellar_agent_edit::agent_protocol::outcome::ResultKind::Stale
    );
    // 올바른 proposal은 승인 대기로 들어가고 문서는 그대로입니다.
    let before = revision(&f);
    let queued = send(AgentRequest::Propose {
        session: session(),
        lease: lease(),
        id: command_id(3),
        request: rename(saved, sid(1), 11, "에이전트"),
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

/// `until`이 참이 될 때까지 최대 [`WAIT`] 기다립니다.
fn wait_for(mut until: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + WAIT;
    while std::time::Instant::now() < deadline {
        if until() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    until()
}

#[test]
fn revert_through_the_runtime_job_is_autosaved() {
    // 검증 기준(XXIX.3.1 엔진 부분, ADR-0112 결정 4): 앱 사용자 UI의 되돌리기는 job 뒤 상태 게시로 자동 저장 예약에
    // 들어가 다음 자동 저장에 기록됩니다(자동 저장 경로를 따로 두지 않음).
    let mut f = app("agent-io-revert-autosave");
    f.app
        .enable_autosave_with(stellar_project::autosave::AutosavePolicy {
            debounce: Duration::from_millis(150),
            max_delay: Duration::from_millis(1_000),
            app_deactivate_flush: stellar_project::autosave::FlushPolicy {
                timeout: Duration::from_millis(500),
            },
            window_close_flush: stellar_project::autosave::FlushPolicy {
                timeout: Duration::from_millis(5_000),
            },
        });
    let io = f.app.agent_io();
    let send = |request: AgentRequest| reply(io.submit(f.window, request).unwrap());
    send(AgentRequest::Start {
        session: session(),
        origin: SessionOrigin::ExternalClient,
        name: "외부".into(),
    })
    .unwrap();
    approve_connection(&f, session());
    send(AgentRequest::BeginEdit {
        session: session(),
        lease: lease(),
        targets: LeaseTargets {
            document: sid(1),
            targets: vec![sid(11)],
            externals: Vec::new(),
        },
    })
    .unwrap();
    let ui = UserUiAction::from_user_interface;
    let mut approved = f
        .app
        .with_agent_edit(f.window, move |agent| agent.approve_lease(lease(), ui()))
        .unwrap();
    approved
        .wait_timeout(WAIT)
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    let Ok(AgentReply::Queued(proposal)) = send(AgentRequest::Propose {
        session: session(),
        lease: lease(),
        id: command_id(1),
        request: rename(base(&f), sid(1), 11, "에이전트"),
    }) else {
        panic!("제안 대기");
    };
    let mut applied = f
        .app
        .with_agent_edit(f.window, move |agent| {
            agent.apply_proposal(session(), proposal, ui())
        })
        .unwrap();
    let batch = applied
        .wait_timeout(WAIT)
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    send(AgentRequest::Complete {
        session: session(),
        entry_id: command_id(2),
    })
    .unwrap();
    let window = f.window;
    let state = |app: &AppRuntime| app.state().window(window).unwrap().clone();
    assert!(
        wait_for(|| !state(&f.app).has_unsaved_changes),
        "적용한 제안이 먼저 자동 저장됩니다"
    );
    let saved = state(&f.app).generation;
    // 되돌리기 미리보기 승인 → 적용. job이 끝나면 상태 게시가 자동 저장을 예약합니다.
    let mut reverted = f
        .app
        .with_agent_edit(f.window, move |agent| {
            let preview = agent.preview_revert(
                stellar_agent_edit::provenance::ProvenanceScope::Batch(batch),
                &std::collections::BTreeSet::new(),
                || stellar_ops::validate::Definitions::Failed("정의 없음"),
            )?;
            agent.apply_revert(&preview, command_id(3), ui())
        })
        .unwrap();
    reverted
        .wait_timeout(WAIT)
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    let after = state(&f.app);
    assert!(after.has_unsaved_changes, "되돌리기는 저장 전 변경입니다");
    assert!(
        matches!(
            after.autosave,
            stellar_project::autosave::schedule::AutosaveStatus::Pending
                | stellar_project::autosave::schedule::AutosaveStatus::Saving
        ),
        "{:?}",
        after.autosave
    );
    assert!(
        wait_for(|| {
            let now = state(&f.app);
            !now.has_unsaved_changes && now.generation > saved
        }),
        "되돌리기가 다음 자동 저장에 기록됩니다"
    );
    let mut names = f.app.agent_history_names(f.window).unwrap();
    assert_eq!(
        names.wait_timeout(WAIT).unwrap().unwrap().unwrap().undo,
        Some("에이전트 변경 되돌리기")
    );
}

#[test]
fn large_approved_proposal_is_applied_in_batch_jobs() {
    // 검증 기준(XXIX.3.4, ADR-0124 결정 6): 큰 제안은 batch 하나가 job 하나로 나뉘어 적용되고, 완료는 history 항목 하나입니다.
    let f = app("agent-io-batches");
    let io = f.app.agent_io();
    let send = |request: AgentRequest| reply(io.submit(f.window, request).unwrap());
    send(AgentRequest::Start {
        session: session(),
        origin: SessionOrigin::ExternalClient,
        name: "외부".into(),
    })
    .unwrap();
    approve_connection(&f, session());
    send(AgentRequest::BeginEdit {
        session: session(),
        lease: lease(),
        targets: LeaseTargets {
            document: sid(1),
            targets: vec![sid(11)],
            externals: Vec::new(),
        },
    })
    .unwrap();
    let ui = UserUiAction::from_user_interface;
    let mut approved = f
        .app
        .with_agent_edit(f.window, move |agent| agent.approve_lease(lease(), ui()))
        .unwrap();
    approved
        .wait_timeout(WAIT)
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    let count = 2 * crate::agent_edit::PROPOSAL_BATCH_COMMANDS + 76;
    let request = ApplyRequest {
        base_revision: base(&f),
        document: sid(1),
        commands: (0..count)
            .map(|i| CommandInput::Rename {
                node: sid(11),
                name: format!("이름 {i}"),
            })
            .collect(),
    };
    let Ok(AgentReply::Queued(proposal)) = send(AgentRequest::Propose {
        session: session(),
        lease: lease(),
        id: command_id(1),
        request,
    }) else {
        panic!("제안 대기");
    };
    let before = revision(&f);
    // batch 사이에 다른 job이 끼어드는지 봅니다: document thread를 막고 적용(첫 batch job)을 넣은 뒤, 사용자 입력 job과
    // 그 뒤의 일반 command job을 넣습니다. 입력은 첫 batch보다 먼저, 일반 job은 첫 batch 뒤·둘째 batch 앞에 실행됩니다.
    let release = block_document_thread(&f.app);
    let applying = f
        .app
        .apply_agent_proposal(f.window, session(), proposal, ui())
        .unwrap();
    let window = f.window;
    let publisher = f.app.job_publisher();
    let batches_seen = |options: SubmitOptions| {
        publisher
            .try_submit(options, move |ctx| {
                ctx.app_sessions_mut().windows[&window]
                    .agent
                    .provenance()
                    .session_batches(session())
                    .count()
            })
            .unwrap()
    };
    let mut between = batches_seen(SubmitOptions::command());
    let mut input = batches_seen(SubmitOptions::input());
    release.send(()).unwrap();
    assert_eq!(input.wait_timeout(WAIT).unwrap(), Some(0), "입력이 먼저");
    assert_eq!(
        between.wait_timeout(WAIT).unwrap(),
        Some(1),
        "다음 batch는 lane 뒤에 다시 들어가 그 사이 job이 실행됨"
    );
    let result = applying.recv_timeout(WAIT).unwrap();
    assert_eq!(result.error, None);
    assert_eq!(result.applied.len(), 3, "512 + 512 + 76");
    assert!(revision(&f) > before);
    let applied = result.applied.clone();
    let mut sizes = f
        .app
        .with_agent_edit(f.window, move |agent| {
            applied
                .iter()
                .map(|batch| {
                    agent
                        .agent()
                        .provenance()
                        .get(*batch)
                        .unwrap()
                        .record
                        .forward
                        .requests
                        .len()
                })
                .collect::<Vec<_>>()
        })
        .unwrap();
    assert_eq!(
        sizes.wait_timeout(WAIT).unwrap().unwrap().unwrap(),
        vec![512, 512, 76]
    );
    let Ok(AgentReply::Completed(completed)) = send(AgentRequest::Complete {
        session: session(),
        entry_id: command_id(2),
    }) else {
        panic!("완료");
    };
    assert_eq!(completed.batches, 3);
    assert!(completed.history_sequence.is_some(), "history 항목 하나");
}

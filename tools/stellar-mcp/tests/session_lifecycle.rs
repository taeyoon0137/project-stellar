//! XIX.1.4 검증: 외부 bridge disconnect 후 session permission 0, 연결 종료 event 1회, 문서 revision
//! 변화 0. 한 bridge process는 한 MCP 연결 세션만 담당합니다.
//!
//! 앱 쪽은 `stellar_local_ipc`의 실제 앱 측 구현(`AppListener` 인증·handshake, `ConnectionMonitor`
//! 연결 수명)을 쓰고, 세션 권한 저장소와 문서는 아직 없으므로(XIX.3 권한, XIX.6.4 종료 절차) 종료
//! 절차 hook(`SessionTermination`)을 구현한 test double [`TestApp`]이 대신합니다. bridge는 빌드된
//! `stellar-mcp --attach --instance <id>` 실제 프로세스입니다.
//!
//! 프로세스·socket을 만드는 테스트는 fd 상속 경합을 피하도록 [`SERIAL`]로 하나씩 실행합니다.

#![cfg(target_os = "macos")]

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use stellar_mcp::exit;
use stellar_mcp::ipc::auth::PeerPolicy;
use stellar_mcp::ipc::listener::{AppEndpoint, AppListener, ListenerEvent};
use stellar_mcp::ipc::registry::{InstanceId, Registry};
use stellar_mcp::ipc::session::{
    CloseCause, ConnectionClosed, ConnectionMonitor, SessionOrigin, SessionTermination,
};

const BIN: &str = env!("CARGO_BIN_EXE_stellar-mcp");
const WAIT: Duration = Duration::from_secs(10);
const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"lifecycle","version":"1"}}}"#;

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn unique_instance() -> InstanceId {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    InstanceId::parse(&format!(
        "{:08x}{:08x}",
        std::process::id(),
        0xb000_0000 | n
    ))
    .unwrap()
}

/// 세션 권한 저장소와 문서 revision을 대신하는 앱 test double.
///
/// 연결이 수립되면 그 세션에 권한을 부여(첫 연결 승인을 대신)하고, 외부 세션 종료 절차가 시작되면
/// 권한을 모두 해제합니다. 종료 절차는 문서를 바꾸지 않습니다(자동 rollback 없음).
struct TestApp {
    permissions: Mutex<BTreeSet<&'static str>>,
    revision: AtomicU64,
    terminations: AtomicUsize,
}

impl TestApp {
    fn new() -> Arc<TestApp> {
        Arc::new(TestApp {
            permissions: Mutex::new(BTreeSet::new()),
            revision: AtomicU64::new(7),
            terminations: AtomicUsize::new(0),
        })
    }

    fn grant(&self) {
        let mut permissions = self.permissions.lock().unwrap();
        permissions.insert("read");
        permissions.insert("propose");
    }

    fn permission_count(&self) -> usize {
        self.permissions.lock().unwrap().len()
    }
}

impl SessionTermination for TestApp {
    fn begin_external_termination(&self, _closed: &ConnectionClosed) {
        self.terminations.fetch_add(1, Ordering::SeqCst);
        self.permissions.lock().unwrap().clear();
    }
}

/// 실행 중인 앱(listener)과 그 앱에 붙은 bridge 프로세스.
struct Attached {
    _app: AppListener,
    monitor: Option<ConnectionMonitor>,
    events: Receiver<ConnectionClosed>,
    bridge: Child,
}

/// 앱을 열고 bridge를 붙인 뒤 initialize 응답까지 확인합니다.
fn attach(app_state: &Arc<TestApp>, origin: SessionOrigin) -> Attached {
    let registry = Registry::for_current_user().unwrap();
    let instance = unique_instance();
    let endpoint = AppEndpoint::bind(&registry, instance.clone()).unwrap();
    let app = AppListener::spawn(endpoint, PeerPolicy::for_current_process().unwrap()).unwrap();
    let mut bridge = Command::new(BIN)
        .args(["--attach", "--instance", instance.as_str()])
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let connection = match app.events().recv_timeout(WAIT).unwrap() {
        ListenerEvent::Connected { connection, .. } => connection,
        other => panic!("{other:?}"),
    };
    let (tx, events) = mpsc::channel();
    let monitor = ConnectionMonitor::watch(
        connection,
        origin,
        tx,
        Arc::clone(app_state) as Arc<dyn SessionTermination>,
    )
    .unwrap();
    app_state.grant();
    // MCP 세션 시작(initialize)을 확인합니다.
    let stdin = bridge.stdin.as_mut().unwrap();
    stdin.write_all(INIT.as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    BufReader::new(bridge.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert!(
        line.contains("\"protocolVersion\":\"2025-11-25\""),
        "{line}"
    );
    Attached {
        _app: app,
        monitor: Some(monitor),
        events,
        bridge,
    }
}

fn wait_exit(child: &mut Child) -> Option<i32> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code();
        }
        if start.elapsed() > WAIT {
            let _ = child.kill();
            panic!("bridge가 끝나지 않음");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// event를 모두 모읍니다(첫 event를 기다린 뒤 짧게 더 확인).
fn collect_events(events: &Receiver<ConnectionClosed>) -> Vec<ConnectionClosed> {
    let mut out = vec![events.recv_timeout(WAIT).expect("연결 종료 event")];
    while let Ok(event) = events.recv_timeout(Duration::from_millis(200)) {
        out.push(event);
    }
    out
}

#[test]
fn fixture_external_bridge_disconnect_releases_permissions_once() {
    let _serial = serial();
    // (a) client가 stdin을 닫는 정상 종료, (b) bridge 강제 종료(SIGKILL).
    for kill in [false, true] {
        let app_state = TestApp::new();
        let mut attached = attach(&app_state, SessionOrigin::External);
        assert_eq!(app_state.permission_count(), 2, "연결 중 권한 있음");
        let revision_before = app_state.revision.load(Ordering::SeqCst);

        if kill {
            attached.bridge.kill().unwrap();
            attached.bridge.wait().unwrap();
        } else {
            drop(attached.bridge.stdin.take());
            assert_eq!(wait_exit(&mut attached.bridge), Some(0), "정상 종료");
        }
        let events = collect_events(&attached.events);
        assert_eq!(
            events.len(),
            1,
            "kill={kill}: 연결 종료 event 1회: {events:?}"
        );
        assert_eq!(events[0].cause, CloseCause::PeerClosed);
        assert_eq!(events[0].origin, SessionOrigin::External);
        assert_eq!(
            app_state.permission_count(),
            0,
            "kill={kill}: session permission 0"
        );
        assert_eq!(app_state.terminations.load(Ordering::SeqCst), 1);
        assert_eq!(
            app_state.revision.load(Ordering::SeqCst),
            revision_before,
            "kill={kill}: 문서 revision 변화 0"
        );
        // 앱이 감시를 정리해도 event는 늘지 않습니다.
        drop(attached.monitor.take());
        assert!(attached
            .events
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        assert_eq!(app_state.terminations.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn app_started_session_disconnect_leaves_lifetime_to_agent_session() {
    let _serial = serial();
    let app_state = TestApp::new();
    let mut attached = attach(&app_state, SessionOrigin::AppStarted);
    drop(attached.bridge.stdin.take());
    assert_eq!(wait_exit(&mut attached.bridge), Some(0));
    let events = collect_events(&attached.events);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].origin, SessionOrigin::AppStarted);
    assert_eq!(
        app_state.terminations.load(Ordering::SeqCst),
        0,
        "앱이 시작한 세션의 권한 수명은 AgentSession이 정함"
    );
    assert_eq!(app_state.permission_count(), 2);
}

#[test]
fn app_closing_the_connection_ends_the_single_bridge_session() {
    let _serial = serial();
    let app_state = TestApp::new();
    let mut attached = attach(&app_state, SessionOrigin::External);
    // 한 bridge는 한 MCP 세션만 담당합니다: 같은 연결에서 다시 initialize할 수 없습니다.
    let stdin = attached.bridge.stdin.as_mut().unwrap();
    stdin
        .write_all(INIT.replace("\"id\":1", "\"id\":2").as_bytes())
        .unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    BufReader::new(attached.bridge.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert!(
        line.contains("\"id\":2") && line.contains("-32600"),
        "{line}"
    );

    // 앱이 연결을 닫으면 bridge는 다시 연결하지 않고 69(unavailable)로 끝납니다.
    drop(attached.monitor.take());
    assert_eq!(
        wait_exit(&mut attached.bridge),
        Some(i32::from(exit::UNAVAILABLE.code))
    );
    let events = collect_events(&attached.events);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cause, CloseCause::AppShutdown);
    assert_eq!(app_state.terminations.load(Ordering::SeqCst), 1);
    assert_eq!(app_state.permission_count(), 0);
    assert!(
        attached
            ._app
            .events()
            .recv_timeout(Duration::from_millis(300))
            .is_err(),
        "bridge 재연결 0"
    );
}

//! 앱 측 bridge 연결 수명과 연결 종료 event (XIX.1.4).
//!
//! 한 bridge process는 한 MCP 연결 세션만 담당하고, 그 세션은 앱과의 IPC 연결 하나에 묶입니다.
//! 앱은 인증·handshake를 마친 연결([`AppConnection`])을 [`ConnectionMonitor::watch`]에 넘겨 수명을
//! 감시합니다.
//!
//! - bridge가 끝나면(정상 종료·강제 종료 모두 OS가 socket을 닫음) 감시 스레드가 EOF를 보고
//!   [`ConnectionClosed`] event를 **정확히 한 번** 기록합니다. 앱 쪽에서 먼저 닫거나(종료·창 닫기),
//!   읽기 오류가 나거나, 정의되지 않은 data가 와도 같은 event 하나로 끝납니다. 여러 경로가 겹쳐도
//!   첫 경로만 기록합니다.
//! - 외부 클라이언트 세션([`SessionOrigin::External`])은 그 시점에
//!   [`SessionTermination::begin_external_termination`]을 한 번 호출해 XIX.6.4 종료 절차(권한·lease
//!   해제, 자동 rollback 없음)를 시작합니다. event는 그 호출이 끝난 뒤 보냅니다.
//! - 앱이 시작한 세션([`SessionOrigin::AppStarted`])은 권한·lease 수명을 AgentSession(XXIX.2.8)이
//!   정하므로 종료 절차를 호출하지 않고 event만 기록합니다.
//!
//! handshake 뒤의 bridge↔app 메시지는 아직 정의되지 않았으므로(XIX.9.4), 감시 스레드가 data를 받으면
//! protocol 위반으로 연결을 닫습니다(fail closed). 이 crate는 document·commands에 의존하지 않으므로
//! 감시와 종료 event 기록은 문서 상태를 바꿀 수 없습니다.

use std::io::{self, Read};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::auth::AuthenticatedPeer;
use crate::listener::AppConnection;

/// 연결 감시 스레드 이름.
pub const CONNECTION_THREAD_NAME: &str = "stellar-ipc-conn";

/// 세션을 시작한 쪽.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOrigin {
    /// 외부 MCP client가 띄운 bridge. bridge 연결이 끝나면 세션도 끝납니다.
    External,
    /// 앱이 시작한 세션(내장 에이전트, XIX.1.8 결합). 권한·lease 수명은 AgentSession이 정합니다.
    AppStarted,
}

/// 앱 프로세스 안에서 유일한 연결 id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConnectionId(u64);

impl ConnectionId {
    fn next() -> ConnectionId {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        ConnectionId(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    /// 숫자 값.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// 연결이 끝난 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseCause {
    /// bridge가 연결을 닫았습니다(정상 종료·강제 종료).
    PeerClosed,
    /// 앱이 연결을 닫았습니다.
    AppShutdown,
    /// handshake 뒤 정의되지 않은 data를 받았습니다.
    UnexpectedData,
    /// 읽기 오류.
    ReadError {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
}

/// 연결 종료 event. 연결마다 정확히 한 번 기록합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionClosed {
    /// 연결 id.
    pub id: ConnectionId,
    /// 세션을 시작한 쪽.
    pub origin: SessionOrigin,
    /// 인증된 bridge.
    pub peer: AuthenticatedPeer,
    /// 종료 이유.
    pub cause: CloseCause,
}

/// 외부 클라이언트 세션 종료 절차를 시작하는 앱 쪽 구현(XIX.6.4).
///
/// 연결 감시 스레드에서 호출되므로 UI·Document 스레드 작업은 앱이 자기 queue로 넘겨야 합니다.
pub trait SessionTermination: Send + Sync {
    /// 외부 클라이언트 세션의 종료 절차를 시작합니다. 연결마다 최대 한 번 호출됩니다.
    fn begin_external_termination(&self, closed: &ConnectionClosed);
}

/// 연결 하나의 공유 상태.
struct Shared {
    id: ConnectionId,
    origin: SessionOrigin,
    peer: AuthenticatedPeer,
    closed: AtomicBool,
    events: Mutex<Sender<ConnectionClosed>>,
    termination: Arc<dyn SessionTermination>,
}

impl Shared {
    /// 처음 호출이면 종료를 기록하고 `true`, 이미 기록했으면 `false`입니다.
    fn record(&self, cause: CloseCause) -> bool {
        if self.closed.swap(true, Ordering::SeqCst) {
            return false;
        }
        let event = ConnectionClosed {
            id: self.id,
            origin: self.origin,
            peer: self.peer,
            cause,
        };
        if self.origin == SessionOrigin::External {
            self.termination.begin_external_termination(&event);
        }
        let sender = self.events.lock().unwrap_or_else(|e| e.into_inner());
        let _ = sender.send(event);
        true
    }
}

/// 감시 중인 연결. drop하면 앱 쪽에서 연결을 닫고([`CloseCause::AppShutdown`], 이미 끝났으면
/// 무시) 감시 스레드를 기다립니다.
pub struct ConnectionMonitor {
    shared: Arc<Shared>,
    stream: UnixStream,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for ConnectionMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionMonitor")
            .field("id", &self.shared.id)
            .field("origin", &self.shared.origin)
            .field("closed", &self.shared.closed.load(Ordering::SeqCst))
            .finish()
    }
}

impl ConnectionMonitor {
    /// 인증된 연결의 감시를 시작합니다. 종료 event는 `events`로 한 번 보냅니다.
    pub fn watch(
        connection: AppConnection,
        origin: SessionOrigin,
        events: Sender<ConnectionClosed>,
        termination: Arc<dyn SessionTermination>,
    ) -> io::Result<ConnectionMonitor> {
        let peer = connection.peer();
        let stream = connection.into_stream();
        // handshake timeout을 풀고 bridge가 끝날 때까지 기다립니다.
        stream.set_read_timeout(None)?;
        let reader = stream.try_clone()?;
        let shared = Arc::new(Shared {
            id: ConnectionId::next(),
            origin,
            peer,
            closed: AtomicBool::new(false),
            events: Mutex::new(events),
            termination,
        });
        let thread_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name(CONNECTION_THREAD_NAME.into())
            .spawn(move || watch_loop(reader, &thread_shared))?;
        Ok(ConnectionMonitor {
            shared,
            stream,
            thread: Some(thread),
        })
    }

    /// 연결 id.
    pub fn id(&self) -> ConnectionId {
        self.shared.id
    }

    /// 종료가 기록되었는지 확인합니다.
    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::SeqCst)
    }

    /// 앱 쪽에서 연결을 닫습니다. 처음 닫는 경로이면 `true`입니다.
    pub fn close(&self) -> bool {
        let first = self.shared.record(CloseCause::AppShutdown);
        let _ = self.stream.shutdown(Shutdown::Both);
        first
    }
}

impl Drop for ConnectionMonitor {
    fn drop(&mut self) {
        self.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn watch_loop(mut reader: UnixStream, shared: &Shared) {
    let mut buf = [0u8; 64];
    let cause = loop {
        match reader.read(&mut buf) {
            Ok(0) => break CloseCause::PeerClosed,
            Ok(_) => break CloseCause::UnexpectedData,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => break CloseCause::ReadError { kind: e.kind() },
        }
    };
    shared.record(cause);
    let _ = reader.shutdown(Shutdown::Both);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::PeerPolicy;
    use crate::handshake::{connect, Session};
    use crate::listener::{AppEndpoint, AppListener, ListenerEvent};
    use crate::nonce::NonceLedger;
    use crate::os;
    use crate::registry::{InstanceId, Registry};
    use crate::test_support::TempDir;
    use std::io::Write;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc::{self, Receiver};
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(10);

    /// 종료 절차 호출을 세는 test double.
    #[derive(Default)]
    struct CountingTermination {
        calls: AtomicUsize,
    }

    impl SessionTermination for CountingTermination {
        fn begin_external_termination(&self, _closed: &ConnectionClosed) {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// 앱 listener와 인증된 bridge session 한 쌍.
    fn pair(tmp: &TempDir) -> (AppListener, Session, AppConnection) {
        let registry = Registry::at(tmp.path.join("r"), os::current_uid());
        let endpoint = AppEndpoint::bind(&registry, InstanceId::from_bytes([3; 8])).unwrap();
        let ep = endpoint.endpoint().clone();
        let policy = PeerPolicy::for_current_process().unwrap();
        let app = AppListener::spawn(endpoint, policy.clone()).unwrap();
        let session = connect(&ep, &policy, &mut NonceLedger::new()).unwrap();
        let connection = match app.events().recv_timeout(WAIT).unwrap() {
            ListenerEvent::Connected { connection, .. } => connection,
            other => panic!("{other:?}"),
        };
        (app, session, connection)
    }

    fn watch(
        connection: AppConnection,
        origin: SessionOrigin,
    ) -> (
        ConnectionMonitor,
        Receiver<ConnectionClosed>,
        Arc<CountingTermination>,
    ) {
        let (tx, rx) = mpsc::channel();
        let termination = Arc::new(CountingTermination::default());
        let monitor =
            ConnectionMonitor::watch(connection, origin, tx, termination.clone()).unwrap();
        (monitor, rx, termination)
    }

    #[test]
    fn bridge_close_records_one_event_and_starts_external_termination_once() {
        let tmp = TempDir::new();
        let (_app, session, connection) = pair(&tmp);
        let (monitor, rx, termination) = watch(connection, SessionOrigin::External);
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "연결 중 event 없음"
        );
        drop(session);
        let event = rx.recv_timeout(WAIT).unwrap();
        assert_eq!(event.cause, CloseCause::PeerClosed);
        assert_eq!(event.origin, SessionOrigin::External);
        assert_eq!(event.id, monitor.id());
        assert_eq!(termination.calls.load(Ordering::SeqCst), 1);
        // 이후 앱 쪽 종료·drop은 event를 더 만들지 않습니다.
        assert!(!monitor.close());
        drop(monitor);
        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "event 1회"
        );
        assert_eq!(termination.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn app_started_session_records_event_without_termination() {
        let tmp = TempDir::new();
        let (_app, session, connection) = pair(&tmp);
        let (_monitor, rx, termination) = watch(connection, SessionOrigin::AppStarted);
        drop(session);
        let event = rx.recv_timeout(WAIT).unwrap();
        assert_eq!(event.origin, SessionOrigin::AppStarted);
        assert_eq!(
            termination.calls.load(Ordering::SeqCst),
            0,
            "AgentSession이 수명 소유"
        );
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[test]
    fn app_shutdown_and_peer_close_race_records_once() {
        for _ in 0..20 {
            let tmp = TempDir::new();
            let (_app, session, connection) = pair(&tmp);
            let (monitor, rx, termination) = watch(connection, SessionOrigin::External);
            let closer = std::thread::spawn(move || drop(session));
            let first = monitor.close();
            closer.join().unwrap();
            drop(monitor);
            let events: Vec<ConnectionClosed> = rx.try_iter().collect();
            assert_eq!(events.len(), 1, "{events:?}");
            assert_eq!(termination.calls.load(Ordering::SeqCst), 1);
            if first {
                assert_eq!(events[0].cause, CloseCause::AppShutdown);
            }
        }
    }

    #[test]
    fn unexpected_data_closes_connection_once() {
        let tmp = TempDir::new();
        let (_app, mut session, connection) = pair(&tmp);
        let (_monitor, rx, termination) = watch(connection, SessionOrigin::External);
        session.stream().write_all(b"x").unwrap();
        let event = rx.recv_timeout(WAIT).unwrap();
        assert_eq!(event.cause, CloseCause::UnexpectedData);
        assert_eq!(termination.calls.load(Ordering::SeqCst), 1);
        // 앱이 닫았으므로 bridge는 EOF를 봅니다. 종료 event는 socket shutdown 직전에 기록되므로, 그 사이에 읽으면
        // non-blocking socket은 EOF 대신 WouldBlock을 돌려줍니다. blocking으로 바꿔 제한 시간 안에서 EOF를 기다립니다.
        let mut rest = Vec::new();
        session.stream().set_nonblocking(false).unwrap();
        session.stream().set_read_timeout(Some(WAIT)).unwrap();
        assert_eq!(session.stream().read_to_end(&mut rest).unwrap(), 0);
    }

    #[test]
    fn connection_ids_are_unique() {
        let a = ConnectionId::next();
        let b = ConnectionId::next();
        assert_ne!(a, b);
        assert!(b.get() > a.get());
    }
}

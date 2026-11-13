//! 앱 측 IPC endpoint와 IPC I/O 스레드 (XIX.1.6).
//!
//! 실행 중 앱이 registry에 자기 endpoint를 열고, 전용 IPC I/O 스레드([`IO_THREAD_NAME`],
//! ADR-0005 §6 "Agent Host·IPC I/O 스레드")에서 연결을 받습니다. 연결마다 순서는 다음과 같습니다.
//!
//! 1. accept 직후, **어떤 byte도 읽거나 쓰기 전에** [`authenticate`]로 상대를 인증합니다. 실패하면
//!    아무것도 읽거나 쓰지 않고 연결을 닫고 [`ListenerEvent::Rejected`]를 보냅니다.
//! 2. 인증을 통과하면 handshake(XIX.1.3)의 앱 측을 수행합니다: HELLO를 읽고, 본 client nonce면
//!    `REJECT nonce-reused`, 아니면 새 session nonce로 WELCOME을 보냅니다. HELLO 없이 닫힌 연결
//!    (registry 탐색의 생존 확인)은 조용히 무시합니다.
//! 3. 성공한 연결은 [`ListenerEvent::Connected`]로 앱에 넘깁니다.
//!
//! 인증·handshake는 모두 I/O 스레드에서만 실행되며, 이 crate는 document·commands에 의존하지
//! 않으므로(crate graph 정책) Document 스레드를 호출할 수 없습니다. 앱 UI 스레드는 이벤트 채널만
//! 읽습니다. 각 이벤트에는 처리한 스레드 id가 들어 있어 이 계약을 검사할 수 있습니다.

use std::fmt;
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{JoinHandle, ThreadId};

use crate::auth::{authenticate, AuthError, AuthenticatedPeer, PeerPolicy};
use crate::code_identity::{CodeIdentityError, ERR_NO_SUCH_CODE};
use crate::handshake::{
    read_line, set_timeouts, write_message, HandshakeError, Message, DEFAULT_TIMEOUT,
};
use crate::nonce::{Nonce, NonceLedger};
use crate::registry::{Endpoint, InstanceId, Registry, RegistryError};

/// IPC I/O 스레드 이름.
pub const IO_THREAD_NAME: &str = "stellar-ipc-io";

/// endpoint를 열 수 없는 이유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListenError {
    /// registry 디렉터리를 만들 수 없거나 안전하지 않습니다.
    Registry(RegistryError),
    /// 같은 instance의 endpoint가 이미 실행 중입니다.
    InUse,
    /// 입출력 오류.
    Io {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
}

impl fmt::Display for ListenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListenError::Registry(e) => write!(f, "{e}"),
            ListenError::InUse => write!(f, "같은 instance의 IPC endpoint가 이미 실행 중입니다"),
            ListenError::Io { kind } => write!(f, "IPC endpoint 입출력 오류: {kind:?}"),
        }
    }
}

impl std::error::Error for ListenError {}

fn io_err(e: &io::Error) -> ListenError {
    ListenError::Io { kind: e.kind() }
}

/// 앱이 registry에 연 endpoint. drop하면 socket 파일을 지웁니다(registry 계약).
#[derive(Debug)]
pub struct AppEndpoint {
    endpoint: Endpoint,
    listener: UnixListener,
}

impl AppEndpoint {
    /// registry 디렉터리(없으면 0700으로 생성)에 `<instance>.sock`을 엽니다.
    ///
    /// 디렉터리가 안전하지 않으면(소유자·권한·symlink) 열지 않습니다. 같은 이름의 socket이 응답하면
    /// [`ListenError::InUse`], listener 없는 잔재면 지우고 다시 엽니다.
    pub fn bind(registry: &Registry, instance: InstanceId) -> Result<AppEndpoint, ListenError> {
        if !registry.verify().map_err(ListenError::Registry)? {
            // 다른 instance가 동시에 만들었으면 AlreadyExists입니다. 그 경우도 아래 verify가 소유자·
            // 권한·symlink를 다시 검사합니다.
            match std::fs::DirBuilder::new()
                .mode(0o700)
                .create(registry.dir())
            {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(io_err(&e)),
            }
        }
        if !registry.verify().map_err(ListenError::Registry)? {
            return Err(ListenError::Io {
                kind: io::ErrorKind::NotFound,
            });
        }
        let socket_path = registry
            .socket_path(&instance)
            .map_err(ListenError::Registry)?;
        remove_stale(&socket_path)?;
        let listener = UnixListener::bind(&socket_path).map_err(|e| io_err(&e))?;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| io_err(&e))?;
        Ok(AppEndpoint {
            endpoint: Endpoint {
                instance,
                socket_path,
            },
            listener,
        })
    }

    /// endpoint 정보.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

impl Drop for AppEndpoint {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.endpoint.socket_path);
    }
}

/// listener 없는 socket 잔재를 지웁니다. 살아 있는 socket이면 [`ListenError::InUse`]입니다.
fn remove_stale(path: &Path) -> Result<(), ListenError> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(io_err(&e)),
    };
    if !meta.file_type().is_socket() {
        return Err(ListenError::InUse);
    }
    match UnixStream::connect(path) {
        Ok(_) => Err(ListenError::InUse),
        Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => {
            std::fs::remove_file(path).map_err(|e| io_err(&e))
        }
        Err(e) => Err(io_err(&e)),
    }
}

/// 인증과 handshake를 마친 앱 측 연결.
#[derive(Debug)]
pub struct AppConnection {
    stream: UnixStream,
    peer: AuthenticatedPeer,
    client_nonce: Nonce,
    session_nonce: Nonce,
}

impl AppConnection {
    /// 인증된 상대.
    pub fn peer(&self) -> AuthenticatedPeer {
        self.peer
    }

    /// 브리지가 보낸 client nonce.
    pub fn client_nonce(&self) -> Nonce {
        self.client_nonce
    }

    /// 앱이 만든 session nonce.
    pub fn session_nonce(&self) -> Nonce {
        self.session_nonce
    }

    /// 연결 stream.
    pub fn stream(&mut self) -> &mut UnixStream {
        &mut self.stream
    }

    /// stream을 꺼냅니다.
    pub fn into_stream(self) -> UnixStream {
        self.stream
    }
}

/// IPC I/O 스레드가 앱에 보내는 이벤트. `thread`는 처리한 스레드입니다.
#[derive(Debug)]
pub enum ListenerEvent {
    /// 인증과 handshake를 마친 연결.
    Connected {
        /// 연결.
        connection: AppConnection,
        /// 처리 스레드.
        thread: ThreadId,
    },
    /// 인증 실패: 아무것도 읽거나 쓰지 않고 닫았습니다.
    Rejected {
        /// 사유.
        error: AuthError,
        /// 처리 스레드.
        thread: ThreadId,
    },
    /// 인증 후 handshake 실패.
    HandshakeFailed {
        /// 사유.
        error: HandshakeError,
        /// 처리 스레드.
        thread: ThreadId,
    },
}

impl ListenerEvent {
    /// 이 이벤트를 처리한 스레드.
    pub fn thread(&self) -> ThreadId {
        match self {
            ListenerEvent::Connected { thread, .. }
            | ListenerEvent::Rejected { thread, .. }
            | ListenerEvent::HandshakeFailed { thread, .. } => *thread,
        }
    }
}

/// 실행 중인 앱 측 listener. drop하면 I/O 스레드를 멈추고 socket 파일을 지웁니다.
#[derive(Debug)]
pub struct AppListener {
    events: Receiver<ListenerEvent>,
    stop: Arc<AtomicBool>,
    socket_path: PathBuf,
    thread: Option<JoinHandle<()>>,
    thread_id: ThreadId,
}

impl AppListener {
    /// IPC I/O 스레드를 시작합니다. 인증 기준은 `policy`입니다.
    pub fn spawn(endpoint: AppEndpoint, policy: PeerPolicy) -> io::Result<AppListener> {
        let (tx, events) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let socket_path = endpoint.endpoint.socket_path.clone();
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name(IO_THREAD_NAME.into())
            .spawn(move || accept_loop(endpoint, policy, &thread_stop, &tx))?;
        let thread_id = thread.thread().id();
        Ok(AppListener {
            events,
            stop,
            socket_path,
            thread: Some(thread),
            thread_id,
        })
    }

    /// 이벤트 채널.
    pub fn events(&self) -> &Receiver<ListenerEvent> {
        &self.events
    }

    /// IPC I/O 스레드 id.
    pub fn io_thread(&self) -> ThreadId {
        self.thread_id
    }

    /// endpoint socket 경로.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}

impl Drop for AppListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // accept 대기를 깨웁니다. 깨운 연결은 인증하지 않고 버립니다.
        let _ = UnixStream::connect(&self.socket_path);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn accept_loop(
    endpoint: AppEndpoint,
    policy: PeerPolicy,
    stop: &AtomicBool,
    tx: &Sender<ListenerEvent>,
) {
    let thread = std::thread::current().id();
    let mut ledger = NonceLedger::new();
    loop {
        let stream = match endpoint.listener.accept() {
            Ok((stream, _)) => stream,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let event = match authenticate(&stream, &policy) {
            // 이미 닫힌 상대(registry 탐색의 생존 확인)나 이미 종료된 상대 프로세스
            // (`errSecCSNoSuchCode`)와는 데이터를 주고받을 수 없으므로 이벤트 없이 닫습니다.
            Err(AuthError::Credentials(e)) if e.peer_closed() => None,
            Err(AuthError::Code(CodeIdentityError::Unverifiable {
                status: ERR_NO_SUCH_CODE,
            })) => None,
            // 인증 전에는 읽거나 쓰지 않았으므로 그대로 닫습니다.
            Err(error) => Some(ListenerEvent::Rejected { error, thread }),
            Ok(peer) => match serve_handshake(stream, peer, &mut ledger) {
                Ok(Some(connection)) => Some(ListenerEvent::Connected { connection, thread }),
                Ok(None) => None,
                Err(error) => Some(ListenerEvent::HandshakeFailed { error, thread }),
            },
        };
        if let Some(event) = event {
            if tx.send(event).is_err() {
                break;
            }
        }
    }
}

/// 인증된 연결에서 앱 측 handshake를 수행합니다. HELLO 없이 닫히면 `Ok(None)`입니다.
fn serve_handshake(
    mut stream: UnixStream,
    peer: AuthenticatedPeer,
    ledger: &mut NonceLedger,
) -> Result<Option<AppConnection>, HandshakeError> {
    // timeout 설정 중이나 HELLO 전에 상대가 닫으면 탐색 연결과 같이 조용히 끝냅니다.
    let line = match set_timeouts(&stream, DEFAULT_TIMEOUT).and_then(|()| read_line(&mut stream)) {
        Err(HandshakeError::Closed) => return Ok(None),
        other => other?,
    };
    let Some(Message::Hello { client }) = Message::decode(&line) else {
        return Err(HandshakeError::Malformed);
    };
    if ledger.record(client).is_err() {
        let reject = Message::Reject {
            reason: "nonce-reused".into(),
        };
        let _ = write_message(&mut stream, &reject);
        return Err(HandshakeError::NonceReused);
    }
    let session = Nonce::generate().map_err(|e| HandshakeError::Io { kind: e.kind() })?;
    ledger
        .record(session)
        .map_err(|_| HandshakeError::NonceReused)?;
    write_message(&mut stream, &Message::Welcome { client, session })?;
    Ok(Some(AppConnection {
        stream,
        peer,
        client_nonce: client,
        session_nonce: session,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_identity::CodeRule;
    use crate::handshake::{connect, Message};
    use crate::os;
    use crate::test_support::TempDir;
    use std::io::{Read, Write};
    use std::time::Duration;

    const WAIT: Duration = Duration::from_secs(10);

    fn id(n: u8) -> InstanceId {
        InstanceId::from_bytes([n; 8])
    }

    fn listener(tmp: &TempDir, policy: PeerPolicy) -> AppListener {
        let registry = Registry::at(tmp.path.join("r"), os::current_uid());
        let endpoint = AppEndpoint::bind(&registry, id(1)).unwrap();
        AppListener::spawn(endpoint, policy).unwrap()
    }

    #[test]
    fn bind_creates_private_registry_and_cleans_up() {
        use std::os::unix::fs::MetadataExt;
        let tmp = TempDir::new();
        let registry = Registry::at(tmp.path.join("r"), os::current_uid());
        let endpoint = AppEndpoint::bind(&registry, id(2)).unwrap();
        let meta = std::fs::metadata(registry.dir()).unwrap();
        assert_eq!(meta.mode() & 0o7777, 0o700);
        let path = endpoint.endpoint().socket_path.clone();
        assert!(std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_socket());
        assert_eq!(
            AppEndpoint::bind(&registry, id(2)).unwrap_err(),
            ListenError::InUse
        );
        drop(endpoint);
        assert!(
            std::fs::symlink_metadata(&path).is_err(),
            "drop 시 socket 삭제"
        );
        // 잔재(listener 없는 socket 파일)는 지우고 다시 엽니다.
        let stale = UnixListener::bind(&path).unwrap();
        drop(stale);
        assert!(AppEndpoint::bind(&registry, id(2)).is_ok());
    }

    #[test]
    fn insecure_registry_is_not_used() {
        let tmp = TempDir::new();
        let dir = tmp.path.join("open");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let registry = Registry::at(&dir, os::current_uid());
        assert!(matches!(
            AppEndpoint::bind(&registry, id(3)),
            Err(ListenError::Registry(RegistryError::Insecure { .. }))
        ));
    }

    #[test]
    fn authenticated_bridge_completes_handshake_on_io_thread() {
        let tmp = TempDir::new();
        let app = listener(&tmp, PeerPolicy::for_current_process().unwrap());
        let endpoint = Endpoint {
            instance: id(1),
            socket_path: app.socket_path().to_path_buf(),
        };
        let session = connect(&endpoint, &mut NonceLedger::new()).unwrap();
        let event = app.events().recv_timeout(WAIT).unwrap();
        assert_eq!(event.thread(), app.io_thread());
        assert_ne!(
            event.thread(),
            std::thread::current().id(),
            "호출 스레드 밖"
        );
        let ListenerEvent::Connected { connection, .. } = event else {
            panic!("{event:?}")
        };
        assert_eq!(connection.client_nonce(), session.client_nonce());
        assert_eq!(connection.session_nonce(), session.session_nonce());
        assert_eq!(connection.peer().euid, os::current_uid());
    }

    #[test]
    fn fixture_auth_failure_exchanges_no_data() {
        for (policy, expected) in [
            (
                PeerPolicy::new(
                    os::current_uid().wrapping_add(1),
                    CodeRule::DevelopmentSigned,
                )
                .unwrap(),
                "user",
            ),
            (
                PeerPolicy::new(os::current_uid(), CodeRule::Custom("anchor apple".into()))
                    .unwrap(),
                "code",
            ),
        ] {
            let tmp = TempDir::new();
            let app = listener(&tmp, policy);
            let mut client = UnixStream::connect(app.socket_path()).unwrap();
            client.set_read_timeout(Some(WAIT)).unwrap();
            // 상대가 먼저 보내도 앱은 인증 전에 읽지 않고, 아무것도 쓰지 않고 닫습니다.
            let _ = client.write_all(
                Message::Hello {
                    client: Nonce::from_bytes([7; 16]),
                }
                .encode()
                .as_bytes(),
            );
            let mut received = Vec::new();
            let _ = client.read_to_end(&mut received);
            assert!(received.is_empty(), "{expected}: 앱이 보낸 byte 0");
            let event = app.events().recv_timeout(WAIT).unwrap();
            assert_eq!(event.thread(), app.io_thread(), "{expected}");
            assert_ne!(event.thread(), std::thread::current().id());
            match (expected, event) {
                (
                    "user",
                    ListenerEvent::Rejected {
                        error: AuthError::UserMismatch { .. },
                        ..
                    },
                ) => {}
                (
                    "code",
                    ListenerEvent::Rejected {
                        error: AuthError::Code(CodeIdentityError::RequirementFailed),
                        ..
                    },
                ) => {}
                (_, other) => panic!("{expected}: {other:?}"),
            }
            assert!(
                app.events()
                    .recv_timeout(Duration::from_millis(100))
                    .is_err(),
                "{expected}: 연결 전달 0"
            );
        }
    }

    #[test]
    fn reused_client_nonce_is_rejected_after_authentication() {
        let tmp = TempDir::new();
        let app = listener(&tmp, PeerPolicy::for_current_process().unwrap());
        let hello = Message::Hello {
            client: Nonce::from_bytes([9; 16]),
        };
        let mut replies = Vec::new();
        for _ in 0..2 {
            let mut stream = UnixStream::connect(app.socket_path()).unwrap();
            set_timeouts(&stream, WAIT).unwrap();
            write_message(&mut stream, &hello).unwrap();
            replies.push(Message::decode(&read_line(&mut stream).unwrap()).unwrap());
        }
        assert!(matches!(replies[0], Message::Welcome { .. }));
        assert_eq!(
            replies[1],
            Message::Reject {
                reason: "nonce-reused".into()
            }
        );
        assert!(matches!(
            app.events().recv_timeout(WAIT).unwrap(),
            ListenerEvent::Connected { .. }
        ));
        assert!(matches!(
            app.events().recv_timeout(WAIT).unwrap(),
            ListenerEvent::HandshakeFailed {
                error: HandshakeError::NonceReused,
                ..
            }
        ));
    }

    #[test]
    fn probe_connections_are_ignored() {
        let tmp = TempDir::new();
        let app = listener(&tmp, PeerPolicy::for_current_process().unwrap());
        drop(UnixStream::connect(app.socket_path()).unwrap());
        if let Ok(event) = app.events().recv_timeout(Duration::from_millis(300)) {
            panic!("탐색 연결은 이벤트 없음: {event:?}");
        }
    }
}

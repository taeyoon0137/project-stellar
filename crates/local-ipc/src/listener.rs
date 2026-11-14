//! 앱 측 IPC endpoint와 IPC I/O 스레드 (XIX.1.6).
//!
//! 실행 중 앱이 registry에 자기 endpoint를 열고, 전용 IPC I/O 스레드([`IO_THREAD_NAME`],
//! ADR-0005 §6 "Agent Host·IPC I/O 스레드")에서 연결을 받습니다. 연결마다 순서는 다음과 같습니다.
//!
//! 1. accept 직후, **어떤 byte도 읽거나 쓰기 전에** [`authenticate`]로 상대를 인증합니다. 실패하면
//!    아무것도 읽거나 쓰지 않고 연결을 닫고 [`ListenerEvent::Rejected`]를 보냅니다.
//! 2. 인증을 통과하면 handshake(XIX.1.3)의 앱 측을 수행합니다: HELLO를 읽고, 본 client nonce면
//!    `REJECT nonce-reused`, 아니면 새 session nonce로 WELCOME을 보냅니다. HELLO 없이 닫힌 연결
//!    (registry 탐색의 생존 확인)은 조용히 무시합니다. HELLO 한 줄 전체에 기한([`DEFAULT_TIMEOUT`])이
//!    있어, 지나면 [`HandshakeError::TimedOut`]으로 닫습니다.
//! 3. 성공한 연결은 [`ListenerEvent::Connected`]로 앱에 넘깁니다.
//!
//! I/O 스레드 하나가 `poll(2)`로 정지 신호(waker), listener, 진행 중 handshake(최대
//! [`MAX_PENDING_HANDSHAKES`]개)를 함께 기다립니다. 연결 socket은 handshake 동안 nonblocking이라
//! 느리거나 멈춘 상대 하나가 accept와 다른 bridge의 handshake를 막지 못합니다. 정지는 socket 경로로
//! 깨우기 연결을 보내지 않고 waker로 알리므로, socket 파일이 지워지거나 바뀌어도 drop이 끝납니다.
//!
//! 인증·handshake는 모두 I/O 스레드에서만 실행되며, 이 crate는 document·commands에 의존하지
//! 않으므로(crate graph 정책) Document 스레드를 호출할 수 없습니다. 앱 UI 스레드는 이벤트 채널만
//! 읽습니다. 각 이벤트에는 처리한 스레드 id가 들어 있어 이 계약을 검사할 수 있습니다.

use std::fmt;
use std::io::{self, Read};
use std::net::Shutdown;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{JoinHandle, ThreadId};
use std::time::{Duration, Instant};

use crate::auth::{authenticate, AuthError, AuthenticatedPeer, PeerPolicy};
use crate::binding::{BindingError, BindingSession, SessionBindings};
use crate::code_identity::{CodeIdentityError, ERR_NO_SUCH_CODE};
use crate::handshake::{
    classify_io, set_timeouts, write_message, HandshakeError, Message, DEFAULT_TIMEOUT,
    MAX_LINE_BYTES,
};
use crate::nonce::{Nonce, NonceLedger};
use crate::os;
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
    /// bind 직후 socket 파일의 (device, inode). drop은 경로가 아직 이 파일일 때만 지웁니다.
    file_id: (u64, u64),
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
        let meta = std::fs::symlink_metadata(&socket_path).map_err(|e| io_err(&e))?;
        Ok(AppEndpoint {
            endpoint: Endpoint {
                instance,
                socket_path,
            },
            listener,
            file_id: (meta.dev(), meta.ino()),
        })
    }

    /// endpoint 정보.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

impl Drop for AppEndpoint {
    fn drop(&mut self) {
        // 이미 지워졌거나 다른 instance의 socket으로 바뀐 경로는 지우지 않습니다.
        let path = &self.endpoint.socket_path;
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            if (meta.dev(), meta.ino()) == self.file_id {
                let _ = std::fs::remove_file(path);
            }
        }
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
    bound: Option<BindingSession>,
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

    /// 세션 결합(XIX.1.8)으로 묶인 세션. 외부 client 연결이면 `None`.
    ///
    /// 결합은 세션 식별일 뿐 권한이 아닙니다. 결합된 연결도 권한 0에서 시작하며 XIX.3 승인을
    /// 대체하지 않습니다.
    pub fn bound_session(&self) -> Option<BindingSession> {
        self.bound
    }

    /// 세션을 시작한 쪽: 결합됐으면 앱이 시작한 세션, 아니면 외부 client.
    pub fn origin(&self) -> crate::session::SessionOrigin {
        match self.bound {
            Some(_) => crate::session::SessionOrigin::AppStarted,
            None => crate::session::SessionOrigin::External,
        }
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

/// 동시에 진행하는 인증 후 handshake 최대 수.
///
/// 이만큼 진행 중이면 그중 하나가 끝나거나 handshake timeout이 지날 때까지 새 연결을 accept하지
/// 않습니다(연결은 kernel backlog에서 기다립니다). 느린 상대 하나가 다른 bridge를 막지 못하면서 fd
/// 사용량은 이 값으로 제한됩니다.
pub const MAX_PENDING_HANDSHAKES: usize = 16;

/// 실행 중인 앱 측 listener. drop하면 I/O 스레드를 멈추고 socket 파일을 지웁니다.
///
/// 정지 신호는 listener 소유 socketpair(waker)로 보냅니다. socket 경로로 깨우기 연결을 보내지 않으므로
/// socket 파일이 지워지거나 다른 listener로 바뀌어도 drop이 끝납니다(IPC-02).
#[derive(Debug)]
pub struct AppListener {
    events: Receiver<ListenerEvent>,
    waker: UnixStream,
    socket_path: PathBuf,
    thread: Option<JoinHandle<()>>,
    thread_id: ThreadId,
}

impl AppListener {
    /// IPC I/O 스레드를 시작합니다. 인증 기준은 `policy`입니다. 세션 결합(XIX.1.8)은 받지 않습니다
    /// (결합 token을 보낸 연결은 `REJECT binding-rejected`).
    pub fn spawn(endpoint: AppEndpoint, policy: PeerPolicy) -> io::Result<AppListener> {
        AppListener::spawn_inner(endpoint, policy, None, DEFAULT_TIMEOUT)
    }

    /// [`AppListener::spawn`]과 같되, 앱이 시작한 세션의 결합 token을 `bindings`로 검증합니다.
    pub fn spawn_with_bindings(
        endpoint: AppEndpoint,
        policy: PeerPolicy,
        bindings: Arc<SessionBindings>,
    ) -> io::Result<AppListener> {
        AppListener::spawn_inner(endpoint, policy, Some(bindings), DEFAULT_TIMEOUT)
    }

    fn spawn_inner(
        endpoint: AppEndpoint,
        policy: PeerPolicy,
        bindings: Option<Arc<SessionBindings>>,
        handshake_timeout: Duration,
    ) -> io::Result<AppListener> {
        endpoint.listener.set_nonblocking(true)?;
        let (waker, wake_rx) = UnixStream::pair()?;
        let (tx, events) = mpsc::channel();
        let socket_path = endpoint.endpoint.socket_path.clone();
        let thread = std::thread::Builder::new()
            .name(IO_THREAD_NAME.into())
            .spawn(move || {
                IoLoop {
                    endpoint,
                    policy,
                    wake_rx,
                    handshake_timeout,
                    bindings,
                    tx,
                    thread: std::thread::current().id(),
                    ledger: NonceLedger::new(),
                    pending: Vec::new(),
                }
                .run();
            })?;
        let thread_id = thread.thread().id();
        Ok(AppListener {
            events,
            waker,
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
        // waker를 닫으면 I/O 스레드의 poll이 EOF로 깨어납니다. I/O 스레드는 nonblocking으로만
        // 읽고 쓰므로 상대가 멈춰 있어도 join이 기다리지 않습니다.
        let _ = self.waker.shutdown(Shutdown::Both);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// 인증을 통과하고 HELLO를 기다리는 연결.
struct Pending {
    stream: UnixStream,
    peer: AuthenticatedPeer,
    line: Vec<u8>,
    /// HELLO 한 줄 전체의 기한(accept 시각 + handshake timeout).
    deadline: Instant,
}

/// 진행 중 handshake의 읽기 결과.
enum Progress {
    /// 아직 개행이 오지 않았습니다.
    Waiting,
    /// 개행까지 읽은 한 줄(개행 제외).
    Line(String),
    /// HELLO 전에 상대가 닫았습니다(탐색 연결과 같이 조용히 끝냄).
    Closed,
    /// handshake 실패.
    Failed(HandshakeError),
}

impl Pending {
    /// 지금 읽을 수 있는 byte를 개행까지 읽습니다. 개행 뒤 데이터는 읽지 않도록 byte 단위로 읽고,
    /// 기다려야 하면(nonblocking `WouldBlock`) 바로 돌아옵니다.
    fn read_available(&mut self) -> Progress {
        let mut byte = [0u8; 1];
        loop {
            match self.stream.read(&mut byte) {
                Ok(0) if self.line.is_empty() => return Progress::Closed,
                Ok(0) => return Progress::Failed(HandshakeError::Malformed),
                Ok(_) if byte[0] == b'\n' => {
                    let line = std::mem::take(&mut self.line);
                    return match String::from_utf8(line) {
                        Ok(line) => Progress::Line(line),
                        Err(_) => Progress::Failed(HandshakeError::Malformed),
                    };
                }
                Ok(_) => {
                    self.line.push(byte[0]);
                    if self.line.len() >= MAX_LINE_BYTES {
                        return Progress::Failed(HandshakeError::Malformed);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Progress::Waiting,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    return match classify_io(&e) {
                        HandshakeError::Closed => Progress::Closed,
                        error => Progress::Failed(error),
                    }
                }
            }
        }
    }
}

/// IPC I/O 스레드 상태.
///
/// 한 스레드가 `poll(2)`로 waker, listener, 진행 중 handshake를 함께 기다립니다. 연결 socket은
/// nonblocking이라 느린 상대 하나가 accept나 다른 handshake를 막지 못하고(IPC-03), 각 handshake는
/// HELLO 한 줄 전체에 기한이 있습니다. 인증·handshake는 모두 이 스레드에서만 실행됩니다.
struct IoLoop {
    endpoint: AppEndpoint,
    policy: PeerPolicy,
    wake_rx: UnixStream,
    bindings: Option<Arc<SessionBindings>>,
    handshake_timeout: Duration,
    tx: Sender<ListenerEvent>,
    /// 이 I/O 스레드(이벤트에 넣는 처리 스레드).
    thread: ThreadId,
    ledger: NonceLedger,
    pending: Vec<Pending>,
}

/// I/O 스레드를 끝낼 이유(앱이 listener를 drop했거나 이벤트 채널이 닫힘).
struct Stop;

impl IoLoop {
    fn run(mut self) {
        while self.step().is_ok() {}
        // 진행 중 handshake는 이벤트 없이 닫습니다(앱이 listener를 버림).
    }

    fn send(&self, event: ListenerEvent) -> Result<(), Stop> {
        self.tx.send(event).map_err(|_| Stop)
    }

    /// poll 한 번과 그 결과 처리.
    fn step(&mut self) -> Result<(), Stop> {
        let now = Instant::now();
        self.expire(now)?;
        let accepting = self.pending.len() < MAX_PENDING_HANDSHAKES;
        let mut fds = vec![self.wake_rx.as_raw_fd()];
        if accepting {
            fds.push(self.endpoint.listener.as_raw_fd());
        }
        let first_pending = fds.len();
        fds.extend(self.pending.iter().map(|p| p.stream.as_raw_fd()));
        let timeout = self
            .pending
            .iter()
            .map(|p| p.deadline.saturating_duration_since(now))
            .min();
        let ready = os::poll_readable(&fds, timeout).map_err(|_| Stop)?;
        if ready[0] {
            return Err(Stop);
        }
        // 거꾸로 돌며 끝난 handshake를 swap_remove합니다. 자리를 옮겨 온 항목은 이미 처리했습니다.
        for index in (0..self.pending.len()).rev() {
            if !ready[first_pending + index] {
                continue;
            }
            match self.pending[index].read_available() {
                Progress::Waiting => {}
                progress => {
                    let pending = self.pending.swap_remove(index);
                    self.finish(pending, progress)?;
                }
            }
        }
        if accepting && ready[1] {
            self.accept_ready()?;
        }
        Ok(())
    }

    /// 기한이 지난 handshake를 [`HandshakeError::TimedOut`]으로 끝냅니다.
    fn expire(&mut self, now: Instant) -> Result<(), Stop> {
        let mut index = 0;
        while index < self.pending.len() {
            if self.pending[index].deadline <= now {
                drop(self.pending.swap_remove(index));
                self.send(ListenerEvent::HandshakeFailed {
                    error: HandshakeError::TimedOut,
                    thread: self.thread,
                })?;
            } else {
                index += 1;
            }
        }
        Ok(())
    }

    /// 대기 중인 연결을 accept해 **어떤 byte도 읽거나 쓰기 전에** 인증합니다.
    fn accept_ready(&mut self) -> Result<(), Stop> {
        while self.pending.len() < MAX_PENDING_HANDSHAKES {
            let stream = match self.endpoint.listener.accept() {
                Ok((stream, _)) => stream,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::Interrupted | io::ErrorKind::ConnectionAborted
                    ) =>
                {
                    continue
                }
                Err(_) => return Err(Stop),
            };
            let thread = self.thread;
            match authenticate(&stream, &self.policy) {
                // 이미 닫힌 상대(registry 탐색의 생존 확인)나 이미 종료된 상대 프로세스
                // (`errSecCSNoSuchCode`)와는 데이터를 주고받을 수 없으므로 이벤트 없이 닫습니다.
                Err(AuthError::Credentials(e)) if e.peer_closed() => {}
                Err(AuthError::Code(CodeIdentityError::Unverifiable {
                    status: ERR_NO_SUCH_CODE,
                })) => {}
                // 인증 전에는 읽거나 쓰지 않았으므로 그대로 닫습니다.
                Err(error) => self.send(ListenerEvent::Rejected { error, thread })?,
                Ok(peer) => self.begin_handshake(stream, peer)?,
            }
        }
        Ok(())
    }

    /// 인증된 연결을 nonblocking으로 바꿔 HELLO를 기다리는 목록에 넣습니다.
    fn begin_handshake(&mut self, stream: UnixStream, peer: AuthenticatedPeer) -> Result<(), Stop> {
        // 넘겨줄 연결의 읽기·쓰기 timeout을 미리 정합니다. 그 사이 상대가 닫으면 탐색 연결과 같이
        // 조용히 끝냅니다.
        let prepared = set_timeouts(&stream, self.handshake_timeout).and_then(|()| {
            stream
                .set_nonblocking(true)
                .map_err(|e| HandshakeError::Io { kind: e.kind() })
        });
        match prepared {
            Ok(()) => {
                self.pending.push(Pending {
                    stream,
                    peer,
                    line: Vec::with_capacity(MAX_LINE_BYTES),
                    deadline: Instant::now() + self.handshake_timeout,
                });
                Ok(())
            }
            Err(HandshakeError::Closed) => Ok(()),
            Err(error) => self.send(ListenerEvent::HandshakeFailed {
                error,
                thread: self.thread,
            }),
        }
    }

    /// 끝난 handshake의 결과를 이벤트로 보냅니다.
    fn finish(&mut self, pending: Pending, progress: Progress) -> Result<(), Stop> {
        let thread = self.thread;
        let event = match progress {
            Progress::Waiting | Progress::Closed => return Ok(()),
            Progress::Failed(error) => ListenerEvent::HandshakeFailed { error, thread },
            Progress::Line(line) => match self.answer_hello(pending, &line) {
                Ok(connection) => ListenerEvent::Connected { connection, thread },
                Err(error) => ListenerEvent::HandshakeFailed { error, thread },
            },
        };
        self.send(event)
    }

    /// HELLO에 답합니다: 본 client nonce면 `REJECT nonce-reused`, 아니면 새 session nonce로 WELCOME.
    ///
    /// 쓰기도 nonblocking입니다. 상대가 아직 아무것도 받지 않은 새 연결이라 한 줄은 socket buffer에
    /// 바로 들어가며, 들어가지 않으면 기다리지 않고 실패로 끝냅니다.
    fn answer_hello(
        &mut self,
        mut pending: Pending,
        line: &str,
    ) -> Result<AppConnection, HandshakeError> {
        let (client, binding) = match Message::decode(line) {
            Some(Message::Hello { client }) => (client, None),
            Some(Message::BoundHello { client, binding }) => (client, Some(binding)),
            _ => return Err(HandshakeError::Malformed),
        };
        if self.ledger.record(client).is_err() {
            let reject = Message::Reject {
                reason: "nonce-reused".into(),
            };
            let _ = write_message(&mut pending.stream, &reject);
            return Err(HandshakeError::NonceReused);
        }
        // 세션 결합(XIX.1.8): 인증·nonce 검사를 통과한 연결에서만 token을 봅니다.
        let bound = match binding {
            None => None,
            Some(token) => {
                let verified = self
                    .bindings
                    .as_deref()
                    .ok_or(BindingError::Unsupported)
                    .and_then(|b| b.verify(&token));
                match verified {
                    Ok(session) => Some(session),
                    Err(error) => {
                        let reject = Message::Reject {
                            reason: "binding-rejected".into(),
                        };
                        let _ = write_message(&mut pending.stream, &reject);
                        return Err(HandshakeError::BindingRejected(error));
                    }
                }
            }
        };
        let session = Nonce::generate().map_err(|e| HandshakeError::Io { kind: e.kind() })?;
        self.ledger
            .record(session)
            .map_err(|_| HandshakeError::NonceReused)?;
        write_message(&mut pending.stream, &Message::Welcome { client, session })?;
        // 앱에 넘기는 연결은 blocking입니다(handshake timeout은 설정된 채로 넘깁니다).
        pending
            .stream
            .set_nonblocking(false)
            .map_err(|e| HandshakeError::Io { kind: e.kind() })?;
        Ok(AppConnection {
            stream: pending.stream,
            peer: pending.peer,
            client_nonce: client,
            session_nonce: session,
            bound,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_identity::CodeRule;
    use crate::handshake::{connect, read_line, Message, PROTOCOL_TAG};
    use crate::test_support::TempDir;
    use std::io::{Read, Write};

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
        let session = connect(
            &endpoint,
            &PeerPolicy::for_current_process().unwrap(),
            &mut NonceLedger::new(),
        )
        .unwrap();
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

    fn bound_listener(tmp: &TempDir, bindings: Arc<SessionBindings>) -> AppListener {
        let registry = Registry::at(tmp.path.join("r"), os::current_uid());
        let endpoint = AppEndpoint::bind(&registry, id(1)).unwrap();
        AppListener::spawn_with_bindings(
            endpoint,
            PeerPolicy::for_current_process().unwrap(),
            bindings,
        )
        .unwrap()
    }

    fn endpoint_of(app: &AppListener) -> Endpoint {
        Endpoint {
            instance: id(1),
            socket_path: app.socket_path().to_path_buf(),
        }
    }

    #[test]
    fn bound_bridge_is_tied_to_the_issuing_session_only() {
        // 검증 기준(XIX.1.8): 결합은 token을 만든 세션 하나에만 묶이고, 결합만으로 얻는 권한은 0입니다.
        use crate::binding::{BindingSession, BindingToken};
        use crate::handshake::connect_bound;
        use crate::session::SessionOrigin;
        let tmp = TempDir::new();
        let bindings = Arc::new(SessionBindings::new());
        let a = bindings.issue(BindingSession(10)).unwrap();
        let _b = bindings.issue(BindingSession(20)).unwrap();
        let app = bound_listener(&tmp, Arc::clone(&bindings));
        let policy = PeerPolicy::for_current_process().unwrap();

        connect_bound(
            &endpoint_of(&app),
            &policy,
            &mut NonceLedger::new(),
            Some(&a),
        )
        .unwrap();
        let ListenerEvent::Connected { connection, thread } =
            app.events().recv_timeout(WAIT).unwrap()
        else {
            panic!("결합 연결 실패")
        };
        assert_eq!(thread, app.io_thread());
        assert_eq!(connection.bound_session(), Some(BindingSession(10)));
        assert_ne!(connection.bound_session(), Some(BindingSession(20)));
        assert_eq!(connection.origin(), SessionOrigin::AppStarted);

        // 결합 없는 연결은 외부 client 세션입니다.
        connect_bound(&endpoint_of(&app), &policy, &mut NonceLedger::new(), None).unwrap();
        let ListenerEvent::Connected { connection, .. } = app.events().recv_timeout(WAIT).unwrap()
        else {
            panic!("외부 연결 실패")
        };
        assert_eq!(connection.bound_session(), None);
        assert_eq!(connection.origin(), SessionOrigin::External);

        // 끝난 세션과 모르는 token은 거부합니다.
        assert!(bindings.end(BindingSession(10)));
        let forged = BindingToken::from_bytes([3; 32]);
        for (token, expected) in [(a, BindingError::Ended), (forged, BindingError::Unknown)] {
            let error = connect_bound(
                &endpoint_of(&app),
                &policy,
                &mut NonceLedger::new(),
                Some(&token),
            )
            .unwrap_err();
            assert_eq!(
                error,
                HandshakeError::Rejected {
                    reason: "binding-rejected".into()
                }
            );
            match app.events().recv_timeout(WAIT).unwrap() {
                ListenerEvent::HandshakeFailed {
                    error: HandshakeError::BindingRejected(found),
                    thread,
                } => {
                    assert_eq!(found, expected);
                    assert_eq!(thread, app.io_thread());
                }
                other => panic!("{other:?}"),
            }
        }
        assert!(
            app.events()
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "거부된 결합은 연결 전달 0"
        );
    }

    #[test]
    fn listener_without_bindings_rejects_any_binding() {
        use crate::binding::BindingToken;
        use crate::handshake::connect_bound;
        let tmp = TempDir::new();
        let app = listener(&tmp, PeerPolicy::for_current_process().unwrap());
        let token = BindingToken::from_bytes([5; 32]);
        let error = connect_bound(
            &endpoint_of(&app),
            &PeerPolicy::for_current_process().unwrap(),
            &mut NonceLedger::new(),
            Some(&token),
        )
        .unwrap_err();
        assert_eq!(
            error,
            HandshakeError::Rejected {
                reason: "binding-rejected".into()
            }
        );
        assert!(matches!(
            app.events().recv_timeout(WAIT).unwrap(),
            ListenerEvent::HandshakeFailed {
                error: HandshakeError::BindingRejected(BindingError::Unsupported),
                ..
            }
        ));
    }

    /// drop은 깨우기 연결에 의존하지 않습니다: socket 파일이 지워지거나 다른 listener로 바뀌어도
    /// I/O 스레드가 끝나고 drop이 돌아옵니다(IPC-02).
    #[test]
    fn drop_finishes_when_socket_file_was_removed_or_replaced() {
        for replace in [false, true] {
            let tmp = TempDir::new();
            let app = listener(&tmp, PeerPolicy::for_current_process().unwrap());
            let path = app.socket_path().to_path_buf();
            std::fs::remove_file(&path).unwrap();
            let other = replace.then(|| {
                let other = UnixListener::bind(&path).unwrap();
                other.set_nonblocking(true).unwrap();
                other
            });
            let (done_tx, done_rx) = mpsc::channel();
            std::thread::spawn(move || {
                drop(app);
                let _ = done_tx.send(());
            });
            assert!(
                done_rx.recv_timeout(WAIT).is_ok(),
                "replace={replace}: drop이 {WAIT:?} 안에 끝나야 함"
            );
            if let Some(other) = other {
                assert_eq!(
                    other.accept().map(|_| ()).unwrap_err().kind(),
                    io::ErrorKind::WouldBlock,
                    "다른 listener에 깨우기 연결을 보내지 않음"
                );
                assert!(
                    std::fs::symlink_metadata(&path).is_ok(),
                    "다른 listener의 socket 파일은 지우지 않음"
                );
            }
        }
    }

    /// 인증을 통과한 상대가 HELLO를 1 byte씩 늦게 보내도 다른 bridge의 handshake가 막히지
    /// 않습니다(IPC-03).
    #[test]
    fn slow_authenticated_peer_does_not_block_other_handshakes() {
        let tmp = TempDir::new();
        let policy = PeerPolicy::for_current_process().unwrap();
        let app = listener(&tmp, policy.clone());
        let endpoint = Endpoint {
            instance: id(1),
            socket_path: app.socket_path().to_path_buf(),
        };
        let mut slow = UnixStream::connect(app.socket_path()).unwrap();
        // 앱 측 timeout(5초)보다 짧은 간격으로 한 byte씩 보냅니다.
        std::thread::spawn(move || {
            for byte in PROTOCOL_TAG.bytes() {
                if slow.write_all(&[byte]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(700));
            }
        });
        std::thread::sleep(Duration::from_millis(200));
        let started = std::time::Instant::now();
        let session = connect(&endpoint, &policy, &mut NonceLedger::new());
        let elapsed = started.elapsed();
        let session =
            session.unwrap_or_else(|e| panic!("정상 bridge handshake: {e:?} ({elapsed:?})"));
        assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
        let ListenerEvent::Connected { connection, .. } = app.events().recv_timeout(WAIT).unwrap()
        else {
            panic!("정상 연결이 먼저 전달됨")
        };
        assert_eq!(connection.client_nonce(), session.client_nonce());
    }

    /// HELLO 한 줄 전체에 기한이 있습니다: timeout보다 짧은 간격으로 1 byte씩 보내도 기한이 지나면
    /// [`HandshakeError::TimedOut`]으로 닫습니다(IPC-03).
    #[test]
    fn trickling_hello_times_out_as_a_whole() {
        let tmp = TempDir::new();
        let registry = Registry::at(tmp.path.join("r"), os::current_uid());
        let endpoint = AppEndpoint::bind(&registry, id(1)).unwrap();
        let timeout = Duration::from_millis(400);
        let app = AppListener::spawn_inner(
            endpoint,
            PeerPolicy::for_current_process().unwrap(),
            None,
            timeout,
        )
        .unwrap();
        let mut slow = UnixStream::connect(app.socket_path()).unwrap();
        let mut reader = slow.try_clone().unwrap();
        let started = std::time::Instant::now();
        let trickle = std::thread::spawn(move || {
            for byte in PROTOCOL_TAG.bytes() {
                if slow.write_all(&[byte]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let event = app.events().recv_timeout(WAIT).unwrap();
        let elapsed = started.elapsed();
        assert!(
            matches!(
                event,
                ListenerEvent::HandshakeFailed {
                    error: HandshakeError::TimedOut,
                    ..
                }
            ),
            "{event:?}"
        );
        assert_eq!(event.thread(), app.io_thread());
        assert!(
            elapsed >= timeout && elapsed < timeout * 3,
            "기한 {timeout:?}, 실제 {elapsed:?}"
        );
        // 앱은 아무것도 쓰지 않고 닫았습니다. 이미 양방향이 닫혔으면 macOS는 timeout 설정에 EINVAL을
        // 돌려주지만, 그 상태의 read는 기다리지 않습니다.
        let _ = reader.set_read_timeout(Some(WAIT));
        let mut received = Vec::new();
        let _ = reader.read_to_end(&mut received);
        assert!(received.is_empty());
        trickle.join().unwrap();
    }

    /// 진행 중 handshake가 [`MAX_PENDING_HANDSHAKES`]개면 새 연결은 그중 하나가 끝날 때까지 kernel
    /// backlog에서 기다리고, 자리가 나면 처리됩니다.
    #[test]
    fn pending_handshakes_are_capped_and_resume_when_one_finishes() {
        let tmp = TempDir::new();
        let policy = PeerPolicy::for_current_process().unwrap();
        let registry = Registry::at(tmp.path.join("r"), os::current_uid());
        let endpoint = AppEndpoint::bind(&registry, id(1)).unwrap();
        endpoint.listener.set_nonblocking(true).unwrap();
        let mut bridge = UnixStream::connect(&endpoint.endpoint().socket_path).unwrap();
        authenticate(&bridge, &policy).unwrap();
        set_timeouts(&bridge, DEFAULT_TIMEOUT).unwrap();
        let client = Nonce::from_bytes([23; 16]);
        write_message(&mut bridge, &Message::Hello { client }).unwrap();
        // 한 슬롯이 났을 때 accept_ready가 bridge 하나만 받고 다시 멈추는지도 확인합니다.
        let _queued = UnixStream::connect(&endpoint.endpoint().socket_path).unwrap();

        // 같은 프로세스의 실제 인증 결과로 pending을 준비합니다. 준비 중 16번의 코드 서명 검사와
        // bridge 응답 기한을 경쟁시키지 않고, backlog의 bridge는 accept 경로에서 실제 인증합니다.
        let peer = {
            let (stream, _other) = UnixStream::pair().unwrap();
            authenticate(&stream, &policy).unwrap()
        };
        let (waker, wake_rx) = UnixStream::pair().unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        // 준비 확인 뒤 오래 멈춰 pending이 전부 만료돼도 poll이 무기한 기다리지 않게 합니다.
        // 정상 진행은 watchdog을 기다리지 않고, 실패로 완료 채널이 닫혀도 즉시 깨웁니다.
        let watchdog = std::thread::spawn(move || {
            if done_rx.recv_timeout(WAIT).is_err() {
                let _ = waker.shutdown(Shutdown::Both);
            }
        });
        let (tx, events) = mpsc::channel();
        let thread = std::thread::current().id();
        let mut io_loop = IoLoop {
            endpoint,
            policy,
            wake_rx,
            bindings: None,
            handshake_timeout: DEFAULT_TIMEOUT,
            tx,
            thread,
            ledger: NonceLedger::new(),
            pending: Vec::new(),
        };
        let mut silent = Vec::new();
        for _ in 0..MAX_PENDING_HANDSHAKES {
            let (stream, other) = UnixStream::pair().unwrap();
            assert!(io_loop.begin_handshake(stream, peer).is_ok());
            silent.push(other);
        }
        let readable = |fd| os::poll_readable(&[fd], Some(Duration::ZERO)).unwrap()[0];
        let listener_fd = io_loop.endpoint.listener.as_raw_fd();

        // 각 step 전에 준비된 fd를 확인해 poll이 외부 스레드나 고정 sleep에 의존하지 않게 합니다.
        // partial HELLO는 pending을 유지하면서 가득 찬 loop를 한 번 깨웁니다.
        silent[0].write_all(b"S").unwrap();
        assert!(readable(io_loop.pending[0].stream.as_raw_fd()));
        assert!(readable(listener_fd));
        assert!(io_loop.step().is_ok());
        assert_eq!(io_loop.pending.len(), MAX_PENDING_HANDSHAKES);
        assert_eq!(io_loop.pending[0].line, b"S");
        assert!(
            readable(listener_fd),
            "자리가 없으면 새 연결은 backlog에 남음"
        );
        assert!(!readable(bridge.as_raw_fd()));
        assert!(matches!(events.try_recv(), Err(mpsc::TryRecvError::Empty)));

        // HELLO를 보내지 않은 pending 하나의 EOF만 처리하면 슬롯 하나가 납니다.
        let closing_fd = io_loop.pending.last().unwrap().stream.as_raw_fd();
        drop(silent.pop());
        assert!(readable(closing_fd));
        assert!(io_loop.step().is_ok());
        assert_eq!(io_loop.pending.len(), MAX_PENDING_HANDSHAKES - 1);
        assert!(matches!(events.try_recv(), Err(mpsc::TryRecvError::Empty)));

        // 다음 step은 backlog 연결을 실제 인증하고, 그다음 step은 이미 도착한 HELLO에 답합니다.
        assert!(readable(listener_fd));
        assert!(io_loop.step().is_ok());
        assert_eq!(io_loop.pending.len(), MAX_PENDING_HANDSHAKES);
        assert!(readable(listener_fd), "두 번째 연결은 backlog에 남음");
        assert!(readable(io_loop.pending.last().unwrap().stream.as_raw_fd()));
        assert!(io_loop.step().is_ok());
        assert_eq!(io_loop.pending.len(), MAX_PENDING_HANDSHAKES - 1);
        assert!(
            readable(listener_fd),
            "가득 찬 상태로 시작한 step은 accept하지 않음"
        );
        assert!(readable(bridge.as_raw_fd()));
        let Message::Welcome {
            client: echoed,
            session,
        } = Message::decode(&read_line(&mut bridge).unwrap()).unwrap()
        else {
            panic!("자리가 나면 WELCOME 응답")
        };
        let ListenerEvent::Connected {
            connection,
            thread: processed,
        } = events.try_recv().unwrap()
        else {
            panic!("정상 연결 전달")
        };
        assert_eq!(echoed, client);
        assert_ne!(session, client);
        assert_eq!(connection.client_nonce(), client);
        assert_eq!(connection.session_nonce(), session);
        assert_eq!(connection.peer(), peer);
        assert_eq!(processed, thread);
        done_tx.send(()).unwrap();
        watchdog.join().unwrap();
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

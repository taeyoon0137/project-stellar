//! 앱 측 IPC test double (XIX.1.2, XIX.1.3).
//!
//! 앱(endpoint를 등록하고 handshake에 응답하는 쪽)은 아직 없으므로, registry·handshake 계약을
//! 따르는 최소 동작을 테스트 전용으로 둡니다. 실제 앱 구현은 이 계약(0700 디렉터리,
//! `<instance-id>.sock`, accept 직후 peer uid 확인, client nonce 재사용 거부)을 따라야 합니다.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::JoinHandle;

use crate::handshake::{
    read_line, set_timeouts, verify_peer, write_message, HandshakeError, Message, DEFAULT_TIMEOUT,
};
use crate::nonce::{Nonce, NonceLedger};
use crate::os;
use crate::registry::{Endpoint, InstanceId, Registry};

/// socket 경로 한도를 넘지 않도록 짧은 이름을 쓰는 임시 디렉터리.
pub(crate) struct TempDir {
    pub(crate) path: PathBuf,
}

impl TempDir {
    pub(crate) fn new() -> TempDir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("smcp-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // 테스트가 바꾼 권한을 되돌린 뒤 지웁니다.
        if let Ok(entries) = std::fs::read_dir(&self.path) {
            for entry in entries.flatten() {
                let _ =
                    std::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o700));
            }
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// `tmp/<name>`에 앱처럼 0700 registry 디렉터리를 만듭니다.
pub(crate) fn secure_registry(tmp: &TempDir, name: &str) -> Registry {
    let dir = tmp.path.join(name);
    std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    Registry::at(dir, os::current_uid())
}

/// 등록된 앱 endpoint(test double).
pub(crate) struct AppDouble {
    pub(crate) endpoint: Endpoint,
    pub(crate) listener: UnixListener,
}

/// 앱처럼 `<instance-id>.sock` listener를 엽니다.
pub(crate) fn register(registry: &Registry, instance: &InstanceId) -> AppDouble {
    let socket_path = registry.socket_path(instance).unwrap();
    let listener = UnixListener::bind(&socket_path).unwrap();
    AppDouble {
        endpoint: Endpoint {
            instance: instance.clone(),
            socket_path,
        },
        listener,
    }
}

/// 앱 double의 handshake 응답 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppBehavior {
    /// 계약대로: client nonce 재사용 거부, 새 session nonce.
    Honest,
    /// 상대를 다른 사용자로 판정(다른 사용자 브리지를 대신함): 아무것도 쓰지 않고 닫음.
    ForeignPeer,
    /// 항상 같은 session nonce(재전송 앱).
    FixedSession(Nonce),
    /// client nonce를 바꿔 echo.
    WrongEcho,
    /// client nonce를 session nonce로 되돌림.
    ClientAsSession,
    /// 주어진 이유로 거부.
    Reject(&'static str),
    /// 형식에 맞지 않는 줄.
    Garbage,
    /// 개행 없는 긴 줄.
    Overlong,
    /// HELLO를 읽고 응답 없이 닫음.
    CloseWithoutReply,
}

/// 앱 double이 처리한 연결 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AppOutcome {
    /// session 수립.
    Established {
        /// 받은 client nonce.
        client: Nonce,
        /// 보낸 session nonce.
        session: Nonce,
    },
    /// 다른 사용자라 데이터 없이 닫음.
    RejectedPeer,
    /// client nonce 재사용 거부.
    RejectedReused,
    /// 그 밖의 응답(테스트용 비정상 동작).
    Replied,
}

/// 앱 double을 thread로 실행해 연결 `connections`개를 처리합니다.
///
/// HELLO 없이 닫힌 연결(registry 탐색의 생존 확인)은 세지 않고 무시합니다.
pub(crate) fn spawn_app(
    double: AppDouble,
    behavior: AppBehavior,
    connections: usize,
) -> JoinHandle<Vec<AppOutcome>> {
    std::thread::spawn(move || {
        let mut ledger = NonceLedger::new();
        let mut outcomes = Vec::new();
        while outcomes.len() < connections {
            let Ok((stream, _)) = double.listener.accept() else {
                break;
            };
            if let Some(outcome) = serve(stream, behavior, &mut ledger) {
                outcomes.push(outcome);
            }
        }
        outcomes
    })
}

/// 연결 하나를 앱 계약(또는 지정한 비정상 동작)대로 처리합니다.
fn serve(
    mut stream: UnixStream,
    behavior: AppBehavior,
    ledger: &mut NonceLedger,
) -> Option<AppOutcome> {
    // accept 직후, 어떤 데이터도 쓰기 전에 상대 사용자를 확인합니다.
    let own = os::current_uid();
    let peer = if behavior == AppBehavior::ForeignPeer {
        verify_peer(&stream, own, |_| Ok(own.wrapping_add(1)))
    } else {
        verify_peer(&stream, own, os::peer_uid)
    };
    if peer.is_err() {
        return Some(AppOutcome::RejectedPeer);
    }
    set_timeouts(&stream, DEFAULT_TIMEOUT).ok()?;
    let client = match read_line(&mut stream) {
        Err(HandshakeError::Closed) => return None,
        Err(_) => return Some(AppOutcome::Replied),
        Ok(line) => match Message::decode(&line) {
            Some(Message::Hello { client }) => client,
            _ => return Some(AppOutcome::Replied),
        },
    };
    let reply = match behavior {
        AppBehavior::Honest => {
            if ledger.record(client).is_err() {
                let reject = Message::Reject {
                    reason: "nonce-reused".into(),
                };
                let _ = write_message(&mut stream, &reject);
                return Some(AppOutcome::RejectedReused);
            }
            let session = Nonce::generate().ok()?;
            let _ = write_message(&mut stream, &Message::Welcome { client, session });
            return Some(AppOutcome::Established { client, session });
        }
        AppBehavior::ForeignPeer => return Some(AppOutcome::RejectedPeer),
        AppBehavior::FixedSession(session) => Message::Welcome { client, session },
        AppBehavior::WrongEcho => {
            let mut hex = client.to_hex().into_bytes();
            hex[0] = if hex[0] == b'0' { b'1' } else { b'0' };
            let wrong = Nonce::parse_hex(std::str::from_utf8(&hex).ok()?)?;
            Message::Welcome {
                client: wrong,
                session: Nonce::generate().ok()?,
            }
        }
        AppBehavior::ClientAsSession => Message::Welcome {
            client,
            session: client,
        },
        AppBehavior::Reject(reason) => Message::Reject {
            reason: reason.into(),
        },
        AppBehavior::Garbage => {
            let _ = stream.write_all(b"hello\n");
            return Some(AppOutcome::Replied);
        }
        AppBehavior::Overlong => {
            let _ = stream.write_all(&[b'x'; 200]);
            return Some(AppOutcome::Replied);
        }
        AppBehavior::CloseWithoutReply => return Some(AppOutcome::Replied),
    };
    let _ = write_message(&mut stream, &reply);
    Some(AppOutcome::Replied)
}

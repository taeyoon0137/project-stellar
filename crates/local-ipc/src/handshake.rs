//! bridge↔app IPC handshake: 상대 인증과 session nonce (XIX.1.3, XIX.1.7).
//!
//! ## 브리지 측 순서
//!
//! 1. 연결 직후, **어떤 데이터도 보내기 전에** 앱을 인증합니다. [`bridge_handshake`]·[`connect`]는
//!    앱 측(XIX.1.6)과 같은 기준 [`PeerPolicy`]와 같은 함수 [`authenticate`]를 씁니다: 같은 OS
//!    사용자(`getpeereid`, audit token)와 코드 서명 신원. 실패하거나 확인할 수 없으면 아무것도
//!    쓰지 않고 연결을 닫습니다([`HandshakeError::Unauthenticated`]).
//! 2. 새 client nonce(128 bit, OS CSPRNG)를 만들어 `STELLAR-IPC/1 HELLO <client-nonce>`를 보냅니다.
//! 3. 앱 응답 `STELLAR-IPC/1 WELCOME <client-nonce> <session-nonce>`를 읽습니다. client nonce
//!    echo가 다르면 [`HandshakeError::NonceMismatch`], session nonce가 이 브리지가 이미 본
//!    값(client nonce 포함)이면 [`HandshakeError::NonceReused`]입니다. `STELLAR-IPC/1 REJECT
//!    <reason>`이면 [`HandshakeError::Rejected`]입니다.
//!
//! ## 앱 측 계약([`crate::listener`]가 구현)
//!
//! - accept 직후 같은 기준으로 상대를 인증하고, 실패하면 아무것도 읽거나 쓰지 않고 닫습니다.
//! - 본 client nonce를 기억해 재사용이면 `REJECT nonce-reused`로 거부합니다.
//! - session마다 새 session nonce를 만듭니다.
//! - HELLO 없이 닫히는 연결(registry 탐색의 생존 확인)은 오류 없이 무시합니다.
//!
//! 메시지는 ASCII 한 줄(개행 포함 최대 [`MAX_LINE_BYTES`])이며 읽기·쓰기에 timeout을 둡니다. 읽기
//! timeout은 read 한 번이 아니라 한 줄 전체의 상한이라([`read_line`]), 1 byte씩 늦게 보내는 상대도
//! timeout 안에 끝납니다.

use std::fmt;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use crate::auth::{authenticate, AuthError, PeerPolicy};
use crate::nonce::{Nonce, NonceLedger};
use crate::registry::Endpoint;

/// handshake protocol 표지.
pub const PROTOCOL_TAG: &str = "STELLAR-IPC/1";
/// handshake 한 줄 최대 byte 수(개행 포함).
pub const MAX_LINE_BYTES: usize = 128;
/// 기본 읽기·쓰기 timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// REJECT 이유 token 최대 길이.
const MAX_REASON_LEN: usize = 32;

/// handshake 메시지.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// 브리지 → 앱: client nonce 제시.
    Hello {
        /// client nonce.
        client: Nonce,
    },
    /// 앱 → 브리지: client nonce echo와 session nonce.
    Welcome {
        /// echo한 client nonce.
        client: Nonce,
        /// 앱이 만든 session nonce.
        session: Nonce,
    },
    /// 앱 → 브리지: 거부(이유 token, `[a-z0-9-]`).
    Reject {
        /// 이유 token.
        reason: String,
    },
}

impl Message {
    /// 개행으로 끝나는 한 줄로 씁니다.
    pub fn encode(&self) -> String {
        match self {
            Message::Hello { client } => format!("{PROTOCOL_TAG} HELLO {}\n", client.to_hex()),
            Message::Welcome { client, session } => format!(
                "{PROTOCOL_TAG} WELCOME {} {}\n",
                client.to_hex(),
                session.to_hex()
            ),
            Message::Reject { reason } => format!("{PROTOCOL_TAG} REJECT {reason}\n"),
        }
    }

    /// 개행을 뺀 한 줄을 엄격하게 읽습니다(공백 하나로 구분, 추가 field 없음).
    pub fn decode(line: &str) -> Option<Message> {
        let parts: Vec<&str> = line.split(' ').collect();
        match parts.as_slice() {
            [tag, "HELLO", client] if *tag == PROTOCOL_TAG => Some(Message::Hello {
                client: Nonce::parse_hex(client)?,
            }),
            [tag, "WELCOME", client, session] if *tag == PROTOCOL_TAG => Some(Message::Welcome {
                client: Nonce::parse_hex(client)?,
                session: Nonce::parse_hex(session)?,
            }),
            [tag, "REJECT", reason] if *tag == PROTOCOL_TAG && is_reason(reason) => {
                Some(Message::Reject {
                    reason: (*reason).to_string(),
                })
            }
            _ => None,
        }
    }
}

fn is_reason(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_REASON_LEN
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// handshake 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeError {
    /// 상대 프로세스가 다른 OS 사용자입니다(데이터를 보내지 않고 닫음).
    PeerUserMismatch {
        /// 기대 uid(현재 사용자).
        expected: u32,
        /// 상대 uid.
        actual: u32,
    },
    /// 상대 자격 증명을 확인할 수 없습니다(데이터를 보내지 않고 닫음).
    PeerIdentity {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
    /// 상대가 인증 기준([`PeerPolicy`])을 만족하지 않거나 확인할 수 없습니다(데이터를 보내지 않고
    /// 닫음, XIX.1.7).
    Unauthenticated(AuthError),
    /// 상대가 응답 없이 연결을 닫았습니다.
    Closed,
    /// 응답 대기 시간이 지났습니다.
    TimedOut,
    /// 형식 위반 또는 너무 긴 줄.
    Malformed,
    /// 응답의 client nonce echo가 보낸 값과 다릅니다.
    NonceMismatch,
    /// 이미 본 nonce를 다시 받았습니다.
    NonceReused,
    /// 상대가 거부했습니다.
    Rejected {
        /// 이유 token.
        reason: String,
    },
    /// 기타 입출력 오류.
    Io {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandshakeError::PeerUserMismatch { expected, actual } => write!(
                f,
                "IPC 상대가 다른 OS 사용자입니다(기대 uid {expected}, 실제 uid {actual})"
            ),
            HandshakeError::PeerIdentity { kind } => {
                write!(f, "IPC 상대 사용자를 확인할 수 없습니다: {kind:?}")
            }
            HandshakeError::Unauthenticated(e) => write!(f, "IPC 상대 인증 실패: {e}"),
            HandshakeError::Closed => write!(f, "IPC 상대가 응답 없이 연결을 닫았습니다"),
            HandshakeError::TimedOut => write!(f, "IPC handshake 응답 시간이 지났습니다"),
            HandshakeError::Malformed => write!(f, "IPC handshake 메시지 형식이 잘못되었습니다"),
            HandshakeError::NonceMismatch => {
                write!(f, "IPC handshake 응답의 client nonce가 다릅니다")
            }
            HandshakeError::NonceReused => write!(f, "이미 사용한 IPC session nonce입니다"),
            HandshakeError::Rejected { reason } => {
                write!(f, "IPC 상대가 handshake를 거부했습니다: {reason}")
            }
            HandshakeError::Io { kind } => write!(f, "IPC 입출력 오류: {kind:?}"),
        }
    }
}

impl std::error::Error for HandshakeError {}

/// io 오류를 handshake 오류로 분류합니다.
pub fn classify_io(error: &io::Error) -> HandshakeError {
    match error.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => HandshakeError::TimedOut,
        io::ErrorKind::BrokenPipe
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::UnexpectedEof => HandshakeError::Closed,
        kind => HandshakeError::Io { kind },
    }
}

/// 개행까지 한 줄을 읽습니다(개행 제외). 다음 데이터를 미리 읽지 않도록 byte 단위로 읽습니다.
///
/// stream의 read timeout([`set_timeouts`])은 read 한 번이 아니라 **한 줄 전체**의 상한입니다. read마다
/// 남은 시간으로 timeout을 다시 걸므로, 상대가 timeout보다 짧은 간격으로 1 byte씩 보내도 한 줄을
/// timeout 안에 끝내지 못하면 [`HandshakeError::TimedOut`]입니다. 끝나면 원래 timeout을 되돌립니다.
pub fn read_line(stream: &mut UnixStream) -> Result<String, HandshakeError> {
    let budget = match stream.read_timeout() {
        Ok(budget) => budget,
        // 양방향이 shut down된 socket(아래 참조)은 read가 기다리지 않으므로 기한이 필요 없습니다.
        Err(e) if e.kind() == io::ErrorKind::InvalidInput => None,
        Err(e) => return Err(classify_io(&e)),
    };
    let deadline = budget.map(|budget| Instant::now() + budget);
    let result = read_line_until(stream, deadline);
    if budget.is_some() {
        // 상대가 이미 닫아 설정할 수 없으면(macOS EINVAL) 이 stream은 더 쓸 수 없으므로 무시합니다.
        let _ = stream.set_read_timeout(budget);
    }
    result
}

fn read_line_until(
    stream: &mut UnixStream,
    deadline: Option<Instant>,
) -> Result<String, HandshakeError> {
    let mut line = Vec::with_capacity(MAX_LINE_BYTES);
    let mut byte = [0u8; 1];
    loop {
        if let Some(deadline) = deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(HandshakeError::TimedOut);
            }
            match stream.set_read_timeout(Some(remaining)) {
                Ok(()) => {}
                // macOS는 상대가 닫아 양방향이 shut down된 socket에 EINVAL을 돌려줍니다. 그 상태의
                // read는 남은 buffer나 EOF를 바로 돌려주고 기다리지 않으므로 계속 읽습니다.
                Err(e) if e.kind() == io::ErrorKind::InvalidInput => {}
                Err(e) => return Err(classify_io(&e)),
            }
        }
        match stream.read(&mut byte) {
            Ok(0) if line.is_empty() => return Err(HandshakeError::Closed),
            Ok(0) => return Err(HandshakeError::Malformed),
            Ok(_) if byte[0] == b'\n' => break,
            Ok(_) => {
                line.push(byte[0]);
                if line.len() >= MAX_LINE_BYTES {
                    return Err(HandshakeError::Malformed);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(classify_io(&e)),
        }
    }
    String::from_utf8(line).map_err(|_| HandshakeError::Malformed)
}

/// 메시지 한 줄을 씁니다.
pub fn write_message(stream: &mut UnixStream, message: &Message) -> Result<(), HandshakeError> {
    stream
        .write_all(message.encode().as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|e| classify_io(&e))
}

/// 읽기·쓰기 timeout을 설정합니다(0은 1 ms로 올림).
///
/// macOS `setsockopt(2)`는 상대가 이미 닫아 shut down된 socket에 `EINVAL`을 돌려주므로, 이 경우는
/// [`HandshakeError::Closed`]로 분류합니다(인자 자체는 항상 유효한 timeout입니다).
pub fn set_timeouts(stream: &UnixStream, timeout: Duration) -> Result<(), HandshakeError> {
    let timeout = timeout.max(Duration::from_millis(1));
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .map_err(|e| match e.kind() {
            io::ErrorKind::InvalidInput => HandshakeError::Closed,
            _ => classify_io(&e),
        })
}

/// 상대가 기대한 OS 사용자인지 확인합니다. 이 함수는 stream에 쓰지 않습니다.
pub fn verify_peer<F>(
    stream: &UnixStream,
    expected_uid: u32,
    peer_uid: F,
) -> Result<u32, HandshakeError>
where
    F: FnOnce(&UnixStream) -> io::Result<u32>,
{
    let actual = peer_uid(stream).map_err(|e| HandshakeError::PeerIdentity { kind: e.kind() })?;
    if actual == expected_uid {
        Ok(actual)
    } else {
        Err(HandshakeError::PeerUserMismatch {
            expected: expected_uid,
            actual,
        })
    }
}

/// 인증된 bridge↔app session.
#[derive(Debug)]
pub struct Session {
    stream: UnixStream,
    client_nonce: Nonce,
    session_nonce: Nonce,
    peer_uid: u32,
}

impl Session {
    /// 브리지가 만든 client nonce.
    pub fn client_nonce(&self) -> Nonce {
        self.client_nonce
    }

    /// 앱이 만든 session nonce.
    pub fn session_nonce(&self) -> Nonce {
        self.session_nonce
    }

    /// 검증한 상대 uid.
    pub fn peer_uid(&self) -> u32 {
        self.peer_uid
    }

    /// 연결 stream.
    pub fn stream(&mut self) -> &mut UnixStream {
        &mut self.stream
    }
}

/// 연결된 stream에서 브리지 측 handshake를 수행합니다(XIX.1.7).
///
/// 데이터를 보내기 전에 앱을 `policy`로 인증합니다(앱 측 XIX.1.6과 같은 기준). 실패하면 아무것도
/// 쓰지 않고 stream을 닫습니다.
pub fn bridge_handshake(
    stream: UnixStream,
    policy: &PeerPolicy,
    ledger: &mut NonceLedger,
    timeout: Duration,
) -> Result<Session, HandshakeError> {
    // 1. 데이터를 보내기 전에 상대를 인증합니다.
    let peer = authenticate(&stream, policy).map_err(|e| match e {
        // 앱이 이미 연결을 닫아 자격 증명을 읽을 수 없으면(ENOTCONN) 응답 없는 종료와 같습니다.
        AuthError::Credentials(c) if c.peer_closed() => HandshakeError::Closed,
        e => HandshakeError::Unauthenticated(e),
    })?;
    exchange_nonces(stream, peer.euid, ledger, timeout)
}

/// 상대 uid만 주입 함수로 확인하는 handshake(XIX.1.3 오류 경로 테스트용).
#[cfg(test)]
pub(crate) fn bridge_handshake_with_uid<F>(
    stream: UnixStream,
    expected_uid: u32,
    peer_uid: F,
    ledger: &mut NonceLedger,
    timeout: Duration,
) -> Result<Session, HandshakeError>
where
    F: FnOnce(&UnixStream) -> io::Result<u32>,
{
    let actual = verify_peer(&stream, expected_uid, peer_uid)?;
    exchange_nonces(stream, actual, ledger, timeout)
}

/// 인증을 마친 stream에서 HELLO/WELCOME nonce를 교환합니다.
fn exchange_nonces(
    mut stream: UnixStream,
    peer_uid: u32,
    ledger: &mut NonceLedger,
    timeout: Duration,
) -> Result<Session, HandshakeError> {
    set_timeouts(&stream, timeout)?;

    // 2. 새 client nonce를 기록하고 보냅니다.
    let client = Nonce::generate().map_err(|e| HandshakeError::Io { kind: e.kind() })?;
    ledger
        .record(client)
        .map_err(|_| HandshakeError::NonceReused)?;
    write_message(&mut stream, &Message::Hello { client })?;

    // 3. 응답을 검증합니다.
    let line = read_line(&mut stream)?;
    match Message::decode(&line) {
        Some(Message::Welcome {
            client: echoed,
            session,
        }) => {
            if echoed != client {
                return Err(HandshakeError::NonceMismatch);
            }
            ledger
                .record(session)
                .map_err(|_| HandshakeError::NonceReused)?;
            Ok(Session {
                stream,
                client_nonce: client,
                session_nonce: session,
                peer_uid,
            })
        }
        Some(Message::Reject { reason }) => Err(HandshakeError::Rejected { reason }),
        Some(Message::Hello { .. }) | None => Err(HandshakeError::Malformed),
    }
}

/// endpoint에 연결해 `policy`로 앱을 인증한 뒤 handshake합니다(XIX.1.7).
pub fn connect(
    endpoint: &Endpoint,
    policy: &PeerPolicy,
    ledger: &mut NonceLedger,
) -> Result<Session, HandshakeError> {
    let stream = UnixStream::connect(&endpoint.socket_path).map_err(|e| classify_io(&e))?;
    bridge_handshake(stream, policy, ledger, DEFAULT_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_identity::{CodeIdentityError, CodeRule};
    use crate::os;
    use crate::registry::{InstanceId, Registry};
    use crate::test_support::{
        register, secure_registry, spawn_app, AppBehavior, AppOutcome, TempDir,
    };

    fn n(byte: u8) -> Nonce {
        Nonce::from_bytes([byte; 16])
    }

    /// 현재 프로세스 기준(앱 측과 같은 기준).
    fn policy() -> PeerPolicy {
        PeerPolicy::for_current_process().unwrap()
    }

    #[test]
    fn bridge_rejects_app_failing_policy_before_sending_any_data() {
        // 실제 Security framework 판정: 상대(이 테스트 프로세스, ad-hoc 서명)는 `anchor apple`을
        // 만족하지 않습니다. 브리지는 HELLO를 보내지 않고 nonce도 만들지 않습니다.
        let (bridge, mut app_side) = UnixStream::pair().unwrap();
        let strict =
            PeerPolicy::new(os::current_uid(), CodeRule::Custom("anchor apple".into())).unwrap();
        let mut ledger = NonceLedger::new();
        let err = bridge_handshake(bridge, &strict, &mut ledger, DEFAULT_TIMEOUT).unwrap_err();
        assert_eq!(
            err,
            HandshakeError::Unauthenticated(AuthError::Code(CodeIdentityError::RequirementFailed))
        );
        let mut received = Vec::new();
        app_side.read_to_end(&mut received).unwrap();
        assert!(received.is_empty(), "거부 전 어떤 데이터도 보내지 않음");
        assert!(ledger.is_empty());

        // 다른 사용자 기준도 같은 경로로 거부합니다.
        let (bridge, mut app_side) = UnixStream::pair().unwrap();
        let other = PeerPolicy::new(
            os::current_uid().wrapping_add(1),
            CodeRule::DevelopmentSigned,
        )
        .unwrap();
        let err = bridge_handshake(bridge, &other, &mut ledger, DEFAULT_TIMEOUT).unwrap_err();
        assert!(matches!(
            err,
            HandshakeError::Unauthenticated(AuthError::UserMismatch { .. })
        ));
        let mut received = Vec::new();
        app_side.read_to_end(&mut received).unwrap();
        assert!(received.is_empty());
    }

    fn app(
        tmp: &TempDir,
        behavior: AppBehavior,
        connections: usize,
    ) -> (Registry, Endpoint, std::thread::JoinHandle<Vec<AppOutcome>>) {
        let registry = secure_registry(tmp, "r");
        let double = register(&registry, &InstanceId::from_bytes([7; 8]));
        let endpoint = double.endpoint.clone();
        let handle = spawn_app(double, behavior, connections);
        (registry, endpoint, handle)
    }

    #[test]
    fn messages_round_trip_and_reject_malformed_lines() {
        for message in [
            Message::Hello { client: n(1) },
            Message::Welcome {
                client: n(1),
                session: n(2),
            },
            Message::Reject {
                reason: "nonce-reused".into(),
            },
        ] {
            let line = message.encode();
            assert!(line.ends_with('\n') && line.len() <= MAX_LINE_BYTES);
            assert_eq!(Message::decode(line.trim_end_matches('\n')), Some(message));
        }
        let hex = n(0xab).to_hex();
        for bad in [
            format!("STELLAR-IPC/2 HELLO {hex}"),
            format!("STELLAR-IPC/1 HELLO {hex} extra"),
            format!("STELLAR-IPC/1  HELLO {hex}"),
            format!("STELLAR-IPC/1 HELLO {}", hex.to_uppercase()),
            format!("STELLAR-IPC/1 WELCOME {hex}"),
            "STELLAR-IPC/1 REJECT Nonce Reused".to_string(),
            "STELLAR-IPC/1 REJECT ".to_string(),
            format!("STELLAR-IPC/1 REJECT {}", "a".repeat(33)),
            format!("STELLAR-IPC/1 HELLO {hex}\r"),
        ] {
            assert_eq!(Message::decode(&bad), None, "{bad:?}");
        }
    }

    #[test]
    fn same_user_handshake_establishes_session() {
        let tmp = TempDir::new();
        let (registry, endpoint, handle) = app(&tmp, AppBehavior::Honest, 1);
        // registry 탐색의 생존 확인 연결은 앱이 무시합니다.
        assert_eq!(registry.discover().unwrap().discovery.endpoints().len(), 1);
        let mut ledger = NonceLedger::new();
        let session = connect(&endpoint, &policy(), &mut ledger).unwrap();
        assert_eq!(session.peer_uid(), os::current_uid());
        let outcomes = handle.join().unwrap();
        assert_eq!(
            outcomes.last(),
            Some(&AppOutcome::Established {
                client: session.client_nonce(),
                session: session.session_nonce(),
            })
        );
        assert_ne!(session.client_nonce(), session.session_nonce());
        assert!(
            ledger.contains(&session.client_nonce()) && ledger.contains(&session.session_nonce())
        );
    }

    #[test]
    fn bridge_rejects_other_user_app_before_sending_any_data() {
        let uid = os::current_uid();
        // (a) 상대 uid가 다른 사용자로 보고되는 경우(다른 사용자 프로세스를 대신함).
        let (bridge, mut app_side) = UnixStream::pair().unwrap();
        let mut ledger = NonceLedger::new();
        let err = bridge_handshake_with_uid(
            bridge,
            uid,
            |_| Ok(uid.wrapping_add(1)),
            &mut ledger,
            DEFAULT_TIMEOUT,
        )
        .unwrap_err();
        assert_eq!(
            err,
            HandshakeError::PeerUserMismatch {
                expected: uid,
                actual: uid.wrapping_add(1)
            }
        );
        let mut received = Vec::new();
        app_side.read_to_end(&mut received).unwrap();
        assert!(received.is_empty(), "거부 전 어떤 데이터도 보내지 않음");
        assert!(ledger.is_empty(), "nonce도 만들지 않음");

        // (b) 실제 getpeereid 결과가 기대 사용자와 다른 경우.
        let (bridge, mut app_side) = UnixStream::pair().unwrap();
        let err = bridge_handshake_with_uid(
            bridge,
            uid.wrapping_add(1),
            os::peer_uid,
            &mut ledger,
            DEFAULT_TIMEOUT,
        )
        .unwrap_err();
        assert!(matches!(err, HandshakeError::PeerUserMismatch { actual, .. } if actual == uid));
        let mut received = Vec::new();
        app_side.read_to_end(&mut received).unwrap();
        assert!(received.is_empty());

        // (c) 자격 증명을 확인할 수 없으면 역시 보내지 않습니다.
        let (bridge, mut app_side) = UnixStream::pair().unwrap();
        let err = bridge_handshake_with_uid(
            bridge,
            uid,
            |_| Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            &mut ledger,
            DEFAULT_TIMEOUT,
        )
        .unwrap_err();
        assert!(matches!(err, HandshakeError::PeerIdentity { .. }));
        let mut received = Vec::new();
        app_side.read_to_end(&mut received).unwrap();
        assert!(received.is_empty());
    }

    #[test]
    fn app_rejects_other_user_bridge_before_sending_any_data() {
        let tmp = TempDir::new();
        let (_registry, endpoint, handle) = app(&tmp, AppBehavior::ForeignPeer, 1);
        let mut ledger = NonceLedger::new();
        let err = connect(&endpoint, &policy(), &mut ledger).unwrap_err();
        assert_eq!(err, HandshakeError::Closed, "앱은 응답 없이 닫음");
        assert_eq!(handle.join().unwrap(), vec![AppOutcome::RejectedPeer]);
    }

    #[test]
    fn app_rejects_reused_client_nonce() {
        let tmp = TempDir::new();
        let (_registry, endpoint, handle) = app(&tmp, AppBehavior::Honest, 2);
        // 같은 HELLO를 다시 보내는 재전송 client.
        let hello = Message::Hello { client: n(9) };
        let mut replies = Vec::new();
        for _ in 0..2 {
            let mut stream = UnixStream::connect(&endpoint.socket_path).unwrap();
            set_timeouts(&stream, DEFAULT_TIMEOUT).unwrap();
            write_message(&mut stream, &hello).unwrap();
            replies.push(Message::decode(&read_line(&mut stream).unwrap()).unwrap());
        }
        assert!(matches!(replies[0], Message::Welcome { client, .. } if client == n(9)));
        assert_eq!(
            replies[1],
            Message::Reject {
                reason: "nonce-reused".into()
            }
        );
        let outcomes = handle.join().unwrap();
        assert!(matches!(outcomes[0], AppOutcome::Established { .. }));
        assert_eq!(outcomes[1], AppOutcome::RejectedReused);
    }

    #[test]
    fn bridge_maps_app_rejection() {
        let tmp = TempDir::new();
        let (_registry, endpoint, handle) = app(&tmp, AppBehavior::Reject("nonce-reused"), 1);
        let err = connect(&endpoint, &policy(), &mut NonceLedger::new()).unwrap_err();
        assert_eq!(
            err,
            HandshakeError::Rejected {
                reason: "nonce-reused".into()
            }
        );
        handle.join().unwrap();
    }

    #[test]
    fn bridge_rejects_reused_session_nonce() {
        let tmp = TempDir::new();
        let (_registry, endpoint, handle) = app(&tmp, AppBehavior::FixedSession(n(5)), 3);
        let mut ledger = NonceLedger::new();
        let first = connect(&endpoint, &policy(), &mut ledger).unwrap();
        assert_eq!(first.session_nonce(), n(5));
        // 같은 session nonce를 다시 주는 앱(재전송)은 거부합니다.
        assert_eq!(
            connect(&endpoint, &policy(), &mut ledger).unwrap_err(),
            HandshakeError::NonceReused
        );
        // 장부가 거부의 근거입니다: 새 장부에서는 같은 값도 처음 보는 nonce입니다.
        assert!(connect(&endpoint, &policy(), &mut NonceLedger::new()).is_ok());
        handle.join().unwrap();
    }

    #[test]
    fn bridge_rejects_wrong_echo_and_echoed_client_as_session() {
        let tmp = TempDir::new();
        let (_registry, endpoint, handle) = app(&tmp, AppBehavior::WrongEcho, 1);
        assert_eq!(
            connect(&endpoint, &policy(), &mut NonceLedger::new()).unwrap_err(),
            HandshakeError::NonceMismatch
        );
        handle.join().unwrap();

        let tmp = TempDir::new();
        let (_registry, endpoint, handle) = app(&tmp, AppBehavior::ClientAsSession, 1);
        assert_eq!(
            connect(&endpoint, &policy(), &mut NonceLedger::new()).unwrap_err(),
            HandshakeError::NonceReused
        );
        handle.join().unwrap();
    }

    /// read timeout은 read 한 번이 아니라 한 줄 전체의 상한입니다: 상대가 timeout보다 짧은 간격으로
    /// 1 byte씩 보내도 timeout 뒤에 끝납니다(IPC-03).
    #[test]
    fn read_timeout_bounds_the_whole_line() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        let timeout = Duration::from_millis(300);
        set_timeouts(&reader, timeout).unwrap();
        let trickle = std::thread::spawn(move || {
            for _ in 0..30 {
                if writer.write_all(b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let started = std::time::Instant::now();
        assert_eq!(read_line(&mut reader), Err(HandshakeError::TimedOut));
        let elapsed = started.elapsed();
        assert!(elapsed < Duration::from_millis(900), "{elapsed:?}");
        drop(reader);
        trickle.join().unwrap();

        // 성공한 뒤에는 stream의 원래 timeout을 되돌립니다.
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        set_timeouts(&reader, timeout).unwrap();
        writer.write_all(b"line\n").unwrap();
        assert_eq!(read_line(&mut reader).unwrap(), "line");
        assert_eq!(reader.read_timeout().unwrap(), Some(timeout));
    }

    #[test]
    fn malformed_silent_or_closing_apps_fail_cleanly() {
        for (behavior, expected) in [
            (AppBehavior::Garbage, HandshakeError::Malformed),
            (AppBehavior::Overlong, HandshakeError::Malformed),
            (AppBehavior::CloseWithoutReply, HandshakeError::Closed),
        ] {
            let tmp = TempDir::new();
            let (_registry, endpoint, handle) = app(&tmp, behavior, 1);
            assert_eq!(
                connect(&endpoint, &policy(), &mut NonceLedger::new()).unwrap_err(),
                expected
            );
            handle.join().unwrap();
        }

        // 인증은 통과했지만 응답하지 않는 앱은 timeout으로 끝납니다.
        let (bridge, app_side) = UnixStream::pair().unwrap();
        let err = bridge_handshake(
            bridge,
            &policy(),
            &mut NonceLedger::new(),
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert_eq!(err, HandshakeError::TimedOut);
        drop(app_side);
    }
}

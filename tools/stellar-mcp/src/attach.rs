//! 실행 중 앱 연결과 브리지 측 인증 (XIX.1.7).
//!
//! `stellar-mcp --attach [--instance <id>]`는 MCP 세션을 시작하기 전에 registry(XIX.1.2)에서 앱을
//! 고르고, 앱 측(XIX.1.6)과 같은 기준([`PeerPolicy::for_current_process`])으로 앱을 인증한 뒤
//! handshake(XIX.1.3)합니다. 인증은 HELLO를 보내기 전에 하므로 실패하면 앱에 보낸 byte가 없습니다.
//! 실패는 XXIV.1.2 종료 코드 표([`crate::exit`])로 끝납니다([`AttachError::exit`]).
//!
//! | 실패 | 종료 코드 |
//! |---|---|
//! | 앱 인증 실패·확인 불가, 다른 사용자, 안전하지 않은 registry | 77 `noperm` |
//! | 실행 중 앱 없음, 지정 instance 없음, 앱이 응답 없이 닫음 | 69 `unavailable` |
//! | 앱이 여러 개인데 instance 미지정 | 64 `usage` |
//! | handshake 형식·nonce 위반, 앱의 REJECT | 76 `protocol` |
//! | handshake 응답 시간 초과 | 75 `tempfail` |
//! | 입출력 오류 | 74 `ioerr` |
//! | 자기 인증 기준을 만들 수 없음 | 70 `software` |
//! | socket 경로가 OS 한도 초과 | 78 `config` |
//!
//! backend 자동 선택(앱 없음이면 headless 등)은 XIX.9.4 범위이며, 이 module은 명시적 `--attach`만
//! 다룹니다.
//!
//! ## 한 bridge = 한 세션 (XIX.1.4)
//!
//! bridge process 하나는 MCP 연결 세션 하나만 담당하고, 그 세션은 앱 연결 하나에 묶입니다. 그래서
//! 연결은 프로세스 수명 동안 한 번만 맺고 다시 연결하지 않습니다. [`watch_app_connection`]이 앱
//! 연결을 감시하다 앱이 닫으면 MCP 세션도 끝냅니다(69 `unavailable`). bridge가 끝나면 OS가 socket을
//! 닫으므로 앱은 연결 종료 event를 한 번 기록합니다(`stellar_local_ipc::session`).

use std::fmt;
use std::io::{self, Read};

use stellar_local_ipc::auth::{AuthError, PeerPolicy};
use stellar_local_ipc::handshake::{connect, HandshakeError, Session};
use stellar_local_ipc::nonce::NonceLedger;
use stellar_local_ipc::registry::{Endpoint, InstanceId, Registry, RegistryError, SelectError};

use crate::exit::{self, Exit};

/// 연결 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachError {
    /// registry를 읽을 수 없거나 안전하지 않습니다.
    Registry(RegistryError),
    /// 연결할 앱을 고를 수 없습니다.
    Select(SelectError),
    /// 자기 인증 기준을 만들 수 없습니다.
    Policy(AuthError),
    /// 앱 인증 또는 handshake 실패.
    Handshake(HandshakeError),
}

impl fmt::Display for AttachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AttachError::Registry(e) => write!(f, "{e}"),
            AttachError::Select(e) => write!(f, "{e}"),
            AttachError::Policy(e) => write!(f, "{e}"),
            AttachError::Handshake(HandshakeError::Closed) => write!(
                f,
                "앱이 응답 없이 연결을 닫았습니다(앱이 이 브리지를 거부했거나 종료됨)"
            ),
            AttachError::Handshake(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AttachError {}

impl AttachError {
    /// XXIV.1.2 표에 따른 종료 상태.
    pub fn exit(&self) -> Exit {
        match self {
            AttachError::Registry(RegistryError::Insecure { .. }) => exit::NOPERM,
            AttachError::Registry(RegistryError::PathTooLong { .. }) => exit::CONFIG,
            AttachError::Registry(RegistryError::Io { .. }) => exit::IOERR,
            AttachError::Select(SelectError::Ambiguous { .. }) => exit::USAGE,
            AttachError::Select(SelectError::NoApp | SelectError::NotFound { .. }) => {
                exit::UNAVAILABLE
            }
            AttachError::Policy(_) => exit::SOFTWARE,
            AttachError::Handshake(e) => match e {
                HandshakeError::Unauthenticated(_)
                | HandshakeError::PeerUserMismatch { .. }
                | HandshakeError::PeerIdentity { .. } => exit::NOPERM,
                HandshakeError::Closed => exit::UNAVAILABLE,
                HandshakeError::TimedOut => exit::TEMPFAIL,
                HandshakeError::Malformed
                | HandshakeError::NonceMismatch
                | HandshakeError::NonceReused
                | HandshakeError::Rejected { .. } => exit::PROTOCOL,
                HandshakeError::Io { .. } => exit::IOERR,
            },
        }
    }
}

/// 현재 사용자 registry에서 앱을 골라 현재 프로세스 기준으로 인증·handshake합니다.
pub fn attach(
    instance: Option<&InstanceId>,
    ledger: &mut NonceLedger,
) -> Result<(Endpoint, Session), AttachError> {
    let registry = Registry::for_current_user().map_err(AttachError::Registry)?;
    let policy = PeerPolicy::for_current_process().map_err(AttachError::Policy)?;
    attach_with(&registry, &policy, instance, ledger)
}

/// 주어진 registry·기준으로 앱을 골라 인증·handshake합니다.
pub fn attach_with(
    registry: &Registry,
    policy: &PeerPolicy,
    instance: Option<&InstanceId>,
    ledger: &mut NonceLedger,
) -> Result<(Endpoint, Session), AttachError> {
    let report = registry.discover().map_err(AttachError::Registry)?;
    let endpoint = report
        .discovery
        .select(instance)
        .map_err(AttachError::Select)?
        .clone();
    let session = connect(&endpoint, policy, ledger).map_err(AttachError::Handshake)?;
    Ok((endpoint, session))
}

/// 앱 연결이 끝난 이유(XIX.1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppConnectionEnd {
    /// 앱이 연결을 닫았습니다.
    Closed,
    /// handshake 뒤 정의되지 않은 data를 받았습니다.
    UnexpectedData,
    /// 읽기 오류.
    ReadError {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
}

impl AppConnectionEnd {
    /// XXIV.1.2 표에 따른 종료 상태.
    pub fn exit(self) -> Exit {
        match self {
            AppConnectionEnd::Closed => exit::UNAVAILABLE,
            AppConnectionEnd::UnexpectedData => exit::PROTOCOL,
            AppConnectionEnd::ReadError { .. } => exit::IOERR,
        }
    }
}

impl fmt::Display for AppConnectionEnd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppConnectionEnd::Closed => write!(f, "Stellar 앱이 연결을 닫았습니다"),
            AppConnectionEnd::UnexpectedData => {
                write!(f, "Stellar 앱이 정의되지 않은 IPC 메시지를 보냈습니다")
            }
            AppConnectionEnd::ReadError { kind } => {
                write!(f, "Stellar 앱 연결 읽기 오류: {kind:?}")
            }
        }
    }
}

/// 앱 연결을 감시하는 thread를 시작합니다(XIX.1.4). 연결이 끝나면 `on_end`를 한 번 호출합니다.
///
/// handshake 뒤 bridge↔app 메시지는 아직 정의되지 않았으므로(XIX.9.4) data를 받아도 끝냅니다.
pub fn watch_app_connection<F>(session: &mut Session, on_end: F) -> io::Result<()>
where
    F: FnOnce(AppConnectionEnd) + Send + 'static,
{
    let mut reader = session.stream().try_clone()?;
    reader.set_read_timeout(None)?;
    std::thread::Builder::new()
        .name("stellar-mcp-app-watch".into())
        .spawn(move || {
            let mut buf = [0u8; 64];
            let end = loop {
                match reader.read(&mut buf) {
                    Ok(0) => break AppConnectionEnd::Closed,
                    Ok(_) => break AppConnectionEnd::UnexpectedData,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => break AppConnectionEnd::ReadError { kind: e.kind() },
                }
            };
            on_end(end);
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::path::PathBuf;
    use stellar_local_ipc::code_identity::{CodeIdentityError, CodeRule};
    use stellar_local_ipc::listener::{AppEndpoint, AppListener, ListenerEvent};
    use stellar_local_ipc::os;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let path = std::env::temp_dir().join(format!("smcp-at-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ =
                std::fs::set_permissions(self.0.join("r"), std::fs::Permissions::from_mode(0o700));
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn id(n: u8) -> InstanceId {
        InstanceId::from_bytes([n; 8])
    }

    fn registry(tmp: &TempDir) -> Registry {
        Registry::at(tmp.0.join("r"), os::current_uid())
    }

    #[test]
    fn app_connection_end_is_reported_once_with_table_exit() {
        use stellar_local_ipc::session::{
            ConnectionClosed, ConnectionMonitor, SessionOrigin, SessionTermination,
        };
        struct NoTermination;
        impl SessionTermination for NoTermination {
            fn begin_external_termination(&self, _closed: &ConnectionClosed) {}
        }

        let tmp = TempDir::new("watch");
        let reg = registry(&tmp);
        let endpoint = AppEndpoint::bind(&reg, id(6)).unwrap();
        let policy = PeerPolicy::for_current_process().unwrap();
        let app = AppListener::spawn(endpoint, policy.clone()).unwrap();
        let (_endpoint, mut session) =
            attach_with(&reg, &policy, None, &mut NonceLedger::new()).unwrap();
        let connection = match app
            .events()
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
        {
            ListenerEvent::Connected { connection, .. } => connection,
            other => panic!("{other:?}"),
        };
        let (tx, _rx) = std::sync::mpsc::channel();
        let monitor = ConnectionMonitor::watch(
            connection,
            SessionOrigin::External,
            tx,
            std::sync::Arc::new(NoTermination),
        )
        .unwrap();

        let (end_tx, end_rx) = std::sync::mpsc::channel();
        watch_app_connection(&mut session, move |end| end_tx.send(end).unwrap()).unwrap();
        assert!(end_rx
            .recv_timeout(std::time::Duration::from_millis(200))
            .is_err());
        // 앱이 연결을 닫으면 bridge 세션도 끝납니다.
        drop(monitor);
        let end = end_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        assert_eq!(end, AppConnectionEnd::Closed);
        assert_eq!(end.exit(), exit::UNAVAILABLE);
        assert_eq!(AppConnectionEnd::UnexpectedData.exit(), exit::PROTOCOL);
        assert_eq!(
            AppConnectionEnd::ReadError {
                kind: std::io::ErrorKind::Other
            }
            .exit(),
            exit::IOERR
        );
    }

    #[test]
    fn exit_mapping_follows_the_table() {
        use HandshakeError as H;
        let cases: Vec<(AttachError, Exit)> = vec![
            (
                AttachError::Registry(RegistryError::PathTooLong { bytes: 200 }),
                exit::CONFIG,
            ),
            (
                AttachError::Registry(RegistryError::Io {
                    kind: std::io::ErrorKind::Other,
                }),
                exit::IOERR,
            ),
            (AttachError::Select(SelectError::NoApp), exit::UNAVAILABLE),
            (
                AttachError::Select(SelectError::NotFound { wanted: id(1) }),
                exit::UNAVAILABLE,
            ),
            (
                AttachError::Select(SelectError::Ambiguous {
                    candidates: vec![id(1), id(2)],
                }),
                exit::USAGE,
            ),
            (
                AttachError::Policy(AuthError::Policy(CodeIdentityError::MalformedTeamId)),
                exit::SOFTWARE,
            ),
            (
                AttachError::Handshake(H::Unauthenticated(AuthError::Code(
                    CodeIdentityError::RequirementFailed,
                ))),
                exit::NOPERM,
            ),
            (
                AttachError::Handshake(H::PeerUserMismatch {
                    expected: 1,
                    actual: 2,
                }),
                exit::NOPERM,
            ),
            (AttachError::Handshake(H::Closed), exit::UNAVAILABLE),
            (AttachError::Handshake(H::TimedOut), exit::TEMPFAIL),
            (AttachError::Handshake(H::NonceReused), exit::PROTOCOL),
            (
                AttachError::Handshake(H::Rejected {
                    reason: "nonce-reused".into(),
                }),
                exit::PROTOCOL,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.exit(), expected, "{error:?}");
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn no_app_and_insecure_registry_fail_without_connecting() {
        let tmp = TempDir::new("none");
        let policy = PeerPolicy::for_current_process().unwrap();
        let err = attach_with(&registry(&tmp), &policy, None, &mut NonceLedger::new()).unwrap_err();
        assert_eq!(err, AttachError::Select(SelectError::NoApp));
        assert_eq!(err.exit(), exit::UNAVAILABLE);

        std::fs::DirBuilder::new()
            .mode(0o755)
            .create(tmp.0.join("r"))
            .unwrap();
        std::fs::set_permissions(tmp.0.join("r"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = attach_with(&registry(&tmp), &policy, None, &mut NonceLedger::new()).unwrap_err();
        assert!(matches!(
            err,
            AttachError::Registry(RegistryError::Insecure { .. })
        ));
        assert_eq!(err.exit(), exit::NOPERM);
    }

    #[test]
    fn authenticated_app_is_attached_and_strict_policy_is_rejected() {
        let tmp = TempDir::new("app");
        let reg = registry(&tmp);
        let endpoint = AppEndpoint::bind(&reg, id(5)).unwrap();
        let app = AppListener::spawn(endpoint, PeerPolicy::for_current_process().unwrap()).unwrap();

        // 앱(이 테스트 프로세스, ad-hoc)이 브리지 기준을 만족하지 않으면 HELLO 없이 실패합니다.
        let strict =
            PeerPolicy::new(os::current_uid(), CodeRule::Custom("anchor apple".into())).unwrap();
        let err = attach_with(&reg, &strict, None, &mut NonceLedger::new()).unwrap_err();
        assert_eq!(
            err,
            AttachError::Handshake(HandshakeError::Unauthenticated(AuthError::Code(
                CodeIdentityError::RequirementFailed
            )))
        );
        assert_eq!(err.exit(), exit::NOPERM);
        if let Ok(event) = app
            .events()
            .recv_timeout(std::time::Duration::from_millis(300))
        {
            panic!("앱이 받은 handshake 0이어야 함: {event:?}");
        }

        // 같은 기준이면 연결됩니다.
        let policy = PeerPolicy::for_current_process().unwrap();
        let (endpoint, session) =
            attach_with(&reg, &policy, Some(&id(5)), &mut NonceLedger::new()).unwrap();
        assert_eq!(endpoint.instance, id(5));
        match app
            .events()
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
        {
            ListenerEvent::Connected { connection, .. } => {
                assert_eq!(connection.session_nonce(), session.session_nonce());
            }
            other => panic!("{other:?}"),
        }
    }
}

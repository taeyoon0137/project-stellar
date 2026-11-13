//! 연결 상대 자격 증명: effective uid와 audit token (XIX.1.6).
//!
//! 두 값 모두 커널이 연결 시점에 기록한 값이며 상대가 위조할 수 없습니다(`getpeereid(3)`: "there is
//! no way for either side to influence the credentials returned to its peer"). accept한 쪽은 상대가
//! `connect(2)`한 시점, connect한 쪽은 상대가 `listen(2)`한 시점의 자격 증명을 받습니다.
//!
//! - effective uid: `getpeereid(3)`.
//! - audit token: `getsockopt(SOL_LOCAL, LOCAL_PEERTOKEN)`(macOS SDK `<sys/un.h>`). pid와 pid
//!   version을 함께 담으므로 pid 재사용과 무관하게 그 프로세스 instance를 가리킵니다. 코드 서명
//!   검사([`crate::code_identity`])는 pid가 아니라 이 token으로 상대를 찾습니다.
//! - token의 필드는 `<bsm/libbsm.h>`가 권하는 accessor(`audit_token_to_euid`, `audit_token_to_pid`)로
//!   읽습니다.
//!
//! 이 module의 함수는 stream에서 읽거나 쓰지 않습니다(socket option 조회만).

use std::fmt;
use std::io;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;

use crate::os;

/// `audit_token_t`(`<mach/message.h>`, `unsigned int val[8]`).
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AuditToken {
    val: [u32; 8],
}

impl fmt::Debug for AuditToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // token 전체는 진단에 남기지 않고 pid만 보입니다.
        write!(f, "AuditToken(pid {})", self.pid())
    }
}

mod sys {
    use super::AuditToken;
    use std::ffi::c_void;

    extern "C" {
        /// `int getsockopt(int socket, int level, int option_name, void *option_value,
        /// socklen_t *option_len)`.
        pub fn getsockopt(
            socket: i32,
            level: i32,
            option_name: i32,
            option_value: *mut c_void,
            option_len: *mut u32,
        ) -> i32;
    }

    #[link(name = "bsm")]
    extern "C" {
        /// `uid_t audit_token_to_euid(audit_token_t atoken)`.
        pub fn audit_token_to_euid(token: AuditToken) -> u32;
        /// `pid_t audit_token_to_pid(audit_token_t atoken)`.
        pub fn audit_token_to_pid(token: AuditToken) -> i32;
    }

    /// `<sys/un.h>` `SOL_LOCAL`.
    pub const SOL_LOCAL: i32 = 0;
    /// `<sys/un.h>` `LOCAL_PEERTOKEN`.
    pub const LOCAL_PEERTOKEN: i32 = 0x006;
}

impl AuditToken {
    /// token의 byte 표현(코드 서명 조회용 CFData).
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (chunk, v) in out.chunks_exact_mut(4).zip(self.val) {
            chunk.copy_from_slice(&v.to_ne_bytes());
        }
        out
    }

    /// token의 effective uid.
    pub fn euid(&self) -> u32 {
        // SAFETY: 값으로 넘기는 순수 accessor이며 메모리를 읽거나 쓰지 않습니다.
        unsafe { sys::audit_token_to_euid(*self) }
    }

    /// token의 pid.
    pub fn pid(&self) -> i32 {
        // SAFETY: 값으로 넘기는 순수 accessor이며 메모리를 읽거나 쓰지 않습니다.
        unsafe { sys::audit_token_to_pid(*self) }
    }
}

/// 연결 상대 자격 증명.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerCredentials {
    /// effective uid(`getpeereid`).
    pub euid: u32,
    /// 상대 프로세스 pid(audit token).
    pub pid: i32,
    /// 상대 프로세스 audit token.
    pub audit_token: AuditToken,
}

/// 자격 증명을 얻을 수 없는 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialError {
    /// `getpeereid` 실패.
    PeerId {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
    /// `LOCAL_PEERTOKEN` 조회 실패 또는 길이 불일치.
    Token {
        /// io 오류 종류.
        kind: io::ErrorKind,
    },
    /// `getpeereid`와 audit token의 euid가 다릅니다.
    Inconsistent,
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CredentialError::PeerId { kind } => {
                write!(f, "IPC 상대 uid를 확인할 수 없습니다: {kind:?}")
            }
            CredentialError::Token { kind } => {
                write!(f, "IPC 상대 audit token을 확인할 수 없습니다: {kind:?}")
            }
            CredentialError::Inconsistent => {
                write!(f, "IPC 상대 uid와 audit token의 uid가 다릅니다")
            }
        }
    }
}

impl std::error::Error for CredentialError {}

impl CredentialError {
    /// 상대가 이미 연결을 닫아 자격 증명을 읽을 수 없는 경우(`ENOTCONN`)인지 확인합니다.
    ///
    /// registry 탐색의 생존 확인 연결은 연결 직후 닫히므로 이 경우가 됩니다. 닫힌 상대와는 데이터를
    /// 주고받을 수 없으므로 거부 사유가 아니라 종료된 연결로 다룹니다.
    pub fn peer_closed(&self) -> bool {
        matches!(
            self,
            CredentialError::PeerId {
                kind: io::ErrorKind::NotConnected
            } | CredentialError::Token {
                kind: io::ErrorKind::NotConnected
            }
        )
    }
}

/// 연결 상대의 audit token(`LOCAL_PEERTOKEN`).
pub fn peer_audit_token(stream: &UnixStream) -> io::Result<AuditToken> {
    let mut token = AuditToken { val: [0; 8] };
    let expected = std::mem::size_of::<AuditToken>();
    let mut len = u32::try_from(expected).map_err(|_| io::Error::other("audit token 크기"))?;
    // SAFETY: fd는 stream이 소유한 열린 socket이고, token은 len byte 쓰기 가능한 지역 값입니다.
    let rc = unsafe {
        sys::getsockopt(
            stream.as_raw_fd(),
            sys::SOL_LOCAL,
            sys::LOCAL_PEERTOKEN,
            (&mut token as *mut AuditToken).cast(),
            &mut len,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    if usize::try_from(len).ok() != Some(expected) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "audit token 길이가 다릅니다",
        ));
    }
    Ok(token)
}

/// 연결 상대의 자격 증명을 얻습니다. stream에서 읽거나 쓰지 않습니다.
pub fn peer_credentials(stream: &UnixStream) -> Result<PeerCredentials, CredentialError> {
    let euid = os::peer_uid(stream).map_err(|e| CredentialError::PeerId { kind: e.kind() })?;
    let audit_token =
        peer_audit_token(stream).map_err(|e| CredentialError::Token { kind: e.kind() })?;
    if audit_token.euid() != euid {
        return Err(CredentialError::Inconsistent);
    }
    Ok(PeerCredentials {
        euid,
        pid: audit_token.pid(),
        audit_token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_token_layout_matches_the_c_type() {
        assert_eq!(std::mem::size_of::<AuditToken>(), 32);
        let token = AuditToken {
            val: [1, 2, 3, 4, 5, 6, 7, 8],
        };
        let bytes = token.to_bytes();
        assert_eq!(&bytes[..4], &1u32.to_ne_bytes());
        assert_eq!(&bytes[28..], &8u32.to_ne_bytes());
    }

    #[test]
    fn socket_pair_peer_is_this_process() {
        let (a, b) = UnixStream::pair().unwrap();
        for stream in [&a, &b] {
            let creds = peer_credentials(stream).unwrap();
            assert_eq!(creds.euid, os::current_uid());
            assert_eq!(creds.pid, i32::try_from(std::process::id()).unwrap());
            assert_eq!(creds.audit_token.euid(), creds.euid);
        }
    }

    #[test]
    fn credentials_do_not_consume_stream_data() {
        use std::io::{Read, Write};
        let (mut a, mut b) = UnixStream::pair().unwrap();
        a.write_all(b"hello\n").unwrap();
        peer_credentials(&b).unwrap();
        let mut buf = [0u8; 6];
        b.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hello\n");
    }
}

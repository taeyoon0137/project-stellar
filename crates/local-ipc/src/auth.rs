//! IPC 연결 상대 인증 기준 (XIX.1.6, XIX.1.7, ADR-0005 §5).
//!
//! 앱과 브리지는 같은 기준([`PeerPolicy`])과 같은 함수([`authenticate`])로, **데이터를 한 byte도
//! 주고받기 전에** 상대를 인증합니다. 기준(모두 만족해야 함, 하나라도 확인할 수 없으면 거부):
//!
//! 1. **같은 OS 사용자**: 상대 effective uid(`getpeereid(3)`, 커널 기록)가 자신의 effective uid와
//!    같습니다. audit token의 euid도 같아야 합니다([`crate::peer`]).
//! 2. **코드 서명 신원**: audit token으로 찾은 상대 실행 코드가 [`CodeRule`]을 만족합니다
//!    ([`crate::code_identity`]). 규칙은 자기 서명에서 정합니다([`PeerPolicy::for_current_process`]).
//!    - 자기 서명에 Team ID가 있으면(서명 빌드) 상대도 같은 Team ID로 서명돼야 합니다.
//!    - 없으면(개발 빌드, ad-hoc·linker 서명) 상대는 유효한 서명이 있고 Apple platform binary가
//!      아니어야 합니다.
//!
//! 인증은 socket option과 Security framework 조회만 하고 stream에서 읽거나 쓰지 않습니다. 느릴 수
//! 있는 호출이므로 UI·Document 스레드가 아니라 IPC I/O 스레드에서 호출합니다(ADR-0005 §6,
//! [`crate::listener`]).

use std::fmt;
use std::os::unix::net::UnixStream;

use crate::code_identity::{self, CodeIdentityError, CodeRule};
use crate::os;
use crate::peer::{self, CredentialError};

/// 연결 상대 인증 기준.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerPolicy {
    expected_uid: u32,
    code: CodeRule,
}

impl PeerPolicy {
    /// 기대 uid와 코드 규칙으로 기준을 만듭니다. 규칙 문자열이 컴파일되지 않으면 오류입니다.
    pub fn new(expected_uid: u32, code: CodeRule) -> Result<PeerPolicy, AuthError> {
        code_identity::validate_rule(&code).map_err(AuthError::Policy)?;
        Ok(PeerPolicy { expected_uid, code })
    }

    /// 현재 프로세스 기준: 자신의 effective uid와 자기 서명에서 정한 코드 규칙.
    pub fn for_current_process() -> Result<PeerPolicy, AuthError> {
        let code = CodeRule::for_current_process().map_err(AuthError::Policy)?;
        PeerPolicy::new(os::current_uid(), code)
    }

    /// 기대 uid.
    pub fn expected_uid(&self) -> u32 {
        self.expected_uid
    }

    /// 코드 규칙.
    pub fn code(&self) -> &CodeRule {
        &self.code
    }
}

/// 인증된 연결 상대.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthenticatedPeer {
    /// 상대 effective uid.
    pub euid: u32,
    /// 상대 pid(audit token 기준, 인증 시점).
    pub pid: i32,
}

/// 인증 실패. 모두 데이터 교환 없이 연결을 닫는 사유입니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// 상대 자격 증명을 확인할 수 없습니다.
    Credentials(CredentialError),
    /// 상대가 다른 OS 사용자입니다.
    UserMismatch {
        /// 기대 uid.
        expected: u32,
        /// 상대 uid.
        actual: u32,
    },
    /// 상대 코드 서명이 기준을 만족하지 않거나 확인할 수 없습니다.
    Code(CodeIdentityError),
    /// 기준 자체를 만들 수 없습니다(요구 사항 문자열·자기 서명 정보).
    Policy(CodeIdentityError),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError::Credentials(e) => write!(f, "{e}"),
            AuthError::UserMismatch { expected, actual } => write!(
                f,
                "IPC 상대가 다른 OS 사용자입니다(기대 uid {expected}, 실제 uid {actual})"
            ),
            AuthError::Code(e) => write!(f, "{e}"),
            AuthError::Policy(e) => write!(f, "IPC 인증 기준을 만들 수 없습니다: {e}"),
        }
    }
}

impl std::error::Error for AuthError {}

/// 연결 상대를 `policy`로 인증합니다. stream에서 읽거나 쓰지 않습니다.
pub fn authenticate(
    stream: &UnixStream,
    policy: &PeerPolicy,
) -> Result<AuthenticatedPeer, AuthError> {
    let creds = peer::peer_credentials(stream).map_err(AuthError::Credentials)?;
    if creds.euid != policy.expected_uid {
        return Err(AuthError::UserMismatch {
            expected: policy.expected_uid,
            actual: creds.euid,
        });
    }
    code_identity::check_peer_code(&creds.audit_token, &policy.code).map_err(AuthError::Code)?;
    Ok(AuthenticatedPeer {
        euid: creds.euid,
        pid: creds.pid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_policy_uses_own_uid_and_signature() {
        let policy = PeerPolicy::for_current_process().unwrap();
        assert_eq!(policy.expected_uid(), os::current_uid());
        // 테스트 실행 파일은 ad-hoc 서명입니다.
        assert_eq!(policy.code(), &CodeRule::DevelopmentSigned);
    }

    #[test]
    fn same_process_peer_is_authenticated() {
        let (a, _b) = UnixStream::pair().unwrap();
        let policy = PeerPolicy::for_current_process().unwrap();
        let peer = authenticate(&a, &policy).unwrap();
        assert_eq!(peer.euid, os::current_uid());
        assert_eq!(peer.pid, i32::try_from(std::process::id()).unwrap());
    }

    #[test]
    fn other_user_policy_is_rejected_before_code_check() {
        let (a, _b) = UnixStream::pair().unwrap();
        let uid = os::current_uid();
        let policy = PeerPolicy::new(uid.wrapping_add(1), CodeRule::DevelopmentSigned).unwrap();
        assert_eq!(
            authenticate(&a, &policy),
            Err(AuthError::UserMismatch {
                expected: uid.wrapping_add(1),
                actual: uid
            })
        );
    }

    #[test]
    fn code_rule_failure_is_rejected_by_security_framework() {
        let (a, _b) = UnixStream::pair().unwrap();
        let policy =
            PeerPolicy::new(os::current_uid(), CodeRule::Custom("anchor apple".into())).unwrap();
        assert_eq!(
            authenticate(&a, &policy),
            Err(AuthError::Code(CodeIdentityError::RequirementFailed))
        );
    }

    #[test]
    fn malformed_rules_are_rejected_at_policy_creation() {
        assert!(matches!(
            PeerPolicy::new(0, CodeRule::Custom("((".into())),
            Err(AuthError::Policy(
                CodeIdentityError::InvalidRequirement { .. }
            ))
        ));
    }
}

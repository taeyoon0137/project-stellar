//! MCP 세션 권한 저장과 판정 (XIX.3.2~XIX.3.4, 아키텍처 §31.3, ADR-0093).
//!
//! - **저장 단위**: 권한 상태는 [`PermissionKey`](프로젝트 identity + 에이전트 세션 id)별로 메모리에만 둡니다.
//!   [`PermissionStore`]에는 파일·직렬화 경로가 없어서 앱을 다시 시작하면 모든 권한이 없는 상태에서
//!   시작합니다(XIX.3.2). 같은 프로젝트라도 다른 worktree는 다른 identity(canonical target)라 따로입니다.
//! - **첫 연결 승인**(XIX.3.3): 앱에 붙는 연결은 신뢰된 프로젝트라도 사용자가 연결을 승인하기 전에는 Read도
//!   허용하지 않습니다. 앱 없는 headless 읽기 backend는 실행 인자로 고정한 신뢰 프로젝트 하나에 한해 상태를
//!   바꾸지 않는 권한(Read·Propose)만 승인 없이 허용하고, 쓰기 권한은 주지 않습니다. 신뢰되지 않은
//!   프로젝트는 어느 backend에서든 모든 권한을 거부합니다.
//! - **독립 확인**(XIX.3.4): Read를 뺀 권한은 권한마다 따로 확인 요청을 만들고 따로 응답받습니다. 한 권한의
//!   승인이 다른 권한의 상태를 바꾸지 않습니다. Propose·Apply·Terminal·FileDelete 승인은 세션 동안 유지되고,
//!   GitCommit·GitPush 승인은 실행 한 번에만 쓰입니다(§31.3 "Git commit·push는 개별 확인").
//! - **쓰는 쪽**: 확인 요청은 에이전트 tool 호출을 받은 앱이 만들 수 있지만, 응답([`PermissionStore::respond`],
//!   [`PermissionStore::respond_connection`])은 앱의 사용자 UI만 부릅니다(ADR-0005 §3 변경 불가 경로).
//!   첫 응답만 유효합니다. 이 저장소는 앱 쪽 crate에 있어 에이전트 경로 crate(MCP 브리지)는 닿지 않습니다(ADR-0112).

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use stellar_ops::core_types::ids::StableId;

pub use crate::agent_protocol::permission::Permission;

/// 에이전트 세션 식별자(ADR-0005 §4 AgentSession, 외부 client 연결이면 bridge 연결 하나).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentSessionId(pub StableId);

/// 권한 저장 key: 프로젝트 identity(IV.2.3: project id + canonical target)와 세션.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PermissionKey {
    /// 프로젝트 id.
    pub project_id: StableId,
    /// 프로젝트 root의 canonical target.
    pub canonical_target: PathBuf,
    /// 에이전트 세션.
    pub session: AgentSessionId,
}

impl PermissionKey {
    /// 프로젝트 identity(project id, canonical target)와 세션으로 만듭니다.
    pub fn new(
        project_id: StableId,
        canonical_target: PathBuf,
        session: AgentSessionId,
    ) -> PermissionKey {
        PermissionKey {
            project_id,
            canonical_target,
            session,
        }
    }
}

/// 연결 backend와 신뢰 상태(판정 입력).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendPolicy {
    /// 실행 중 앱에 붙는 연결.
    Attached {
        /// 프로젝트가 신뢰됨.
        trusted: bool,
    },
    /// 앱 없이 실행 인자로 고정한 프로젝트의 읽기 backend.
    Headless {
        /// 고정 프로젝트가 신뢰됨.
        trusted: bool,
    },
}

/// 첫 연결 승인 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionApproval {
    /// 사용자 응답 대기(기본).
    Pending,
    /// 승인.
    Approved,
    /// 거부.
    Denied,
}

/// 확인 요청 id(세션 안에서 고유).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(pub u64);

/// 권한 하나의 확인 상태(XIX.3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmation {
    /// 요청한 적 없음.
    NotRequested,
    /// 사용자 응답 대기.
    Pending(RequestId),
    /// 승인. 실행 한 번 권한(GitCommit·GitPush)은 쓰면 [`Confirmation::NotRequested`]로 돌아갑니다.
    Approved(RequestId),
    /// 거부.
    Denied(RequestId),
}

/// 권한 판정 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// 허용.
    Allow,
    /// 첫 연결 승인 대기 중이라 아직 허용하지 않습니다(attached).
    AwaitingConnection,
    /// 이 권한의 확인이 필요합니다(요청하지 않았거나 대기 중).
    AwaitingConfirmation,
    /// 거부.
    Denied(DenyReason),
}

/// 거부 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// 프로젝트가 신뢰되지 않았습니다.
    NotTrusted,
    /// headless backend는 이 권한을 제공하지 않습니다(쓰기 권한 0).
    NotOffered,
    /// 사용자가 연결을 거부했습니다.
    ConnectionDenied,
    /// 사용자가 이 권한 요청을 거부했습니다.
    ConfirmationDenied,
}

/// 권한 저장 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionError {
    /// 등록되지 않은 세션입니다.
    UnknownSession,
    /// 모르는 요청 id입니다.
    UnknownRequest(RequestId),
    /// 이미 응답한 요청입니다(첫 응답만 유효).
    AlreadyAnswered(RequestId),
    /// 첫 연결 요청에는 이미 응답했습니다.
    ConnectionAnswered,
    /// Read는 첫 연결 승인으로만 정해집니다.
    ReadFollowsConnection,
}

impl fmt::Display for PermissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PermissionError::UnknownSession => f.write_str("등록되지 않은 에이전트 세션입니다"),
            PermissionError::UnknownRequest(id) => write!(f, "확인 요청 {}이 없습니다", id.0),
            PermissionError::AlreadyAnswered(id) => {
                write!(f, "확인 요청 {}에는 이미 응답했습니다", id.0)
            }
            PermissionError::ConnectionAnswered => {
                f.write_str("첫 연결 요청에는 이미 응답했습니다")
            }
            PermissionError::ReadFollowsConnection => {
                f.write_str("Read 권한은 첫 연결 승인으로 정해집니다")
            }
        }
    }
}

impl std::error::Error for PermissionError {}

/// 승인이 세션 동안 유지되는지(아니면 실행 한 번).
pub fn is_session_grant(permission: Permission) -> bool {
    !matches!(permission, Permission::GitCommit | Permission::GitPush)
}

/// 세션 하나의 권한 상태.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionPermissions {
    connection: ConnectionApproval,
    confirmations: BTreeMap<Permission, Confirmation>,
}

/// 메모리 전용 권한 저장소(XIX.3.2). 앱 프로세스 수명 동안만 있습니다.
#[derive(Debug, Default)]
pub struct PermissionStore {
    sessions: BTreeMap<PermissionKey, SessionPermissions>,
    next_request: u64,
}

impl PermissionStore {
    /// 빈 저장소. 이전 실행의 권한을 읽는 경로는 없습니다.
    pub fn new() -> PermissionStore {
        PermissionStore::default()
    }

    /// 세션을 등록합니다. 처음 등록하면 첫 연결 승인 대기 상태입니다. 이미 있으면 상태를 바꾸지 않습니다.
    pub fn connect(&mut self, key: PermissionKey) -> ConnectionApproval {
        self.sessions
            .entry(key)
            .or_insert_with(|| SessionPermissions {
                connection: ConnectionApproval::Pending,
                confirmations: BTreeMap::new(),
            })
            .connection
    }

    /// 세션을 끝내고 권한을 모두 버립니다(§31.5 세션 종료).
    pub fn end_session(&mut self, key: &PermissionKey) -> bool {
        self.sessions.remove(key).is_some()
    }

    /// 첫 연결 승인 상태.
    pub fn connection(&self, key: &PermissionKey) -> Option<ConnectionApproval> {
        self.sessions.get(key).map(|s| s.connection)
    }

    /// 첫 연결 요청에 응답합니다(앱 사용자 UI 전용). 대기 중일 때의 첫 응답만 유효합니다. 앱이 시작한 내장
    /// 세션은 세션 시작 확인을 이 응답으로 기록합니다(XXIX.3.5).
    pub fn respond_connection(
        &mut self,
        key: &PermissionKey,
        approve: bool,
    ) -> Result<ConnectionApproval, PermissionError> {
        let session = self
            .sessions
            .get_mut(key)
            .ok_or(PermissionError::UnknownSession)?;
        if session.connection != ConnectionApproval::Pending {
            return Err(PermissionError::ConnectionAnswered);
        }
        session.connection = if approve {
            ConnectionApproval::Approved
        } else {
            ConnectionApproval::Denied
        };
        Ok(session.connection)
    }

    /// 권한 하나의 확인 상태.
    pub fn confirmation(&self, key: &PermissionKey, permission: Permission) -> Confirmation {
        self.sessions
            .get(key)
            .and_then(|s| s.confirmations.get(&permission).copied())
            .unwrap_or(Confirmation::NotRequested)
    }

    /// 권한 하나의 확인을 요청합니다(에이전트 tool 호출 처리에서 부를 수 있음). 이미 대기·승인 중이면 그 상태를, 아니면 새 대기
    /// 요청을 돌려줍니다. 거부된 권한은 다시 요청하면 새 대기 요청이 됩니다. 다른 권한의 상태는 바꾸지
    /// 않습니다(XIX.3.4).
    pub fn request(
        &mut self,
        key: &PermissionKey,
        permission: Permission,
    ) -> Result<Confirmation, PermissionError> {
        if permission == Permission::Read {
            return Err(PermissionError::ReadFollowsConnection);
        }
        let next = RequestId(self.next_request + 1);
        let session = self
            .sessions
            .get_mut(key)
            .ok_or(PermissionError::UnknownSession)?;
        let current = session
            .confirmations
            .get(&permission)
            .copied()
            .unwrap_or(Confirmation::NotRequested);
        match current {
            Confirmation::Pending(_) | Confirmation::Approved(_) => Ok(current),
            Confirmation::NotRequested | Confirmation::Denied(_) => {
                self.next_request = next.0;
                let state = Confirmation::Pending(next);
                session.confirmations.insert(permission, state);
                Ok(state)
            }
        }
    }

    /// 확인 요청에 응답합니다(앱 사용자 UI 전용). 첫 응답만 유효하고, 그 요청의 권한 하나만 바뀝니다.
    pub fn respond(
        &mut self,
        key: &PermissionKey,
        request: RequestId,
        approve: bool,
    ) -> Result<Confirmation, PermissionError> {
        let session = self
            .sessions
            .get_mut(key)
            .ok_or(PermissionError::UnknownSession)?;
        let state = session
            .confirmations
            .values_mut()
            .find(|state| request_of(**state) == Some(request))
            .ok_or(PermissionError::UnknownRequest(request))?;
        if !matches!(state, Confirmation::Pending(_)) {
            return Err(PermissionError::AlreadyAnswered(request));
        }
        *state = if approve {
            Confirmation::Approved(request)
        } else {
            Confirmation::Denied(request)
        };
        Ok(*state)
    }

    /// 권한 판정(XIX.3.3). 상태를 바꾸지 않습니다.
    pub fn decide(
        &self,
        policy: BackendPolicy,
        key: &PermissionKey,
        permission: Permission,
    ) -> Decision {
        match policy {
            BackendPolicy::Headless { trusted: false }
            | BackendPolicy::Attached { trusted: false } => {
                Decision::Denied(DenyReason::NotTrusted)
            }
            BackendPolicy::Headless { trusted: true } => {
                if permission.is_non_mutating() {
                    Decision::Allow
                } else {
                    Decision::Denied(DenyReason::NotOffered)
                }
            }
            BackendPolicy::Attached { trusted: true } => {
                let Some(session) = self.sessions.get(key) else {
                    return Decision::AwaitingConnection;
                };
                match session.connection {
                    ConnectionApproval::Pending => return Decision::AwaitingConnection,
                    ConnectionApproval::Denied => {
                        return Decision::Denied(DenyReason::ConnectionDenied)
                    }
                    ConnectionApproval::Approved => {}
                }
                if permission == Permission::Read {
                    return Decision::Allow;
                }
                match session.confirmations.get(&permission) {
                    Some(Confirmation::Approved(_)) => Decision::Allow,
                    Some(Confirmation::Denied(_)) => {
                        Decision::Denied(DenyReason::ConfirmationDenied)
                    }
                    Some(Confirmation::Pending(_) | Confirmation::NotRequested) | None => {
                        Decision::AwaitingConfirmation
                    }
                }
            }
        }
    }

    /// 판정하고, 허용이면 실행 한 번 권한의 승인을 씁니다. 실행 직전에 부릅니다.
    pub fn authorize(
        &mut self,
        policy: BackendPolicy,
        key: &PermissionKey,
        permission: Permission,
    ) -> Decision {
        let decision = self.decide(policy, key, permission);
        if decision == Decision::Allow
            && matches!(policy, BackendPolicy::Attached { .. })
            && permission != Permission::Read
            && !is_session_grant(permission)
        {
            if let Some(session) = self.sessions.get_mut(key) {
                session.confirmations.remove(&permission);
            }
        }
        decision
    }
}

fn request_of(state: Confirmation) -> Option<RequestId> {
    match state {
        Confirmation::NotRequested => None,
        Confirmation::Pending(id) | Confirmation::Approved(id) | Confirmation::Denied(id) => {
            Some(id)
        }
    }
}

#[cfg(test)]
#[path = "permission_tests.rs"]
mod tests;

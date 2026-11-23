//! 에이전트 편집 lease 요청·보유 관리 (XIX.4.2, XIX.4.5~XIX.4.7, ADR-0093, ADR-0112).
//!
//! 흐름(아키텍처 §31.4): 에이전트가 `document.begin_agent_edit`로 대상 root StableId 목록·전역 resource·작업
//! 설명을 제출합니다. tool 인자 해석은 MCP 브리지 쪽(`stellar_mcp::lease::parse_begin_agent_edit`)이고, 앱은 그
//! 대상 선언을 [`LeaseTargets`]로 받습니다 → [`AgentEditLeases::submit`]이 문서에서 대상을 확인하고 승인 대기로
//! 둡니다(아직 잠그지 않음) → 사용자가 앱 UI에서 승인하면 [`AgentEditLeases::approve`]가 그때의 문서로 대상을
//! 다시 확인하고 프로젝트 lease 조정자([`ProjectLockCoordinator`])에서 lease를 획득합니다. 제출과 승인 사이에
//! 사용자가 대상을 옮기거나 지울 수 있으므로, 겹침 판정에 쓰는 조상 목록은 획득 시점 문서로 만듭니다. 진행 중인
//! 사용자 조작(gesture)과 겹치면 대기([`LeaseState::WaitingForGesture`])로 남고, gesture가 끝난 뒤
//! [`AgentEditLeases::retry_waiting`]이 획득합니다(XIX.4.7). 다른 lease와 겹치면 거부합니다(XIX.4.6).
//!
//! 잠금 판정·겹침·파일 공유는 `stellar_commands::lock::{lease, coordinator}` 한 곳에 있고, 이 module은 요청
//! 상태와 문서 기준 대상 확인만 다룹니다. 앱이 소유하는 상태라 에이전트 경로 crate(mcp-core)는 이 module에
//! 닿지 않습니다(ADR-0112).

use std::collections::BTreeMap;
use std::fmt;

use stellar_ops::commands::document::model::DocumentModel;
use stellar_ops::commands::lock::coordinator::{
    AcquireError, CoordinatorError, HeldLease, ProjectLockCoordinator,
};
use stellar_ops::commands::lock::lease::{
    Conflict, ExternalResource, Lease, LeaseId, LeaseShapeError, LeaseTable, SubtreeLock,
};
use stellar_ops::core_types::ids::StableId;

use crate::agent_protocol::outcome::ResultKind;

/// 앱이 받은 `BeginAgentEdit` 대상 선언(브리지가 형식을 검사한 tool 인자).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseTargets {
    /// 대상 문서.
    pub document: StableId,
    /// 잠글 subtree root(제출 순서).
    pub targets: Vec<StableId>,
    /// 따로 잠글 전역 resource(XIX.4.4).
    pub externals: Vec<ExternalResource>,
}

/// 문서 기준 대상 확인 실패.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetError {
    /// 대상 문서가 이 창 문서가 아닙니다.
    ForeignDocument(StableId),
    /// 대상 root가 문서에 없습니다(`index`는 제출 순서).
    MissingNode {
        /// 제출 순서.
        index: usize,
        /// 없는 node.
        node: StableId,
    },
    /// 대상 모양 오류(빈 목록, 서로 겹치는 대상 등).
    Shape(LeaseShapeError),
}

impl TargetError {
    /// 고정 코드.
    pub fn code(&self) -> &'static str {
        match self {
            TargetError::ForeignDocument(_) => "not-found",
            TargetError::MissingNode { .. } => "missing-node",
            TargetError::Shape(_) => "invalid-arguments",
        }
    }

    /// MCP 결과 종류.
    pub fn kind(&self) -> ResultKind {
        match self {
            TargetError::ForeignDocument(_) | TargetError::MissingNode { .. } => {
                ResultKind::NotFound
            }
            TargetError::Shape(_) => ResultKind::InvalidArguments,
        }
    }
}

impl fmt::Display for TargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TargetError::ForeignDocument(id) => write!(f, "문서 {id}가 열려 있지 않습니다"),
            TargetError::MissingNode { index, node } => {
                write!(f, "targets[{index}] {node}가 문서에 없습니다")
            }
            TargetError::Shape(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TargetError {}

impl LeaseTargets {
    /// 대상 문서에서 lease를 만듭니다. 문서가 다르거나 없는 node가 있으면 거부합니다(XIX.4.1). 한 요청 안의
    /// 대상이 서로 겹치면(조상·자손) 거부합니다.
    pub fn resolve(&self, model: &DocumentModel, id: LeaseId) -> Result<Lease, TargetError> {
        if model.document_id() != self.document {
            return Err(TargetError::ForeignDocument(self.document));
        }
        let mut subtrees = Vec::with_capacity(self.targets.len());
        for (index, target) in self.targets.iter().enumerate() {
            let lock = SubtreeLock::capture(model, *target).ok_or(TargetError::MissingNode {
                index,
                node: *target,
            })?;
            subtrees.push(lock);
        }
        let lease = Lease {
            id,
            subtrees,
            externals: self.externals.clone(),
        };
        lease.check_shape().map_err(TargetError::Shape)?;
        Ok(lease)
    }
}

/// 프로젝트 identity(project id + canonical target)의 lease 조정자를 엽니다. `lock_dir`은 저장 잠금 디렉터리
/// (제품 경로는 `stellar_ops::lock::default_lock_dir`)이고, key는 저장 잠금과 같은 identity key라 다른 worktree는
/// 다른 조정자입니다(XIX.4.5).
pub fn open_project_coordinator(
    lock_dir: &std::path::Path,
    project_id: StableId,
    root: &std::path::Path,
) -> Result<ProjectLockCoordinator, LeaseServiceError> {
    let key =
        stellar_ops::lock::lock_key(project_id, root).map_err(|_| LeaseServiceError::Identity)?;
    ProjectLockCoordinator::open(
        &lock_dir.join(stellar_ops::commands::lock::coordinator::LEASE_DIR),
        &key,
    )
    .map_err(LeaseServiceError::Coordinator)
}

/// 요청 하나의 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseState {
    /// 사용자 승인 대기. 아직 아무것도 잠그지 않았습니다.
    AwaitingApproval,
    /// 승인했지만 진행 중 사용자 조작과 겹쳐 기다립니다(XIX.4.7).
    WaitingForGesture(Conflict),
    /// lease를 쥐었습니다.
    Granted,
}

/// lease 요청·보유 관리 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseServiceError {
    /// 모르는 lease 요청입니다.
    Unknown(LeaseId),
    /// 같은 id의 요청이 이미 있습니다.
    Duplicate(LeaseId),
    /// 대상 오류(문서가 다름, 없는 node, 대상 모양).
    Target(TargetError),
    /// 프로젝트 identity를 확인할 수 없습니다.
    Identity,
    /// 다른 lease와 겹쳐 거부했습니다(요청은 버림, XIX.4.6).
    Conflict(Conflict),
    /// 조정자 오류.
    Coordinator(CoordinatorError),
}

impl LeaseServiceError {
    /// MCP 결과 종류.
    pub fn kind(&self) -> ResultKind {
        match self {
            LeaseServiceError::Unknown(_) => ResultKind::NotFound,
            LeaseServiceError::Duplicate(_) => ResultKind::InvalidArguments,
            LeaseServiceError::Target(e) => e.kind(),
            LeaseServiceError::Identity => ResultKind::Unavailable,
            LeaseServiceError::Conflict(_) => ResultKind::Rejected,
            LeaseServiceError::Coordinator(_) => ResultKind::Unavailable,
        }
    }
}

impl fmt::Display for LeaseServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LeaseServiceError::Unknown(id) => write!(f, "lease 요청 {id}가 없습니다"),
            LeaseServiceError::Duplicate(id) => write!(f, "lease 요청 {id}가 이미 있습니다"),
            LeaseServiceError::Target(e) => write!(f, "{e}"),
            LeaseServiceError::Identity => f.write_str("프로젝트 identity를 확인할 수 없습니다"),
            LeaseServiceError::Conflict(c) => write!(f, "{c}"),
            LeaseServiceError::Coordinator(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LeaseServiceError {}

#[derive(Debug)]
enum Slot {
    Pending {
        request: LeaseTargets,
        waiting: Option<Conflict>,
    },
    Held(HeldLease),
}

/// 한 프로젝트(창 묶음)의 에이전트 lease 요청과 보유 lease.
#[derive(Debug)]
pub struct AgentEditLeases {
    coordinator: ProjectLockCoordinator,
    slots: BTreeMap<LeaseId, Slot>,
}

impl AgentEditLeases {
    /// 프로젝트 lease 조정자 위에서 만듭니다.
    pub fn new(coordinator: ProjectLockCoordinator) -> AgentEditLeases {
        AgentEditLeases {
            coordinator,
            slots: BTreeMap::new(),
        }
    }

    /// `BeginAgentEdit` 대상 선언을 승인 대기로 둡니다. `id`는 에이전트 편집 세션 id입니다.
    pub fn submit(
        &mut self,
        model: &DocumentModel,
        id: LeaseId,
        request: &LeaseTargets,
    ) -> Result<LeaseState, LeaseServiceError> {
        if self.slots.contains_key(&id) {
            return Err(LeaseServiceError::Duplicate(id));
        }
        request
            .resolve(model, id)
            .map_err(LeaseServiceError::Target)?;
        self.slots.insert(
            id,
            Slot::Pending {
                request: request.clone(),
                waiting: None,
            },
        );
        Ok(LeaseState::AwaitingApproval)
    }

    /// 요청 상태.
    pub fn state(&self, id: LeaseId) -> Option<LeaseState> {
        self.slots.get(&id).map(|slot| match slot {
            Slot::Pending { waiting: None, .. } => LeaseState::AwaitingApproval,
            Slot::Pending {
                waiting: Some(conflict),
                ..
            } => LeaseState::WaitingForGesture(*conflict),
            Slot::Held(_) => LeaseState::Granted,
        })
    }

    /// 사용자가 승인한 요청의 lease를 획득합니다(앱 사용자 UI 전용). `model`은 지금 문서이고, 대상을 이 문서로
    /// 다시 확인해 잠금을 만듭니다. 대상이 사라졌으면 요청을 버리고 거부합니다. gesture와 겹치면 대기로 남고,
    /// 다른 lease와 겹치면 요청을 버리고 거부합니다.
    pub fn approve(
        &mut self,
        model: &DocumentModel,
        id: LeaseId,
    ) -> Result<LeaseState, LeaseServiceError> {
        let Some(Slot::Pending { request, .. }) = self.slots.get(&id) else {
            return match self.slots.get(&id) {
                Some(Slot::Held(_)) => Ok(LeaseState::Granted),
                _ => Err(LeaseServiceError::Unknown(id)),
            };
        };
        let lease = match request.resolve(model, id) {
            Ok(lease) => lease,
            Err(error) => {
                self.slots.remove(&id);
                return Err(LeaseServiceError::Target(error));
            }
        };
        match self.coordinator.try_acquire(lease) {
            Ok(held) => {
                self.slots.insert(id, Slot::Held(held));
                Ok(LeaseState::Granted)
            }
            Err(AcquireError::Conflict(conflict @ Conflict::Gesture { .. })) => {
                if let Some(Slot::Pending { waiting, .. }) = self.slots.get_mut(&id) {
                    *waiting = Some(conflict);
                }
                Ok(LeaseState::WaitingForGesture(conflict))
            }
            Err(error) => {
                self.slots.remove(&id);
                Err(match error {
                    AcquireError::Conflict(conflict) => LeaseServiceError::Conflict(conflict),
                    AcquireError::Shape(shape) => {
                        LeaseServiceError::Target(TargetError::Shape(shape))
                    }
                    AcquireError::DuplicateId(_) => LeaseServiceError::Duplicate(id),
                    AcquireError::Coordinator(e) => LeaseServiceError::Coordinator(e),
                })
            }
        }
    }

    /// gesture 때문에 기다리는 요청을 지금 문서(`model`)로 다시 시도합니다. gesture가 끝난 뒤 부릅니다
    /// (XIX.4.7). 결과는 다시 시도한 요청별 상태 또는 거부입니다.
    pub fn retry_waiting(
        &mut self,
        model: &DocumentModel,
    ) -> Vec<(LeaseId, Result<LeaseState, LeaseServiceError>)> {
        let waiting: Vec<LeaseId> = self
            .slots
            .iter()
            .filter(|(_, slot)| {
                matches!(
                    slot,
                    Slot::Pending {
                        waiting: Some(_),
                        ..
                    }
                )
            })
            .map(|(id, _)| *id)
            .collect();
        waiting
            .into_iter()
            .map(|id| (id, self.approve(model, id)))
            .collect()
    }

    /// 요청을 거부하거나 lease를 풉니다(사용자 거부, 정상 완료, 세션 종료).
    pub fn release(&mut self, id: LeaseId) -> Result<(), LeaseServiceError> {
        match self.slots.remove(&id) {
            None => Err(LeaseServiceError::Unknown(id)),
            Some(Slot::Pending { .. }) => Ok(()),
            Some(Slot::Held(held)) => held.release().map_err(LeaseServiceError::Coordinator),
        }
    }

    /// 이 프로젝트의 모든 창·프로세스의 활성 lease(판정 입력).
    pub fn table(&self) -> Result<LeaseTable, LeaseServiceError> {
        self.coordinator
            .leases()
            .map_err(LeaseServiceError::Coordinator)
    }

    /// 조정자.
    pub fn coordinator(&self) -> &ProjectLockCoordinator {
        &self.coordinator
    }
}

#[cfg(test)]
#[path = "lease_tests.rs"]
mod tests;

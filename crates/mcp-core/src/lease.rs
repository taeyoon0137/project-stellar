//! 에이전트 편집 대상 선언과 lease 요청 (XIX.4.1, XIX.4.2, XIX.4.4~XIX.4.7, ADR-0093).
//!
//! 흐름(아키텍처 §31.4): 에이전트가 `document.begin_agent_edit`로 대상 root StableId 목록·전역 resource·작업
//! 설명을 제출합니다 → [`AgentEditLeases::submit`]이 문서에서 대상을 확인하고 승인 대기로 둡니다(아직 잠그지
//! 않음) → 사용자가 앱 UI에서 승인하면 [`AgentEditLeases::approve`]가 프로젝트 lease 조정자
//! ([`ProjectLockCoordinator`])에서 lease를 획득합니다. 진행 중인 사용자 조작(gesture)과 겹치면 대기
//! ([`LeaseState::WaitingForGesture`])로 남고, gesture가 끝난 뒤 [`AgentEditLeases::retry_waiting`]이
//! 획득합니다(XIX.4.7). 다른 lease와 겹치면 거부합니다(XIX.4.6).
//!
//! 잠금 판정·겹침·파일 공유는 `stellar_commands::lock::{lease, coordinator}` 한 곳에 있고, 이 module은 tool
//! 인자 해석과 요청 상태만 다룹니다.

use std::collections::BTreeMap;
use std::fmt;

use stellar_ops::commands::document::model::DocumentModel;
use stellar_ops::commands::lock::coordinator::{
    AcquireError, CoordinatorError, HeldLease, ProjectLockCoordinator,
};
use stellar_ops::commands::lock::lease::{
    Conflict, ExternalKind, ExternalResource, Lease, LeaseId, LeaseShapeError, LeaseTable,
    SubtreeLock,
};
use stellar_ops::core_types::ids::StableId;

use crate::agent_protocol::catalog::resources::Availability;
use crate::agent_protocol::catalog::tools::ToolDef;
use crate::agent_protocol::outcome::ResultKind;
use crate::agent_protocol::permission::Permission;
use crate::json::JsonValue;

/// 대상 root·전역 resource 수 상한(tool schema `maxItems`와 같음).
pub const MAX_TARGETS: usize = 1000;
/// 작업 설명 최대 길이(문자 수, schema `maxLength`와 같음).
pub const MAX_DESCRIPTION_CHARS: usize = 2000;

/// `document.begin_agent_edit` tool 정의(attached 전용). 대상 선언과 잠금 요청이며 문서를 바꾸지 않으므로
/// Propose 권한입니다. 승인은 Stellar 사용자 UI가 받습니다.
pub const BEGIN_AGENT_EDIT: ToolDef = ToolDef {
    name: "document.begin_agent_edit",
    title: "Begin agent edit",
    description: "편집할 target root node, 따로 잠글 전역 resource, 작업 설명을 제출해 사용자 승인 뒤 편집 lease를 요청합니다",
    input_schema: r#"{"type":"object","additionalProperties":false,"required":["document","targets","description"],"properties":{"document":{"type":"string"},"targets":{"type":"array","minItems":1,"maxItems":1000,"items":{"type":"string"}},"externals":{"type":"array","maxItems":1000,"items":{"type":"object","additionalProperties":false,"required":["kind","id"],"properties":{"kind":{"type":"string","enum":["token","component-definition","asset"]},"id":{"type":"string"}}}},"description":{"type":"string","minLength":1,"maxLength":2000}}}"#,
    read_only: false,
    destructive: false,
    idempotent: false,
    availability: Availability::AttachedOnly,
    grant: Permission::Propose,
};

/// 해석한 `BeginAgentEdit` 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginAgentEdit {
    /// 대상 문서.
    pub document: StableId,
    /// 잠글 subtree root(제출 순서).
    pub targets: Vec<StableId>,
    /// 따로 잠글 전역 resource(XIX.4.4).
    pub externals: Vec<ExternalResource>,
    /// 사용자에게 보일 작업 설명(데이터로만 다룸).
    pub description: String,
}

/// lease 요청 오류. 문구에는 요청 위치와 StableId만 들어갑니다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseRequestError {
    /// 고정 코드.
    pub code: &'static str,
    /// MCP 결과 종류.
    pub kind: ResultKind,
    /// 원인.
    pub message: String,
}

impl LeaseRequestError {
    fn new(code: &'static str, kind: ResultKind, message: impl Into<String>) -> Self {
        LeaseRequestError {
            code,
            kind,
            message: message.into(),
        }
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid-arguments", ResultKind::InvalidArguments, message)
    }
}

impl fmt::Display for LeaseRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LeaseRequestError {}

fn stable_id(value: Option<&JsonValue>, at: &str) -> Result<StableId, LeaseRequestError> {
    value
        .and_then(JsonValue::as_str)
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| LeaseRequestError::invalid(format!("{at}는 StableId여야 합니다")))
}

/// tool 인자를 해석합니다(XIX.4.1). 빈 대상, 중복 대상, 빈 설명은 거부합니다. 선언되지 않은 key는 tool 인자
/// 범위 검사(`catalog::scope`)가 먼저 거부합니다.
pub fn parse_begin_agent_edit(args: &JsonValue) -> Result<BeginAgentEdit, LeaseRequestError> {
    let document = stable_id(args.get("document"), "document")?;
    let items = args
        .get("targets")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| LeaseRequestError::invalid("targets는 배열이어야 합니다"))?;
    if items.is_empty() {
        return Err(LeaseRequestError::new(
            "empty-targets",
            ResultKind::InvalidArguments,
            "편집할 target root가 없습니다",
        ));
    }
    if items.len() > MAX_TARGETS {
        return Err(LeaseRequestError::invalid(format!(
            "target {}개는 상한 {MAX_TARGETS}개를 넘습니다",
            items.len()
        )));
    }
    let mut targets = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let id = stable_id(Some(item), &format!("targets[{index}]"))?;
        if targets.contains(&id) {
            return Err(LeaseRequestError::invalid(format!(
                "targets[{index}] {id}가 중복입니다"
            )));
        }
        targets.push(id);
    }
    let mut externals = Vec::new();
    if let Some(value) = args.get("externals") {
        let items = value
            .as_array()
            .ok_or_else(|| LeaseRequestError::invalid("externals는 배열이어야 합니다"))?;
        if items.len() > MAX_TARGETS {
            return Err(LeaseRequestError::invalid("externals 수가 상한을 넘습니다"));
        }
        for (index, item) in items.iter().enumerate() {
            let at = format!("externals[{index}]");
            let keys_ok = item
                .as_object()
                .is_some_and(|pairs| pairs.iter().all(|(k, _)| k == "kind" || k == "id"));
            if !keys_ok {
                return Err(LeaseRequestError::invalid(format!(
                    "{at}는 kind·id만 가진 object여야 합니다"
                )));
            }
            let kind = item
                .get("kind")
                .and_then(JsonValue::as_str)
                .and_then(ExternalKind::from_name)
                .ok_or_else(|| {
                    LeaseRequestError::invalid(format!("{at}.kind가 올바르지 않습니다"))
                })?;
            let id = stable_id(item.get("id"), &format!("{at}.id"))?;
            let resource = ExternalResource { kind, id };
            if externals.contains(&resource) {
                return Err(LeaseRequestError::invalid(format!("{at}가 중복입니다")));
            }
            externals.push(resource);
        }
    }
    let description = args
        .get("description")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| LeaseRequestError::invalid("description 문자열이 필요합니다"))?;
    if description.trim().is_empty() {
        return Err(LeaseRequestError::invalid("작업 설명이 비었습니다"));
    }
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(LeaseRequestError::invalid(format!(
            "작업 설명은 {MAX_DESCRIPTION_CHARS}자를 넘을 수 없습니다"
        )));
    }
    Ok(BeginAgentEdit {
        document,
        targets,
        externals,
        description: description.to_owned(),
    })
}

impl BeginAgentEdit {
    /// 대상 문서에서 lease를 만듭니다. 문서가 다르거나 없는 node가 있으면 거부합니다(XIX.4.1). 한 요청 안의
    /// 대상이 서로 겹치면(조상·자손) 거부합니다.
    pub fn resolve(&self, model: &DocumentModel, id: LeaseId) -> Result<Lease, LeaseRequestError> {
        if model.document_id() != self.document {
            return Err(LeaseRequestError::new(
                "not-found",
                ResultKind::NotFound,
                format!("문서 {}가 열려 있지 않습니다", self.document),
            ));
        }
        let mut subtrees = Vec::with_capacity(self.targets.len());
        for (index, target) in self.targets.iter().enumerate() {
            let lock = SubtreeLock::capture(model, *target).ok_or_else(|| {
                LeaseRequestError::new(
                    "missing-node",
                    ResultKind::NotFound,
                    format!("targets[{index}] {target}가 문서에 없습니다"),
                )
            })?;
            subtrees.push(lock);
        }
        let lease = Lease {
            id,
            subtrees,
            externals: self.externals.clone(),
        };
        lease.check_shape().map_err(shape_error)?;
        Ok(lease)
    }
}

fn shape_error(error: LeaseShapeError) -> LeaseRequestError {
    LeaseRequestError::invalid(error.to_string())
}

/// 프로젝트 identity(project id + canonical target)의 lease 조정자를 엽니다. `lock_dir`은 저장 잠금 디렉터리
/// (제품 경로는 `stellar_ops::lock::default_lock_dir`)이고, key는 저장 잠금과 같은 identity key라 다른 worktree는
/// 다른 조정자입니다(XIX.4.5).
pub fn open_project_coordinator(
    lock_dir: &std::path::Path,
    project_id: StableId,
    root: &std::path::Path,
) -> Result<ProjectLockCoordinator, LeaseServiceError> {
    let key = stellar_ops::lock::lock_key(project_id, root).map_err(|_| {
        LeaseServiceError::Request(LeaseRequestError::new(
            "unavailable",
            ResultKind::Unavailable,
            "프로젝트 identity를 확인할 수 없습니다",
        ))
    })?;
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
    /// 요청 인자·대상 오류.
    Request(LeaseRequestError),
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
            LeaseServiceError::Request(e) => e.kind,
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
            LeaseServiceError::Request(e) => write!(f, "{e}"),
            LeaseServiceError::Conflict(c) => write!(f, "{c}"),
            LeaseServiceError::Coordinator(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LeaseServiceError {}

#[derive(Debug)]
enum Slot {
    Pending {
        lease: Lease,
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

    /// `BeginAgentEdit` 요청을 승인 대기로 둡니다. `id`는 에이전트 편집 세션 id입니다.
    pub fn submit(
        &mut self,
        model: &DocumentModel,
        id: LeaseId,
        request: &BeginAgentEdit,
    ) -> Result<LeaseState, LeaseServiceError> {
        if self.slots.contains_key(&id) {
            return Err(LeaseServiceError::Duplicate(id));
        }
        let lease = request
            .resolve(model, id)
            .map_err(LeaseServiceError::Request)?;
        self.slots.insert(
            id,
            Slot::Pending {
                lease,
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

    /// 사용자가 승인한 요청의 lease를 획득합니다(앱 사용자 UI 전용). gesture와 겹치면 대기로 남고, 다른 lease와
    /// 겹치면 요청을 버리고 거부합니다.
    pub fn approve(&mut self, id: LeaseId) -> Result<LeaseState, LeaseServiceError> {
        let Some(Slot::Pending { lease, .. }) = self.slots.get(&id) else {
            return match self.slots.get(&id) {
                Some(Slot::Held(_)) => Ok(LeaseState::Granted),
                _ => Err(LeaseServiceError::Unknown(id)),
            };
        };
        match self.coordinator.try_acquire(lease.clone()) {
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
                    AcquireError::Shape(shape) => LeaseServiceError::Request(shape_error(shape)),
                    AcquireError::DuplicateId(_) => LeaseServiceError::Duplicate(id),
                    AcquireError::Coordinator(e) => LeaseServiceError::Coordinator(e),
                })
            }
        }
    }

    /// gesture 때문에 기다리는 요청을 다시 시도합니다. gesture가 끝난 뒤 부릅니다(XIX.4.7). 결과는 다시 시도한
    /// 요청별 상태 또는 거부입니다.
    pub fn retry_waiting(&mut self) -> Vec<(LeaseId, Result<LeaseState, LeaseServiceError>)> {
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
            .map(|id| (id, self.approve(id)))
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
pub(crate) mod tests;

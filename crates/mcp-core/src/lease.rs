//! 에이전트 편집 대상 선언과 lease 요청 (XIX.4.1, XIX.4.2, XIX.4.4~XIX.4.7, ADR-0093).
//!
//! 흐름(아키텍처 §31.4): 에이전트가 `document.begin_agent_edit`로 대상 root StableId 목록·전역 resource·작업
//! 설명을 제출합니다 → [`AgentEditLeases::submit`]이 문서에서 대상을 확인하고 승인 대기로 둡니다(아직 잠그지
//! 않음) → 사용자가 앱 UI에서 승인하면 [`AgentEditLeases::approve`]가 그때의 문서로 대상을 다시 확인하고
//! 프로젝트 lease 조정자([`ProjectLockCoordinator`])에서 lease를 획득합니다. 제출과 승인 사이에 사용자가 대상을
//! 옮기거나 지울 수 있으므로, 겹침 판정에 쓰는 조상 목록은 획득 시점 문서로 만듭니다. 진행 중인 사용자 조작(gesture)과 겹치면 대기
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
        request: BeginAgentEdit,
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
        request
            .resolve(model, id)
            .map_err(LeaseServiceError::Request)?;
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
                return Err(LeaseServiceError::Request(error));
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
                    AcquireError::Shape(shape) => LeaseServiceError::Request(shape_error(shape)),
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
mod tests {
    //! BeginAgentEdit·lease 요청 test (XIX.4.1, XIX.4.2, XIX.4.5~XIX.4.7).

    use super::*;
    use crate::json::{parse, JsonLimits};
    use stellar_ops::commands::document::definitions::{
        DocumentDefinition, PageDefinition, ViewDefinition,
    };
    use stellar_ops::commands::document::frame::FramePayload;
    use stellar_ops::commands::document::records::{NodePayload, NodeRecord};
    use stellar_ops::commands::lock::coordinator::Gesture;
    use stellar_ops::core_types::order_key::OrderKey;
    use stellar_ops::test_support::TempDir;

    pub(crate) fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    /// 문서 sid(1), View root sid(10). 10 ─ 11 ─ 13 ─ 14, 10 ─ 12.
    pub(crate) fn model() -> DocumentModel {
        model_with(&[
            (10, None, 0x10u8),
            (11, Some(10), 0x10),
            (12, Some(10), 0x20),
            (13, Some(11), 0x10),
            (14, Some(13), 0x10),
        ])
    }

    /// 문서 sid(1), View root sid(10)에 (id, 부모, 순서 byte) node를 차례로 넣습니다.
    fn model_with(nodes: &[(u64, Option<u64>, u8)]) -> DocumentModel {
        let mut definition = DocumentDefinition::new(sid(1), String::new());
        let mut page = PageDefinition::new(sid(2), String::new());
        page.add_view(ViewDefinition::new(sid(3), String::new(), sid(10)))
            .unwrap();
        definition.add_page(page).unwrap();
        let mut model = DocumentModel::new(definition);
        for &(id, parent, key) in nodes {
            model
                .insert_node(NodeRecord {
                    id: sid(id),
                    parent: parent.map(sid),
                    order_key: OrderKey::from_bytes(vec![key]).unwrap(),
                    visible: true,
                    locked: false,
                    display_name: format!("n{id}"),
                    payload: NodePayload::Frame(FramePayload::default()),
                })
                .unwrap();
        }
        model
    }

    fn args(text: &str) -> JsonValue {
        parse(text, JsonLimits::default()).unwrap()
    }

    fn begin(targets: &[u64]) -> BeginAgentEdit {
        BeginAgentEdit {
            document: sid(1),
            targets: targets.iter().map(|t| sid(*t)).collect(),
            externals: Vec::new(),
            description: "버튼 정리".into(),
        }
    }

    fn service(temp: &TempDir) -> AgentEditLeases {
        AgentEditLeases::new(open_project_coordinator(&temp.path, sid(500), &temp.path).unwrap())
    }

    #[test]
    fn tool_definition_is_attached_only_and_well_formed() {
        assert!(crate::agent_protocol::catalog::tools::valid_tool_name(
            BEGIN_AGENT_EDIT.name
        ));
        assert_eq!(BEGIN_AGENT_EDIT.availability, Availability::AttachedOnly);
        assert_eq!(BEGIN_AGENT_EDIT.grant, Permission::Propose);
        let schema = args(BEGIN_AGENT_EDIT.input_schema);
        assert_eq!(
            schema
                .get("additionalProperties")
                .and_then(JsonValue::as_bool),
            Some(false)
        );
    }

    #[test]
    fn begin_agent_edit_rejects_empty_and_missing_targets() {
        // 검증 기준(XIX.4.1): empty/missing node 거부.
        let doc = sid(1);
        let empty = format!(r#"{{"document":"{doc}","targets":[],"description":"x"}}"#);
        assert_eq!(
            parse_begin_agent_edit(&args(&empty)).unwrap_err().code,
            "empty-targets"
        );
        let missing = format!(
            r#"{{"document":"{doc}","targets":["{}","{}"],"description":"작업"}}"#,
            sid(13),
            sid(99)
        );
        let request = parse_begin_agent_edit(&args(&missing)).unwrap();
        let error = request.resolve(&model(), LeaseId(sid(900))).unwrap_err();
        assert_eq!(
            (error.code, error.kind),
            ("missing-node", ResultKind::NotFound)
        );
        for bad in [
            format!(r#"{{"document":"{doc}","description":"x"}}"#),
            format!(r#"{{"document":"{doc}","targets":["nope"],"description":"x"}}"#),
            format!(
                r#"{{"document":"{doc}","targets":["{0}","{0}"],"description":"x"}}"#,
                sid(13)
            ),
            format!(
                r#"{{"document":"{doc}","targets":["{}"],"description":"  "}}"#,
                sid(13)
            ),
            format!(
                r#"{{"document":"{doc}","targets":["{}"],"description":"x","externals":[{{"kind":"script","id":"{}"}}]}}"#,
                sid(13),
                sid(5)
            ),
            format!(
                r#"{{"document":"{doc}","targets":["{}"],"description":"x","externals":[{{"kind":"token","id":"{}","path":"/"}}]}}"#,
                sid(13),
                sid(5)
            ),
        ] {
            assert_eq!(
                parse_begin_agent_edit(&args(&bad)).unwrap_err().kind,
                ResultKind::InvalidArguments,
                "{bad}"
            );
        }
        // 다른 문서, 서로 겹치는 대상.
        let mut other = begin(&[13]);
        other.document = sid(77);
        assert_eq!(
            other.resolve(&model(), LeaseId(sid(900))).unwrap_err().kind,
            ResultKind::NotFound
        );
        assert_eq!(
            begin(&[11, 14])
                .resolve(&model(), LeaseId(sid(900)))
                .unwrap_err()
                .kind,
            ResultKind::InvalidArguments
        );
        // 정상 요청: 대상 root와 전역 resource.
        let ok = format!(
            r#"{{"document":"{doc}","targets":["{}"],"description":"작업","externals":[{{"kind":"component-definition","id":"{}"}}]}}"#,
            sid(13),
            sid(600)
        );
        let lease = parse_begin_agent_edit(&args(&ok))
            .unwrap()
            .resolve(&model(), LeaseId(sid(900)))
            .unwrap();
        assert_eq!(lease.subtrees[0].root, sid(13));
        assert_eq!(lease.subtrees[0].ancestors, [sid(11), sid(10)]);
        assert_eq!(
            lease.externals,
            [ExternalResource {
                kind: ExternalKind::ComponentDefinition,
                id: sid(600)
            }]
        );
    }

    #[test]
    fn lease_is_created_only_after_approval() {
        // XIX.4.2: 승인 후에 AgentEditLease를 만듭니다. 승인 전에는 아무것도 잠그지 않습니다.
        let temp = TempDir::new("lease-approval");
        let mut leases = service(&temp);
        let model = model();
        let id = LeaseId(sid(900));
        assert_eq!(
            leases.submit(&model, id, &begin(&[13])),
            Ok(LeaseState::AwaitingApproval)
        );
        assert!(leases.table().unwrap().is_empty());
        assert_eq!(leases.approve(&model, id), Ok(LeaseState::Granted));
        let table = leases.table().unwrap();
        assert_eq!(table.holder(&model, sid(14)), Some(id));
        assert_eq!(table.holder(&model, sid(12)), None);
        // 겹치는 다른 세션 요청은 거부되고, 겹치지 않는 요청은 함께 잠급니다(XIX.4.6).
        let other = LeaseId(sid(901));
        leases.submit(&model, other, &begin(&[11])).unwrap();
        assert!(matches!(
            leases.approve(&model, other),
            Err(LeaseServiceError::Conflict(Conflict::Subtree { holder, .. })) if holder == id
        ));
        assert_eq!(leases.state(other), None, "거부된 요청은 버립니다");
        leases.submit(&model, other, &begin(&[12])).unwrap();
        assert_eq!(leases.approve(&model, other), Ok(LeaseState::Granted));
        leases.release(id).unwrap();
        assert_eq!(leases.table().unwrap().len(), 1);
        assert_eq!(leases.release(id), Err(LeaseServiceError::Unknown(id)));
    }

    #[test]
    fn approval_resolves_targets_against_the_current_document() {
        // 제출과 승인 사이에 대상이 옮겨지면 승인 시점 조상으로 잠그고, 사라졌으면 요청을 버립니다.
        let temp = TempDir::new("lease-approve-current");
        let mut leases = service(&temp);
        let id = LeaseId(sid(900));
        leases.submit(&model(), id, &begin(&[13])).unwrap();
        // 13을 12 아래로 옮긴 문서.
        let moved = model_with(&[
            (10, None, 0x10),
            (11, Some(10), 0x10),
            (12, Some(10), 0x20),
            (13, Some(12), 0x10),
            (14, Some(13), 0x10),
        ]);
        assert_eq!(leases.approve(&moved, id), Ok(LeaseState::Granted));
        let table = leases.table().unwrap();
        assert_eq!(
            table.get(id).unwrap().subtrees[0].ancestors,
            [sid(12), sid(10)]
        );
        // 겹침 판정도 새 조상으로 합니다: 12를 요청하면 겹치고, 11은 겹치지 않습니다.
        let other = LeaseId(sid(901));
        leases.submit(&moved, other, &begin(&[12])).unwrap();
        assert!(matches!(
            leases.approve(&moved, other),
            Err(LeaseServiceError::Conflict(Conflict::Subtree { .. }))
        ));
        leases.submit(&moved, other, &begin(&[11])).unwrap();
        assert_eq!(leases.approve(&moved, other), Ok(LeaseState::Granted));
        // 대상이 사라진 문서에서 승인하면 missing-node로 거부하고 요청을 버립니다.
        let third = LeaseId(sid(902));
        let without_14 = model_with(&[(10, None, 0x10), (11, Some(10), 0x10)]);
        let mut fresh = AgentEditLeases::new(
            open_project_coordinator(&temp.path.join("other"), sid(500), &temp.path).unwrap(),
        );
        fresh.submit(&model(), third, &begin(&[14])).unwrap();
        match fresh.approve(&without_14, third) {
            Err(LeaseServiceError::Request(error)) => assert_eq!(error.code, "missing-node"),
            other => panic!("예상하지 않은 결과: {other:?}"),
        }
        assert_eq!(fresh.state(third), None);
        assert!(fresh.table().unwrap().is_empty());
    }

    #[test]
    fn approved_request_waits_for_gesture_end() {
        // 검증 기준(XIX.4.7): drag 중 lock 대기 fixture.
        let temp = TempDir::new("lease-gesture");
        let mut leases = service(&temp);
        let model = model();
        let drag = leases
            .coordinator()
            .begin_gesture(Gesture {
                id: sid(700),
                nodes: vec![SubtreeLock::capture(&model, sid(14)).unwrap()],
            })
            .unwrap();
        let id = LeaseId(sid(900));
        leases.submit(&model, id, &begin(&[13])).unwrap();
        assert!(matches!(
            leases.approve(&model, id),
            Ok(LeaseState::WaitingForGesture(Conflict::Gesture { .. }))
        ));
        assert!(matches!(
            leases.state(id),
            Some(LeaseState::WaitingForGesture(_))
        ));
        assert!(
            leases.table().unwrap().is_empty(),
            "drag 중에는 잠그지 않습니다"
        );
        let retried = leases.retry_waiting(&model);
        assert!(matches!(
            retried.as_slice(),
            [(_, Ok(LeaseState::WaitingForGesture(_)))]
        ));
        drag.end().unwrap();
        assert_eq!(
            leases.retry_waiting(&model),
            vec![(id, Ok(LeaseState::Granted))]
        );
        assert_eq!(leases.state(id), Some(LeaseState::Granted));
    }

    #[test]
    fn worktrees_are_separate_project_identities() {
        // XIX.4.5: 다른 worktree는 별도 project identity라 lease를 공유하지 않습니다. 같은 프로젝트를 다른 경로
        // (symlink)로 열면 같은 identity입니다.
        let temp = TempDir::new("lease-worktree");
        let main = temp.path.join("main");
        let worktree = temp.path.join("worktree");
        std::fs::create_dir_all(&main).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        let link = temp.path.join("main-link");
        std::os::unix::fs::symlink(&main, &link).unwrap();
        let locks = temp.path.join("locks");
        let model = model();
        let mut first =
            AgentEditLeases::new(open_project_coordinator(&locks, sid(500), &main).unwrap());
        let same = AgentEditLeases::new(open_project_coordinator(&locks, sid(500), &link).unwrap());
        let other =
            AgentEditLeases::new(open_project_coordinator(&locks, sid(500), &worktree).unwrap());
        let id = LeaseId(sid(900));
        first.submit(&model, id, &begin(&[13])).unwrap();
        first.approve(&model, id).unwrap();
        assert_eq!(same.table().unwrap().len(), 1);
        assert!(other.table().unwrap().is_empty());
    }
}

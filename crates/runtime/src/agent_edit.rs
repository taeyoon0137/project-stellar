//! 창 하나의 에이전트 편집 상태를 창 history와 잇는 runtime 연결 (XXIX.3.1 엔진 부분, ADR-0112, ADR-0100).
//!
//! 앱이 소유하는 에이전트 편집 구현(`stellar-agent-edit`)의 상태를 창마다 하나씩 둡니다([`WindowAgentEdit`]).
//! document thread만 이 상태를 바꾸고, 창을 닫으면 함께 버립니다. 직렬화 경로가 없어 프로젝트 파일·Git·복구
//! snapshot에 들어가지 않습니다.
//!
//! - **창 history 연결 지점**: 세션 완료·종료와 되돌리기는 이 창의 `WindowHistory`·문서·이름표로 만든
//!   `stellar_history::agent::WindowAgentHistory`를 연결 지점으로 씁니다([`AgentWindow`]).
//! - **Undo·Redo**: 창 Undo·Redo는 [`replay`]를 거칩니다. 재생한 항목의 sequence로 출처 표시 상태를 전이하고
//!   (`on_history`), 재생 change set을 그 항목 출처와 함께 `observe`에 넘깁니다.
//! - **되돌리기와 자동 저장**: 되돌리기 적용은 일반 편집처럼 작업 문서 revision을 올립니다. 앱 runtime의 job
//!   경로([`crate::app::AppRuntime::with_agent_edit`])가 끝나면 상태 snapshot 게시가 자동 저장 예약을 관찰하므로
//!   되돌리기 change set이 다음 자동 저장에 들어갑니다.
//! - **Undo 메뉴 이름**: [`AgentWindow::history_names`]가 `AgentEntryLabels`로 에이전트 항목 이름을, 그 밖의 항목은
//!   일반 이름을 냅니다.
//! - **사용자 UI 전용 동작**(lease 승인, 제안 적용, 유지, 되돌리기)은 `UserUiAction` 표식을 받습니다. 에이전트가
//!   보낸 요청은 [`crate::agent_io`]를 거쳐 들어오고 이 동작을 부를 수 없습니다(ADR-0005 §3).

use std::collections::BTreeSet;
use std::path::PathBuf;

use stellar_agent_edit::agent_history::{
    AgentEditSessions, ChunkApplied, CompletedEdit, EndReason, ProposalId, SessionContext,
    SessionEndReport, SessionError, SessionOrigin,
};
use stellar_agent_edit::attached::revert::{
    apply_revert, preview_revert, RevertFlowError, RevertPreview,
};
use stellar_agent_edit::lease::{
    open_project_coordinator, AgentEditLeases, LeaseServiceError, LeaseState, LeaseTargets,
};
use stellar_agent_edit::permission::{
    AgentSessionId, ConnectionApproval, PermissionKey, PermissionStore,
};
use stellar_agent_edit::provenance::{
    AgentChangeProvenance, HistoryDirection, ProvenanceError, ProvenanceScope, UserUiAction,
};
use stellar_commands::agent_batch::AgentBatchId;
use stellar_commands::agent_revert::RevertItem;
use stellar_commands::agent_sink::AppliedRevert;
use stellar_commands::id::CommandId;
use stellar_commands::lock::lease::{LeaseId, LeaseTable};
use stellar_commands::transaction::CommandTransaction;
use stellar_history::agent::{AgentEntryLabels, WindowAgentHistory};
use stellar_ops::revision::ProjectRevision;
use stellar_ops::validate::Definitions;

use crate::app::{AppRuntime, JobPublisher, WindowSessionHandle};
use crate::command_queue::SubmitOptions;
use crate::document_thread::{DocumentContext, JobTicket, SubmitError};
use crate::window_session::{ProjectWindowSession, WindowSessionEditError};

/// 종료 절차가 확정하는 history 항목이 창과 함께 버려질 때 쓰는 고정 id(창 닫기).
const CLOSE_ENTRY_ID: CommandId = CommandId::from_bytes([
    0x41, 0x47, 0x43, 0x4c, 0x4f, 0x53, 0x70, 0x00, 0x80, 0, 0, 0, 0, 0, 0, 1,
]);

/// 창 하나의 에이전트 편집 상태(창 메모리 전용).
#[derive(Debug, Default)]
pub struct WindowAgentEdit {
    pub(crate) sessions: AgentEditSessions,
    pub(crate) provenance: AgentChangeProvenance,
    pub(crate) labels: AgentEntryLabels,
    pub(crate) permissions: PermissionStore,
    pub(crate) leases: Option<AgentEditLeases>,
    /// lease 조정 디렉터리(없으면 OS 사용자 데이터의 공용 잠금 디렉터리).
    lock_dir: Option<PathBuf>,
    /// 앱이 시작한 세션(agent-host AgentSession)과 편집 세션 id 대응(ADR-0124 결정 1).
    pub(crate) hosted: crate::agent_session::HostedSessions,
}

impl WindowAgentEdit {
    /// lease 조정 디렉터리를 정해 만듭니다(테스트의 임시 디렉터리 등).
    pub fn with_lock_dir(lock_dir: PathBuf) -> WindowAgentEdit {
        WindowAgentEdit {
            lock_dir: Some(lock_dir),
            ..WindowAgentEdit::default()
        }
    }

    /// 세션 관리자(읽기).
    pub fn sessions(&self) -> &AgentEditSessions {
        &self.sessions
    }

    /// 출처 색인(읽기).
    pub fn provenance(&self) -> &AgentChangeProvenance {
        &self.provenance
    }

    /// 에이전트 history 항목 이름표(읽기).
    pub fn labels(&self) -> &AgentEntryLabels {
        &self.labels
    }

    /// 앱 시작 세션 표(읽기).
    pub fn hosted(&self) -> &crate::agent_session::HostedSessions {
        &self.hosted
    }

    /// 권한 저장소(읽기).
    pub fn permissions(&self) -> &PermissionStore {
        &self.permissions
    }

    /// 권한 저장소(앱 사용자 UI의 응답 전용, ADR-0005 §3).
    pub fn permissions_mut(&mut self, _ui: UserUiAction) -> &mut PermissionStore {
        &mut self.permissions
    }

    /// 이 프로젝트의 모든 창·프로세스의 활성 lease. 조정자를 아직 열지 않았으면 빈 표입니다.
    pub fn lease_table(&self) -> Result<LeaseTable, AgentEditError> {
        match &self.leases {
            Some(leases) => leases.table().map_err(AgentEditError::Lease),
            None => Ok(LeaseTable::new()),
        }
    }

    fn leases_mut(
        &mut self,
        session: &ProjectWindowSession,
    ) -> Result<&mut AgentEditLeases, AgentEditError> {
        if self.leases.is_none() {
            let lock_dir = match &self.lock_dir {
                Some(dir) => dir.clone(),
                None => stellar_ops::lock::default_lock_dir()
                    .map_err(|_| AgentEditError::Lease(LeaseServiceError::Identity))?,
            };
            let identity = session.identity();
            let coordinator = open_project_coordinator(
                &lock_dir,
                identity.project_id(),
                identity.canonical_target(),
            )
            .map_err(AgentEditError::Lease)?;
            self.leases = Some(AgentEditLeases::new(coordinator));
        }
        Ok(self.leases.as_mut().expect("방금 열었습니다"))
    }
}

/// runtime 에이전트 편집 실패. 어느 경우든 문서·history는 호출 전 상태입니다(세션 종료 절차는 보고서에 적음).
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEditError {
    /// 창에 작업 문서가 없습니다.
    NoDocument,
    /// lease 요청·조정자 실패.
    Lease(LeaseServiceError),
    /// 세션 절차 실패.
    Session(SessionError),
    /// 되돌리기 실패.
    Revert(RevertFlowError),
    /// 출처 색인 실패.
    Provenance(ProvenanceError),
    /// 앱 시작 세션 연결 실패(ADR-0124 결정 1).
    Link(crate::agent_session::LinkError),
}

impl std::fmt::Display for AgentEditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentEditError::NoDocument => f.write_str("편집할 문서가 없습니다"),
            AgentEditError::Lease(e) => write!(f, "{e}"),
            AgentEditError::Session(e) => write!(f, "{e}"),
            AgentEditError::Revert(e) => write!(f, "{e}"),
            AgentEditError::Provenance(e) => write!(f, "{e}"),
            AgentEditError::Link(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AgentEditError {}

/// Undo·Redo 메뉴 동작 이름. 항목이 없으면 `None`입니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryNames {
    /// Undo 맨 위 항목의 이름.
    pub undo: Option<&'static str>,
    /// Redo 맨 위 항목의 이름.
    pub redo: Option<&'static str>,
}

/// 창 Undo·Redo 하나를 재생하고 출처 표시 상태를 맞춥니다. 재생할 항목이 없으면 `Ok(false)`.
///
/// 재생한 항목이 에이전트 그룹·되돌리기 항목이면 `on_history`가 표시 상태를 전이하고, 재생 change set은 그 항목
/// 출처로 `observe`에 넘깁니다(같은 세션의 그룹 항목은 `observe`가 건너뜁니다).
pub fn replay(
    session: &mut ProjectWindowSession,
    agent: &mut WindowAgentEdit,
    redo: bool,
) -> Result<bool, WindowSessionEditError> {
    let Some(outcome) = session.replay_history(redo)? else {
        return Ok(false);
    };
    let direction = if redo {
        HistoryDirection::Redo
    } else {
        HistoryDirection::Undo
    };
    agent
        .provenance
        .on_history(outcome.entry.sequence.get(), direction);
    if let (Some(changes), Some(document)) = (outcome.changes.as_ref(), session.document()) {
        agent
            .provenance
            .observe(changes, outcome.entry.origin, document.model());
    }
    Ok(true)
}

/// 창 문서·history와 그 창의 에이전트 편집 상태를 함께 빌린 view. document thread의 job 안에서만 만듭니다.
#[derive(Debug)]
pub struct AgentWindow<'a> {
    pub(crate) session: &'a mut ProjectWindowSession,
    pub(crate) agent: &'a mut WindowAgentEdit,
}

/// `WindowAgentHistory`와 세션 절차 입력을 같은 창에서 나눠 빌립니다.
macro_rules! split {
    ($window:expr, |$sessions:ident, $ctx:ident, $sink:ident| $body:expr) => {{
        let AgentWindow { session, agent } = $window;
        let WindowAgentEdit {
            sessions: $sessions,
            provenance,
            labels,
            permissions,
            leases,
            ..
        } = &mut **agent;
        let Some(leases) = leases.as_mut() else {
            return Err(AgentEditError::Session(SessionError::NoEdit));
        };
        let Some(document) = session.document.as_mut() else {
            return Err(AgentEditError::NoDocument);
        };
        let mut $sink = WindowAgentHistory {
            window: &mut session.history,
            doc: &mut document.state,
            labels,
        };
        let mut $ctx = SessionContext {
            leases,
            permissions,
            provenance,
        };
        $body
    }};
}

impl<'a> AgentWindow<'a> {
    /// 창과 그 에이전트 편집 상태를 묶습니다.
    pub fn new(session: &'a mut ProjectWindowSession, agent: &'a mut WindowAgentEdit) -> Self {
        AgentWindow { session, agent }
    }

    /// 창 session(읽기).
    pub fn session(&self) -> &ProjectWindowSession {
        self.session
    }

    /// 에이전트 편집 상태(읽기).
    pub fn agent(&self) -> &WindowAgentEdit {
        self.agent
    }

    /// 에이전트 세션을 등록합니다. 권한 key는 창 프로젝트 identity(project id + canonical target)이고, 첫 연결
    /// 승인 대기를 만듭니다(XIX.3.3). 앱이 시작한 세션의 시작 확인은 사용자 UI가 첫 연결 응답으로 기록합니다.
    pub fn start_session(
        &mut self,
        id: AgentSessionId,
        origin: SessionOrigin,
        name: &str,
        now_ms: u64,
    ) -> Result<ConnectionApproval, AgentEditError> {
        let identity = self.session.identity();
        let key = PermissionKey::new(
            identity.project_id(),
            identity.canonical_target().to_path_buf(),
            id,
        );
        self.agent
            .sessions
            .start(id, key.clone(), origin, name, now_ms)
            .map_err(AgentEditError::Session)?;
        Ok(self.agent.permissions.connect(key))
    }

    /// `BeginAgentEdit` 대상 선언을 승인 대기로 두고 세션의 진행 중 편집으로 묶습니다. 아직 아무것도 잠그지
    /// 않습니다(XIX.4.2). 세션에 이미 진행 중 편집이 있으면 요청을 버리고 거부합니다.
    pub fn submit_lease(
        &mut self,
        id: AgentSessionId,
        lease: LeaseId,
        targets: &LeaseTargets,
    ) -> Result<LeaseState, AgentEditError> {
        if !self.agent.sessions.is_active(id) {
            return Err(AgentEditError::Session(
                match self.agent.sessions.end_reason(id) {
                    Some(reason) => SessionError::Ended(id, reason),
                    None => SessionError::UnknownSession(id),
                },
            ));
        }
        if let Some(current) = self.agent.sessions.lease(id) {
            return Err(AgentEditError::Session(SessionError::EditInProgress(
                current,
            )));
        }
        let model = self
            .session
            .document()
            .ok_or(AgentEditError::NoDocument)?
            .model();
        let leases = self.agent.leases_mut(self.session)?;
        let state = leases
            .submit(model, lease, targets)
            .map_err(AgentEditError::Lease)?;
        if let Err(error) = self.agent.sessions.begin_edit(id, lease) {
            let _ = self.agent.leases_mut(self.session)?.release(lease);
            return Err(AgentEditError::Session(error));
        }
        Ok(state)
    }

    /// 사용자가 승인한 lease 요청을 지금 문서로 획득합니다(앱 사용자 UI 전용).
    pub fn approve_lease(
        &mut self,
        lease: LeaseId,
        _ui: UserUiAction,
    ) -> Result<LeaseState, AgentEditError> {
        let model = self
            .session
            .document()
            .ok_or(AgentEditError::NoDocument)?
            .model();
        self.agent
            .leases_mut(self.session)?
            .approve(model, lease)
            .map_err(AgentEditError::Lease)
    }

    /// gesture 때문에 기다리는 lease 요청을 지금 문서로 다시 시도합니다(gesture가 끝난 뒤, XIX.4.7).
    pub fn retry_waiting_leases(
        &mut self,
    ) -> Vec<(LeaseId, Result<LeaseState, LeaseServiceError>)> {
        let Some(model) = self.session.document().map(|d| d.model()) else {
            return Vec::new();
        };
        match self.agent.leases.as_mut() {
            Some(leases) => leases.retry_waiting(model),
            None => Vec::new(),
        }
    }

    /// 사용자 승인을 기다리는 제안을 둡니다. 문서는 바뀌지 않습니다.
    pub fn queue_proposal(
        &mut self,
        id: AgentSessionId,
        transaction: CommandTransaction,
    ) -> Result<ProposalId, AgentEditError> {
        self.agent
            .sessions
            .queue_proposal(id, transaction)
            .map_err(AgentEditError::Session)
    }

    /// 사용자가 거부한 제안을 버립니다(앱 사용자 UI 전용).
    pub fn reject_proposal(
        &mut self,
        id: AgentSessionId,
        proposal: ProposalId,
        _ui: UserUiAction,
    ) -> Result<(), AgentEditError> {
        self.agent
            .sessions
            .reject_proposal(id, proposal)
            .map_err(AgentEditError::Session)
    }

    /// 사용자가 승인한 제안을 lease 판정과 함께 적용하고 출처 색인에 batch로 기록합니다(앱 사용자 UI 전용). 적용
    /// change set은 다른 세션의 표시된 batch에 `observe`로 알립니다.
    pub fn apply_proposal(
        &mut self,
        id: AgentSessionId,
        proposal: ProposalId,
        ui: UserUiAction,
    ) -> Result<AgentBatchId, AgentEditError> {
        self.apply_proposal_chunk(id, proposal, usize::MAX, ui)
            .map(|chunk| chunk.batch)
    }

    /// 승인한 제안의 앞 command 최대 `max_commands`개를 한 batch로 적용합니다(앱 사용자 UI 전용, ADR-0124 결정 6). 남은
    /// command는 같은 제안으로 대기에 남습니다. 큰 제안은 [`AppRuntime::apply_agent_proposal`]이 batch마다 job 하나로
    /// 나눠 부릅니다.
    pub fn apply_proposal_chunk(
        &mut self,
        id: AgentSessionId,
        proposal: ProposalId,
        max_commands: usize,
        _ui: UserUiAction,
    ) -> Result<ChunkApplied, AgentEditError> {
        let table = self.agent.lease_table()?;
        let WindowAgentEdit {
            sessions,
            provenance,
            ..
        } = &mut *self.agent;
        let document = self
            .session
            .document
            .as_mut()
            .ok_or(AgentEditError::NoDocument)?;
        let chunk = sessions
            .apply_proposal_chunk(
                id,
                proposal,
                max_commands,
                &mut document.state,
                &table,
                provenance,
            )
            .map_err(AgentEditError::Session)?;
        // 다른 세션의 표시된 batch에 닿았는지 다시 판정합니다(같은 세션 batch는 `observe`가 건너뜀).
        if let Some(record) = provenance.get(chunk.batch).map(|b| &b.record) {
            let (changes, origin) = (record.changes.clone(), record.forward.origin);
            provenance.observe(&changes, origin, document.state.model());
        }
        Ok(chunk)
    }

    /// 정상 완료(CompleteAgentEdit, XIX.6.2): 적용 batch를 "에이전트 편집" 항목 하나로 확정하고 lease를 풉니다.
    pub fn complete(
        &mut self,
        id: AgentSessionId,
        entry_id: CommandId,
    ) -> Result<CompletedEdit, AgentEditError> {
        split!(self, |sessions, ctx, sink| sessions
            .complete(id, &mut ctx, &mut sink, entry_id)
            .map_err(AgentEditError::Session))
    }

    /// 종료 절차(XIX.6.4). 자동 rollback하지 않습니다.
    pub fn end(
        &mut self,
        id: AgentSessionId,
        reason: EndReason,
        entry_id: CommandId,
        now_ms: u64,
    ) -> Result<SessionEndReport, AgentEditError> {
        if self.agent.leases.is_none() {
            // 편집을 요청한 적 없는 세션도 같은 절차로 끝냅니다(권한 폐기·진단).
            self.agent.leases_mut(self.session)?;
        }
        split!(self, |sessions, ctx, sink| sessions
            .end(id, reason, &mut ctx, &mut sink, entry_id, now_ms)
            .map_err(AgentEditError::Session))
    }

    /// bridge 연결 종료(외부 클라이언트 세션은 종료 절차, 앱 시작 세션은 유지).
    pub fn on_bridge_disconnected(
        &mut self,
        id: AgentSessionId,
        entry_id: CommandId,
        now_ms: u64,
    ) -> Result<Option<SessionEndReport>, AgentEditError> {
        if self.agent.leases.is_none() {
            self.agent.leases_mut(self.session)?;
        }
        split!(self, |sessions, ctx, sink| sessions
            .on_bridge_disconnected(id, &mut ctx, &mut sink, entry_id, now_ms)
            .map_err(AgentEditError::Session))
    }

    /// 되돌리기 미리보기(XIX.6.8). 문서와 revision은 바뀌지 않습니다.
    pub fn preview_revert<'d>(
        &self,
        scope: ProvenanceScope,
        include: &BTreeSet<RevertItem>,
        definitions: impl FnOnce() -> Definitions<'d>,
    ) -> Result<RevertPreview, AgentEditError> {
        let table = self.agent.lease_table()?;
        let state = self.session.document().ok_or(AgentEditError::NoDocument)?;
        preview_revert(
            state,
            &self.agent.provenance,
            scope,
            include,
            &table,
            definitions,
        )
        .map_err(AgentEditError::Revert)
    }

    /// 승인한 되돌리기 미리보기를 "에이전트 변경 되돌리기" 항목 하나로 적용합니다(앱 사용자 UI 전용). 결과 change
    /// set은 작업 문서 revision을 올리므로 다음 자동 저장에 들어갑니다.
    pub fn apply_revert(
        &mut self,
        approved: &RevertPreview,
        id: CommandId,
        ui: UserUiAction,
    ) -> Result<AppliedRevert, AgentEditError> {
        let table = self.agent.lease_table()?;
        let WindowAgentEdit {
            provenance, labels, ..
        } = &mut *self.agent;
        let document = self
            .session
            .document
            .as_mut()
            .ok_or(AgentEditError::NoDocument)?;
        let mut sink = WindowAgentHistory {
            window: &mut self.session.history,
            doc: &mut document.state,
            labels,
        };
        apply_revert(approved, &mut sink, &table, provenance, id, ui)
            .map_err(AgentEditError::Revert)
    }

    /// "유지": 문서·history를 바꾸지 않고 표시만 지웁니다(앱 사용자 UI 전용).
    pub fn keep(
        &mut self,
        scope: ProvenanceScope,
        ui: UserUiAction,
    ) -> Result<usize, AgentEditError> {
        self.agent
            .provenance
            .keep(scope, ui)
            .map_err(AgentEditError::Provenance)
    }

    /// 창 Undo·Redo([`replay`]).
    pub fn replay(&mut self, redo: bool) -> Result<bool, WindowSessionEditError> {
        replay(self.session, self.agent, redo)
    }

    /// Undo·Redo 메뉴 동작 이름: 에이전트 항목은 `AgentEntryLabels` 이름, 그 밖은 일반 이름입니다.
    pub fn history_names(&self) -> HistoryNames {
        let history = self.session.history().history();
        HistoryNames {
            undo: history
                .entries()
                .last()
                .map(|entry| self.agent.labels.action_name(entry)),
            redo: history
                .redo_entries()
                .last()
                .map(|entry| self.agent.labels.action_name(entry)),
        }
    }

    /// 창을 닫습니다: 활성 세션을 모두 사용자 중지로 끝내(lease 해제·권한 폐기, 자동 rollback 없음) 출처 색인을
    /// 버립니다. 끝낸 세션 보고서를 돌려줍니다.
    pub fn close(&mut self, now_ms: u64) -> Vec<SessionEndReport> {
        let active: Vec<AgentSessionId> = self.agent.sessions.active_ids();
        let mut reports = Vec::new();
        for id in active {
            // 앱 시작 세션은 AgentSession에 사용자 중지를 넣어 프로세스 종료 효과도 함께 냅니다.
            if let Some(host) = self.agent.hosted.host_of(id) {
                if let Ok(report) = self.host_event(
                    host,
                    stellar_agent_host::session::SessionEvent::UserStop,
                    CLOSE_ENTRY_ID,
                    now_ms,
                ) {
                    reports.extend(report.ended.map(|r| *r));
                }
                if !self.agent.sessions.is_active(id) {
                    continue;
                }
            }
            if let Ok(report) = self.end(id, EndReason::UserStopped, CLOSE_ENTRY_ID, now_ms) {
                reports.push(report);
            }
        }
        self.agent.provenance.close_window();
        reports
    }
}

/// 창이 열어 둔 저장 generation의 프로젝트 revision(`stellar_ops::revision` 표기, 저장한 적 없으면 `g0`). 에이전트
/// proposal의 `base_revision`을 이 값과 대조합니다(ADR-0093 Consequences, ADR-0112 결정 5).
pub fn project_revision(session: &ProjectWindowSession) -> ProjectRevision {
    session
        .loaded_generation()
        .map_or(ProjectRevision::Unsaved, |loaded| ProjectRevision::Saved {
            generation: loaded.manifest.generation,
            hash: loaded.manifest_hash,
        })
}

/// runtime 단조 시각(ms, 프로세스 안에서만 비교). 세션 시작·종료 시각과 진단 duration에 씁니다.
pub fn monotonic_ms() -> u64 {
    static ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let origin = *ORIGIN.get_or_init(std::time::Instant::now);
    u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// 창 session을 닫을 때 부릅니다(창·프로젝트 닫기). 남은 에이전트 세션의 종료 절차를 실행하고 출처 색인을 버립니다.
pub(crate) fn close_window(entry: &mut crate::app::WindowEntry) {
    AgentWindow::new(&mut entry.session, &mut entry.agent).close(monotonic_ms());
}

/// [`AppRuntime::with_agent_edit`] 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentJobError {
    /// 열려 있지 않은 창 session입니다.
    UnknownWindow(WindowSessionHandle),
    /// document thread queue가 찼습니다(busy).
    Busy,
    /// document thread가 멈췄습니다.
    Stopped,
}

impl std::fmt::Display for AgentJobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentJobError::UnknownWindow(handle) => {
                write!(f, "열려 있지 않은 창 session입니다: {}", handle.0)
            }
            AgentJobError::Busy => f.write_str("document thread queue가 찼습니다"),
            AgentJobError::Stopped => f.write_str("document thread가 멈췄습니다"),
        }
    }
}

impl std::error::Error for AgentJobError {}

impl From<SubmitError> for AgentJobError {
    fn from(error: SubmitError) -> AgentJobError {
        match error {
            SubmitError::QueueFull { .. } => AgentJobError::Busy,
            SubmitError::Stopped => AgentJobError::Stopped,
        }
    }
}

/// document thread에서 창 하나의 [`AgentWindow`]로 `job`을 실행합니다(`options` 우선순위). job 뒤 상태 snapshot을
/// 게시하므로 job이 바꾼 문서는 자동 저장 예약에 들어갑니다.
pub(crate) fn submit_window_job<T: Send + 'static>(
    publisher: &JobPublisher,
    options: SubmitOptions,
    window: WindowSessionHandle,
    job: impl FnOnce(&mut AgentWindow<'_>) -> T + Send + 'static,
) -> Result<JobTicket<Result<T, AgentJobError>>, AgentJobError> {
    publisher
        .try_submit(options, move |ctx: &mut DocumentContext| {
            let entry = ctx
                .app_sessions_mut()
                .windows
                .get_mut(&window)
                .ok_or(AgentJobError::UnknownWindow(window))?;
            Ok(job(&mut AgentWindow::new(
                &mut entry.session,
                &mut entry.agent,
            )))
        })
        .map_err(AgentJobError::from)
}

impl AppRuntime {
    /// 앱 사용자 UI의 에이전트 편집 동작(lease 승인, 제안 적용·거부, 완료·중지, 유지, 되돌리기 미리보기·적용, Undo
    /// 메뉴 이름 조회)을 document thread에서 실행합니다. 일반 command와 같은 priority이고, 끝나면 상태 snapshot을
    /// 게시해 되돌리기 change set 같은 문서 변경이 자동 저장 예약에 들어갑니다. 에이전트가 보낸 요청은
    /// [`crate::agent_io::AgentIo`]를 씁니다.
    pub fn with_agent_edit<T: Send + 'static>(
        &self,
        window: WindowSessionHandle,
        job: impl FnOnce(&mut AgentWindow<'_>) -> T + Send + 'static,
    ) -> Result<JobTicket<Result<T, AgentJobError>>, AgentJobError> {
        submit_window_job(&self.job_publisher(), SubmitOptions::command(), window, job)
    }

    /// 사용자가 승인한 제안을 batch 여러 개로 나눠 적용합니다(XXIX.3.4, ADR-0124 결정 6). batch 하나([`PROPOSAL_BATCH_COMMANDS`]
    /// command)가 document thread job 하나이고, 남으면 다음 batch job을 `Command` lane 뒤에 다시 넣어 그 사이 사용자
    /// 입력(`Input` lane)이 먼저 처리됩니다. 마지막 batch 뒤(또는 실패하면 그때) 결과를 한 번 보냅니다.
    pub fn apply_agent_proposal(
        &self,
        window: WindowSessionHandle,
        session: AgentSessionId,
        proposal: ProposalId,
        ui: UserUiAction,
    ) -> Result<std::sync::mpsc::Receiver<ProposalApplied>, AgentJobError> {
        let (sender, receiver) = std::sync::mpsc::channel();
        submit_batch(
            self.job_publisher(),
            BatchJob {
                window,
                session,
                proposal,
                ui,
                applied: Vec::new(),
                sender,
            },
        )?;
        Ok(receiver)
    }

    /// 창의 Undo·Redo 메뉴 동작 이름(에이전트 항목은 `AgentEntryLabels` 이름).
    pub fn agent_history_names(
        &self,
        window: WindowSessionHandle,
    ) -> Result<JobTicket<Result<HistoryNames, AgentJobError>>, AgentJobError> {
        self.with_agent_edit(window, |agent| agent.history_names())
    }
}

/// 큰 제안을 나눠 적용할 때 batch 하나의 최대 command 수(ADR-0124 결정 6). 20만 node 문서 release 측정에서 command 하나의
/// 적용과 출처 기록이 약 1.4 µs라 512개가 약 0.7 ms이고, document thread 단일 점유 상한 2 ms(아키텍처 §6.3) 안에 상태 게시
/// 여유를 남깁니다(측정값은 `docs/task-reports/XXIX.3.4.md`).
pub const PROPOSAL_BATCH_COMMANDS: usize = 512;

/// [`AppRuntime::apply_agent_proposal`] 결과.
#[derive(Debug, Clone, PartialEq)]
pub struct ProposalApplied {
    /// 적용한 batch(순서대로).
    pub applied: Vec<AgentBatchId>,
    /// 실패했으면 그 원인(그 batch와 남은 부분은 적용되지 않음).
    pub error: Option<AgentApplyError>,
}

/// 나눠 적용하는 중의 실패.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentApplyError {
    /// batch 적용 실패.
    Edit(AgentEditError),
    /// 다음 batch job을 넣지 못했습니다.
    Job(AgentJobError),
}

struct BatchJob {
    window: WindowSessionHandle,
    session: AgentSessionId,
    proposal: ProposalId,
    ui: UserUiAction,
    applied: Vec<AgentBatchId>,
    sender: std::sync::mpsc::Sender<ProposalApplied>,
}

fn submit_batch(publisher: JobPublisher, job: BatchJob) -> Result<(), AgentJobError> {
    let next = publisher.clone();
    let window = job.window;
    submit_window_job(
        &publisher,
        SubmitOptions::command(),
        window,
        move |agent: &mut AgentWindow<'_>| {
            let BatchJob {
                window,
                session,
                proposal,
                ui,
                mut applied,
                sender,
            } = job;
            let error =
                match agent.apply_proposal_chunk(session, proposal, PROPOSAL_BATCH_COMMANDS, ui) {
                    Ok(chunk) => {
                        applied.push(chunk.batch);
                        if chunk.remaining == 0 {
                            None
                        } else {
                            let job = BatchJob {
                                window,
                                session,
                                proposal,
                                ui,
                                applied,
                                sender,
                            };
                            let fallback = (job.applied.clone(), job.sender.clone());
                            match submit_batch(next, job) {
                                Ok(()) => return,
                                Err(error) => {
                                    let _ = fallback.1.send(ProposalApplied {
                                        applied: fallback.0,
                                        error: Some(AgentApplyError::Job(error)),
                                    });
                                    return;
                                }
                            }
                        }
                    }
                    Err(error) => Some(AgentApplyError::Edit(error)),
                };
            let _ = sender.send(ProposalApplied { applied, error });
        },
    )
    .map(|_ticket| ())
}

#[cfg(test)]
#[path = "agent_edit_tests.rs"]
mod tests;

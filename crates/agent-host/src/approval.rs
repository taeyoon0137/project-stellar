//! 승인 요청 저장 (XXIX.3.2, ADR-0005 §4.5 승인 요청 저장, ADR-0101 결정 1).
//!
//! 사용자 응답을 기다리는 승인 요청을 프로젝트 identity([`ProjectKey`]) 단위로 앱 코어 메모리에 둡니다.
//! 요청의 수명 판정(첫 응답, 응답 기한, 창 닫힘, 세션 종료)은 이 모듈 하나만 합니다. 확정된 결과
//! ([`Resolution`])를 받아 권한 저장소(XIX.3.4)와 lease 요청 관리자(XIX.4.1)에 넘기는 일은 앱 runtime이
//! 합니다. 그쪽은 결과만 받고 수명을 따로 판정하지 않습니다.
//!
//! - **첫 응답만 유효**: 확정된 요청에 다시 응답하면 [`Answered::Ignored`]이고 결과가 바뀌지 않습니다.
//! - **기본 거부**: 응답 기한이 지나면([`ApprovalStore::expire`], 기한 뒤 도착한 응답 포함), 요청을 연
//!   창이 닫히면([`ApprovalStore::close_window`]), 세션이 끝나면([`ApprovalStore::end_session`]) 거부로
//!   확정합니다. 닫힌 창과 끝난 세션으로는 새 요청을 열 수 없습니다.
//! - **순서**: 같은 프로젝트의 대기 요청은 연 순서(요청 id 순서) 그대로 보입니다. id는 저장소 전체에서
//!   단조 증가합니다.
//! - **승인 근거**: 적용 전 batch 승인 요청([`ApprovalSubject::Batch`])은 Stellar 코어가 만든 semantic diff,
//!   validation 결과, 렌더 프리뷰를 모두 갖춰야 열립니다(XIX.5.3). 근거 타입은 ops·renderer 결과를 담는
//!   앱 runtime이 정하고, 이 crate는 [`ApprovalEvidence`]로 갖춤 여부만 봅니다(ADR-0005 §8: 이 crate는
//!   ops에 도달하지 않음).
//! - **쓰는 쪽**: 요청은 에이전트 경로(MCP tool 처리)가 열 수 있지만, 응답([`ApprovalStore::answer`])은 앱의
//!   사용자 UI만 부릅니다(ADR-0005 §3 변경 불가 경로, `tests/immutable_paths.rs`).
//! - **저장 범위**: 메모리에만 있습니다. 파일·직렬화 경로가 없고 `Debug`는 개수만 씁니다(XVI.8).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};

use stellar_agent_protocol::core_types::ids::StableId;
use stellar_agent_protocol::permission::Permission;

use crate::session::{AgentSessionId, ProposalId, RequestId};

/// 응답 기한 기본값(ms). 근거는 XXIX.3.2 보고서(XIX.9.5 실측 전 잠정값).
pub const DEFAULT_RESPONSE_DEADLINE_MS: u64 = 5 * 60 * 1000;

/// 확정 결과를 기억하는 요청 수 상한. 넘으면 오래된 기록부터 잊고, 잊은 요청에 온 응답은
/// [`ApprovalError::UnknownRequest`]입니다(어느 쪽이든 효과 0).
pub const MAX_RESOLVED_RECORDS: usize = 65_536;

/// 프로젝트 identity(IV.2.3: project id + canonical target). 앱 runtime이 `ProjectIdentity`에서 만듭니다.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectKey {
    project_id: StableId,
    canonical_target: PathBuf,
}

impl ProjectKey {
    /// project id와 canonical target(symlink를 모두 해석한 실경로)으로 만듭니다.
    pub fn new(project_id: StableId, canonical_target: PathBuf) -> ProjectKey {
        ProjectKey {
            project_id,
            canonical_target,
        }
    }

    /// project id.
    pub fn project_id(&self) -> StableId {
        self.project_id
    }

    /// canonical target.
    pub fn canonical_target(&self) -> &Path {
        &self.canonical_target
    }
}

impl fmt::Debug for ProjectKey {
    /// 경로는 쓰지 않습니다(진단에 프로젝트 경로 0, XVI.8).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProjectKey(..)")
    }
}

/// 요청을 연 창 세션. 세션은 시작한 창에 묶입니다(ADR-0005 §4.2).
pub use stellar_agent_protocol::core_types::ids::WindowSessionId;

/// 승인 대상.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalSubject {
    /// `BeginAgentEdit` 범위 승인(XIX.4.1). 대상 node 수와 외부 resource 수만 담습니다.
    Scope {
        /// 대상 root 수.
        targets: u32,
        /// 전역 resource 수.
        externals: u32,
    },
    /// 적용 전 batch 승인(XIX.5.3).
    Batch {
        /// 제안.
        proposal: ProposalId,
    },
    /// 권한 하나의 개별 확인(XIX.3.4).
    Permission(Permission),
}

impl ApprovalSubject {
    /// 고정 이름.
    pub fn kind(self) -> &'static str {
        match self {
            ApprovalSubject::Scope { .. } => "scope",
            ApprovalSubject::Batch { .. } => "batch",
            ApprovalSubject::Permission(_) => "permission",
        }
    }
}

/// 승인 근거의 갖춤 여부.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvidenceParts {
    /// Stellar 코어 semantic diff.
    pub semantic_diff: bool,
    /// validation 결과(`stellar validate`와 같은 진단).
    pub validation: bool,
    /// 렌더 프리뷰.
    pub render_preview: bool,
}

impl EvidenceParts {
    /// 셋을 모두 갖췄는지.
    pub fn complete(self) -> bool {
        self.semantic_diff && self.validation && self.render_preview
    }
}

/// 승인 근거. 앱 runtime이 Stellar 코어 결과로 구현합니다.
pub trait ApprovalEvidence {
    /// 갖춘 근거.
    fn parts(&self) -> EvidenceParts;
}

/// 근거가 필요 없는 요청(범위·권한 확인)에 쓰는 빈 근거.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoEvidence;

impl ApprovalEvidence for NoEvidence {
    fn parts(&self) -> EvidenceParts {
        EvidenceParts::default()
    }
}

/// 열 요청.
#[derive(Clone, PartialEq, Eq)]
pub struct ApprovalRequest<E> {
    /// 요청한 에이전트 세션.
    pub session: AgentSessionId,
    /// 세션이 묶인 창.
    pub window: WindowSessionId,
    /// 대상.
    pub subject: ApprovalSubject,
    /// 승인 근거.
    pub evidence: E,
    /// 응답 기한(단조 시각 ms). 이 시각부터 기한 초과입니다.
    pub deadline_ms: u64,
}

impl<E> fmt::Debug for ApprovalRequest<E> {
    /// 근거 내용은 쓰지 않습니다.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApprovalRequest")
            .field("session", &self.session)
            .field("window", &self.window)
            .field("subject", &self.subject.kind())
            .field("deadline_ms", &self.deadline_ms)
            .finish()
    }
}

/// 사용자 응답.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserAnswer {
    /// 승인.
    Approve,
    /// 거부.
    Deny,
}

/// 거부 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denial {
    /// 사용자가 거부함.
    User,
    /// 응답 기한 초과.
    Expired,
    /// 요청을 연 창이 닫힘.
    WindowClosed,
    /// 세션이 끝남.
    SessionEnded,
}

impl Denial {
    /// 고정 이름.
    pub fn code(self) -> &'static str {
        match self {
            Denial::User => "user",
            Denial::Expired => "expired",
            Denial::WindowClosed => "window-closed",
            Denial::SessionEnded => "session-ended",
        }
    }
}

/// 확정 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 승인.
    Approved,
    /// 거부.
    Denied(Denial),
}

impl Outcome {
    /// 고정 이름(`approved` 또는 `denied:<이유>`).
    pub fn code(self) -> String {
        match self {
            Outcome::Approved => "approved".to_string(),
            Outcome::Denied(d) => format!("denied:{}", d.code()),
        }
    }
}

/// 확정된 요청 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    /// 요청.
    pub id: RequestId,
    /// 세션.
    pub session: AgentSessionId,
    /// 창.
    pub window: WindowSessionId,
    /// 대상.
    pub subject: ApprovalSubject,
    /// 결과.
    pub outcome: Outcome,
    /// 확정 시각(ms).
    pub at_ms: u64,
}

/// 응답 처리 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answered {
    /// 이 응답으로(또는 응답 시점의 기한 초과로) 확정됨.
    Resolved(Resolution),
    /// 이미 확정된 요청이라 무시함(첫 응답만 유효).
    Ignored {
        /// 먼저 확정된 결과.
        prior: Outcome,
    },
}

/// 저장소 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalError {
    /// 모르는 요청(이 프로젝트에 연 적 없거나 기록 상한으로 잊음).
    UnknownRequest(RequestId),
    /// 응답 기한이 이미 지난 요청은 열지 않습니다.
    DeadlineNotInFuture,
    /// 끝난 세션으로는 요청을 열지 않습니다.
    SessionEnded,
    /// 닫힌 창으로는 요청을 열지 않습니다.
    WindowClosed,
    /// batch 승인 요청에 semantic diff·validation·렌더 프리뷰가 모두 있지 않습니다(XIX.5.3).
    MissingEvidence(EvidenceParts),
}

impl fmt::Display for ApprovalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApprovalError::UnknownRequest(id) => write!(f, "승인 요청 {}이 없습니다", id.0),
            ApprovalError::DeadlineNotInFuture => f.write_str("응답 기한이 이미 지났습니다"),
            ApprovalError::SessionEnded => f.write_str("끝난 세션입니다"),
            ApprovalError::WindowClosed => f.write_str("닫힌 창입니다"),
            ApprovalError::MissingEvidence(_) => f.write_str(
                "batch 승인 요청에는 semantic diff·validation·렌더 프리뷰가 모두 필요합니다",
            ),
        }
    }
}

impl std::error::Error for ApprovalError {}

struct Pending<E> {
    id: RequestId,
    request: ApprovalRequest<E>,
}

struct ProjectQueue<E> {
    pending: Vec<Pending<E>>,
    resolved: BTreeMap<RequestId, Outcome>,
    resolved_order: VecDeque<RequestId>,
}

impl<E> ProjectQueue<E> {
    fn new() -> ProjectQueue<E> {
        ProjectQueue {
            pending: Vec::new(),
            resolved: BTreeMap::new(),
            resolved_order: VecDeque::new(),
        }
    }

    fn remember(&mut self, id: RequestId, outcome: Outcome) {
        self.resolved.insert(id, outcome);
        self.resolved_order.push_back(id);
        while self.resolved_order.len() > MAX_RESOLVED_RECORDS {
            if let Some(old) = self.resolved_order.pop_front() {
                self.resolved.remove(&old);
            }
        }
    }

    /// `pick`이 고른 대기 요청을 `outcome`으로 확정합니다(연 순서 유지).
    fn resolve_where(
        &mut self,
        mut pick: impl FnMut(&Pending<E>) -> Option<Outcome>,
        at_ms: u64,
    ) -> Vec<Resolution> {
        let mut out = Vec::new();
        let mut kept = Vec::with_capacity(self.pending.len());
        for entry in std::mem::take(&mut self.pending) {
            match pick(&entry) {
                Some(outcome) => out.push(Resolution {
                    id: entry.id,
                    session: entry.request.session,
                    window: entry.request.window,
                    subject: entry.request.subject,
                    outcome,
                    at_ms,
                }),
                None => kept.push(entry),
            }
        }
        self.pending = kept;
        for r in &out {
            self.remember(r.id, r.outcome);
        }
        out
    }
}

/// 승인 요청 저장소. 앱 프로세스 수명 동안 메모리에만 있습니다.
pub struct ApprovalStore<E> {
    projects: BTreeMap<ProjectKey, ProjectQueue<E>>,
    ended_sessions: BTreeSet<AgentSessionId>,
    closed_windows: BTreeSet<WindowSessionId>,
    next_id: u64,
}

impl<E> Default for ApprovalStore<E> {
    fn default() -> Self {
        ApprovalStore {
            projects: BTreeMap::new(),
            ended_sessions: BTreeSet::new(),
            closed_windows: BTreeSet::new(),
            next_id: 1,
        }
    }
}

impl<E> fmt::Debug for ApprovalStore<E> {
    /// 개수만 씁니다(경로·근거·대상 내용 0).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApprovalStore")
            .field("projects", &self.projects.len())
            .field(
                "pending",
                &self
                    .projects
                    .values()
                    .map(|q| q.pending.len())
                    .sum::<usize>(),
            )
            .finish()
    }
}

impl<E: ApprovalEvidence> ApprovalStore<E> {
    /// 빈 저장소. 이전 실행의 요청을 읽는 경로는 없습니다.
    pub fn new() -> ApprovalStore<E> {
        ApprovalStore::default()
    }

    /// 요청을 엽니다. 대기 목록 끝에 붙고 id를 돌려줍니다.
    pub fn open(
        &mut self,
        project: ProjectKey,
        request: ApprovalRequest<E>,
        now_ms: u64,
    ) -> Result<RequestId, ApprovalError> {
        if self.ended_sessions.contains(&request.session) {
            return Err(ApprovalError::SessionEnded);
        }
        if self.closed_windows.contains(&request.window) {
            return Err(ApprovalError::WindowClosed);
        }
        if request.deadline_ms <= now_ms {
            return Err(ApprovalError::DeadlineNotInFuture);
        }
        if let ApprovalSubject::Batch { .. } = request.subject {
            let parts = request.evidence.parts();
            if !parts.complete() {
                return Err(ApprovalError::MissingEvidence(parts));
            }
        }
        let id = RequestId(self.next_id);
        self.next_id += 1;
        self.projects
            .entry(project)
            .or_insert_with(ProjectQueue::new)
            .pending
            .push(Pending { id, request });
        Ok(id)
    }

    /// 프로젝트의 대기 요청(연 순서).
    pub fn pending(
        &self,
        project: &ProjectKey,
    ) -> impl Iterator<Item = (RequestId, &ApprovalRequest<E>)> {
        self.projects
            .get(project)
            .into_iter()
            .flat_map(|q| q.pending.iter().map(|p| (p.id, &p.request)))
    }

    /// 모든 프로젝트의 대기 요청 수.
    pub fn pending_len(&self) -> usize {
        self.projects.values().map(|q| q.pending.len()).sum()
    }

    /// 세션의 대기 요청 수.
    pub fn pending_for_session(&self, session: AgentSessionId) -> usize {
        self.projects
            .values()
            .flat_map(|q| q.pending.iter())
            .filter(|p| p.request.session == session)
            .count()
    }

    /// 사용자 응답. 앱 사용자 UI만 부릅니다(ADR-0005 §3). 첫 응답만 유효하고, 기한 뒤 도착한 응답은
    /// 승인하지 않고 기한 초과 거부로 확정합니다.
    pub fn answer(
        &mut self,
        project: &ProjectKey,
        id: RequestId,
        answer: UserAnswer,
        now_ms: u64,
    ) -> Result<Answered, ApprovalError> {
        let queue = self
            .projects
            .get_mut(project)
            .ok_or(ApprovalError::UnknownRequest(id))?;
        if let Some(prior) = queue.resolved.get(&id) {
            return Ok(Answered::Ignored { prior: *prior });
        }
        let mut resolved = queue.resolve_where(
            |p| {
                if p.id != id {
                    None
                } else if now_ms >= p.request.deadline_ms {
                    Some(Outcome::Denied(Denial::Expired))
                } else {
                    Some(match answer {
                        UserAnswer::Approve => Outcome::Approved,
                        UserAnswer::Deny => Outcome::Denied(Denial::User),
                    })
                }
            },
            now_ms,
        );
        resolved
            .pop()
            .map(Answered::Resolved)
            .ok_or(ApprovalError::UnknownRequest(id))
    }

    /// 응답 기한이 지난 요청을 거부로 확정합니다.
    pub fn expire(&mut self, now_ms: u64) -> Vec<Resolution> {
        self.resolve_all(
            |p| (now_ms >= p.request.deadline_ms).then_some(Outcome::Denied(Denial::Expired)),
            now_ms,
        )
    }

    /// 창이 닫혔습니다. 그 창의 대기 요청을 거부로 확정하고 이후 그 창으로는 요청을 열지 않습니다.
    pub fn close_window(&mut self, window: WindowSessionId, now_ms: u64) -> Vec<Resolution> {
        self.closed_windows.insert(window);
        self.resolve_all(
            |p| (p.request.window == window).then_some(Outcome::Denied(Denial::WindowClosed)),
            now_ms,
        )
    }

    /// 세션이 끝났습니다(XIX.6.4 4단계). 그 세션의 대기 요청을 거부로 확정하고 이후 그 세션으로는
    /// 요청을 열지 않습니다.
    pub fn end_session(&mut self, session: AgentSessionId, now_ms: u64) -> Vec<Resolution> {
        self.ended_sessions.insert(session);
        self.resolve_all(
            |p| (p.request.session == session).then_some(Outcome::Denied(Denial::SessionEnded)),
            now_ms,
        )
    }

    /// 세션 상태 기계 효과 중 이 저장소가 맡는 것을 실행합니다. `RejectPendingRequests`(XIX.6.4 4단계)가
    /// 있으면 그 세션의 대기 요청을 모두 세션 종료 거부로 확정합니다. 다른 효과는 runtime 몫입니다.
    pub fn apply_session_effects(
        &mut self,
        session: AgentSessionId,
        effects: &[crate::session::SessionEffect],
        now_ms: u64,
    ) -> Vec<Resolution> {
        if effects
            .iter()
            .any(|e| matches!(e, crate::session::SessionEffect::RejectPendingRequests(_)))
        {
            self.end_session(session, now_ms)
        } else {
            Vec::new()
        }
    }

    fn resolve_all(
        &mut self,
        mut pick: impl FnMut(&Pending<E>) -> Option<Outcome>,
        now_ms: u64,
    ) -> Vec<Resolution> {
        let mut out = Vec::new();
        for queue in self.projects.values_mut() {
            out.extend(queue.resolve_where(&mut pick, now_ms));
        }
        out.sort_by_key(|r| r.id);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct Full;
    impl ApprovalEvidence for Full {
        fn parts(&self) -> EvidenceParts {
            EvidenceParts {
                semantic_diff: true,
                validation: true,
                render_preview: true,
            }
        }
    }

    #[test]
    fn debug_has_no_path() {
        let key = ProjectKey::new(
            StableId::NIL,
            PathBuf::from("/Users/someone/secret-project"),
        );
        assert!(!format!("{key:?}").contains("secret"));
        let mut store: ApprovalStore<Full> = ApprovalStore::new();
        store
            .open(
                key,
                ApprovalRequest {
                    session: AgentSessionId(1),
                    window: WindowSessionId::from_stable_id(StableId::NIL),
                    subject: ApprovalSubject::Batch {
                        proposal: ProposalId(1),
                    },
                    evidence: Full,
                    deadline_ms: 10,
                },
                0,
            )
            .unwrap();
        assert!(!format!("{store:?}").contains("secret"));
    }
}

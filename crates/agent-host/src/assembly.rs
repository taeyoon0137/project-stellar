//! 채팅 보기 조립 (XXIX.3.6, ADR-0005 §4.4, 아키텍처 31.3~31.5·31.9, ADR-0101 결정 4).
//!
//! 세션 하나의 채팅 보기 목록([`TranscriptEntry`])을 도착 순서대로 조립합니다. 두 출처가 섞이지만 서로의
//! 근거가 되지 않습니다.
//!
//! - **CLI 보고**([`TranscriptEntry::Chat`]): 디코더가 만든 [`AgentChatItem`] 그대로입니다. CLI가 tool 호출이
//!   성공했다고 보고해도 문서 효과 표시가 생기지 않습니다. 채팅 보기는 문서 상태나 승인 결과의 근거가
//!   아닙니다.
//! - **문서 효과**([`TranscriptEntry::Effect`]): Stellar 승인 기록([`crate::approval::Resolution`])과 출처 기록
//!   ([`BatchRecord`], XIX.6.7 `AgentChangeProvenance`의 batch 요약)에서만 만듭니다. 기록과 어긋나는 효과는
//!   넣지 않고 [`AssemblyError`]로 거부합니다.
//!   - 승인 요청은 먼저 열린 기록이 있어야 확정을 받습니다. 다른 세션의 기록은 받지 않습니다.
//!   - batch 적용 표시는 batch 순번이 늘어날 때만 받습니다. 제안 id가 있으면 그 제안의 batch 승인이
//!     `approved`로 확정되어 있어야 합니다. 제안 없는 적용(승인 범위 내 바로 적용)은 그 모드에서만 받습니다.
//!   - [`ChatAssembly::verify`]는 표시된 효과가 기록 목록과 정확히 같은지 다시 확인합니다.
//! - **저장 범위**: 세션 메모리에만 둡니다. 직렬화·파일 경로가 없고 `Debug`는 개수만 씁니다. golden 직렬화
//!   ([`TranscriptEntry::to_golden`])는 테스트 기준이며 저장 형식이 아닙니다.
//! - **상한**: CLI 보고 item은 [`crate::chat::MAX_SESSION_ITEMS`]를 넘으면 오래된 것부터 버리고 수를 셉니다.
//!   문서 효과는 버리지 않습니다(검토 대기 출처 표시의 근거라 잃으면 안 됨, batch·요청 수만큼만 생김).

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use crate::approval::{ApprovalSubject, Outcome, Resolution};
use crate::chat::{AgentChatItem, MAX_SESSION_ITEMS};
use crate::json::{self, JsonValue};
use crate::session::{AgentSessionId, ProposalId, RequestId, SessionMode};

/// 출처 기록의 batch 요약(앱 runtime이 창 메모리의 출처 색인에서 만듦, XIX.6.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchRecord {
    /// 세션.
    pub session: AgentSessionId,
    /// 세션 안 batch 순번.
    pub batch_seq: u32,
    /// history sequence.
    pub history_seq: u64,
    /// 이 batch를 승인한 제안(검토 후 적용). 승인 범위 내 바로 적용이면 None.
    pub proposal: Option<ProposalId>,
    /// 변경 node 수(VII.1.4 결과).
    pub changed_nodes: u32,
    /// 변경 StableId+PropertyPath 수.
    pub changed_properties: u32,
    /// 생성 node 수.
    pub created_nodes: u32,
    /// 삭제 node 수.
    pub deleted_nodes: u32,
}

/// 문서 효과 표시.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectEntry {
    /// 승인 요청이 열림.
    ApprovalRequested {
        /// 요청.
        request: RequestId,
        /// 대상.
        subject: ApprovalSubject,
    },
    /// 승인 요청이 확정됨.
    ApprovalResolved {
        /// 요청.
        request: RequestId,
        /// 대상.
        subject: ApprovalSubject,
        /// 결과.
        outcome: Outcome,
    },
    /// batch가 적용됨(출처 기록).
    BatchApplied(BatchRecord),
}

/// 채팅 보기 목록의 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptEntry {
    /// CLI가 보고한 item.
    Chat(AgentChatItem),
    /// Stellar 기록에서 만든 문서 효과.
    Effect(EffectEntry),
}

fn subject_golden(subject: ApprovalSubject) -> Vec<(&'static str, JsonValue)> {
    let mut pairs = vec![("subject", JsonValue::string(subject.kind()))];
    match subject {
        ApprovalSubject::Scope { targets, externals } => {
            pairs.push(("targets", JsonValue::int(i64::from(targets))));
            pairs.push(("externals", JsonValue::int(i64::from(externals))));
        }
        ApprovalSubject::Batch { proposal } => {
            pairs.push(("proposal", JsonValue::int(proposal.0 as i64)));
        }
        ApprovalSubject::Permission(p) => pairs.push(("permission", JsonValue::string(p.name()))),
    }
    pairs
}

impl TranscriptEntry {
    /// 결정론적 golden 직렬화(한 줄 JSON).
    pub fn to_golden(&self) -> String {
        match self {
            TranscriptEntry::Chat(item) => item.to_golden(),
            TranscriptEntry::Effect(effect) => {
                let mut pairs: Vec<(&str, JsonValue)> = Vec::new();
                match effect {
                    EffectEntry::ApprovalRequested { request, subject } => {
                        pairs.push(("effect", JsonValue::string("approval-requested")));
                        pairs.push(("request", JsonValue::int(request.0 as i64)));
                        pairs.extend(subject_golden(*subject));
                    }
                    EffectEntry::ApprovalResolved {
                        request,
                        subject,
                        outcome,
                    } => {
                        pairs.push(("effect", JsonValue::string("approval-resolved")));
                        pairs.push(("request", JsonValue::int(request.0 as i64)));
                        pairs.extend(subject_golden(*subject));
                        pairs.push(("outcome", JsonValue::string(outcome.code())));
                    }
                    EffectEntry::BatchApplied(b) => {
                        pairs.push(("effect", JsonValue::string("batch-applied")));
                        pairs.push(("batch", JsonValue::int(i64::from(b.batch_seq))));
                        pairs.push(("history", JsonValue::int(b.history_seq as i64)));
                        pairs.push((
                            "proposal",
                            b.proposal
                                .map_or(JsonValue::Null, |p| JsonValue::int(p.0 as i64)),
                        ));
                        pairs.push(("nodes", JsonValue::int(i64::from(b.changed_nodes))));
                        pairs.push((
                            "properties",
                            JsonValue::int(i64::from(b.changed_properties)),
                        ));
                        pairs.push(("created", JsonValue::int(i64::from(b.created_nodes))));
                        pairs.push(("deleted", JsonValue::int(i64::from(b.deleted_nodes))));
                    }
                }
                json::write(&JsonValue::object(pairs))
            }
        }
    }
}

/// 기록과 어긋나 받지 않은 효과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssemblyError {
    /// 다른 세션의 기록.
    OtherSession,
    /// 같은 요청을 다시 열려 함.
    DuplicateRequest(RequestId),
    /// 열린 적 없는 요청의 확정.
    UnknownRequest(RequestId),
    /// 이미 확정된 요청의 확정(첫 응답만 유효).
    AlreadyResolved(RequestId),
    /// 열린 요청과 대상이 다른 확정.
    SubjectMismatch(RequestId),
    /// batch 순번이 늘지 않음.
    BatchOutOfOrder {
        /// 받은 순번.
        batch_seq: u32,
    },
    /// 제안의 batch 승인이 `approved`로 확정되지 않음.
    BatchNotApproved(ProposalId),
    /// 제안 없는 적용인데 모드가 승인 범위 내 바로 적용이 아님.
    DirectApplyAboveMode,
}

impl AssemblyError {
    /// 고정 코드.
    pub fn code(self) -> &'static str {
        match self {
            AssemblyError::OtherSession => "other-session",
            AssemblyError::DuplicateRequest(_) => "duplicate-request",
            AssemblyError::UnknownRequest(_) => "unknown-request",
            AssemblyError::AlreadyResolved(_) => "already-resolved",
            AssemblyError::SubjectMismatch(_) => "subject-mismatch",
            AssemblyError::BatchOutOfOrder { .. } => "batch-out-of-order",
            AssemblyError::BatchNotApproved(_) => "batch-not-approved",
            AssemblyError::DirectApplyAboveMode => "direct-apply-above-mode",
        }
    }
}

/// 표시와 기록의 불일치(검증 결과).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mismatch {
    /// 기록에 없는 효과가 표시됨.
    Unrecorded(EffectEntry),
    /// 기록이 표시되지 않음.
    Missing(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestState {
    Open(ApprovalSubject),
    Resolved(ApprovalSubject, Outcome),
}

/// 세션 하나의 채팅 보기 조립.
#[derive(Clone)]
pub struct ChatAssembly {
    session: AgentSessionId,
    mode: SessionMode,
    entries: VecDeque<TranscriptEntry>,
    chat_count: usize,
    dropped_chat: u64,
    requests: BTreeMap<RequestId, RequestState>,
    last_batch: Option<u32>,
}

impl fmt::Debug for ChatAssembly {
    /// 개수만 씁니다(내용 0).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatAssembly")
            .field("entries", &self.entries.len())
            .field("dropped_chat", &self.dropped_chat)
            .finish()
    }
}

impl ChatAssembly {
    /// 새 조립. `mode`는 세션 시작 때 사용자가 고른 모드입니다.
    pub fn new(session: AgentSessionId, mode: SessionMode) -> ChatAssembly {
        ChatAssembly {
            session,
            mode,
            entries: VecDeque::new(),
            chat_count: 0,
            dropped_chat: 0,
            requests: BTreeMap::new(),
            last_batch: None,
        }
    }

    /// 사용자가 모드를 바꿨습니다(이후 batch 판정에 씀).
    pub fn set_mode(&mut self, mode: SessionMode) {
        self.mode = mode;
    }

    /// CLI 보고 item을 더합니다.
    pub fn push_chat(&mut self, item: AgentChatItem) {
        if self.chat_count == MAX_SESSION_ITEMS {
            if let Some(at) = self
                .entries
                .iter()
                .position(|e| matches!(e, TranscriptEntry::Chat(_)))
            {
                self.entries.remove(at);
                self.chat_count -= 1;
                self.dropped_chat += 1;
            }
        }
        self.entries.push_back(TranscriptEntry::Chat(item));
        self.chat_count += 1;
    }

    /// 승인 요청이 열렸습니다(저장소가 준 id).
    pub fn record_request(
        &mut self,
        session: AgentSessionId,
        request: RequestId,
        subject: ApprovalSubject,
    ) -> Result<(), AssemblyError> {
        if session != self.session {
            return Err(AssemblyError::OtherSession);
        }
        if self.requests.contains_key(&request) {
            return Err(AssemblyError::DuplicateRequest(request));
        }
        self.requests.insert(request, RequestState::Open(subject));
        self.entries
            .push_back(TranscriptEntry::Effect(EffectEntry::ApprovalRequested {
                request,
                subject,
            }));
        Ok(())
    }

    /// 승인 저장소가 확정한 결과를 더합니다.
    pub fn record_resolution(&mut self, resolution: &Resolution) -> Result<(), AssemblyError> {
        if resolution.session != self.session {
            return Err(AssemblyError::OtherSession);
        }
        match self.requests.get(&resolution.id) {
            None => Err(AssemblyError::UnknownRequest(resolution.id)),
            Some(RequestState::Resolved(..)) => Err(AssemblyError::AlreadyResolved(resolution.id)),
            Some(RequestState::Open(subject)) if *subject != resolution.subject => {
                Err(AssemblyError::SubjectMismatch(resolution.id))
            }
            Some(RequestState::Open(subject)) => {
                let subject = *subject;
                self.requests.insert(
                    resolution.id,
                    RequestState::Resolved(subject, resolution.outcome),
                );
                self.entries
                    .push_back(TranscriptEntry::Effect(EffectEntry::ApprovalResolved {
                        request: resolution.id,
                        subject,
                        outcome: resolution.outcome,
                    }));
                Ok(())
            }
        }
    }

    /// 출처 기록의 batch 적용을 더합니다.
    pub fn record_batch(&mut self, batch: &BatchRecord) -> Result<(), AssemblyError> {
        if batch.session != self.session {
            return Err(AssemblyError::OtherSession);
        }
        if self.last_batch.is_some_and(|last| batch.batch_seq <= last) {
            return Err(AssemblyError::BatchOutOfOrder {
                batch_seq: batch.batch_seq,
            });
        }
        match batch.proposal {
            Some(proposal) => {
                let approved = self.requests.values().any(|r| {
                    *r == RequestState::Resolved(
                        ApprovalSubject::Batch { proposal },
                        Outcome::Approved,
                    )
                });
                if !approved {
                    return Err(AssemblyError::BatchNotApproved(proposal));
                }
            }
            None => {
                if self.mode != SessionMode::ApplyInScope {
                    return Err(AssemblyError::DirectApplyAboveMode);
                }
            }
        }
        self.last_batch = Some(batch.batch_seq);
        self.entries
            .push_back(TranscriptEntry::Effect(EffectEntry::BatchApplied(*batch)));
        Ok(())
    }

    /// 현재 목록(도착 순서).
    pub fn entries(&self) -> impl Iterator<Item = &TranscriptEntry> {
        self.entries.iter()
    }

    /// 상한 때문에 버린 CLI 보고 item 수.
    pub fn dropped_chat(&self) -> u64 {
        self.dropped_chat
    }

    /// 표시된 문서 효과가 승인 기록(`resolutions`, 연 요청 `opened`)·출처 기록(`batches`)과 정확히 같은지
    /// 확인합니다. 불일치 목록이 비어야 합니다.
    pub fn verify(
        &self,
        opened: &[(RequestId, ApprovalSubject)],
        resolutions: &[Resolution],
        batches: &[BatchRecord],
    ) -> Vec<Mismatch> {
        let mut expected: Vec<EffectEntry> = Vec::new();
        expected.extend(
            opened
                .iter()
                .map(|(request, subject)| EffectEntry::ApprovalRequested {
                    request: *request,
                    subject: *subject,
                }),
        );
        expected.extend(
            resolutions
                .iter()
                .filter(|r| r.session == self.session)
                .map(|r| EffectEntry::ApprovalResolved {
                    request: r.id,
                    subject: r.subject,
                    outcome: r.outcome,
                }),
        );
        expected.extend(
            batches
                .iter()
                .filter(|b| b.session == self.session)
                .map(|b| EffectEntry::BatchApplied(*b)),
        );
        let mut out = Vec::new();
        let shown: Vec<EffectEntry> = self
            .entries
            .iter()
            .filter_map(|e| match e {
                TranscriptEntry::Effect(effect) => Some(*effect),
                TranscriptEntry::Chat(_) => None,
            })
            .collect();
        for effect in &shown {
            if let Some(at) = expected.iter().position(|e| e == effect) {
                expected.remove(at);
            } else {
                out.push(Mismatch::Unrecorded(*effect));
            }
        }
        for effect in expected {
            out.push(Mismatch::Missing(
                TranscriptEntry::Effect(effect).to_golden(),
            ));
        }
        out
    }
}

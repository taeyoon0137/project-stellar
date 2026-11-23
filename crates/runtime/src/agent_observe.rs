//! 사용자 편집·외부 변경을 에이전트 출처 색인에 알리는 관찰 대기열 (XIX.6.7, XIX.7.2, ADR-0124 결정 4·5).
//!
//! 창 session의 문서 기록 함수(`ProjectWindowSession::apply` 등, 외부 반영)가 기록한 change set과 출처를 이 대기열에
//! 넣고, 모든 document thread job 뒤 상태 snapshot 게시 앞에서 [`drain_windows`]가 창마다 비웁니다.
//!
//! - **change set**: 그 창의 `AgentChangeProvenance::observe`에 넘겨 표시된 batch의 "이후 변경됨"을 판정합니다. 표시된
//!   batch가 없으면(`watch.changes == false`) 쌓지 않습니다.
//! - **외부 변경 관찰값**(base 원본, 외부 원본): 그 창 세션이 lease를 쥘 때만(`watch.external`) 쌓습니다. 비울 때
//!   `respond_to_external`이 lease 겹침을 판정해 겹친 세션을 `LeaseConflict`로 끝냅니다(자동 rollback 없음). 외부 변경을
//!   창 문서에 넣는 일은 창 session의 반영 경로 하나가 이미 했습니다.
//! - history preview(속성 scrub)는 preview 중 넣지 않고 끝에서 마지막 change set 하나를 넣습니다. Undo·Redo 재생과 에이전트
//!   batch 적용은 그 자리에서 `observe`를 부르므로 이 대기열에 넣지 않습니다.

use stellar_agent_edit::agent_history::external::respond_to_external;
use stellar_agent_edit::agent_history::{SessionContext, SessionEndReport};
use stellar_commands::change::ChangeSet;
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_document::records::DocumentRecords;
use stellar_history::agent::WindowAgentHistory;

use crate::agent_edit::{monotonic_ms, AgentWindow, WindowAgentEdit};
use crate::agent_session::fresh_id;

/// 관찰 하나.
#[derive(Debug, Clone)]
pub(crate) enum Observation {
    /// 기록한 문서 변경.
    Changes {
        changes: ChangeSet,
        origin: CommandOrigin,
    },
    /// 외부 변경(창이 마지막으로 맞춘 원본, 외부 원본).
    External(Box<(DocumentRecords, DocumentRecords)>),
}

/// 창 session의 관찰 대기열과 켜짐 여부. 에이전트 상태가 없으면 둘 다 꺼져 비용이 없습니다.
#[derive(Debug, Default)]
pub(crate) struct AgentWatch {
    /// 표시된 batch가 있어 change set을 쌓는지.
    pub(crate) changes: bool,
    /// lease를 쥔 세션이 있어 외부 변경 관찰값을 쌓는지.
    pub(crate) external: bool,
    queue: Vec<Observation>,
}

impl AgentWatch {
    /// 기록한 문서 변경을 넣습니다(켜져 있을 때만).
    pub(crate) fn note(&mut self, changes: &ChangeSet, origin: CommandOrigin) {
        if self.changes && !changes.is_empty() {
            self.queue.push(Observation::Changes {
                changes: changes.clone(),
                origin,
            });
        }
    }

    /// 외부 변경 관찰값을 넣습니다(켜져 있을 때만). 값은 켜져 있을 때만 만듭니다.
    pub(crate) fn note_external(
        &mut self,
        values: impl FnOnce() -> Option<(DocumentRecords, DocumentRecords)>,
    ) {
        if self.external {
            if let Some((base, incoming)) = values() {
                self.queue
                    .push(Observation::External(Box::new((base, incoming))));
            }
        }
    }

    fn take(&mut self) -> Vec<Observation> {
        std::mem::take(&mut self.queue)
    }
}

/// 창 하나의 관찰 대기열을 비웁니다. 외부 변경으로 끝낸 세션 보고서를 돌려줍니다.
pub(crate) fn drain(window: &mut AgentWindow<'_>) -> Vec<SessionEndReport> {
    let mut ended = Vec::new();
    for observation in window.session.agent_watch.take() {
        match observation {
            Observation::Changes { changes, origin } => {
                if let Some(document) = window.session.document() {
                    window
                        .agent
                        .provenance
                        .observe(&changes, origin, document.model());
                }
            }
            Observation::External(values) => {
                let (base, incoming) = *values;
                ended.extend(respond(window, &base, &incoming));
            }
        }
    }
    rearm(window.session, window.agent);
    ended
}

/// 외부 변경에 대응합니다: 이 창 세션의 lease와 겹치면 그 세션을 끝냅니다(문서는 바꾸지 않음).
fn respond(
    window: &mut AgentWindow<'_>,
    base: &DocumentRecords,
    incoming: &DocumentRecords,
) -> Vec<SessionEndReport> {
    let Ok(table) = window.agent.lease_table() else {
        return Vec::new();
    };
    let now = monotonic_ms();
    let entry_id = CommandId::from_stable_id(fresh_id());
    let ended = {
        let AgentWindow { session, agent } = &mut *window;
        let WindowAgentEdit {
            sessions,
            provenance,
            labels,
            permissions,
            leases,
            ..
        } = &mut **agent;
        let (Some(leases), Some(document)) = (leases.as_mut(), session.document.as_mut()) else {
            return Vec::new();
        };
        let mut sink = WindowAgentHistory {
            window: &mut session.history,
            doc: &mut document.state,
            labels,
        };
        let mut ctx = SessionContext {
            leases,
            permissions,
            provenance,
        };
        match respond_to_external(
            sessions, &mut ctx, &mut sink, base, incoming, &table, entry_id, now,
        ) {
            Ok(decision) => decision.ended,
            Err(_) => Vec::new(),
        }
    };
    for report in &ended {
        window.notify_host_ended(report, now);
    }
    ended
}

/// 대기열 켜짐 여부를 지금 에이전트 상태로 맞춥니다.
pub(crate) fn rearm(
    session: &mut crate::window_session::ProjectWindowSession,
    agent: &WindowAgentEdit,
) {
    session.agent_watch.changes = agent
        .provenance
        .batches()
        .iter()
        .any(|b| b.display().is_shown());
    session.agent_watch.external = agent
        .sessions
        .active_ids()
        .into_iter()
        .any(|id| agent.sessions.lease(id).is_some());
}

/// 모든 창의 관찰 대기열을 비웁니다(상태 snapshot 게시 앞, document thread).
pub(crate) fn drain_windows(sessions: &mut crate::app::AppSessions) {
    for entry in sessions.windows.values_mut() {
        drain(&mut AgentWindow::new(&mut entry.session, &mut entry.agent));
    }
}

#[cfg(test)]
impl AgentWatch {
    pub(crate) fn len(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
#[path = "agent_observe_tests.rs"]
mod tests;

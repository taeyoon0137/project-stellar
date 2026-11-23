//! 사용자 편집을 에이전트 출처 색인에 알리는 관찰 대기열 (XIX.6.7, ADR-0124 결정 4).
//!
//! 창 session의 문서 기록 함수(`ProjectWindowSession::apply` 등, 외부 반영)가 기록한 change set과 출처를 이 대기열에
//! 넣고, 모든 document thread job 뒤 상태 snapshot 게시 앞에서 [`drain_windows`]가 창마다 비웁니다.
//!
//! - **change set**: 그 창의 `AgentChangeProvenance::observe`에 넘겨 표시된 batch의 "이후 변경됨"을 판정합니다. 표시된
//!   batch가 없으면(`watch.changes == false`) 쌓지 않습니다.
//! - history preview(속성 scrub)는 preview 중 넣지 않고 끝에서 마지막 change set 하나를 넣습니다. Undo·Redo 재생과 에이전트
//!   batch 적용은 그 자리에서 `observe`를 부르므로 이 대기열에 넣지 않습니다.

use stellar_commands::change::ChangeSet;
use stellar_commands::id::CommandOrigin;

use crate::agent_edit::{AgentWindow, WindowAgentEdit};

/// 관찰 하나.
#[derive(Debug, Clone)]
pub(crate) enum Observation {
    /// 기록한 문서 변경.
    Changes {
        changes: ChangeSet,
        origin: CommandOrigin,
    },
}

/// 창 session의 관찰 대기열과 켜짐 여부. 에이전트 상태가 없으면 꺼져 비용이 없습니다.
#[derive(Debug, Default)]
pub(crate) struct AgentWatch {
    /// 표시된 batch가 있어 change set을 쌓는지.
    pub(crate) changes: bool,
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

    fn take(&mut self) -> Vec<Observation> {
        std::mem::take(&mut self.queue)
    }
}

/// 창 하나의 관찰 대기열을 비웁니다.
pub(crate) fn drain(window: &mut AgentWindow<'_>) {
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
        }
    }
    rearm(window.session, window.agent);
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

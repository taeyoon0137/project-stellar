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
//! - **문서 밖 의미 변경**(컴포넌트 원본·token·asset, ADR-0132 결정 4): 창 세션이 lease를 쥘 때 창 원본의 문서 밖 의미
//!   (`SourceLibrary`)를 기억해 두고, 비울 때 지금 값과 다르면 바뀐 resource(컴포넌트 정의, token·token set, asset)를
//!   StableId 단위로 셉니다. 그 resource를 전역 resource로 잠근 이 창 세션은 `LeaseConflict`로 끝냅니다. 창 원본을
//!   바꾸는 경로(외부 변경의 문서 밖 의미 받기 `adopt_library_change`, 다시 열기)와 관계없이 기억한 값과 지금 값만
//!   비교하므로 경로마다 관찰 함수를 따로 부르지 않습니다. 기억한 의미가 없던 창(cache로 열어 아직 원본을 읽지 않음)은
//!   원본 파일 hash로 문서 밖 의미 파일이 바뀌었는지만 알 수 있어, 바뀌었으면 전역 resource를 쥔 세션을 모두 끝냅니다
//!   (fail closed).
//! - **판정 실패**(ADR-0132 결정 4): lease 표를 읽지 못하거나 외부 변경 분석이 실패하면 겹침을 판정할 수 없으므로 lease를
//!   쥔 이 창 세션을 모두 `LeaseConflict`로 끝냅니다(fail closed, 자동 rollback 없음). 판정 없이 세션을 이어 가지 않습니다.
//! - history preview(속성 scrub)는 preview 중 넣지 않고 끝에서 마지막 change set 하나를 넣습니다. Undo·Redo 재생과 에이전트
//!   batch 적용은 그 자리에서 `observe`를 부르므로 이 대기열에 넣지 않습니다.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use stellar_agent_edit::agent_history::external::respond_to_external;
use stellar_agent_edit::agent_history::{EndReason, SessionContext, SessionEndReport};
use stellar_agent_edit::permission::AgentSessionId;
use stellar_commands::change::ChangeSet;
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::lock::lease::{ExternalKind, ExternalResource, LeaseTable};
use stellar_core_types::ids::StableId;
use stellar_document::component::definition::ComponentEntry;
use stellar_history::agent::WindowAgentHistory;
use stellar_ops::diff::DocumentSource;
use stellar_project::document_io::session::{is_library_path, SourceBase, SourceLibrary};
use stellar_project::document_io::source_v4::token::{TokenSetSource, TokenSource};

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
    External(Box<(DocumentSource, DocumentSource)>),
}

/// 마지막으로 본 창 원본의 문서 밖 의미(lease를 쥔 세션이 있을 때만 기억).
#[derive(Debug, Clone)]
pub(crate) struct LibrarySeen {
    /// 원본 파일 hash(chunk 전용이면 `None`).
    base: Option<SourceBase>,
    /// 문서 밖 의미(cache로 연 창은 원본을 읽기 전까지 `None`).
    library: Option<Arc<SourceLibrary>>,
}

impl LibrarySeen {
    fn of(session: &crate::window_session::ProjectWindowSession) -> LibrarySeen {
        LibrarySeen {
            base: session.source.base().cloned(),
            library: session.source.library().cloned(),
        }
    }
}

/// 문서 밖 의미 변경 판정.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LibraryChange {
    /// 바뀌지 않았습니다.
    Unchanged,
    /// 바뀐 resource(StableId 단위).
    Resources(BTreeSet<ExternalResource>),
    /// 바뀌었지만 이전 의미를 몰라 resource를 셀 수 없습니다.
    Unknown,
}

/// 기억한 값과 지금 값의 문서 밖 의미 변경(모듈 문서).
pub(crate) fn library_change(seen: &LibrarySeen, now: &LibrarySeen) -> LibraryChange {
    match (&seen.library, &now.library) {
        (Some(old), Some(new)) if Arc::ptr_eq(old, new) => LibraryChange::Unchanged,
        (Some(old), Some(new)) => {
            let changed = changed_resources(old, new);
            if changed.is_empty() {
                LibraryChange::Unchanged
            } else {
                LibraryChange::Resources(changed)
            }
        }
        // 한쪽 의미를 모르면 원본 파일 hash로 문서 밖 의미 파일이 바뀌었는지만 봅니다.
        _ => match (&seen.base, &now.base) {
            (Some(old), Some(new))
                if old
                    .changed_paths(new)
                    .iter()
                    .any(|path| is_library_path(path)) =>
            {
                LibraryChange::Unknown
            }
            _ => LibraryChange::Unchanged,
        },
    }
}

/// 두 문서 밖 의미에서 추가·삭제·내용 변경된 resource. 컴포넌트는 정의 entry(정의·tree·slot·property·상태 머신), token은
/// token 하나와 그 token set(set 이름·구성이 바뀌면 set id), asset은 image record 단위입니다.
pub(crate) fn changed_resources(
    old: &SourceLibrary,
    new: &SourceLibrary,
) -> BTreeSet<ExternalResource> {
    let resources =
        |kind, ids: Vec<StableId>| ids.into_iter().map(move |id| ExternalResource { kind, id });
    resources(
        ExternalKind::ComponentDefinition,
        diff_keys(&components(old), &components(new)),
    )
    .chain(resources(
        ExternalKind::Token,
        diff_keys(&sets(old), &sets(new)),
    ))
    .chain(resources(
        ExternalKind::Token,
        diff_keys(&tokens(old), &tokens(new)),
    ))
    .chain(resources(
        ExternalKind::Asset,
        diff_keys(&old.images, &new.images),
    ))
    .collect()
}

fn components(library: &SourceLibrary) -> BTreeMap<StableId, &ComponentEntry> {
    library
        .components
        .iter()
        .map(|entry| (entry.definition().id, entry))
        .collect()
}

fn sets(library: &SourceLibrary) -> BTreeMap<StableId, &TokenSetSource> {
    library.token_sets.iter().map(|set| (set.id, set)).collect()
}

fn tokens(library: &SourceLibrary) -> BTreeMap<StableId, (StableId, &TokenSource)> {
    library
        .token_sets
        .iter()
        .flat_map(|set| {
            set.tokens
                .iter()
                .map(move |token| (token.id, (set.id, token)))
        })
        .collect()
}

/// 한쪽에만 있거나 값이 다른 key(StableId 순).
fn diff_keys<V: PartialEq>(
    old: &BTreeMap<StableId, V>,
    new: &BTreeMap<StableId, V>,
) -> Vec<StableId> {
    old.keys()
        .chain(new.keys())
        .filter(|id| old.get(*id) != new.get(*id))
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// 창 session의 관찰 대기열과 켜짐 여부. 에이전트 상태가 없으면 둘 다 꺼져 비용이 없습니다.
#[derive(Debug, Default)]
pub(crate) struct AgentWatch {
    /// 표시된 batch가 있어 change set을 쌓는지.
    pub(crate) changes: bool,
    /// lease를 쥔 세션이 있어 외부 변경 관찰값을 쌓는지.
    pub(crate) external: bool,
    queue: Vec<Observation>,
    /// lease를 쥔 세션이 있을 때 마지막으로 본 문서 밖 의미.
    library: Option<LibrarySeen>,
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
        values: impl FnOnce() -> Option<(DocumentSource, DocumentSource)>,
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
    ended.extend(respond_library(window));
    // 앱 시작 세션의 재연결 유예를 확인합니다(ADR-0124 결정 1).
    ended.extend(window.tick_hosted(monotonic_ms()));
    rearm(window.session, window.agent);
    ended
}

/// 외부 변경에 대응합니다: 이 창 세션의 lease와 겹치면 그 세션을 끝냅니다(문서는 바꾸지 않음).
fn respond(
    window: &mut AgentWindow<'_>,
    base: &DocumentSource,
    incoming: &DocumentSource,
) -> Vec<SessionEndReport> {
    let now = monotonic_ms();
    let Ok(table) = window.agent.lease_table() else {
        // 판정할 수 없으면 lease를 쥔 이 창 세션을 끝냅니다(fail closed, 모듈 문서).
        return end_sessions(window, None, |_| true, now);
    };
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
        respond_to_external(
            sessions, &mut ctx, &mut sink, base, incoming, &table, entry_id, now,
        )
        .map(|decision| decision.ended)
    };
    match ended {
        Ok(ended) => {
            for report in &ended {
                window.notify_host_ended(report, now);
            }
            ended
        }
        // 분석·종료가 실패하면 남은 lease 세션을 끝냅니다(fail closed). 실패 전에 끝낸 세션은 이미 끝났습니다.
        Err(_) => end_sessions(window, None, |_| true, now),
    }
}

/// 문서 밖 의미 변경에 대응합니다(모듈 문서): 바뀐 resource를 전역 resource로 잠근 이 창 세션을 끝냅니다.
fn respond_library(window: &mut AgentWindow<'_>) -> Vec<SessionEndReport> {
    let Some(seen) = window.session.agent_watch.library.take() else {
        return Vec::new();
    };
    // 의미 객체가 같으면(같은 `Arc`) 원본 hash 표를 복제하지 않고 바로 끝냅니다(ADR-0132 Performance). 대기열은 모든
    // document thread job 뒤에 비우므로 이 경로가 대부분입니다.
    if let (Some(old), Some(now)) = (&seen.library, window.session.source.library()) {
        if Arc::ptr_eq(old, now) {
            window.session.agent_watch.library = Some(seen);
            return Vec::new();
        }
    }
    let current = LibrarySeen::of(window.session);
    let change = library_change(&seen, &current);
    window.session.agent_watch.library = Some(current);
    let now = monotonic_ms();
    match change {
        LibraryChange::Unchanged => Vec::new(),
        LibraryChange::Unknown => match window.agent.lease_table() {
            Ok(table) => end_sessions(window, Some(&table), |externals| !externals.is_empty(), now),
            Err(_) => end_sessions(window, None, |_| true, now),
        },
        LibraryChange::Resources(changed) => match window.agent.lease_table() {
            Ok(table) => end_sessions(
                window,
                Some(&table),
                |externals| externals.iter().any(|resource| changed.contains(resource)),
                now,
            ),
            Err(_) => end_sessions(window, None, |_| true, now),
        },
    }
}

/// lease를 쥔 이 창의 활성 세션 가운데 `pick`(그 lease의 전역 resource)이 고른 세션을 `LeaseConflict` 종료 절차로
/// 끝냅니다(자동 rollback 없음). `table`이 없으면(lease 표를 읽지 못함) 판정하지 않고 `pick`에 빈 목록을 넘깁니다.
fn end_sessions(
    window: &mut AgentWindow<'_>,
    table: Option<&LeaseTable>,
    pick: impl Fn(&[ExternalResource]) -> bool,
    now: u64,
) -> Vec<SessionEndReport> {
    let sessions = &window.agent.sessions;
    let targets: Vec<AgentSessionId> = sessions
        .active_ids()
        .into_iter()
        .filter(|id| {
            sessions.lease(*id).is_some_and(|lease| {
                let externals = table
                    .and_then(|table| table.get(lease))
                    .map_or(&[][..], |lease| lease.externals.as_slice());
                pick(externals)
            })
        })
        .collect();
    let mut ended = Vec::new();
    for id in targets {
        let entry_id = CommandId::from_stable_id(fresh_id());
        if let Ok(report) = window.end(id, EndReason::LeaseConflict, entry_id, now) {
            window.notify_host_ended(&report, now);
            ended.push(report);
        }
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
    // lease를 쥔 세션이 생기면 그때의 문서 밖 의미를 기억합니다. 없으면 버립니다(비용 0).
    if !session.agent_watch.external {
        session.agent_watch.library = None;
    } else if session.agent_watch.library.is_none() {
        session.agent_watch.library = Some(LibrarySeen::of(session));
    }
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

//! 독립 컴포넌트 파일은 창 문서의 undo/history에 속하지 않습니다.
//!
//! 컴포넌트 원본 감시(XIII.2): 신뢰된 프로젝트 session마다 [`FsEventsWatcher`] 하나를 둡니다. 감시는 그
//! 프로젝트 창의 첫 원본 읽기에서 시작하고, 프로젝트가 닫히거나 신뢰를 잃거나 창이 모두 닫히면 끝납니다.
//! 컴포넌트 원본 변경([`FileWatchEvent::affects_component_sources`])을 보면 그 프로젝트의 모든 창에 원본
//! 다시 읽기를 예약합니다. 정의가 실제로 바뀐 창에는 요청 없는 `WINDOW_VIEW_CHANGED`(request 0)를 보내고
//! event 준비 신호를 깨웁니다(모듈 [`super`]의 [`AppRuntime::event_ready_fd`]).
//!
//! [`FsEventsWatcher`]: stellar_project::watcher::fsevents::FsEventsWatcher
//! [`FileWatchEvent::affects_component_sources`]: stellar_project::watcher::FileWatchEvent::affects_component_sources
use super::*;
use crate::canvas::{CanvasComponentLibrary, LibraryOrder};
use crate::document_thread::DocumentSender;

/// 감시 thread가 프로젝트·창 상태를 다시 확인하는 주기.
#[cfg(target_os = "macos")]
const COMPONENT_WATCH_POLL: Duration = Duration::from_millis(250);

impl AppShared {
    /// 원본 읽기 하나의 시작 순서를 정합니다.
    pub(super) fn next_library_order(&self) -> LibraryOrder {
        LibraryOrder(self.library_reads.fetch_add(1, Ordering::Relaxed) + 1)
    }
}

/// 문서 thread 밖(호출 thread)에서 창 하나의 컴포넌트 원본을 읽고, document thread에서 장면 정의를 바꿉니다.
///
/// `request`가 있으면 그 완료 event를 보냅니다. 없으면(감시가 예약한 읽기) 정의가 바뀐 경우에만 요청 없는
/// `WINDOW_VIEW_CHANGED`를 보냅니다. 반영 시점에 프로젝트가 신뢰되지 않으면 정의를 비웁니다.
fn reload_window_components(
    shared: &Arc<AppShared>,
    sender: &DocumentSender,
    session: WindowSessionHandle,
    request: Option<RequestId>,
) {
    let order = shared.next_library_order();
    let state = shared.current_state();
    let read = std::panic::catch_unwind(|| {
        let window = state.window(session)?;
        state
            .project(window.project)?
            .read_canvas_components(window.component_library.as_ref())
    })
    .unwrap_or(None);
    let publish = Arc::clone(shared);
    let submitted = sender.submit(move |ctx| {
        let sessions = ctx.app_sessions_mut();
        let Some(entry) = sessions.windows.get_mut(&session) else {
            if let Some(request) = request {
                publish.outbox().push_back(RuntimeEvent::RequestFailed {
                    request,
                    failure: RequestFailure::UnknownWindowSession,
                    message: "창이 닫혔습니다".into(),
                });
            }
            return;
        };
        let trusted = sessions
            .projects
            .get(&entry.project)
            .is_some_and(|p| p.trust == TrustStatus::Trusted);
        let changed = entry.canvas.update_library(order, read.filter(|_| trusted));
        if changed || request.is_some() {
            publish.publish_state(build_state(ctx));
        }
        match request {
            Some(request) => publish
                .outbox()
                .push_back(RuntimeEvent::WindowViewChanged { request, session }),
            None if changed => publish.push_unsolicited_view_change(session),
            None => {}
        }
    });
    if submitted.is_err() {
        if let Some(request) = request {
            shared.outbox().push_back(RuntimeEvent::RequestFailed {
                request,
                failure: RequestFailure::SaveFailed,
                message: "컴포넌트 캔버스 갱신을 접수하지 못했습니다".into(),
            });
        }
    }
}

/// 신뢰된 프로젝트에 컴포넌트 원본 감시가 없으면 시작합니다(프로젝트 session마다 하나).
#[cfg(target_os = "macos")]
fn ensure_component_watch(
    shared: &Arc<AppShared>,
    sender: &DocumentSender,
    project: ProjectSessionId,
) {
    use stellar_project::watcher::fsevents::FsEventsWatcher;
    use stellar_project::watcher::{FileWatchEvent, ProjectFileWatcher};

    let state = shared.current_state();
    let Some(summary) = state
        .project(project)
        .filter(|p| p.trust == TrustStatus::Trusted)
    else {
        return;
    };
    let mut watches = shared
        .component_watches
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if watches.contains_key(&project) {
        return;
    }
    let Ok(mut watcher) = FsEventsWatcher::start(&summary.canonical_root, &[]) else {
        return;
    };
    let token = Arc::new(());
    watches.insert(project, Arc::clone(&token));
    drop(watches);
    // runtime이 끝나면 AppShared가 사라지므로 약한 참조로 감시를 끝냅니다.
    let weak = Arc::downgrade(shared);
    let sender = sender.clone();
    let live_windows = move |shared: &AppShared| -> Option<Vec<WindowSessionHandle>> {
        let state = shared.current_state();
        state
            .project(project)
            .filter(|p| p.trust == TrustStatus::Trusted)?;
        let windows: Vec<_> = state
            .windows
            .iter()
            .filter(|w| w.project == project)
            .map(|w| w.session)
            .collect();
        (!windows.is_empty()).then_some(windows)
    };
    let spawned = std::thread::Builder::new()
        .name("stellar-component-watch".into())
        .spawn(move || loop {
            let events = watcher.wait(COMPONENT_WATCH_POLL);
            let Some(shared) = weak.upgrade() else {
                break;
            };
            let Some(windows) = live_windows(&shared) else {
                // 새 창이 감시를 찾는 동안 끝내지 않도록 표를 잠근 채 다시 확인합니다.
                let mut watches = shared
                    .component_watches
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                if live_windows(&shared).is_some() {
                    continue;
                }
                if watches
                    .get(&project)
                    .is_some_and(|own| Arc::ptr_eq(own, &token))
                {
                    watches.remove(&project);
                }
                break;
            };
            if events.iter().any(FileWatchEvent::affects_component_sources) {
                for session in windows {
                    reload_window_components(&shared, &sender, session, None);
                }
            }
        });
    if spawned.is_err() {
        shared
            .component_watches
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&project);
    }
}

/// 프로젝트 범위 디스크 작업은 document thread 밖에서 실행합니다.
impl AppRuntime {
    /// 파일 읽기는 별도 worker, 메모리 장면 교체는 document thread에서 수행합니다. 신뢰된 프로젝트면 그
    /// 프로젝트의 컴포넌트 원본 감시도 시작합니다(이미 있으면 그대로).
    pub fn reload_canvas_components(
        &self,
        session: WindowSessionHandle,
    ) -> Result<RequestId, RequestError> {
        let request = self.reserve_request()?;
        let sender = self.sender.clone();
        let shared = Arc::clone(&self.shared);
        let spawned = std::thread::Builder::new()
            .name("stellar-canvas-components".into())
            .spawn(move || {
                reload_window_components(&shared, &sender, session, Some(request));
                #[cfg(target_os = "macos")]
                if let Some(window) = shared.current_state().window(session) {
                    ensure_component_watch(&shared, &sender, window.project);
                }
            });
        if spawned.is_err() {
            self.shared.release_one();
            return Err(RequestError::Stopped);
        }
        Ok(request)
    }
}

impl ProjectSummary {
    /// 원본 파일과 라이브러리의 조회는 worker에서 수행합니다.
    fn read_canvas_components(
        &self,
        previous: Option<&Arc<CanvasComponentLibrary>>,
    ) -> Option<Arc<CanvasComponentLibrary>> {
        if self.trust != TrustStatus::Trusted {
            return None;
        }
        self.verify_trust().ok()?;
        let snapshot = stellar_ops::snapshot::open_snapshot(&self.canonical_root).ok()?;
        if snapshot.project_id() != self.project_id {
            return None;
        }
        let revision = snapshot.revision();
        if let Some(cached) = previous.filter(|cached| cached.revision == revision) {
            let observed = stellar_ops::compare::working_component_hash(&snapshot).ok()?;
            if observed == cached.source_hash {
                self.verify_trust().ok()?;
                return Some(Arc::clone(cached));
            }
        }
        // 창 세션의 신뢰 상태로 symlink 읽기 정책을 정합니다(ADR-0061 결정 8).
        let (definitions, files) = stellar_ops::component_source::project_library_with(
            &snapshot,
            stellar_ops::component_source::ExternalAccess::from_trust(self.trust),
        )
        .ok()?;
        let source_hash =
            stellar_project::document_io::component_source::source_set_hash(files.into_values())
                .ok()?;
        self.verify_trust().ok()?;
        Some(Arc::new(CanvasComponentLibrary {
            revision,
            source_hash,
            definitions,
        }))
    }
}

#[cfg(test)]
mod watch_tests {
    use super::*;
    use crate::app_window_tests::{close_window, next_event, open_project, open_window};
    use crate::window_session::test_fixtures::saved_project;

    #[test]
    fn reload_without_a_request_reports_only_changed_definitions() {
        let dir = saved_project("component-reload-unchanged");
        let app = AppRuntime::start().unwrap();
        let (a, _, _) = open_window(&app, open_project(&app, dir.path()));
        // 신뢰되지 않은 프로젝트는 정의가 없고 그대로 없으므로 요청 없는 event를 만들지 않습니다.
        reload_window_components(&app.shared, &app.sender, a, None);
        let request = app.reload_canvas_components(a).unwrap();
        assert_eq!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged {
                request,
                session: a
            }
        );
        assert_eq!(app.pop_event(), None);
        app.shutdown().unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn one_component_watch_per_trusted_project_ends_with_its_windows() {
        let dir = saved_project("component-watch-lifecycle");
        let app = AppRuntime::start().unwrap();
        let owner = open_project(&app, dir.path());
        let (a, _, _) = open_window(&app, owner);
        let (b, _, _) = open_window(&app, owner);
        let watches = || {
            app.shared
                .component_watches
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        };
        ensure_component_watch(&app.shared, &app.sender, owner);
        assert!(
            watches().is_empty(),
            "신뢰하지 않은 프로젝트는 감시하지 않음"
        );
        // 신뢰 상태를 document thread에서 바꾸고 다음 요청으로 상태를 게시합니다.
        app.sender
            .submit(move |ctx| {
                ctx.app_sessions_mut()
                    .projects
                    .get_mut(&owner)
                    .unwrap()
                    .trust = TrustStatus::Trusted;
            })
            .unwrap()
            .wait()
            .unwrap();
        let request = app.set_selection(a, Vec::new()).unwrap();
        assert_eq!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged {
                request,
                session: a
            }
        );
        ensure_component_watch(&app.shared, &app.sender, owner);
        let first = watches();
        ensure_component_watch(&app.shared, &app.sender, owner);
        let second = watches();
        assert_eq!(first.len(), 1);
        assert!(
            Arc::ptr_eq(&first[&owner], &second[&owner]),
            "프로젝트당 하나"
        );
        close_window(&app, a);
        close_window(&app, b);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !watches().is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "창이 모두 닫혀도 감시가 남음"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        app.shutdown().unwrap();
    }
}

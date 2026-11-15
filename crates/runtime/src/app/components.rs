//! 독립 컴포넌트 파일은 창 문서의 undo/history에 속하지 않습니다.
//!
//! 원본 읽기와 감시는 신뢰와 무관합니다(D1, ADR-0061 결정 8). 신뢰 상태는 symlink 정책
//! ([`ExternalAccess::from_trust`])만 정합니다. 신뢰하지 않은 프로젝트는 프로젝트 안 원본을 읽고, 프로젝트 밖
//! target의 내용은 읽지 않습니다.
//!
//! 컴포넌트 원본 감시(XIII.2): 프로젝트 session마다 [`FsEventsWatcher`] 하나를 둡니다. 감시는 그 프로젝트 창의
//! 첫 원본 읽기에서 시작하고, 프로젝트가 닫히거나 창이 모두 닫히면 끝납니다.
//! 컴포넌트 원본 변경([`FileWatchEvent::affects_component_sources`])을 보면 그 프로젝트의 모든 창에 원본
//! 다시 읽기를 예약합니다. 정의가 실제로 바뀐 창에는 요청 없는 `WINDOW_VIEW_CHANGED`(request 0)를 보내고
//! event 준비 신호를 깨웁니다(모듈 [`super`]의 [`AppRuntime::event_ready_fd`]).
//!
//! [`FsEventsWatcher`]: stellar_project::watcher::fsevents::FsEventsWatcher
//! [`FileWatchEvent::affects_component_sources`]: stellar_project::watcher::FileWatchEvent::affects_component_sources
//! [`ExternalAccess::from_trust`]: stellar_ops::component_source::ExternalAccess::from_trust
use super::*;
use crate::canvas::{CanvasComponentLibrary, LibraryOrder};
use crate::document_thread::DocumentSender;
use stellar_ops::component_source::ExternalAccess;

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
/// `WINDOW_VIEW_CHANGED`를 보냅니다. 신뢰한 정책(외부 target 읽기 허용)으로 읽은 결과는 반영 시점에 신뢰가
/// 철회됐으면 버립니다. 철회 뒤 셸이 다시 요청한 읽기는 외부 target을 읽지 않는 정책으로 정의를 채웁니다.
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
        let read = read.filter(|library| trusted || library.access != ExternalAccess::Allowed);
        let changed = entry.canvas.update_library(order, read);
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

/// 프로젝트에 컴포넌트 원본 감시가 없으면 시작합니다(프로젝트 session마다 하나, 신뢰와 무관).
#[cfg(target_os = "macos")]
fn ensure_component_watch(
    shared: &Arc<AppShared>,
    sender: &DocumentSender,
    project: ProjectSessionId,
) {
    use stellar_project::watcher::fsevents::FsEventsWatcher;
    use stellar_project::watcher::{FileWatchEvent, ProjectFileWatcher};

    let state = shared.current_state();
    let Some(summary) = state.project(project) else {
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
        state.project(project)?;
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
    /// 파일 읽기는 별도 worker, 메모리 장면 교체는 document thread에서 수행합니다. 그 프로젝트의 컴포넌트
    /// 원본 감시도 시작합니다(이미 있으면 그대로).
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
    /// 원본 파일과 라이브러리의 조회는 worker에서 수행합니다. 신뢰와 무관하게 읽고, 창 세션의 신뢰 상태로
    /// symlink 읽기 정책만 정합니다(ADR-0061 결정 8). 외부 target을 읽는 정책이면 OS 신뢰 저장소도 다시
    /// 확인합니다.
    fn read_canvas_components(
        &self,
        previous: Option<&Arc<CanvasComponentLibrary>>,
    ) -> Option<Arc<CanvasComponentLibrary>> {
        let access = ExternalAccess::from_trust(self.trust);
        let verify = || match access {
            ExternalAccess::Allowed => self.verify_trust(),
            ExternalAccess::MetadataOnly => self.verify_location(),
        };
        verify().ok()?;
        let snapshot = stellar_ops::snapshot::open_snapshot(&self.canonical_root).ok()?;
        if snapshot.project_id() != self.project_id {
            return None;
        }
        let revision = snapshot.revision();
        if let Some(cached) =
            previous.filter(|cached| cached.revision == revision && cached.access == access)
        {
            let observed = stellar_ops::compare::working_component_hash(&snapshot).ok()?;
            if observed == cached.source_hash {
                verify().ok()?;
                return Some(Arc::clone(cached));
            }
        }
        let (definitions, files) =
            stellar_ops::component_source::project_library_with(&snapshot, access).ok()?;
        let source_hash =
            stellar_project::document_io::component_source::source_set_hash(files.into_values())
                .ok()?;
        verify().ok()?;
        Some(Arc::new(CanvasComponentLibrary {
            revision,
            source_hash,
            access,
            definitions,
        }))
    }
}

#[cfg(test)]
mod watch_tests {
    use super::*;
    use crate::app_window_tests::{close_window, next_event, open_project, open_window};
    use crate::window_session::test_fixtures::{
        sample_document, saved_project, saved_project_with, sid,
    };
    use stellar_core_types::reference::ComponentRef;
    use stellar_document::component::instance::ComponentInstancePayload;
    use stellar_document::records::NodePayload;
    use stellar_project::document_io::component_source::{create, ComponentSource};

    /// 문서가 끝날 때까지 기다립니다(앞서 접수한 document thread 작업이 모두 끝남).
    fn drain(app: &AppRuntime) {
        app.sender.submit(|_| {}).unwrap().wait().unwrap();
    }

    #[test]
    fn reload_without_a_request_reports_only_changed_definitions() {
        let dir = saved_project("component-reload-unchanged");
        let app = AppRuntime::start().unwrap();
        let (a, _, _) = open_window(&app, open_project(&app, dir.path()));
        let request = app.reload_canvas_components(a).unwrap();
        assert_eq!(
            next_event(&app),
            RuntimeEvent::WindowViewChanged {
                request,
                session: a
            }
        );
        // 원본이 그대로면 요청 없는 읽기는 event를 만들지 않습니다.
        reload_window_components(&app.shared, &app.sender, a, None);
        drain(&app);
        assert_eq!(app.pop_event(), None);
        app.shutdown().unwrap();
    }

    /// 제한 모드(신뢰하지 않은 프로젝트)도 프로젝트 안 원본을 읽어 인스턴스를 그립니다(D1, ADR-0061 결정 8).
    /// 프로젝트 밖 target의 내용만 읽지 않습니다.
    #[cfg(unix)]
    #[test]
    fn restricted_mode_draws_instances_but_not_external_sources() {
        let component = StableId::new_v7(7, 0, 1);
        let mut model = sample_document();
        let mut record = model.record(sid(11)).unwrap();
        record.id = sid(30);
        record.parent = Some(sid(11));
        record.payload = NodePayload::ComponentInstance(ComponentInstancePayload::new(
            ComponentRef::new(component),
        ));
        model.insert_node(record).unwrap();
        let source =
            ComponentSource::new(component, "Card".into(), StableId::new_v7(7, 0, 2)).unwrap();
        let instance_rows = |external: bool| {
            let dir = saved_project_with("component-restricted", &[&model]);
            let root = dir.path().canonicalize().unwrap();
            let outside = crate::window_session::test_fixtures::TempDir::new("component-outside");
            let folder = if external {
                let folder = outside.path().canonicalize().unwrap();
                std::os::unix::fs::symlink(&folder, root.join("design/components")).unwrap();
                folder
            } else {
                let folder = root.join("design/components");
                std::fs::create_dir_all(&folder).unwrap();
                folder
            };
            create(&folder, &source).unwrap();
            let app = AppRuntime::start().unwrap();
            let project = open_project(&app, dir.path());
            assert_eq!(
                app.state().project(project).unwrap().trust,
                TrustStatus::NotTrusted
            );
            let (a, _, _) = open_window(&app, project);
            let request = app.reload_canvas_components(a).unwrap();
            assert_eq!(
                next_event(&app),
                RuntimeEvent::WindowViewChanged {
                    request,
                    session: a
                }
            );
            let state = app.state();
            let scene = state.window(a).unwrap().canvas.clone().unwrap();
            let rows = scene
                .frames
                .iter()
                // 입력 대상이어야 캔버스에서 고르고 옮길 수 있습니다.
                .filter(|frame| frame.instance_frame && frame.hit_target && frame.id == sid(30))
                .count();
            app.shutdown().unwrap();
            (rows, scene.skipped)
        };
        assert_eq!(
            instance_rows(false),
            (1, 0),
            "프로젝트 안 원본은 제한 모드에서도 그림"
        );
        assert_eq!(
            instance_rows(true),
            (0, 1),
            "프로젝트 밖 원본 내용은 읽지 않음"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn one_component_watch_per_project_ends_with_its_windows() {
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
        // 신뢰하지 않은 프로젝트도 감시합니다(D1).
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

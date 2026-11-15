//! 독립 컴포넌트 파일은 창 문서의 undo/history에 속하지 않습니다.
use super::*;
use crate::canvas::CanvasComponentLibrary;

/// 프로젝트 범위 디스크 작업은 document thread 밖에서 실행합니다.
impl AppRuntime {
    /// 별도 worker에서 신뢰를 확인하고 감시를 시작한 뒤 살아 있는 창에 연결합니다.
    #[cfg(target_os = "macos")]
    pub fn component_file_watch(
        &self,
        handle: WindowSessionHandle,
    ) -> impl FnOnce() -> Option<std::os::unix::net::UnixDatagram> + Send + 'static {
        let shared = Arc::clone(&self.shared);
        let sender = self.sender.clone();
        move || {
            let state = shared.current_state();
            let project = state.project(state.window(handle)?.project)?;
            if project.trust != TrustStatus::Trusted {
                return None;
            }
            project.verify_trust().ok()?;
            let watch = stellar_project::watcher::fsevents::ComponentFileWatch::start(
                &project.canonical_root,
            )
            .ok()?;
            sender
                .submit(move |ctx| {
                    let sessions = ctx.app_sessions_mut();
                    let window = sessions.windows.get_mut(&handle)?;
                    if sessions.projects.get(&window.project)?.trust != TrustStatus::Trusted {
                        return None;
                    }
                    window
                        .component_watch
                        .get_or_insert(watch)
                        .1
                        .try_clone()
                        .ok()
                })
                .ok()?
                .wait()
                .ok()?
        }
    }

    /// 파일 읽기는 별도 worker, 메모리 장면 교체는 document thread에서 수행합니다.
    pub fn reload_canvas_components(
        &self,
        session: WindowSessionHandle,
    ) -> Result<RequestId, RequestError> {
        let request = self.reserve_request()?;
        let state = self.state();
        let sender = self.sender.clone();
        let shared = Arc::clone(&self.shared);
        let spawned = std::thread::Builder::new()
            .name("stellar-canvas-components".into())
            .spawn(move || {
                let read = std::panic::catch_unwind(|| {
                    let window = state.window(session)?;
                    state
                        .project(window.project)?
                        .read_canvas_components(window.component_library.as_ref())
                })
                .unwrap_or(None);
                let publish = Arc::clone(&shared);
                if sender
                    .submit(move |ctx| {
                        let sessions = ctx.app_sessions_mut();
                        let result = match sessions.windows.get_mut(&session) {
                            Some(entry) => {
                                let trusted = sessions
                                    .projects
                                    .get(&entry.project)
                                    .is_some_and(|p| p.trust == TrustStatus::Trusted);
                                entry
                                    .canvas
                                    .update_library(request.0, read.filter(|_| trusted));
                                publish.publish_state(build_state(ctx));
                                RuntimeEvent::WindowViewChanged { request, session }
                            }
                            None => RuntimeEvent::RequestFailed {
                                request,
                                failure: RequestFailure::UnknownWindowSession,
                                message: "창이 닫혔습니다".into(),
                            },
                        };
                        publish.outbox().push_back(result);
                    })
                    .is_err()
                {
                    shared.outbox().push_back(RuntimeEvent::RequestFailed {
                        request,
                        failure: RequestFailure::SaveFailed,
                        message: "컴포넌트 캔버스 갱신을 접수하지 못했습니다".into(),
                    });
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

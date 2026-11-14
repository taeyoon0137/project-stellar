//! 독립 컴포넌트 파일은 창 문서의 undo/history에 속하지 않습니다.
use super::*;
use stellar_ops::revision::ProjectRevision;
use stellar_project::document_io::component_source::ComponentSource;

/// 프로젝트 범위 디스크 작업은 document thread 밖에서 실행합니다.
impl AppRuntime {
    /// 창이 가리키는 실제 프로젝트의 신뢰를 다시 검사한 뒤 이름을 가진 정의 한 개를 만듭니다.
    pub fn create_component(
        &self,
        session: WindowSessionHandle,
        name: String,
    ) -> Result<RequestId, RequestError> {
        self.create_named_component(session, name, None)
    }

    /// 이름 sheet를 열 때 표시한 단일 Frame을 현재 문서에서 복제합니다.
    pub fn create_component_from_selection(
        &self,
        session: WindowSessionHandle,
        name: String,
        view: StableId,
        revision: DocumentRevision,
        selected: StableId,
    ) -> Result<RequestId, RequestError> {
        self.create_named_component(session, name, Some((view, revision, selected)))
    }

    fn create_named_component(
        &self,
        session: WindowSessionHandle,
        name: String,
        selection: Option<(StableId, DocumentRevision, StableId)>,
    ) -> Result<RequestId, RequestError> {
        let request = self.reserve_request()?;
        if self.shared.creating_component.swap(true, Ordering::AcqRel) {
            self.shared.release_one();
            return Err(RequestError::QueueFull);
        }
        let sender = self.sender.clone();
        let shared = Arc::clone(&self.shared);
        let spawned = std::thread::Builder::new()
            .name("stellar-component-create".into())
            .spawn(move || {
                let create = || -> Result<RuntimeEvent, Failure> {
                    let captured = if let Some((view, revision, node)) = selection {
                        let name = name.clone();
                        let ticket = sender
                            .submit(move |ctx| {
                                let window =
                                    super::frame_move::trusted_canvas_window(ctx, session)?;
                                capture_selection(window, name, view, revision, node)
                            })
                            .map_err(|e| Failure::new(RequestFailure::SaveFailed, e.to_string()))?;
                        Some(ticket.wait().map_err(|e| {
                            Failure::new(RequestFailure::SaveFailed, e.to_string())
                        })??)
                    } else {
                        None
                    };
                    let state = shared.current_state();
                    let project = state
                        .window(session)
                        .and_then(|w| state.project(w.project))
                        .ok_or_else(|| unknown_window(session))?;
                    if project.trust != TrustStatus::Trusted {
                        return Err(Failure::new(
                            RequestFailure::TrustUnavailable,
                            "컴포넌트를 만들려면 프로젝트를 신뢰해야 합니다",
                        ));
                    }
                    project
                        .verify_trust()
                        .map_err(|e| Failure::new(RequestFailure::TrustUnavailable, e))?;
                    let info = match captured {
                        Some((source, base)) => {
                            stellar_ops::component_source::create_project_source(
                                &project.canonical_root,
                                project.project_id,
                                &source,
                                Some(base),
                            )
                        }
                        None => stellar_ops::component_source::create_in_project(
                            &project.canonical_root,
                            project.project_id,
                            name,
                        ),
                    }
                    .map_err(|e| Failure::new(RequestFailure::SaveFailed, e.to_string()))?;
                    let filename = info
                        .file
                        .file_name()
                        .and_then(|n| n.to_str())
                        .expect("validated component filename")
                        .into();
                    Ok(RuntimeEvent::ComponentCreated {
                        request,
                        session,
                        filename,
                    })
                };
                let result = std::panic::catch_unwind(create).unwrap_or_else(|_| {
                    Err(Failure::new(
                        RequestFailure::SaveFailed,
                        "컴포넌트 파일을 기록할 수 없습니다",
                    ))
                });
                let event = result.unwrap_or_else(|e| RuntimeEvent::RequestFailed {
                    request,
                    failure: e.failure,
                    message: e.message,
                });
                shared.creating_component.store(false, Ordering::Release);
                shared.outbox().push_back(event);
            });
        if spawned.is_err() {
            self.shared
                .creating_component
                .store(false, Ordering::Release);
            self.shared.release_one();
            return Err(RequestError::Stopped);
        }
        Ok(request)
    }
}

fn capture_selection(
    window: &ProjectWindowSession,
    name: String,
    view: StableId,
    revision: DocumentRevision,
    node: StableId,
) -> Result<(ComponentSource, ProjectRevision), Failure> {
    if window.tabs().active_view() != Some(view)
        || window.revision() != revision
        || window.view().selection().ids() != [node]
    {
        return Err(Failure::new(
            RequestFailure::UnknownView,
            "선택이나 문서가 변경되었습니다. Frame을 다시 선택하세요",
        ));
    }
    let model = window
        .document()
        .ok_or_else(|| Failure::new(RequestFailure::UnknownView, "문서가 없습니다"))?
        .model();
    let source = ComponentSource::from_selection(model, view, node, name)
        .map_err(|e| Failure::new(RequestFailure::SaveFailed, e.to_string()))?;
    let base = window
        .loaded_generation()
        .map_or(ProjectRevision::Unsaved, |g| ProjectRevision::Saved {
            generation: g.manifest.generation,
            manifest_hash: g.manifest_hash,
        });
    Ok((source, base))
}

impl ProjectSummary {
    /// 창이 표시했던 전체 ID와 원본 hash로 프로젝트 파일 하나를 조건부 저장합니다.
    pub fn resize_component_root(
        &self,
        id: StableId,
        hash: &str,
        width: f32,
        height: f32,
    ) -> Result<(), String> {
        if self.trust != TrustStatus::Trusted {
            return Err("프로젝트 신뢰가 필요합니다".into());
        }
        self.verify_trust()?;
        let size = stellar_core_types::value::Size2::new(width, height)
            .map_err(|_| "유한한 비음수 크기가 필요합니다")?;
        let hash = hash.parse().map_err(|_| "잘못된 원본 hash입니다")?;
        stellar_ops::component_source::resize_project_root(
            &self.canonical_root,
            self.project_id,
            id,
            hash,
            size,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    /// 사용자가 연 창의 프로젝트 식별자를 고정하고 폴더의 현재 원본을 읽습니다.
    /// 호출자는 UI/document thread 밖에서 실행해야 합니다.
    pub fn component_library(
        &self,
    ) -> Result<stellar_ops::component_source::ProjectComponents, String> {
        if self.trust != TrustStatus::Trusted {
            return Err("프로젝트 신뢰가 필요합니다".into());
        }
        self.verify_trust()?;
        let snapshot = stellar_ops::snapshot::open_snapshot(&self.canonical_root)
            .map_err(|e| e.to_string())?;
        if snapshot.project_id() != self.project_id {
            return Err("열린 프로젝트 ID가 바뀌었습니다".into());
        }
        let library = stellar_ops::component_source::query(&snapshot).map_err(|e| e.to_string())?;
        self.verify_trust()?;
        Ok(library)
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use crate::window_session::test_fixtures::{
        identity, sample_document, saved_project_with, sid, window,
    };
    use stellar_commands::selection::Selection;
    use stellar_document::{
        fragment::FragmentPayload,
        frame::FramePayload,
        modifier::{ModifierId, ModifierKind, ModifierStack},
        records::NodePayload,
    };

    #[test]
    fn selection_identity_generation_and_unsupported_data_are_checked_before_writing() {
        use stellar_core_types::reference::NodeRef;
        use stellar_document::frame::Clipping;
        let model = sample_document();
        let dir = saved_project_with("component-selection", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        session.select(Selection::new([sid(11)], None)).unwrap();
        let revision = session.revision();
        let undo = session.history().history().len();
        for (view, rev, selected) in [
            (3, revision, 12),
            (4, revision, 11),
            (3, DocumentRevision(revision.0 + 1), 11),
        ] {
            assert!(
                capture_selection(&session, "Card".into(), sid(view), rev, sid(selected)).is_err()
            );
        }
        let (source, _) = capture_selection(&session, "Card".into(), sid(3), revision, sid(11))
            .unwrap_or_else(|e| panic!("{}", e.message));
        assert!(stellar_ops::component_source::create_project_source(
            dir.path(),
            session.identity().project_id(),
            &source,
            Some(ProjectRevision::Unsaved)
        )
        .is_err());
        assert!(!dir.path().join("design/components").exists());
        assert_eq!(session.view().selection().ids(), [sid(11)]);
        assert_eq!(session.history().history().len(), undo);
        assert_eq!(
            session.document().unwrap().model().to_records().unwrap(),
            model.to_records().unwrap()
        );
        for restriction in 0..4 {
            let mut model = sample_document();
            let mut child = model.remove_leaf(sid(12)).unwrap();
            child.parent = Some(sid(11));
            child.payload = match restriction {
                0 => NodePayload::Fragment(FragmentPayload),
                1 => NodePayload::Frame(FramePayload {
                    layout_mode: Some(sid(400)),
                    ..FramePayload::default()
                }),
                2 => NodePayload::Frame(FramePayload {
                    clipping: Clipping::Path(NodeRef::new(sid(400))),
                    ..FramePayload::default()
                }),
                _ => NodePayload::Frame(FramePayload::default()),
            };
            model.insert_node(child).unwrap();
            if restriction == 3 {
                let mut stack = ModifierStack::new(sid(12));
                stack
                    .push_back(ModifierId::new(sid(90)), ModifierKind::Fill)
                    .unwrap();
                model.set_modifier_stack(stack).unwrap();
            }
            assert!(
                ComponentSource::from_selection(&model, sid(3), sid(11), "Card".into()).is_err()
            );
        }
    }
}

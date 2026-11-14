//! 독립 컴포넌트 파일은 창 문서의 undo/history에 속하지 않습니다.
use super::*;

/// 프로젝트 범위 디스크 작업은 document thread 밖에서 실행합니다.
impl AppRuntime {
    /// 창이 가리키는 실제 프로젝트의 신뢰를 다시 검사한 뒤 이름을 가진 정의 한 개를 만듭니다.
    pub fn create_component(
        &self,
        session: WindowSessionHandle,
        name: String,
    ) -> Result<RequestId, RequestError> {
        let request = self.reserve_request()?;
        if self.shared.creating_component.swap(true, Ordering::AcqRel) {
            self.shared.release_one();
            return Err(RequestError::QueueFull);
        }
        let shared = Arc::clone(&self.shared);
        let spawned = std::thread::Builder::new()
            .name("stellar-component-create".into())
            .spawn(move || {
                let create = || -> Result<RuntimeEvent, Failure> {
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
                    let info = stellar_ops::component_source::create_in_project(
                        &project.canonical_root,
                        project.project_id,
                        name,
                    )
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

impl ProjectSummary {
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

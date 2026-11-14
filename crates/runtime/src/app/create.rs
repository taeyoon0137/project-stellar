//! 새 프로젝트 저장·Git 초기화는 창 편집 thread 밖에서 실행합니다.
use super::*;
use stellar_git_porcelain::discovery::{discover_git, GitDiscoveryConfig};
use stellar_git_porcelain::runner::GitRunner;
use stellar_git_porcelain::version::probe_git_version;
use stellar_project::create::{
    create_editable_project, IncompleteReason, ProjectCreateRequest, ProjectCreated,
};

pub(super) fn project_git(
) -> Result<(GitRunner, stellar_git_porcelain::version::GitVersion), String> {
    let git = discover_git(&GitDiscoveryConfig::default()).ok_or("시스템 Git을 찾지 못했습니다")?;
    let version = probe_git_version(&git).map_err(|e| format!("Git 버전 확인 실패: {e:?}"))?;
    let runner = GitRunner::new(
        git,
        [
            "HOME",
            "XDG_CONFIG_HOME",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_SYSTEM",
            "GIT_CONFIG_NOSYSTEM",
            "PATH",
            "GIT_AUTHOR_NAME",
            "GIT_AUTHOR_EMAIL",
            "GIT_COMMITTER_NAME",
            "GIT_COMMITTER_EMAIL",
            "EMAIL",
        ]
        .into_iter()
        .map(Into::into)
        .collect(),
        Duration::from_secs(30),
    );
    Ok((runner, version))
}

fn initialize(request: &ProjectCreateRequest) -> Result<u32, String> {
    let (runner, version) = project_git()?;
    Ok(
        match create_editable_project(request, &runner, &version).map_err(|e| e.to_string())? {
            ProjectCreated::Created { .. } => 0,
            ProjectCreated::InitializationIncomplete { reason, .. } => match reason {
                IncompleteReason::MissingIdentity { .. } => 1,
                IncompleteReason::AddFailed(_) => 2,
                IncompleteReason::CommitFailed(_) => 3,
            },
        },
    )
}

impl AppRuntime {
    /// 새 폴더에 편집 가능한 첫 세대를 만듭니다. 완료 후 기존 open_project 경로로 열 수 있습니다.
    pub fn create_project(&self, target: &Path) -> Result<RequestId, RequestError> {
        validate_project_path(target)?;
        let name =
            target
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or(RequestError::InvalidPath {
                    reason: "새 폴더 이름이 필요합니다",
                })?;
        let parent = target.parent().ok_or(RequestError::InvalidPath {
            reason: "부모 폴더가 필요합니다",
        })?;
        let input = ProjectCreateRequest {
            parent_dir: parent.into(),
            folder_name: name.into(),
            project_name: name.into(),
        };
        // 느린 Git 명령이 반복 요청 수만큼 worker를 만들지 않도록 한 번에 한 생성만 접수합니다.
        let request = self.reserve_request()?;
        if self.shared.creating.swap(true, Ordering::AcqRel) {
            self.shared.release_one();
            return Err(RequestError::QueueFull);
        }
        let shared = Arc::clone(&self.shared);
        if std::thread::Builder::new()
            .name("stellar-project-create".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| initialize(&input))
                    .unwrap_or_else(|_| Err("프로젝트 초기화 작업이 중단되었습니다".into()));
                let event = match result {
                    Ok(git_state) => RuntimeEvent::ProjectCreated { request, git_state },
                    Err(message) => RuntimeEvent::RequestFailed {
                        request,
                        failure: RequestFailure::ProjectCreateFailed,
                        message,
                    },
                };
                shared.creating.store(false, Ordering::Release);
                shared.outbox().push_back(event);
            })
            .is_err()
        {
            self.shared.creating.store(false, Ordering::Release);
            self.shared.release_one();
            return Err(RequestError::Stopped);
        }
        Ok(request)
    }
}

//! 새 프로젝트 저장·초기화와 Git clone은 창 편집 thread 밖에서 실행합니다.
use super::*;
use stellar_git_porcelain::cancel::GitCancelToken;
use stellar_git_porcelain::discovery::{discover_git, GitDiscoveryConfig};
use stellar_git_porcelain::remote::credentials::CredentialDelegation;
use stellar_git_porcelain::repo::RepoError;
use stellar_git_porcelain::runner::GitRunError;
use stellar_git_porcelain::runner::GitRunner;
use stellar_git_porcelain::version::probe_git_version;
use stellar_project::clone::{clone_project_cancellable, CloneError, CloneRequest, SensitiveUrl};
use stellar_project::create::{
    create_editable_project, IncompleteReason, ProjectCreateRequest, ProjectCreated,
};

/// 다른 새 프로젝트 생성이 끝나기 전에 들어온 생성 요청의 실패 문구.
const CREATE_BUSY_MESSAGE: &str = "다른 새 프로젝트를 만드는 중입니다. 끝난 뒤 다시 시도하세요";

/// 다른 복제가 끝나기 전에 들어온 복제 요청의 실패 문구.
const CLONE_BUSY_MESSAGE: &str = "다른 프로젝트를 복제하는 중입니다. 끝난 뒤 다시 시도하세요";

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
    /// 새 폴더에 복제한 뒤 일반 Open과 같은 제한 모드 검사로 프로젝트 session을 엽니다.
    pub fn clone_project(&self, remote: String, target: &Path) -> Result<RequestId, RequestError> {
        validate_project_path(target)?;
        let folder =
            target
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or(RequestError::InvalidPath {
                    reason: "새 폴더 이름이 필요합니다",
                })?;
        if folder.starts_with('.')
            || stellar_project::folder_name::validate_folder_name(folder).is_err()
            || remote.is_empty()
            || remote.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(RequestError::InvalidPath {
                reason: "Clone 주소 또는 대상 이름이 잘못되었습니다",
            });
        }
        let parent = target.parent().ok_or(RequestError::InvalidPath {
            reason: "상위 폴더가 필요합니다",
        })?;
        let input = CloneRequest {
            url: SensitiveUrl::new(remote),
            parent_dir: parent.into(),
            folder_name: folder.into(),
        };
        let request = self.reserve_request()?;
        let token = GitCancelToken::new();
        let busy = {
            let mut active = self
                .shared
                .cloning
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let busy = active.is_some();
            if !busy {
                *active = Some((request, token.clone()));
            }
            busy
        };
        if busy {
            // 생성과 같은 규칙: 한 번에 한 복제만 실행하고, 겹친 요청은 이유를 담은 실패 event로 끝냅니다.
            self.shared.outbox().push_back(RuntimeEvent::RequestFailed {
                request,
                failure: RequestFailure::ProjectCloneFailed,
                message: CLONE_BUSY_MESSAGE.into(),
            });
            return Ok(request);
        }
        let sender = self.sender.clone();
        let shared = Arc::clone(&self.shared);
        let spawned = std::thread::Builder::new()
            .name("stellar-project-clone".into())
            .spawn(move || {
                let failed = |failure| RuntimeEvent::RequestFailed {
                    request,
                    failure,
                    message: "프로젝트 복제를 완료하지 못했습니다".into(),
                };
                let result = std::panic::catch_unwind(|| {
                    let (runner, _) =
                        project_git().map_err(|_| RequestFailure::ProjectCloneFailed)?;
                    let runner = CredentialDelegation::system_git()
                        .apply(&runner)
                        .with_allowed_env(vec!["GIT_ALLOW_PROTOCOL".into()]);
                    clone_project_cancellable(&input, &runner, &token).map_err(|e| match e {
                        CloneError::TargetExists => RequestFailure::CloneTargetExists,
                        CloneError::Clone(RepoError::Run(GitRunError::Cancelled)) => {
                            RequestFailure::CloneCancelled
                        }
                        _ => RequestFailure::ProjectCloneFailed,
                    })
                })
                .unwrap_or(Err(RequestFailure::ProjectCloneFailed));
                shared
                    .cloning
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                match result {
                    Ok(path) => {
                        let publish = Arc::clone(&shared);
                        if sender
                            .submit(move |ctx| {
                                let event = match open_project_on_thread(ctx, request, &path) {
                                    Ok(event) => {
                                        publish.publish_state(build_state(ctx));
                                        event
                                    }
                                    Err(failure) => RuntimeEvent::RequestFailed {
                                        request,
                                        failure: failure.failure,
                                        message: "복제한 프로젝트를 열 수 없습니다".into(),
                                    },
                                };
                                publish.outbox().push_back(event);
                            })
                            .is_err()
                        {
                            shared
                                .outbox()
                                .push_back(failed(RequestFailure::ProjectCloneFailed));
                        }
                    }
                    Err(reason) => shared.outbox().push_back(failed(reason)),
                }
            });
        if spawned.is_err() {
            self.shared
                .cloning
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            self.shared.release_one();
            return Err(RequestError::Stopped);
        }
        Ok(request)
    }

    /// 해당 request가 아직 복제 중이면 취소합니다. 다른 요청은 영향을 받지 않습니다.
    pub fn cancel_project_clone(&self, request: RequestId) {
        if let Some((_, token)) = self
            .shared
            .cloning
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|(id, _)| *id == request)
        {
            token.cancel();
        }
    }

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
        // 느린 Git 명령이 반복 요청 수만큼 worker를 만들지 않도록 한 번에 한 생성만 실행합니다.
        // 겹친 요청은 접수한 뒤 이유를 담은 생성 실패 event로 끝냅니다(대기열 포화와 구분).
        let request = self.reserve_request()?;
        if self.shared.creating.swap(true, Ordering::AcqRel) {
            self.shared.outbox().push_back(RuntimeEvent::RequestFailed {
                request,
                failure: RequestFailure::ProjectCreateFailed,
                message: CREATE_BUSY_MESSAGE.into(),
            });
            return Ok(request);
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

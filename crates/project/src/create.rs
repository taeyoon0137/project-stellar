//! 새 프로젝트 생성 요청과 사전 검증 (IV.4.1).
//!
//! [`ProjectCreateRequest`]는 새 프로젝트의 입력(부모 디렉터리·폴더 이름·프로젝트 이름)을
//! 담습니다. [`validate_create_request`]는 **사전 검증만** 수행하며 파일을 만들지 않습니다
//! (실제 생성은 IV.4.2 이후). 폴더 이름 규칙은 IV.2.1을 재사용합니다.

mod initial_document;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use stellar_git_porcelain::repo::{self, CommitReadiness, RepoError};
use stellar_git_porcelain::runner::GitRunner;
use stellar_git_porcelain::version::GitVersion;

use crate::folder_name::{validate_folder_name, FolderNameError};
use crate::fs_transaction::DirTransaction;
use crate::scaffold::write_default_scaffold;

/// 새 프로젝트 생성 요청.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectCreateRequest {
    /// 부모 디렉터리(여기 아래에 새 폴더 생성).
    pub parent_dir: PathBuf,
    /// 새 폴더 이름.
    pub folder_name: String,
    /// 사람이 읽는 프로젝트 이름.
    pub project_name: String,
}

/// 사전 검증 실패.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateValidationError {
    /// 프로젝트 이름이 비어 있음.
    EmptyProjectName,
    /// 폴더 이름 규칙 위반(IV.2.1).
    InvalidFolderName(FolderNameError),
    /// 부모가 디렉터리가 아님(없거나 파일).
    ParentNotADirectory,
    /// 대상 경로(부모/폴더)가 이미 존재함.
    TargetExists,
    /// 부모 디렉터리에 쓰기 권한이 없음.
    ParentNotWritable,
}

impl std::fmt::Display for CreateValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreateValidationError::EmptyProjectName => f.write_str("프로젝트 이름이 비어 있습니다"),
            CreateValidationError::InvalidFolderName(e) => write!(f, "폴더 이름 오류: {e}"),
            CreateValidationError::ParentNotADirectory => {
                f.write_str("부모 경로가 디렉터리가 아닙니다")
            }
            CreateValidationError::TargetExists => f.write_str("대상 경로가 이미 존재합니다"),
            CreateValidationError::ParentNotWritable => {
                f.write_str("부모 디렉터리에 쓰기 권한이 없습니다")
            }
        }
    }
}

impl std::error::Error for CreateValidationError {}

impl ProjectCreateRequest {
    /// 생성될 대상 경로(부모/폴더).
    pub fn target_path(&self) -> PathBuf {
        self.parent_dir.join(&self.folder_name)
    }
}

/// 생성 요청을 사전 검증합니다(파일을 만들지 않음; 쓰기 권한은 임시 probe로만 확인).
pub fn validate_create_request(
    request: &ProjectCreateRequest,
) -> Result<(), CreateValidationError> {
    if request.project_name.trim().is_empty() {
        return Err(CreateValidationError::EmptyProjectName);
    }
    validate_folder_name(&request.folder_name).map_err(CreateValidationError::InvalidFolderName)?;
    if !request.parent_dir.is_dir() {
        return Err(CreateValidationError::ParentNotADirectory);
    }
    if request.target_path().exists() {
        return Err(CreateValidationError::TargetExists);
    }
    if !is_writable(&request.parent_dir) {
        return Err(CreateValidationError::ParentNotWritable);
    }
    Ok(())
}

/// git init 이후 단계 실패 사유(폴더는 유지됨).
#[derive(Debug)]
pub enum IncompleteReason {
    /// user.name/email 누락으로 커밋 중단.
    MissingIdentity {
        /// `user.name` 누락.
        name_missing: bool,
        /// `user.email` 누락.
        email_missing: bool,
    },
    /// `git add` 단계 실패.
    AddFailed(RepoError),
    /// 커밋 단계 실패.
    CommitFailed(RepoError),
}

/// 프로젝트 생성 결과.
#[derive(Debug)]
pub enum ProjectCreated {
    /// 생성 완료(최초 커밋까지).
    Created {
        /// 생성된 프로젝트 경로.
        path: PathBuf,
    },
    /// git repo는 생성됐으나 이후 단계 실패 — **폴더를 삭제하지 않음**.
    InitializationIncomplete {
        /// 생성된(불완전) 프로젝트 경로.
        path: PathBuf,
        /// 실패 사유.
        reason: IncompleteReason,
    },
}

/// git init **이전/at** 실패(폴더가 롤백되어 제거됨).
#[derive(Debug)]
pub enum ProjectCreateError {
    /// 사전 검증 실패(아무 것도 생성되지 않음).
    Validation(CreateValidationError),
    /// 폴더 생성·scaffold io 오류(롤백됨).
    Io(std::io::ErrorKind),
    /// git init 실패(롤백됨).
    GitInit(RepoError),
}

impl std::fmt::Display for ProjectCreateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectCreateError::Validation(e) => write!(f, "사전 검증 실패: {e}"),
            ProjectCreateError::Io(kind) => write!(f, "파일시스템 오류(롤백됨): {kind:?}"),
            ProjectCreateError::GitInit(e) => write!(f, "git init 실패(롤백됨): {e}"),
        }
    }
}

impl std::error::Error for ProjectCreateError {}

/// 새 프로젝트를 생성합니다(검증 → 폴더 → scaffold → git init → add → 커밋).
///
/// **rollback 정책**(IV.4.7):
/// - git init **이전/at** 실패(검증·scaffold·init)는 생성한 폴더를 롤백해 제거하고 `Err`를
///   반환합니다.
/// - git init **이후** 실패(add·identity·commit)는 폴더를 **삭제하지 않고**
///   `Ok(InitializationIncomplete)`를 반환합니다(사용자가 복구 가능).
pub fn create_project(
    request: &ProjectCreateRequest,
    project_id: &str,
    runner: &GitRunner,
    version: &GitVersion,
) -> Result<ProjectCreated, ProjectCreateError> {
    create_with(request, project_id, runner, version, |_| Ok(()))
}

/// 새 편집 프로젝트를 생성합니다. 첫 문서·Page·View·Frame을 최초 commit 전에 generation 1로 저장합니다.
pub fn create_editable_project(
    request: &ProjectCreateRequest,
    runner: &GitRunner,
    version: &GitVersion,
) -> Result<ProjectCreated, ProjectCreateError> {
    let ids = crate::new_ids::new_ids().map_err(|e| ProjectCreateError::Io(e.kind()))?;
    create_with(request, &ids[0].to_string(), runner, version, |root| {
        initial_document::save(root, ids)
    })
}

fn create_with(
    request: &ProjectCreateRequest,
    project_id: &str,
    runner: &GitRunner,
    version: &GitVersion,
    initialize: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<ProjectCreated, ProjectCreateError> {
    validate_create_request(request).map_err(ProjectCreateError::Validation)?;

    // 폴더 생성 트랜잭션(commit 전 드롭되면 롤백).
    let tx = DirTransaction::create(&request.target_path())
        .map_err(|e| ProjectCreateError::Io(e.kind()))?;

    // scaffold(pre-init): 실패 시 tx 드롭 → 롤백.
    if let Err(e) = write_default_scaffold(tx.path(), &request.project_name, project_id) {
        return Err(ProjectCreateError::Io(e.kind()));
    }

    initialize(tx.path()).map_err(|e| ProjectCreateError::Io(e.kind()))?;

    // git init(at-init): 실패 시 tx 드롭 → 롤백.
    if let Err(e) = repo::git_init_main(runner, version, tx.path()) {
        return Err(ProjectCreateError::GitInit(e));
    }

    // git init 성공 → 이후 실패는 폴더를 유지. 롤백 취소.
    let path = tx.path().to_path_buf();
    tx.commit();

    // post-init: add + identity 검사.
    let readiness = match repo::stage_and_check_identity(runner, &path) {
        Ok(r) => r,
        Err(e) => {
            return Ok(ProjectCreated::InitializationIncomplete {
                path,
                reason: IncompleteReason::AddFailed(e),
            });
        }
    };
    if let CommitReadiness::HaltedMissingIdentity {
        name_missing,
        email_missing,
    } = readiness
    {
        return Ok(ProjectCreated::InitializationIncomplete {
            path,
            reason: IncompleteReason::MissingIdentity {
                name_missing,
                email_missing,
            },
        });
    }

    // 커밋(post-init): 실패해도 폴더 유지.
    if let Err(e) = repo::create_initial_commit(runner, &path) {
        return Ok(ProjectCreated::InitializationIncomplete {
            path,
            reason: IncompleteReason::CommitFailed(e),
        });
    }

    Ok(ProjectCreated::Created { path })
}

/// 디렉터리 쓰기 권한을 임시 probe 파일로 확인합니다(즉시 삭제).
fn is_writable(dir: &Path) -> bool {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let probe = dir.join(format!(".stellar-write-probe-{}-{n}", std::process::id()));
    match std::fs::File::create_new(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use stellar_git_porcelain::discovery::{discover_git, GitDiscoveryConfig};
    use stellar_git_porcelain::runner::argv;
    use stellar_git_porcelain::version::probe_git_version;

    const ID: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";

    struct TempDir {
        path: PathBuf,
    }
    impl TempDir {
        fn new() -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("stellar-create-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir { path }
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            // 권한 테스트가 0555로 바꿨을 수 있으니 정리 전에 복원.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o755));
            }
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn request(parent: &Path, folder: &str) -> ProjectCreateRequest {
        ProjectCreateRequest {
            parent_dir: parent.to_path_buf(),
            folder_name: folder.to_string(),
            project_name: "My Project".to_string(),
        }
    }

    #[test]
    fn valid_request_passes() {
        let dir = TempDir::new();
        assert_eq!(
            validate_create_request(&request(&dir.path, "my-project")),
            Ok(())
        );
    }

    #[test]
    fn existing_target_is_rejected() {
        let dir = TempDir::new();
        std::fs::create_dir_all(dir.path.join("taken")).unwrap();
        assert_eq!(
            validate_create_request(&request(&dir.path, "taken")),
            Err(CreateValidationError::TargetExists)
        );
    }

    #[test]
    fn forbidden_folder_name_is_rejected() {
        let dir = TempDir::new();
        // 예약 장치 이름.
        assert!(matches!(
            validate_create_request(&request(&dir.path, "CON")),
            Err(CreateValidationError::InvalidFolderName(_))
        ));
        // 예약 문자.
        assert!(matches!(
            validate_create_request(&request(&dir.path, "a/b")),
            Err(CreateValidationError::InvalidFolderName(_))
        ));
    }

    #[test]
    fn parent_not_a_directory_is_rejected() {
        let dir = TempDir::new();
        // 존재하지 않는 부모.
        let missing = dir.path.join("nope");
        assert_eq!(
            validate_create_request(&request(&missing, "x")),
            Err(CreateValidationError::ParentNotADirectory)
        );
        // 파일을 부모로 지정.
        let file = dir.path.join("afile");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(
            validate_create_request(&request(&file, "x")),
            Err(CreateValidationError::ParentNotADirectory)
        );
    }

    #[test]
    fn empty_project_name_is_rejected() {
        let dir = TempDir::new();
        let mut req = request(&dir.path, "ok-folder");
        req.project_name = "   ".to_string();
        assert_eq!(
            validate_create_request(&req),
            Err(CreateValidationError::EmptyProjectName)
        );
    }

    #[cfg(unix)]
    #[test]
    fn unwritable_parent_is_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new();
        // 부모를 읽기 전용(0555)으로.
        std::fs::set_permissions(&dir.path, std::fs::Permissions::from_mode(0o555)).unwrap();

        // root로 실행하면 0555도 우회되므로, 실제로 쓰기가 막혔는지 직접 확인.
        let probe = dir.path.join(".perm-probe");
        let write_blocked = std::fs::File::create(&probe).is_err();

        let result = validate_create_request(&request(&dir.path, "child"));

        // 권한 복원(드롭 정리용) + probe 정리.
        std::fs::set_permissions(&dir.path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_file(&probe);

        if write_blocked {
            assert_eq!(result, Err(CreateValidationError::ParentNotWritable));
        }
        // 쓰기가 막히지 않은 환경(root 등)에서는 권한 검증을 건너뜁니다.
    }

    // --- IV.4.7: rollback 정책 fault injection ---

    #[test]
    fn git_init_failure_before_commit_rolls_back_folder() {
        // 잘못된 git 경로 → git init 실패(at-init) → 폴더 롤백 제거.
        let dir = TempDir::new();
        let req = request(&dir.path, "proj");
        let runner = GitRunner::new(
            PathBuf::from("/nonexistent/git"),
            Vec::new(),
            Duration::from_secs(5),
        );
        let version = GitVersion {
            major: 2,
            minor: 39,
            patch: 0,
            raw: "git version 2.39.0".to_string(),
        };
        let result = create_project(&req, ID, &runner, &version);
        assert!(matches!(result, Err(ProjectCreateError::GitInit(_))));
        // 폴더가 롤백되어 제거되고 부모가 깨끗.
        assert!(!req.target_path().exists());
        assert_eq!(std::fs::read_dir(&dir.path).unwrap().count(), 0);
    }

    #[cfg(unix)]
    fn hermetic_overrides(
        global: std::ffi::OsString,
    ) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
        vec![
            (std::ffi::OsString::from("GIT_CONFIG_GLOBAL"), global),
            (
                std::ffi::OsString::from("GIT_CONFIG_SYSTEM"),
                std::ffi::OsString::from("/dev/null"),
            ),
        ]
    }

    #[cfg(unix)]
    #[test]
    fn missing_identity_after_init_keeps_folder_as_incomplete() {
        let Some(git) = discover_git(&GitDiscoveryConfig::default()) else {
            return;
        };
        let version = probe_git_version(&git).unwrap();
        if (version.major, version.minor) < (2, 28) {
            return;
        }
        let dir = TempDir::new();
        let req = request(&dir.path, "proj");
        // identity 없음(hermetic) → git init은 성공하지만 커밋 단계 중단.
        let runner = GitRunner::new(
            git,
            vec![std::ffi::OsString::from("PATH")],
            Duration::from_secs(30),
        )
        .with_env_overrides(hermetic_overrides(std::ffi::OsString::from("/dev/null")));

        let result = create_project(&req, ID, &runner, &version).unwrap();
        match result {
            ProjectCreated::InitializationIncomplete {
                path,
                reason: IncompleteReason::MissingIdentity { .. },
            } => {
                // 폴더는 유지(삭제하지 않음), git repo는 생성됨.
                assert!(path.exists());
                assert!(path.join("stellar.yaml").is_file());
                assert!(path.join(".git").exists());
            }
            other => panic!("예상과 다른 결과: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn editable_project_is_openable_and_original_files_survive_every_outcome() {
        use crate::saved_project::SavedProjectLoader;
        let git = discover_git(&GitDiscoveryConfig::default()).expect("system Git");
        let version = probe_git_version(&git).unwrap();
        let dir = TempDir::new();
        let config = dir.path.join("gitconfig");
        std::fs::write(
            &config,
            "[user]\n name = Fixture\n email = fixture@example.invalid\n",
        )
        .unwrap();
        let make_runner = |config: &Path| {
            GitRunner::new(git.clone(), vec![], Duration::from_secs(30))
                .with_env_overrides(hermetic_overrides(config.as_os_str().into()))
        };
        for (name, settings, committed) in [
            ("한글 UI", config.as_path(), true),
            ("No identity", Path::new("/dev/null"), false),
        ] {
            let runner = make_runner(settings);
            let req = request(&dir.path, name);
            let saved = create_editable_project(&req, &runner, &version).unwrap();
            match (saved, committed) {
                (ProjectCreated::Created { .. }, true)
                | (
                    ProjectCreated::InitializationIncomplete {
                        reason: IncompleteReason::MissingIdentity { .. },
                        ..
                    },
                    false,
                ) => {}
                other => panic!("unexpected create outcome {other:?}"),
            }
            let root = req.target_path();
            let loaded = SavedProjectLoader::new().open(&root).unwrap();
            assert_eq!(loaded.generation_number(), 1);
            let ids = loaded.document_ids().unwrap();
            assert_eq!(ids.len(), 1);
            let document = loaded.load_document(ids[0]).unwrap().model;
            let source = dir.path.join(format!("{name}.stellar.yaml"));
            let hash = crate::document_io::text_source::save_new(&source, &document).unwrap();
            let reopened = crate::document_io::text_source::open(&source).unwrap();
            assert_eq!(hash, reopened.source_hash);
            assert_eq!(
                crate::document_io::serialize::serialize_document(&document).unwrap(),
                crate::document_io::serialize::serialize_document(&reopened.model).unwrap()
            );
            let records = reopened.model.to_records().unwrap();
            let style = records.definition.text_styles.resolve(0, None).unwrap();
            let typography = style.typography.record().unwrap();
            assert!(typography.families.is_empty());
            assert_eq!(
                typography.size,
                stellar_core_types::typography::FontSize::Medium
            );
            assert_eq!(style.fill, None);
            assert_eq!(records.definition.pages.len(), 1);
            assert_eq!(records.definition.pages[0].views.len(), 1);
            assert_eq!(records.nodes.len(), 1);
            let page = &records.definition.pages[0];
            let persistent_ids = [
                loaded.project_id,
                ids[0],
                page.id,
                page.views[0].id,
                records.nodes[0].id,
            ];
            assert_eq!(
                persistent_ids
                    .into_iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                5
            );
            for stable_id in persistent_ids {
                assert_eq!((stable_id.version(), stable_id.variant()), (7, 2));
            }
            assert_eq!(
                records.nodes[0].id,
                records.definition.pages[0].views[0].root_frame
            );
            assert!(records.nodes[0].visible && !records.nodes[0].locked);
            // Git 원본은 텍스트 원본이고 chunk cache는 Git 밖입니다(ADR-0122).
            assert!(runner
                .run(
                    &argv([
                        "ls-files",
                        "--error-unmatch",
                        "design/document.stellar.yaml"
                    ]),
                    &root
                )
                .unwrap()
                .success());
            assert!(!runner
                .run(
                    &argv([
                        "ls-files",
                        "--error-unmatch",
                        ".stellar/cache/chunks/current"
                    ]),
                    &root
                )
                .unwrap()
                .success());
            assert!(stellar_storage::generation::current::current_pointer_path(&root).is_file());
            assert_eq!(
                runner
                    .run(&argv(["rev-parse", "--verify", "HEAD"]), &root)
                    .unwrap()
                    .success(),
                committed
            );
            if committed {
                assert_eq!(
                    runner
                        .run(&argv(["show", "HEAD:design/document.stellar.yaml"]), &root)
                        .unwrap()
                        .stdout,
                    std::fs::read(root.join("design/document.stellar.yaml")).unwrap()
                );
                assert!(runner
                    .run(&argv(["status", "--porcelain"]), &root)
                    .unwrap()
                    .stdout
                    .is_empty());
            }
            std::fs::write(root.join("keep.txt"), b"keep").unwrap();
            assert!(matches!(
                create_editable_project(&req, &runner, &version),
                Err(ProjectCreateError::Validation(
                    CreateValidationError::TargetExists
                ))
            ));
            assert_eq!(std::fs::read(root.join("keep.txt")).unwrap(), b"keep");
            assert_eq!(
                SavedProjectLoader::new().open(&root).unwrap().project_id,
                loaded.project_id
            );
        }
        let req = request(&dir.path, "rollback");
        assert!(
            create_with(&req, ID, &make_runner(&config), &version, |_| Err(
                std::io::ErrorKind::WriteZero.into()
            ))
            .is_err()
        );
        assert!(!req.target_path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn create_project_succeeds_with_identity_and_clean_tree() {
        let Some(git) = discover_git(&GitDiscoveryConfig::default()) else {
            return;
        };
        let version = probe_git_version(&git).unwrap();
        if (version.major, version.minor) < (2, 28) {
            return;
        }
        let dir = TempDir::new();
        // identity가 든 gitconfig를 부모에 두고 GIT_CONFIG_GLOBAL로 지정.
        let config = dir.path.join("gitconfig");
        std::fs::write(
            &config,
            "[user]\n\tname = Test User\n\temail = test@example.com\n",
        )
        .unwrap();
        let req = request(&dir.path, "proj");
        let runner = GitRunner::new(
            git,
            vec![std::ffi::OsString::from("PATH")],
            Duration::from_secs(30),
        )
        .with_env_overrides(hermetic_overrides(config.into_os_string()));

        let result = create_project(&req, ID, &runner, &version).unwrap();
        match result {
            ProjectCreated::Created { path } => {
                assert!(path.join("stellar.yaml").is_file());
                assert!(path.join("stellar.lock.yaml").is_file());
                assert!(path.join(".git").exists());
                // working tree clean.
                let status = runner.run(&argv(["status", "--porcelain"]), &path).unwrap();
                assert!(
                    status.stdout.is_empty(),
                    "tree dirty: {}",
                    String::from_utf8_lossy(&status.stdout)
                );
            }
            other => panic!("예상과 다른 결과: {other:?}"),
        }
    }
}

//! 실제 시스템 Git과 저장 파이프라인으로 Stellar 프로젝트 fixture를 만드는 helper.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::time::Duration;

use stellar_core_types::ids::StableId;
use stellar_document::model::DocumentModel;
use stellar_git_porcelain::discovery::{discover_git, GitDiscoveryConfig};
use stellar_git_porcelain::runner::{argv, GitOutput, GitRunner};
use stellar_project::document_io::serialize::serialize_document;
use stellar_project::identity::parse_project_id;
use stellar_project::manifest::{Manifest, ProjectMeta, CURRENT_SCHEMA_VERSION};
use stellar_project::root::MANIFEST_FILE;
use stellar_project::yaml::write_canonical;
use stellar_storage::generation::loader::load_current_generation;
use stellar_storage::save::cancel::{run_save, CancelToken};
use stellar_storage::save::commit::SavedGenerationState;
use stellar_storage::save::plan::build_save_plan;

/// fixture 프로젝트 id.
pub const PROJECT_ID_TEXT: &str = "0195c530-3acd-7bc1-8414-c213f61cb965";

/// fixture 프로젝트 id(StableId).
pub fn project_id() -> StableId {
    parse_project_id(PROJECT_ID_TEXT).unwrap()
}

/// 머신 global/system Git 설정과 무관한 실행기. Git이 없으면 `None`(호출 테스트는 건너뜀).
pub fn hermetic_runner() -> Option<GitRunner> {
    let git = discover_git(&GitDiscoveryConfig::default())?;
    Some(
        GitRunner::new(
            git,
            vec![OsString::from("PATH"), OsString::from("HOME")],
            Duration::from_secs(60),
        )
        .with_env_overrides(vec![
            (
                OsString::from("GIT_CONFIG_GLOBAL"),
                OsString::from("/dev/null"),
            ),
            (
                OsString::from("GIT_CONFIG_SYSTEM"),
                OsString::from("/dev/null"),
            ),
            (OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0")),
        ]),
    )
}

/// fixture 구성용 Git 실행(성공을 강제).
pub fn git_ok<I, S>(runner: &GitRunner, dir: &Path, args: I) -> GitOutput
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args = argv(args);
    let out = runner.run(&args, dir).expect("fixture git 실행");
    assert!(
        out.success(),
        "fixture git 실패 {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// fixture 구성용 Git 실행(실패 허용).
pub fn git_any<I, S>(runner: &GitRunner, dir: &Path, args: I) -> GitOutput
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    runner.run(&argv(args), dir).expect("fixture git 실행")
}

/// `git init -b main` + 로컬 identity.
pub fn init_repo(runner: &GitRunner, dir: &Path) {
    git_ok(runner, dir, ["init", "-q", "-b", "main"]);
    git_ok(runner, dir, ["config", "user.name", "Test User"]);
    git_ok(runner, dir, ["config", "user.email", "test@example.com"]);
}

/// 모든 변경을 stage하고 커밋합니다.
pub fn commit_all(runner: &GitRunner, dir: &Path, message: &str) {
    git_ok(runner, dir, ["add", "-A"]);
    git_ok(runner, dir, ["commit", "-q", "-m", message]);
}

/// 유효한 `stellar.yaml`을 씁니다.
pub fn write_manifest(root: &Path, name: &str) {
    let manifest = Manifest {
        schema_version: CURRENT_SCHEMA_VERSION,
        project: ProjectMeta {
            id: PROJECT_ID_TEXT.to_string(),
            name: name.to_string(),
        },
        target_profiles: Vec::new(),
        dependencies: Vec::new(),
        style_defaults: None,
    };
    std::fs::write(
        root.join(MANIFEST_FILE),
        write_canonical(&manifest.to_yaml()),
    )
    .unwrap();
}

/// 문서들을 실제 저장 파이프라인(V.5)으로 current generation 위에 저장합니다(디스크 기록).
pub fn save_documents(root: &Path, documents: &[&DocumentModel]) {
    let base = load_current_generation(root, project_id()).ok();
    let serialized: Vec<_> = documents
        .iter()
        .map(|model| serialize_document(model).unwrap())
        .collect();
    let states: Vec<_> = serialized
        .iter()
        .flat_map(|d| d.to_dirty_states())
        .collect();
    let roots = serialized.iter().map(|d| d.document_id).collect();
    let plan = build_save_plan(base.as_ref(), project_id(), 1, &states, roots).unwrap();
    let mut memory = SavedGenerationState::new(
        base.as_ref()
            .map(|b| (b.manifest.generation, b.manifest_hash)),
    );
    run_save(root, &plan, &mut memory, &CancelToken::new()).unwrap();
}

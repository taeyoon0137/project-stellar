//! XIV.11.2 conflict 세 stage virtual project를 실제 프로젝트 loader로 여는 통합 테스트(시스템 Git).
//!
//! 실제 저장 파이프라인(V.5)으로 저장한 Stellar 프로젝트를 두 branch에서 서로 다르게 저장해 병합하면
//! `design/current`가 conflict가 됩니다. base·ours·theirs snapshot을 [`SavedProjectLoader`]
//! (매니페스트 → current pointer → manifest → chunk, V.4.4)로 열어 각 branch의 문서를 확인합니다.

#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::git::{
    commit_all, git_any, git_ok, hermetic_runner, init_repo, save_documents, write_manifest,
};
use common::{rich_model, rich_nodes, TempDir};
use stellar_core_types::ids::StableId;
use stellar_document::model::DocumentModel;
use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_git_porcelain::conflict_snapshot::ConflictSide;
use stellar_git_porcelain::runner::GitRunner;
use stellar_git_porcelain::trust::{GatedGitRunner, GitTrust};
use stellar_project::conflict_open::open_conflict_projects;
use stellar_project::saved_project::{SavedProject, SavedProjectError};

/// 문서 0번 node 이름만 바꾼 rich 문서.
fn renamed(name: &str) -> DocumentModel {
    let mut model = rich_model(1, 0);
    model
        .set_display_name(rich_nodes(1)[0].id, name.to_string())
        .unwrap();
    model
}

fn document_json(project: &SavedProject, id: StableId) -> String {
    model_snapshot_json(&project.load_document(id).unwrap().model).unwrap()
}

/// 작업 트리 파일(.git 제외)의 (경로, bytes) 목록.
fn tree_bytes(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == ".git") {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                stack.push(path);
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

struct ConflictRepo {
    _dir: TempDir,
    repo: PathBuf,
    temp_parent: PathBuf,
    runner: GitRunner,
    base: DocumentModel,
    ours: DocumentModel,
    theirs: DocumentModel,
}

/// base에서 갈라진 두 branch가 같은 문서를 다르게 저장한 뒤 병합해 conflict를 만듭니다.
fn conflict_repo(runner: GitRunner) -> ConflictRepo {
    let dir = TempDir::new("stellar-conflict-open");
    let repo = dir.path.join("repo");
    let temp_parent = dir.path.join("tmp");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&temp_parent).unwrap();
    init_repo(&runner, &repo);
    write_manifest(&repo, "Conflict");
    std::fs::write(repo.join("README.md"), b"base\n").unwrap();
    let base = rich_model(1, 0);
    save_documents(&repo, &[&base]);
    commit_all(&runner, &repo, "base");

    git_ok(&runner, &repo, ["checkout", "-q", "-b", "theirs"]);
    let theirs = renamed("theirs edit");
    save_documents(&repo, &[&theirs]);
    std::fs::write(repo.join("README.md"), b"theirs\n").unwrap();
    commit_all(&runner, &repo, "theirs");

    git_ok(&runner, &repo, ["checkout", "-q", "main"]);
    let ours = renamed("ours edit");
    save_documents(&repo, &[&ours]);
    std::fs::write(repo.join("README.md"), b"ours\n").unwrap();
    commit_all(&runner, &repo, "ours");

    let merge = git_any(&runner, &repo, ["merge", "--no-edit", "theirs"]);
    assert!(!merge.success(), "conflict가 나야 함");
    ConflictRepo {
        _dir: dir,
        repo,
        temp_parent,
        runner,
        base,
        ours,
        theirs,
    }
}

#[test]
fn base_ours_theirs_projects_open_with_the_real_loader() {
    let Some(runner) = hermetic_runner() else {
        return; // Git 미설치 환경 건너뜀.
    };
    let fx = conflict_repo(runner);
    let gate = GatedGitRunner::new(fx.runner.clone(), GitTrust::Trusted);
    let index_before = git_ok(&fx.runner, &fx.repo, ["ls-files", "--stage"]).stdout;
    let tree_before = tree_bytes(&fx.repo);

    let projects = open_conflict_projects(&gate, &fx.repo, &fx.temp_parent).unwrap();

    // Stellar conflict는 design/current, README는 raw conflict로만.
    let stellar: Vec<&[u8]> = projects
        .stellar_conflicts()
        .iter()
        .map(|p| p.as_bytes())
        .collect();
    assert_eq!(stellar, vec![&b"design/current"[..]]);
    let raw: Vec<&[u8]> = projects
        .raw_conflicts()
        .iter()
        .map(|p| p.path.as_bytes())
        .collect();
    assert_eq!(raw, vec![&b"README.md"[..]]);

    // 세 side 모두 실제 loader로 열리고 각 branch의 문서·generation을 가짐.
    let id = fx.base.document_id();
    for (side, model, generation) in [
        (ConflictSide::Base, &fx.base, 1),
        (ConflictSide::Ours, &fx.ours, 2),
        (ConflictSide::Theirs, &fx.theirs, 2),
    ] {
        let project = projects
            .side(side)
            .as_ref()
            .unwrap_or_else(|e| panic!("{side:?} 열기 실패: {e}"));
        assert_eq!(project.manifest.project.name, "Conflict", "{side:?}");
        assert_eq!(project.generation_number(), generation, "{side:?}");
        assert_eq!(project.document_ids().unwrap(), vec![id], "{side:?}");
        assert_eq!(
            document_json(project, id),
            model_snapshot_json(model).unwrap(),
            "{side:?} 문서"
        );
        let root = projects.root(side);
        assert!(!root.join(".git").exists(), "{side:?}: .git 없음");
        assert!(
            !root.join("README.md").exists(),
            "{side:?}: raw conflict 미포함"
        );
        assert_eq!(root.parent(), Some(projects.session_root()));
    }
    assert_ne!(
        model_snapshot_json(&fx.ours).unwrap(),
        model_snapshot_json(&fx.theirs).unwrap()
    );
    // 비공개 session root(XVI.7.5).
    let mode = std::fs::metadata(projects.session_root())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);

    // 작업 트리·index는 읽기만 함.
    assert_eq!(
        git_ok(&fx.runner, &fx.repo, ["ls-files", "--stage"]).stdout,
        index_before
    );
    assert_eq!(tree_bytes(&fx.repo), tree_before);

    // drop하면 세 snapshot과 session이 모두 지워짐.
    let session = projects.session_root().to_path_buf();
    drop(projects);
    assert!(!session.exists());
    assert_eq!(std::fs::read_dir(&fx.temp_parent).unwrap().count(), 0);
}

#[test]
fn side_without_the_project_reports_its_own_open_error() {
    let Some(runner) = hermetic_runner() else {
        return;
    };
    // theirs가 프로젝트 매니페스트를 바꾸고 ours가 지움 → stellar.yaml도 conflict, ours side에 없음.
    let dir = TempDir::new("stellar-conflict-open-deleted");
    let repo = dir.path.join("repo");
    let temp_parent = dir.path.join("tmp");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&temp_parent).unwrap();
    init_repo(&runner, &repo);
    write_manifest(&repo, "Before");
    save_documents(&repo, &[&rich_model(1, 0)]);
    commit_all(&runner, &repo, "base");
    git_ok(&runner, &repo, ["checkout", "-q", "-b", "theirs"]);
    write_manifest(&repo, "Renamed by theirs");
    commit_all(&runner, &repo, "theirs");
    git_ok(&runner, &repo, ["checkout", "-q", "main"]);
    git_ok(&runner, &repo, ["rm", "-q", "stellar.yaml"]);
    git_ok(&runner, &repo, ["commit", "-q", "-m", "ours deletes"]);
    assert!(!git_any(&runner, &repo, ["merge", "--no-edit", "theirs"]).success());

    let gate = GatedGitRunner::new(runner, GitTrust::Trusted);
    let projects = open_conflict_projects(&gate, &repo, &temp_parent).unwrap();
    assert_eq!(
        projects.base.as_ref().unwrap().manifest.project.name,
        "Before"
    );
    assert_eq!(
        projects.theirs.as_ref().unwrap().manifest.project.name,
        "Renamed by theirs"
    );
    assert!(
        matches!(
            projects.ours,
            Err(SavedProjectError::ManifestRead {
                kind: std::io::ErrorKind::NotFound
            })
        ),
        "{:?}",
        projects.ours
    );
    drop(projects);
    assert_eq!(std::fs::read_dir(&temp_parent).unwrap().count(), 0);
}

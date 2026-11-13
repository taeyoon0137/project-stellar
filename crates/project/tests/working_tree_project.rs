//! XIV.2.7 WorkingTree provider + 실제 프로젝트 loader 통합 테스트(시스템 Git).
//!
//! WorkingTree snapshot의 프로젝트는 [`SavedProjectLoader`](current pointer → manifest → chunk,
//! V.4.4)로 디스크에서 엽니다. 편집기 메모리의 저장 대기 편집은 Git status와 snapshot 어디에도
//! 나타나지 않고, 저장(V.5) 뒤에만 나타납니다.

#![cfg(unix)]

mod common;

use common::git::{commit_all, hermetic_runner, init_repo, save_documents, write_manifest};
use common::{rich_model, rich_nodes, TempDir};
use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_git_porcelain::repo_status::read_repository_status;
use stellar_git_porcelain::snapshot::working_tree::SnapshotBasis;
use stellar_git_porcelain::status::{read_status, StatusEntry, StatusOptions};
use stellar_git_porcelain::trust::{GatedGitRunner, GitTrust};
use stellar_project::saved_project::{read_working_tree_project, SavedProject};

/// 저장된 프로젝트의 문서 snapshot(JSON).
fn saved_document_json(
    project: &SavedProject,
    model_id: stellar_core_types::ids::StableId,
) -> String {
    let loaded = project.load_document(model_id).unwrap();
    model_snapshot_json(&loaded.model).unwrap()
}

#[test]
fn unsaved_memory_edit_does_not_appear_in_git_status_or_working_tree_project() {
    let Some(runner) = hermetic_runner() else {
        return; // Git 미설치 환경 건너뜀.
    };
    let dir = TempDir::new("stellar-wt-project");
    let repo = dir.path.clone();
    init_repo(&runner, &repo);
    write_manifest(&repo, "Working Tree");
    let saved = rich_model(1, 0);
    save_documents(&repo, &[&saved]);
    commit_all(&runner, &repo, "saved");
    let gate = GatedGitRunner::new(runner, GitTrust::Trusted);

    // 편집기 메모리에서만 문서를 바꿈(autosave 전).
    let mut memory = rich_model(1, 0);
    let target = rich_nodes(1)[0].id;
    memory
        .set_display_name(target, "unsaved memory edit".to_string())
        .unwrap();
    let saved_json = model_snapshot_json(&saved).unwrap();
    let memory_json = model_snapshot_json(&memory).unwrap();
    assert_ne!(saved_json, memory_json);

    // 저장 전: Git status는 깨끗하고 snapshot 프로젝트는 디스크(커밋된) 내용.
    let status = read_status(&gate, &repo, StatusOptions::default()).unwrap();
    assert!(status.entries.is_empty(), "{:?}", status.entries);
    let summary = read_repository_status(&gate, &repo).unwrap();
    assert_eq!(summary.counts.unstaged, 0);
    assert_eq!(summary.counts.untracked, 0);
    assert_eq!(summary.basis(), SnapshotBasis::SavedFilesOnDisk);
    let snap = read_working_tree_project(&gate, &repo).unwrap();
    assert_eq!(snap.basis, SnapshotBasis::SavedFilesOnDisk);
    assert!(snap
        .files
        .iter()
        .any(|f| f.path.as_bytes() == b"design/current"));
    let project = snap.project.as_ref().unwrap();
    assert_eq!(project.generation_number(), 1);
    assert_eq!(project.manifest.project.name, "Working Tree");
    assert_eq!(project.document_ids().unwrap(), vec![saved.document_id()]);
    let on_disk = saved_document_json(project, saved.document_id());
    assert_eq!(on_disk, saved_json, "snapshot = 디스크 저장 내용");
    assert_ne!(on_disk, memory_json, "메모리 편집은 snapshot에 없음");

    // 저장(V.5) 뒤에만 Git status와 snapshot에 나타남.
    save_documents(&repo, &[&memory]);
    let status = read_status(&gate, &repo, StatusOptions::default()).unwrap();
    assert!(
        status.entries.iter().any(|e| matches!(
            e,
            StatusEntry::Ordinary(c) if c.path.as_bytes() == b"design/current"
        )),
        "{:?}",
        status.entries
    );
    let summary = read_repository_status(&gate, &repo).unwrap();
    assert_eq!(summary.counts.unstaged, 1, "design/current");
    assert!(summary.counts.untracked >= 2, "새 manifest·chunk");
    let snap = read_working_tree_project(&gate, &repo).unwrap();
    let project = snap.project.as_ref().unwrap();
    assert_eq!(project.generation_number(), 2);
    assert_eq!(
        saved_document_json(project, memory.document_id()),
        memory_json
    );
}

#[test]
fn working_tree_project_failure_keeps_the_file_listing() {
    let Some(runner) = hermetic_runner() else {
        return;
    };
    let dir = TempDir::new("stellar-wt-project-broken");
    let repo = dir.path.clone();
    init_repo(&runner, &repo);
    write_manifest(&repo, "Broken");
    save_documents(&repo, &[&rich_model(2, 0)]);
    commit_all(&runner, &repo, "saved");
    // current pointer가 가리키는 manifest를 지움(손상된 디스크 상태).
    let generations = repo.join("design/generations");
    for entry in std::fs::read_dir(&generations).unwrap() {
        std::fs::remove_file(entry.unwrap().path()).unwrap();
    }
    let gate = GatedGitRunner::new(runner, GitTrust::Trusted);
    let snap = read_working_tree_project(&gate, &repo).unwrap();
    assert!(snap.project.is_err(), "loader 실패는 보존");
    assert!(
        !snap.missing_tracked.is_empty(),
        "삭제된 manifest는 목록에 남음"
    );
    assert!(snap
        .files
        .iter()
        .any(|f| f.path.as_bytes() == b"stellar.yaml"));
}

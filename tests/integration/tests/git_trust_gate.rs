//! 프로젝트 신뢰 gate(XIV.1.1) 종단 시나리오.
//!
//! 신뢰 상태의 원본은 project trust store(XVI.7.2)이고, capability matrix(XVI.7.1)의 `RunGit` 행이
//! git-porcelain gate 상태를 정합니다. 신뢰되지 않은 저장소에서는 snapshot provider·materializer·conflict
//! 열기·status·commit이 모두 **프로세스 생성 없이** 거부되고(저장소가 지정한 fsmonitor hook도 실행되지
//! 않음), 임시 디렉터리도 남지 않습니다. 문서 읽기는 제한 모드로 허용되어 Git 없이 디스크 loader로 열립니다.
//! 신뢰를 부여하면 같은 경로가 동작하고, 철회하거나 다른 경로(clone)·다른 project id면 다시 막힙니다.

#![cfg(unix)] // Windows 경로는 보류 상태라 검증하지 않습니다(ADR-0006).

mod git_common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use git_common::*;
use stellar_core_types::ids::StableId;
use stellar_git_porcelain::commit::{commit_with_message, CommitError};
use stellar_git_porcelain::conflict_snapshot::ConflictSnapshotError;
use stellar_git_porcelain::fixture::GitFixture;
use stellar_git_porcelain::ops::GitOpError;
use stellar_git_porcelain::repo_status::read_repository_status;
use stellar_git_porcelain::snapshot::{
    materialize_snapshot, read_index_snapshot, read_tree_snapshot, BlobError, MaterializeError,
    SnapshotError, SnapshotSource,
};
use stellar_git_porcelain::status::{read_status, StatusOptions};
use stellar_git_porcelain::trust::{GatedGitRunner, GatedRunError, GitTrust};
use stellar_project::capability::{
    check_capability, git_trust, CapabilityDenied, CapabilityLevel, ProjectCapability,
};
use stellar_project::conflict_open::{open_conflict_projects, ConflictOpenError};
use stellar_project::identity::ProjectIdentity;
use stellar_project::saved_project::read_working_tree_project;
use stellar_project::trust::TrustStatus;
use stellar_project::trust_store::FileTrustStore;

/// 실행되면 marker 파일을 만들고 실패하는 fsmonitor hook(저장소가 지정한 프로그램)을 설치합니다.
fn install_fsmonitor(fx: &GitFixture, script: &Path, marker: &Path) {
    let body = format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display());
    std::fs::write(script, body).unwrap();
    std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o755)).unwrap();
    fx.git(["config", "core.fsmonitor", script.to_str().unwrap()])
        .unwrap();
}

/// untrusted gate가 모든 Git 경로를 프로세스 생성 없이 거부하는지 확인합니다.
fn assert_all_git_paths_refused(gate: &GatedGitRunner, work: &Path, scratch: &Path) {
    let refused = GitOpError::Gate(GatedRunError::ProjectNotTrusted);
    assert_eq!(
        read_tree_snapshot(gate, work, &SnapshotSource::Head).map(|_| ()),
        Err(SnapshotError::Op(refused.clone())),
        "HEAD"
    );
    assert_eq!(
        read_index_snapshot(gate, work).map(|_| ()),
        Err(refused.clone()),
        "Index"
    );
    assert_eq!(
        read_working_tree_project(gate, work).map(|_| ()),
        Err(refused.clone()),
        "WorkingTree"
    );
    assert_eq!(
        read_status(gate, work, StatusOptions::default()).map(|_| ()),
        Err(refused.clone()),
        "status"
    );
    assert_eq!(
        read_repository_status(gate, work).map(|_| ()),
        Err(refused.clone()),
        "repository status"
    );
    assert_eq!(
        open_conflict_projects(gate, work, scratch).map(|_| ()),
        Err(ConflictOpenError::Snapshot(ConflictSnapshotError::Op(
            refused.clone()
        ))),
        "conflict 열기"
    );
    assert_eq!(
        commit_with_message(gate, work, "untrusted commit"),
        Err(CommitError::Gate(GatedRunError::ProjectNotTrusted)),
        "commit"
    );
}

#[test]
fn untrusted_repository_blocks_git_until_trust_is_granted() {
    let Some(fx) = git_fixture("e2e-trust") else {
        return;
    };
    let work = fx.work().to_path_buf();
    let card = card_document();
    write_manifest(&work, "Trust");
    save_documents(&work, &[&card]);
    let head = fx.commit_all("base").unwrap();
    fx.push("main").unwrap();
    let expected = ProjectState::expected("Trust", &[&card]);
    // materialize 거부 확인용 항목은 신뢰된 fixture gate로 미리 읽어 둡니다.
    let entries = read_tree_snapshot(&fx.gated(), &work, &SnapshotSource::Head)
        .unwrap()
        .entries;

    // 저장소가 지정한 프로그램: status가 실행하면 marker를 만드는 fsmonitor hook.
    let tools = scratch_dir(&fx, "tools");
    let marker = fx.root().join("PWNED_fsmonitor");
    install_fsmonitor(&fx, &tools.join("fsmonitor.sh"), &marker);
    let _ = std::fs::remove_file(&marker);

    // 1) 처음 여는 저장소는 신뢰되지 않음 → capability matrix → gate 비활성.
    let data = scratch_dir(&fx, "user-data");
    let identity = ProjectIdentity::new(project_id(), &work).unwrap();
    let mut store = FileTrustStore::open(&data).unwrap();
    assert_eq!(store.status(&identity), TrustStatus::NotTrusted);
    assert!(
        !store.file_path().exists(),
        "열기만으로 신뢰 파일을 만들지 않음"
    );
    let trust = git_trust(store.status(&identity));
    assert_eq!(trust, GitTrust::NotTrusted);
    assert_eq!(
        check_capability(ProjectCapability::RunGit, TrustStatus::NotTrusted),
        Err(CapabilityDenied {
            capability: ProjectCapability::RunGit
        })
    );
    let mut gate = GatedGitRunner::new(fx.runner().clone(), trust);

    // 2) 모든 Git 경로가 프로세스 생성 없이 거부되고 임시 디렉터리·hook 실행 흔적이 남지 않음.
    let snapshots = scratch_dir(&fx, "snapshots");
    let conflicts = scratch_dir(&fx, "conflicts");
    let spawned = fx.runner().spawn_count();
    assert_all_git_paths_refused(&gate, &work, &conflicts);
    assert_eq!(
        materialize_snapshot(&gate, &work, &entries, &snapshots).map(|_| ()),
        Err(MaterializeError::Blob(BlobError::Op(GitOpError::Gate(
            GatedRunError::ProjectNotTrusted
        )))),
        "materialize"
    );
    assert_eq!(
        fx.runner().spawn_count(),
        spawned,
        "untrusted에서 프로세스 생성 0"
    );
    assert!(!marker.exists(), "저장소 지정 fsmonitor가 실행됨");
    assert_eq!(entry_count(&snapshots), 0, "실패한 materialize 정리");
    assert_eq!(entry_count(&conflicts), 0, "실패한 conflict session 정리");

    // 3) 문서 읽기는 제한 모드로 허용: Git 없이 디스크 loader로 연 문서가 commit과 같음.
    assert_eq!(
        check_capability(ProjectCapability::ReadDocument, TrustStatus::NotTrusted),
        Ok(CapabilityLevel::Restricted)
    );
    assert_same_state(
        &ProjectState::load(&open_project(&work)),
        &expected,
        "untrusted 디스크 열기",
    );
    assert_eq!(
        fx.runner().spawn_count(),
        spawned,
        "디스크 열기는 Git 미사용"
    );
    assert_eq!(
        fx.rev_parse("HEAD").unwrap(),
        head,
        "거부된 commit은 HEAD 불변"
    );

    // 4) 신뢰 부여(영속) → 같은 gate 경로가 동작하고 snapshot이 commit 문서와 같음.
    store.grant(&identity).unwrap();
    let reopened = FileTrustStore::open(&data).unwrap();
    assert_eq!(reopened.status(&identity), TrustStatus::Trusted);
    gate.set_trust(git_trust(reopened.status(&identity)));
    assert!(gate.is_enabled());
    let before = fx.runner().spawn_count();
    let state = source_state(&gate, &work, &SnapshotSource::Head, &snapshots);
    assert_same_state(&state, &expected, "trusted HEAD");
    assert!(fx.runner().spawn_count() > before, "trusted는 Git 실행");
    // 대조군: trusted status는 저장소 지정 fsmonitor를 실제로 실행합니다(2의 marker 부재가 gate 덕분임).
    read_status(&gate, &work, StatusOptions::default()).unwrap();
    assert!(marker.exists(), "대조군 fsmonitor 미실행");

    // 5) 신뢰는 경로·project id 단위: clone한 저장소나 다른 project id는 신뢰되지 않음.
    let peer = fx.clone_bare("peer").unwrap();
    let peer_identity = ProjectIdentity::new(project_id(), &peer).unwrap();
    assert_eq!(reopened.status(&peer_identity), TrustStatus::NotTrusted);
    let foreign = ProjectIdentity::new(StableId::new_v7(1_700_000_999_999, 1, 1), &work).unwrap();
    assert_eq!(reopened.status(&foreign), TrustStatus::NotTrusted);
    let peer_gate = GatedGitRunner::new(
        fx.runner().clone(),
        git_trust(reopened.status(&peer_identity)),
    );
    let spawned = fx.runner().spawn_count();
    assert_all_git_paths_refused(&peer_gate, &peer, &conflicts);
    assert_eq!(
        fx.runner().spawn_count(),
        spawned,
        "clone 경로도 프로세스 생성 0"
    );

    // 6) 신뢰 철회 → 다시 거부.
    store.revoke(&identity).unwrap();
    let revoked = FileTrustStore::open(&data).unwrap();
    assert_eq!(revoked.status(&identity), TrustStatus::NotTrusted);
    gate.set_trust(git_trust(revoked.status(&identity)));
    let _ = std::fs::remove_file(&marker);
    let spawned = fx.runner().spawn_count();
    assert_all_git_paths_refused(&gate, &work, &conflicts);
    assert_eq!(
        fx.runner().spawn_count(),
        spawned,
        "철회 뒤 프로세스 생성 0"
    );
    assert!(!marker.exists());
    assert_eq!(entry_count(&snapshots), 0);
    assert_eq!(entry_count(&conflicts), 0);
}

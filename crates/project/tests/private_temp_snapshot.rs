//! XVI.7.5 Git snapshot materialize(XIV.2.6)를 비공개 임시 디렉터리 아래에 두는 권한·정리 테스트.

#![cfg(unix)]

mod common;

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

use common::git::{commit_all, hermetic_runner, init_repo};
use common::TempDir;
use stellar_git_porcelain::snapshot::{materialize_snapshot, read_index_snapshot};
use stellar_git_porcelain::trust::{GatedGitRunner, GitTrust};
use stellar_project::private_temp::{PrivateTempDir, PRIVATE_TEMP_MARKER};

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o7777
}

#[test]
fn snapshots_live_under_an_owner_only_root_that_cleans_up_everything() {
    let Some(runner) = hermetic_runner() else {
        return; // Git 미설치 환경 건너뜀.
    };
    let work = TempDir::new("stellar-private-snapshot");
    let repo = work.path.join("repo");
    std::fs::create_dir_all(repo.join("design/objects")).unwrap();
    init_repo(&runner, &repo);
    std::fs::write(repo.join("stellar.yaml"), b"schema_version: 1\n").unwrap();
    std::fs::write(repo.join("design/objects/a.starchunk"), [0u8, 1, 2]).unwrap();
    std::fs::write(repo.join("tool.sh"), b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(repo.join("tool.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    commit_all(&runner, &repo, "base");
    let gate = GatedGitRunner::new(runner, GitTrust::Trusted);

    let session = PrivateTempDir::create_in(&work.path, "stellar-snapshots").unwrap();
    let owner = std::fs::metadata(&repo).unwrap().uid();
    assert_eq!(mode(session.path()), 0o700);
    assert_eq!(std::fs::metadata(session.path()).unwrap().uid(), owner);

    let index = read_index_snapshot(&gate, &repo).unwrap();
    let snapshot = materialize_snapshot(&gate, &repo, &index.entries, session.path()).unwrap();
    assert_eq!(snapshot.root().parent(), Some(session.path()));
    assert_eq!(mode(snapshot.root()), 0o700, "snapshot root도 소유자 전용");
    assert_eq!(
        std::fs::read(snapshot.root().join("design/objects/a.starchunk")).unwrap(),
        [0u8, 1, 2]
    );
    // snapshot 안 파일 권한(0644·0755)과 무관하게 group·other는 상위 0700 root에서 막힘.
    assert_eq!(mode(&snapshot.root().join("tool.sh")), 0o755);
    assert_eq!(mode(session.path()) & 0o077, 0);

    // snapshot을 keep()하고 읽기 전용 디렉터리를 남겨도 session 정리가 전부 지움.
    let kept = snapshot.keep();
    std::fs::set_permissions(
        kept.join("design/objects"),
        std::fs::Permissions::from_mode(0o555),
    )
    .unwrap();
    assert!(session.path().join(PRIVATE_TEMP_MARKER).is_file());
    let session_path = session.path().to_path_buf();
    session.cleanup().unwrap();
    assert!(!kept.exists());
    assert!(!session_path.exists());
    // 원본 작업 트리는 그대로.
    assert!(repo.join("design/objects/a.starchunk").is_file());
}

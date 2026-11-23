//! 결과 target 검증·생성 시험(시스템 Git fixture). Git이 없는 환경에서는 건너뜁니다.

use std::path::Path;

use stellar_git_porcelain::object::ObjectId;

use super::fixture::repo;
use crate::merge_apply::target::{
    create_branch_worktree, validate_target, ResultTarget, TargetError,
};
use crate::snapshot::open_snapshot;

#[test]
fn target_validation_rejects_bad_targets_without_side_effects() {
    let Some(repo) = repo("merge-apply-validate", false) else {
        return;
    };
    let work = repo.fx.work().to_path_buf();
    let git = repo.fx.gated();
    let id = open_snapshot(&work).unwrap().project_id();
    let fresh = repo.fx.root().join("fresh");
    let new = |branch: &str, path: &Path, start: &ObjectId| ResultTarget::NewBranchWorktree {
        branch: branch.into(),
        path: path.to_path_buf(),
        start: start.clone(),
    };
    let check = |target: ResultTarget| validate_target(&git, &work, id, &target).unwrap_err();
    assert!(matches!(
        check(new("a..b", &fresh, &repo.a)),
        TargetError::InvalidBranch(_)
    ));
    assert!(matches!(
        check(new("a", &fresh, &repo.a)),
        TargetError::BranchExists(_)
    ));
    assert!(matches!(
        check(new("n", &repo.fx.root().join("no-parent/child"), &repo.a)),
        TargetError::InvalidPath(_)
    ));
    std::fs::create_dir_all(repo.fx.root().join("taken")).unwrap();
    assert!(matches!(
        check(new("n", &repo.fx.root().join("taken"), &repo.a)),
        TargetError::PathExists(_)
    ));
    assert!(matches!(
        check(new("n", &work.join("nested"), &repo.a)),
        TargetError::InsideWorktree { .. }
    ));
    assert!(matches!(
        check(new("n", Path::new("relative/path"), &repo.a)),
        TargetError::InvalidPath(_)
    ));
    let missing = ObjectId::parse(&"1".repeat(40)).unwrap();
    assert!(matches!(
        check(new("n", &fresh, &missing)),
        TargetError::StartNotFound(_)
    ));
    assert!(matches!(
        check(new("n", &fresh, &repo.before_project)),
        TargetError::StartWithoutProject(_)
    ));
    assert!(matches!(
        check(ResultTarget::ExistingWorktree {
            path: repo.fx.root().join("taken"),
        }),
        TargetError::NotAWorktree(_)
    ));
    // 어떤 실패도 branch·worktree를 만들지 않습니다.
    assert!(!fresh.exists());
    assert!(repo
        .fx
        .git(["rev-parse", "-q", "--verify", "refs/heads/n"])
        .is_err());
}

#[test]
fn new_branch_worktree_is_created_by_one_system_git_command() {
    let Some(repo) = repo("merge-apply-create", false) else {
        return;
    };
    let work = repo.fx.work().to_path_buf();
    let git = repo.fx.gated();
    let id = open_snapshot(&work).unwrap().project_id();
    let path = repo.fx.root().join("created");
    let target = ResultTarget::NewBranchWorktree {
        branch: "result".into(),
        path: path.clone(),
        start: repo.a.clone(),
    };
    let validated = validate_target(&git, &work, id, &target).unwrap();
    assert_eq!(validated.expected_revision(), None);
    let spawned = git.spawn_count();
    let main_head = repo.fx.rev_parse("HEAD").unwrap();
    let created = create_branch_worktree(&git, &work, id, validated).unwrap();
    // Git이 만든 linked worktree: `.git`은 gitfile이고 branch는 시작 commit, 원래 worktree HEAD는 그대로입니다.
    assert!(path.join(".git").is_file());
    assert_eq!(repo.fx.rev_parse("refs/heads/result").unwrap(), repo.a);
    assert_eq!(repo.fx.rev_parse_in(&path, "HEAD").unwrap(), repo.a);
    assert_eq!(repo.fx.rev_parse("HEAD").unwrap(), main_head);
    let list = String::from_utf8(
        repo.fx
            .git(["worktree", "list", "--porcelain"])
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(list.contains("branch refs/heads/result"), "{list}");
    // 만든 프로젝트를 열어 revision을 정합니다(이후 쓰기의 기대 revision).
    let opened = open_snapshot(created.project_root()).unwrap();
    assert_eq!(created.expected_revision(), Some(opened.revision()));
    assert!(git.spawn_count() > spawned);
    // 다른 target은 아무것도 하지 않습니다.
    let current = validate_target(&git, &work, id, &ResultTarget::CurrentWorktree).unwrap();
    let before = git.spawn_count();
    assert_eq!(
        create_branch_worktree(&git, &work, id, current.clone()).unwrap(),
        current
    );
    assert_eq!(git.spawn_count(), before, "Git 실행 0");
}

#[test]
fn creation_failure_reports_the_git_error() {
    let Some(repo) = repo("merge-apply-create-fail", false) else {
        return;
    };
    let work = repo.fx.work().to_path_buf();
    let git = repo.fx.gated();
    let id = open_snapshot(&work).unwrap().project_id();
    let path = repo.fx.root().join("raced");
    let validated = validate_target(
        &git,
        &work,
        id,
        &ResultTarget::NewBranchWorktree {
            branch: "raced".into(),
            path: path.clone(),
            start: repo.a.clone(),
        },
    )
    .unwrap();
    // 검증 뒤 다른 프로세스가 그 경로에 파일을 만들었습니다. Git이 거부하고 그 결과를 그대로 알립니다.
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("occupied"), b"x").unwrap();
    assert!(matches!(
        create_branch_worktree(&git, &work, id, validated),
        Err(TargetError::Git(_))
    ));
    assert_eq!(std::fs::read(path.join("occupied")).unwrap(), b"x");
}

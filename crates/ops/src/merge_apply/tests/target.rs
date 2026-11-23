//! 결과 target 검증·생성 시험(시스템 Git fixture). Git이 없는 환경에서는 건너뜁니다.

use std::path::Path;

use stellar_git_porcelain::object::ObjectId;

use super::fixture::repo;
use crate::merge_apply::target::{validate_target, ResultTarget, TargetError};
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

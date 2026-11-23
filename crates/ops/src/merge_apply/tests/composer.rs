//! Merge Composer 결과 적용 통합 시험(시스템 Git fixture). Git이 없는 환경에서는 건너뜁니다.

use stellar_git_porcelain::composer::SourceRef;
use stellar_semantic_merge::candidate::{CandidateCommand, Resolution};
use stellar_semantic_merge::model::ConflictTarget;

use super::fixture::{branches, git_state, name_of, rename, repo, scratch};
use crate::lock::LockPolicy;
use crate::merge_apply::composer::{apply_composition, compose, ComposerApplyError};
use crate::merge_apply::target::{validate_target, ResultTarget};
use crate::test_support::file_digest;

#[test]
fn clean_composition_is_saved_to_current_worktree_without_commit() {
    let Some(repo) = repo("merge-apply-current", false) else {
        return;
    };
    let work = repo.fx.work().to_path_buf();
    let git = repo.fx.gated();
    let (a, b) = branches();
    let composition = compose(&git, &work, &a, &b, &scratch(&repo)).unwrap();
    assert_eq!(composition.candidate().unresolved_count(), 0);
    assert_eq!(composition.base(), &repo.base);
    let before = git_state(&repo.fx, &work);
    let target = validate_target(
        &git,
        &work,
        composition.project_id(),
        &ResultTarget::CurrentWorktree,
    )
    .unwrap();
    let report = apply_composition(
        &git,
        &composition,
        target,
        &LockPolicy::None,
        &scratch(&repo),
    )
    .unwrap();
    assert_eq!(report.target, "current-worktree");
    assert_eq!(report.created_branch, None);
    assert_eq!(
        report.revision.generation(),
        report.base_revision.generation() + 1
    );
    assert!(report.written_chunks > 0);
    // C의 두 변경이 모두 target에 있습니다.
    assert_eq!(name_of(&work, repo.document, repo.children[0]), "from-a");
    assert_eq!(name_of(&work, repo.document, repo.children[1]), "from-b");
    // HEAD·index는 그대로이고 merge 진행 표시도 없습니다(commit·add·merge 없음).
    assert_eq!(git_state(&repo.fx, &work), before);
    let status = String::from_utf8(repo.fx.git(["status", "--porcelain"]).unwrap().stdout).unwrap();
    assert!(status.contains(" M design/current"), "{status}");
}

#[test]
fn new_branch_worktree_is_created_by_git_and_receives_c() {
    let Some(repo) = repo("merge-apply-new", false) else {
        return;
    };
    let work = repo.fx.work().to_path_buf();
    let git = repo.fx.gated();
    let (a, b) = branches();
    let composition = compose(&git, &work, &a, &b, &scratch(&repo)).unwrap();
    let path = repo.fx.root().join("merged-wt");
    let target = validate_target(
        &git,
        &work,
        composition.project_id(),
        &ResultTarget::NewBranchWorktree {
            branch: "merged".into(),
            path: path.clone(),
            start: repo.a.clone(),
        },
    )
    .unwrap();
    // 검증은 아무것도 만들지 않습니다.
    assert!(!path.exists());
    assert!(repo
        .fx
        .git(["rev-parse", "-q", "--verify", "refs/heads/merged"])
        .is_err());
    let main_before = git_state(&repo.fx, &work);
    let report = apply_composition(
        &git,
        &composition,
        target,
        &LockPolicy::None,
        &scratch(&repo),
    )
    .unwrap();
    assert_eq!(report.target, "new-branch-worktree");
    assert_eq!(report.created_branch.as_deref(), Some("merged"));
    // Git이 만든 linked worktree(`.git` gitfile)와 branch이며 새 branch는 시작 commit 그대로입니다.
    assert!(path.join(".git").is_file());
    assert_eq!(repo.fx.rev_parse("refs/heads/merged").unwrap(), repo.a);
    let list = String::from_utf8(
        repo.fx
            .git(["worktree", "list", "--porcelain"])
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(list.contains("branch refs/heads/merged"), "{list}");
    let (head, _, merging) = git_state(&repo.fx, &path);
    assert_eq!(head, repo.a);
    assert!(!merging);
    assert_eq!(name_of(&path, repo.document, repo.children[0]), "from-a");
    assert_eq!(name_of(&path, repo.document, repo.children[1]), "from-b");
    // 원래 worktree는 바뀌지 않습니다.
    assert_eq!(git_state(&repo.fx, &work), main_before);
    assert_ne!(name_of(&work, repo.document, repo.children[0]), "from-a");
}

#[test]
fn existing_worktree_target_receives_c() {
    let Some(repo) = repo("merge-apply-existing", false) else {
        return;
    };
    let work = repo.fx.work().to_path_buf();
    let git = repo.fx.gated();
    let other = repo.fx.add_worktree("other", "b").unwrap();
    let (a, b) = branches();
    let composition = compose(&git, &work, &a, &b, &scratch(&repo)).unwrap();
    let target = validate_target(
        &git,
        &work,
        composition.project_id(),
        &ResultTarget::ExistingWorktree {
            path: other.clone(),
        },
    )
    .unwrap();
    let report = apply_composition(
        &git,
        &composition,
        target,
        &LockPolicy::None,
        &scratch(&repo),
    )
    .unwrap();
    assert_eq!(report.target, "existing-worktree");
    assert_eq!(name_of(&other, repo.document, repo.children[0]), "from-a");
    assert_eq!(name_of(&other, repo.document, repo.children[1]), "from-b");
}

#[test]
fn moved_source_blocks_apply_and_writes_nothing() {
    let Some(repo) = repo("merge-apply-stale", false) else {
        return;
    };
    let work = repo.fx.work().to_path_buf();
    let git = repo.fx.gated();
    let (a, b) = branches();
    let composition = compose(&git, &work, &a, &b, &scratch(&repo)).unwrap();
    let target = validate_target(
        &git,
        &work,
        composition.project_id(),
        &ResultTarget::CurrentWorktree,
    )
    .unwrap();
    // C를 만든 뒤 branch b가 움직입니다.
    let other = repo.fx.add_worktree("mover", "b").unwrap();
    repo.fx.git_in(&other, ["switch", "-q", "b"]).unwrap();
    repo.fx.write_in(&other, "notes.txt", b"moved\n").unwrap();
    repo.fx.commit_all_in(&other, "move b").unwrap();
    let before = file_digest(&work);
    let error = apply_composition(
        &git,
        &composition,
        target,
        &LockPolicy::None,
        &scratch(&repo),
    )
    .unwrap_err();
    assert!(
        matches!(error, ComposerApplyError::StaleSource { side: "b", ref reason } if reason == "commit"),
        "{error}"
    );
    assert_eq!(file_digest(&work), before, "쓰기 0");
}

#[test]
fn changed_working_tree_source_blocks_apply() {
    let Some(repo) = repo("merge-apply-stale-wt", false) else {
        return;
    };
    let work = repo.fx.work().to_path_buf();
    let git = repo.fx.gated();
    // A = 현재 작업 트리(main), B = branch b.
    let composition = compose(
        &git,
        &work,
        &SourceRef::WorkingTree,
        &SourceRef::Branch("b".into()),
        &scratch(&repo),
    )
    .unwrap();
    let target = validate_target(
        &git,
        &work,
        composition.project_id(),
        &ResultTarget::CurrentWorktree,
    )
    .unwrap();
    std::fs::write(work.join("README.md"), b"edited after compose\n").unwrap();
    let before = file_digest(&work);
    let error = apply_composition(
        &git,
        &composition,
        target,
        &LockPolicy::None,
        &scratch(&repo),
    )
    .unwrap_err();
    assert!(
        matches!(error, ComposerApplyError::StaleSource { side: "a", ref reason } if reason == "content"),
        "{error}"
    );
    assert_eq!(file_digest(&work), before, "쓰기 0");
}

#[test]
fn target_changed_after_validation_is_stale() {
    let Some(repo) = repo("merge-apply-stale-target", false) else {
        return;
    };
    let work = repo.fx.work().to_path_buf();
    let git = repo.fx.gated();
    let (a, b) = branches();
    let composition = compose(&git, &work, &a, &b, &scratch(&repo)).unwrap();
    let target = validate_target(
        &git,
        &work,
        composition.project_id(),
        &ResultTarget::CurrentWorktree,
    )
    .unwrap();
    rename(&work, repo.document, repo.children[1], "meanwhile");
    let before = file_digest(&work);
    let error = apply_composition(
        &git,
        &composition,
        target,
        &LockPolicy::None,
        &scratch(&repo),
    )
    .unwrap_err();
    assert!(
        matches!(error, ComposerApplyError::StaleTarget { .. }),
        "{error}"
    );
    assert_eq!(file_digest(&work), before, "쓰기 0");
}

#[test]
fn unresolved_conflict_blocks_until_resolved() {
    let Some(repo) = repo("merge-apply-conflict", true) else {
        return;
    };
    let work = repo.fx.work().to_path_buf();
    let git = repo.fx.gated();
    let (a, b) = branches();
    let mut composition = compose(&git, &work, &a, &b, &scratch(&repo)).unwrap();
    assert_eq!(composition.candidate().unresolved_count(), 1);
    let id = composition.project_id();
    let target = || validate_target(&git, &work, id, &ResultTarget::CurrentWorktree).unwrap();
    let before = file_digest(&work);
    assert!(matches!(
        apply_composition(
            &git,
            &composition,
            target(),
            &LockPolicy::None,
            &scratch(&repo)
        ),
        Err(ComposerApplyError::Blocked(_))
    ));
    assert_eq!(file_digest(&work), before, "쓰기 0");
    let conflict = composition.candidate().conflicts()[0].clone();
    assert!(matches!(conflict.target, ConflictTarget::Property(_)));
    composition
        .candidate_mut()
        .apply(&CandidateCommand::Resolve {
            object: conflict.object,
            target: conflict.target,
            resolution: Resolution::ChooseTheirs,
        })
        .unwrap();
    apply_composition(
        &git,
        &composition,
        target(),
        &LockPolicy::None,
        &scratch(&repo),
    )
    .unwrap();
    assert_eq!(name_of(&work, repo.document, repo.children[0]), "from-b");
}

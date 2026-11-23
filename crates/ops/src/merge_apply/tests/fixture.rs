//! Merge Composer 시험 공용 시스템 Git fixture. Git이 없는 환경에서는 `repo`가 `None`입니다.

use std::path::{Path, PathBuf};

use stellar_git_porcelain::composer::SourceRef;
use stellar_git_porcelain::fixture::GitFixture;
use stellar_git_porcelain::object::ObjectId;

use crate::apply::{apply, ApplyMode, ApplyRequest, CommandInput};
use crate::core_types::ids::StableId;
use crate::lock::LockPolicy;
use crate::snapshot::open_snapshot;
use crate::test_support::{random_valid_document, save_text_document, scaffold_project};

/// fixture 저장소와 문서 정보.
pub(super) struct Repo {
    pub(super) fx: GitFixture,
    pub(super) document: StableId,
    pub(super) children: Vec<StableId>,
    /// 프로젝트가 생기기 전 commit.
    pub(super) before_project: ObjectId,
    /// 공통 base(main).
    pub(super) base: ObjectId,
    /// branch a 끝.
    pub(super) a: ObjectId,
}

pub(super) fn rename(root: &Path, document: StableId, node: StableId, name: &str) {
    let request = ApplyRequest {
        base_revision: open_snapshot(root).unwrap().revision(),
        document,
        commands: vec![CommandInput::Rename {
            node,
            name: name.into(),
        }],
    };
    apply(root, &request, ApplyMode::Write, &LockPolicy::None).unwrap();
}

/// `readme`(프로젝트 없음) ─ main(base) ─┬─ a: 첫 자식 이름 = `from-a`
///                                    └─ b: 둘째 자식(`b_same_node`면 첫 자식) 이름 = `from-b`
/// 끝에 `main`을 checkout합니다.
pub(super) fn repo(tag: &str, b_same_node: bool) -> Option<Repo> {
    let fx = GitFixture::with_system_git(tag).unwrap()?;
    fx.write("README.md", b"fixture\n").unwrap();
    let before_project = fx.commit_all("readme").unwrap();
    let work = fx.work().to_path_buf();
    scaffold_project(&work);
    let model = random_valid_document(7);
    save_text_document(&work, &model);
    let children: Vec<StableId> = model
        .to_records()
        .unwrap()
        .nodes
        .iter()
        .filter(|n| n.parent.is_some())
        .map(|n| n.id)
        .collect();
    assert!(children.len() >= 2);
    let document = model.document_id();
    let base = fx.commit_all("base").unwrap();
    fx.git(["switch", "-q", "-c", "a"]).unwrap();
    rename(&work, document, children[0], "from-a");
    let a = fx.commit_all("a").unwrap();
    fx.git(["switch", "-q", "-c", "b", base.as_str()]).unwrap();
    let b_node = if b_same_node {
        children[0]
    } else {
        children[1]
    };
    rename(&work, document, b_node, "from-b");
    fx.commit_all("b").unwrap();
    fx.git(["switch", "-q", "main"]).unwrap();
    Some(Repo {
        fx,
        document,
        children,
        before_project,
        base,
        a,
    })
}

pub(super) fn scratch(repo: &Repo) -> PathBuf {
    let dir = repo.fx.root().join("scratch");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub(super) fn name_of(root: &Path, document: StableId, node: StableId) -> String {
    open_snapshot(root)
        .unwrap()
        .document_records(document)
        .unwrap()
        .nodes
        .into_iter()
        .find(|n| n.id == node)
        .unwrap()
        .display_name
}

pub(super) fn branches() -> (SourceRef, SourceRef) {
    (SourceRef::Branch("a".into()), SourceRef::Branch("b".into()))
}

/// HEAD·index·진행 중 merge 표시를 읽습니다(적용 뒤 Git 상태 비교용).
pub(super) fn git_state(fx: &GitFixture, worktree: &Path) -> (ObjectId, String, bool) {
    let head = fx.rev_parse_in(worktree, "HEAD").unwrap();
    let index =
        String::from_utf8(fx.git_in(worktree, ["ls-files", "--stage"]).unwrap().stdout).unwrap();
    let merge_head = String::from_utf8(
        fx.git_in(worktree, ["rev-parse", "--git-path", "MERGE_HEAD"])
            .unwrap()
            .stdout,
    )
    .unwrap();
    let merging = worktree.join(merge_head.trim()).exists();
    (head, index, merging)
}

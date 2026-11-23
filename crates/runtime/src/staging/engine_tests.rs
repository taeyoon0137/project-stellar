//! Git 엔진 통합 시나리오(XIV.14.1 fixture: 로컬 bare 저장소, 네트워크 없음).
//!
//! - XIV.14.2: 같은 chunk 안 속성 일부 stage·unstage·commit
//! - XIV.14.3: 구조·컴포넌트 의존 staging(부분 선택 차단 → 승인 후 성공)
//! - XIV.14.5: linked worktree open·status·stage·commit
//! - XIV.8.1·XIV.8.4·XIV.8.5·XVI.7.3: 재검증 commit, commit 뒤 재조회, hook 실패, untrusted
//! - UX.5.1: worker 상태 조회와 실제 HEAD·upstream·remote·identity

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::PropertyPath;
use stellar_document::model::DocumentModel;
use stellar_document::records::NodeRecord;
use stellar_git_porcelain::commit::CommitError;
use stellar_git_porcelain::fixture::GitFixture;
use stellar_git_porcelain::location::WorktreeKind;
use stellar_git_porcelain::repo_status::{HeadState, UpstreamState};
use stellar_git_porcelain::runner::GitRunner;
use stellar_git_porcelain::trace::{GitCommandKind, GitCommandTrace, GitTraceSink};
use stellar_git_porcelain::trust::{GatedGitRunner, GitTrust};
use stellar_ops::candidate::{ChangeId, DISPLAY_NAME};
use stellar_ops::git::{GitError, GitItem, GitSession, GitStatusReport, SourceUnit, StatusRequest};
use stellar_ops::git::{SemanticChangeKind, StatusArea};
use stellar_ops::test_support::{large_document, save_documents, scaffold_project, web_frame};

use super::{submit_git_status, GitStatusJob};
use crate::worker_pool::{WorkerPool, WorkerPoolConfig};

fn fixture(tag: &str) -> GitFixture {
    GitFixture::with_system_git(tag)
        .unwrap()
        .expect("system Git required")
}

fn session(fx: &GitFixture) -> GitSession {
    GitSession::open(fx.gated(), fx.work()).unwrap()
}

fn status(session: &GitSession) -> GitStatusReport {
    session.status(&StatusRequest::default()).unwrap()
}

fn area(report: &GitStatusReport, area: StatusArea) -> Vec<ChangeId> {
    report
        .semantic
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .filter(|e| e.area == area)
        .map(|e| e.id.clone())
        .collect()
}

fn git_text(fx: &GitFixture, dir: &Path, args: &[&str]) -> String {
    String::from_utf8(fx.git_in(dir, args.iter().copied()).unwrap().stdout).unwrap()
}

fn root_of(model: &DocumentModel) -> StableId {
    model
        .to_records()
        .unwrap()
        .nodes
        .iter()
        .find(|n| n.parent.is_none())
        .unwrap()
        .id
}

/// root display_name과 locked를 함께 바꾼 working 문서(같은 node → 같은 chunk).
fn edit_root(model: &DocumentModel) -> DocumentModel {
    let mut records = model.to_records().unwrap();
    let root = records
        .nodes
        .iter_mut()
        .find(|n| n.parent.is_none())
        .unwrap();
    root.display_name = "staged name".into();
    root.locked = true;
    DocumentModel::from_records(records).unwrap().0
}

/// commit만 세는 trace sink.
#[derive(Default)]
struct CommitCounter(AtomicU64);

impl GitTraceSink for CommitCounter {
    fn record(&self, trace: &GitCommandTrace) {
        if trace.kind == GitCommandKind::Commit {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}

fn counted(fx: &GitFixture) -> (GatedGitRunner, Arc<CommitCounter>) {
    let counter = Arc::new(CommitCounter::default());
    let runner: GitRunner = fx.runner().clone().with_trace_sink(counter.clone());
    (GatedGitRunner::new(runner, GitTrust::Trusted), counter)
}

fn baseline(tag: &str) -> (GitFixture, DocumentModel) {
    let fx = fixture(tag);
    scaffold_project(fx.work());
    let model = large_document(4);
    save_documents(fx.work(), &[&model]);
    fx.commit_all("baseline").unwrap();
    (fx, model)
}

#[test]
fn partial_same_chunk_property_stage_unstage_commit() {
    let (fx, model) = baseline("engine-partial");
    let root = root_of(&model);
    save_documents(fx.work(), &[&edit_root(&model)]);
    let s = session(&fx);
    let before = status(&s);
    let unstaged = area(&before, StatusArea::Unstaged);
    assert_eq!(unstaged.len(), 2, "display_name·locked");
    assert!(area(&before, StatusArea::Staged).is_empty());
    let name = ChangeId::property(PropertyPath::with_keys(root, vec![DISPLAY_NAME]));
    assert!(unstaged.contains(&name));
    let locked = unstaged.iter().find(|id| **id != name).unwrap().clone();

    // stage: display_name만.
    s.stage(vec![GitItem::Change(name.clone())], vec![])
        .unwrap();
    let staged = status(&s);
    assert_eq!(area(&staged, StatusArea::Staged), vec![name.clone()]);
    assert_eq!(area(&staged, StatusArea::Unstaged), vec![locked.clone()]);
    assert_ne!(staged.revision, before.revision);

    // unstage: 다시 원래대로.
    s.unstage(vec![GitItem::Change(name.clone())], vec![])
        .unwrap();
    let back = status(&s);
    assert!(area(&back, StatusArea::Staged).is_empty());
    assert_eq!(area(&back, StatusArea::Unstaged).len(), 2);
    assert!(git_text(&fx, fx.work(), &["diff", "--cached", "--name-only"]).is_empty());

    // stage → commit.
    s.stage(vec![GitItem::Change(name.clone())], vec![])
        .unwrap();
    let previous = fx.rev_parse("HEAD").unwrap();
    let working_before = std::fs::read(fx.work().join("design/current")).unwrap();
    let outcome = s.commit("이름만 커밋\n").unwrap();
    assert_eq!(outcome.previous_head, Some(previous.clone()));
    assert_ne!(outcome.head, previous);
    assert_eq!(outcome.staged_remaining, 0);
    assert!(outcome.tree_matches_validated);
    assert_eq!(outcome.repository.counts.staged, 0);
    assert!(git_text(&fx, fx.work(), &["diff", "--cached", "--name-only"]).is_empty());
    // commit은 선택 속성이 든 chunk·새 generation·current만 바꿉니다.
    let shown = git_text(
        &fx,
        fx.work(),
        &["show", "--name-only", "--format=", "HEAD"],
    );
    let files: Vec<&str> = shown.lines().collect();
    assert_eq!(files.len(), 3, "{files:?}");
    assert!(files.iter().all(|f| f.starts_with("design/")), "{files:?}");
    assert!(files.contains(&"design/current"));
    // HEAD = baseline + 이름, WT = HEAD + locked.
    let after = status(&s);
    assert!(area(&after, StatusArea::Staged).is_empty());
    assert_eq!(area(&after, StatusArea::Unstaged), vec![locked]);
    assert_eq!(
        std::fs::read(fx.work().join("design/current")).unwrap(),
        working_before,
        "작업 트리 불변"
    );
    // 자동 push 없음: bare 저장소에 main ref가 없습니다.
    assert!(fx
        .git_in(
            fx.bare(),
            ["rev-parse", "-q", "--verify", "refs/heads/main"]
        )
        .is_err());
    assert_eq!(after.revision, outcome.revision);
}

#[test]
fn invalid_staged_snapshot_never_runs_git_commit() {
    let (fx, _) = baseline("engine-invalid");
    // 기존 chunk object를 손상시켜 stage합니다.
    let object = git_text(&fx, fx.work(), &["ls-files", "design/objects"])
        .lines()
        .next()
        .unwrap()
        .to_string();
    std::fs::write(fx.work().join(&object), b"corrupt chunk").unwrap();
    fx.git(["add", "--", object.as_str()]).unwrap();
    let head = fx.rev_parse("HEAD").unwrap();
    let (gate, counter) = counted(&fx);
    let s = GitSession::open(gate, fx.work()).unwrap();
    match s.commit("손상\n") {
        Err(GitError::InvalidStagedSnapshot(findings)) => {
            assert!(findings.iter().any(|f| f.error), "{findings:?}")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(counter.0.load(Ordering::SeqCst), 0, "git commit spawn 0");
    assert_eq!(fx.rev_parse("HEAD").unwrap(), head);
}

#[test]
fn failing_hook_blocks_commit_and_result_is_returned_as_is() {
    use std::os::unix::fs::PermissionsExt;
    let (fx, model) = baseline("engine-hook");
    save_documents(fx.work(), &[&edit_root(&model)]);
    let s = session(&fx);
    s.stage(vec![GitItem::Unit(SourceUnit::Generation)], vec![])
        .unwrap();
    let hook = fx.work().join(".git/hooks/pre-commit");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, b"#!/bin/sh\necho hook-rejected >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let head = fx.rev_parse("HEAD").unwrap();
    match s.commit("hook\n") {
        Err(GitError::Commit(CommitError::Git(failure))) => {
            assert_ne!(failure.exit_code, Some(0));
            assert!(
                String::from_utf8_lossy(&failure.stderr).contains("hook-rejected"),
                "{failure:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(fx.rev_parse("HEAD").unwrap(), head, "commit 없음");
    assert!(
        !area(&status(&s), StatusArea::Staged).is_empty(),
        "index 유지"
    );
}

#[test]
fn untrusted_project_spawns_no_git_for_any_operation() {
    let (fx, _) = baseline("engine-untrusted");
    let runner = fx.runner().clone();
    let before = runner.spawn_count();
    let gate = GatedGitRunner::new(runner.clone(), GitTrust::NotTrusted);
    assert!(matches!(
        GitSession::open(gate, fx.work()),
        Err(GitError::NotTrusted)
    ));
    assert_eq!(runner.spawn_count(), before);
}

#[test]
fn moving_into_new_frame_requires_approval_of_the_addition() {
    let (fx, model) = baseline("engine-structural");
    let mut records = model.to_records().unwrap();
    let root = root_of(&model);
    let group = records
        .nodes
        .iter()
        .find(|n| n.parent == Some(root))
        .unwrap()
        .clone();
    let moved = records
        .nodes
        .iter()
        .find(|n| n.parent == Some(group.id))
        .unwrap()
        .id;
    let added = StableId::new_v7(1_950_000_000_000, 0x0abc, 9);
    records.nodes.push(NodeRecord {
        id: added,
        parent: Some(root),
        order_key: OrderKey::between(Some(&group.order_key), None),
        visible: true,
        locked: false,
        display_name: "new frame".into(),
        payload: group.payload.clone(),
    });
    let node = records.nodes.iter_mut().find(|n| n.id == moved).unwrap();
    node.parent = Some(added);
    node.order_key = OrderKey::between(None, None);
    save_documents(
        fx.work(),
        &[&DocumentModel::from_records(records).unwrap().0],
    );
    let s = session(&fx);
    let index_before = std::fs::read(fx.work().join(".git/index")).unwrap();
    let position = ChangeId::position(moved);
    let required = match s.stage(vec![GitItem::Change(position.clone())], vec![]) {
        Err(GitError::CompanionRequired { changes, units, .. }) => {
            assert!(units.is_empty());
            assert!(changes.contains(&ChangeId::add(added)), "{changes:?}");
            changes
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(
        std::fs::read(fx.work().join(".git/index")).unwrap(),
        index_before,
        "부분 선택 차단: index 불변"
    );
    // 필수가 아닌 승인은 거부합니다.
    assert!(matches!(
        s.stage(
            vec![GitItem::Change(position.clone())],
            vec![GitItem::Change(ChangeId::remove(root))]
        ),
        Err(GitError::UnexpectedApproval(_))
    ));
    let approved = required.into_iter().map(GitItem::Change).collect();
    s.stage(vec![GitItem::Change(position.clone())], approved)
        .unwrap();
    let staged: BTreeSet<_> = area(&status(&s), StatusArea::Staged).into_iter().collect();
    assert!(staged.contains(&position) && staged.contains(&ChangeId::add(added)));
    assert!(s.validate_staged().unwrap().is_valid());
}

fn write_component(root: &Path) -> PathBuf {
    use stellar_project::document_io::component_source::{create, ComponentSource};
    let dir = root.join("design/components");
    std::fs::create_dir_all(&dir).unwrap();
    let source = ComponentSource::new(
        web_frame::id(70),
        "Card".into(),
        StableId::new_v7(7_000, 0, 7_001),
    )
    .unwrap();
    let (path, _) = create(&dir, &source).unwrap();
    PathBuf::from("design/components").join(path.file_name().unwrap())
}

#[test]
fn component_definition_is_a_required_companion_unit_not_auto_included() {
    let fx = fixture("engine-component");
    scaffold_project(fx.work());
    save_documents(fx.work(), &[&web_frame::document()]);
    fx.commit_all("baseline").unwrap();
    let file = write_component(fx.work());
    save_documents(fx.work(), &[&web_frame::placed_components("")]);
    let s = session(&fx);
    let file_unit = SourceUnit::File(stellar_git_porcelain::decode::GitPath::from_bytes(
        file.to_str().unwrap().as_bytes(),
    ));
    let generation = GitItem::Unit(SourceUnit::Generation);
    match s.stage(vec![generation.clone()], vec![]) {
        Err(GitError::CompanionRequired {
            changes,
            units,
            reasons,
        }) => {
            assert!(changes.is_empty());
            assert_eq!(units, BTreeSet::from([file_unit.clone()]));
            assert_eq!(reasons.len(), 1);
            assert_eq!(reasons[0].component, web_frame::id(70));
        }
        other => panic!("{other:?}"),
    }
    assert!(git_text(&fx, fx.work(), &["diff", "--cached", "--name-only"]).is_empty());
    s.stage(
        vec![generation.clone()],
        vec![GitItem::Unit(file_unit.clone())],
    )
    .unwrap();
    let report = status(&s);
    for unit in [&SourceUnit::Generation, &file_unit] {
        let u = report.units.iter().find(|u| &u.unit == unit).unwrap();
        assert!(u.staged && !u.unstaged && !u.untracked, "{u:?}");
    }
    // 정의 파일만 빼면 index 인스턴스가 끊기므로 generation 동반 unstage가 필요합니다.
    match s.unstage(vec![GitItem::Unit(file_unit.clone())], vec![]) {
        Err(GitError::CompanionRequired { units, .. }) => {
            assert_eq!(units, BTreeSet::from([SourceUnit::Generation]))
        }
        other => panic!("{other:?}"),
    }
    let outcome = s.commit("컴포넌트와 배치\n").unwrap();
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert!(git_text(
        &fx,
        fx.work(),
        &["show", "--name-only", "--format=", "HEAD"]
    )
    .lines()
    .any(|l| Path::new(l) == file));
}

#[test]
fn linked_worktree_uses_its_own_index_and_leaves_main_index_alone() {
    let (fx, model) = baseline("engine-worktree");
    let wt = fx.add_worktree("linked", "HEAD").unwrap();
    let main_index = std::fs::read(fx.work().join(".git/index")).unwrap();
    let main_head = fx.rev_parse("HEAD").unwrap();
    save_documents(&wt, &[&edit_root(&model)]);
    let s = GitSession::open(fx.gated(), &wt).unwrap();
    assert_eq!(s.location().kind, WorktreeKind::Linked);
    let main = session(&fx);
    assert_eq!(s.location().common_dir, main.location().common_dir);
    assert_ne!(s.location().git_dir, main.location().git_dir);
    let report = status(&s);
    assert_eq!(area(&report, StatusArea::Unstaged).len(), 2);
    assert!(matches!(report.repository.head, HeadState::Detached { .. }));
    // 주 worktree는 깨끗합니다.
    assert!(area(&status(&main), StatusArea::Unstaged).is_empty());
    s.stage(vec![GitItem::Unit(SourceUnit::Generation)], vec![])
        .unwrap();
    assert_eq!(
        std::fs::read(fx.work().join(".git/index")).unwrap(),
        main_index,
        "다른 worktree index 변경 없음"
    );
    let outcome = s.commit("linked commit\n").unwrap();
    assert_eq!(outcome.staged_remaining, 0);
    assert_eq!(fx.rev_parse("HEAD").unwrap(), main_head);
    assert_eq!(fx.rev_parse_in(&wt, "HEAD").unwrap(), outcome.head);
    assert_eq!(
        std::fs::read(fx.work().join(".git/index")).unwrap(),
        main_index
    );
    assert!(area(&status(&s), StatusArea::Unstaged).is_empty());
}

#[test]
fn status_runs_on_worker_with_real_head_upstream_remote_identity() {
    let (fx, model) = baseline("engine-worker");
    fx.push("main").unwrap();
    fx.git(["branch", "--set-upstream-to=origin/main"]).unwrap();
    save_documents(fx.work(), &[&edit_root(&model)]);
    let s = session(&fx);
    let pool = WorkerPool::new(WorkerPoolConfig::new(2).unwrap()).unwrap();
    let (tx, rx) = mpsc::channel();
    submit_git_status(
        &pool,
        GitStatusJob {
            sequence: 7,
            session: s.clone(),
            request: StatusRequest {
                cursor: None,
                limit: 1,
            },
        },
        tx,
    )
    .unwrap();
    let result = rx.recv_timeout(Duration::from_secs(60)).unwrap();
    assert_eq!(result.sequence, 7);
    let thread = result.thread.unwrap();
    assert!(thread.starts_with("stellar-worker"), "{thread}");
    assert_ne!(Some(thread.as_str()), std::thread::current().name());
    let report = result.status.unwrap();
    assert_eq!(report.revision, s.revision().unwrap());
    assert!(
        report.missing_identity.is_none(),
        "identity가 설정된 저장소"
    );
    assert!(matches!(&report.repository.head, HeadState::Branch { name, .. } if name == b"main"));
    assert!(matches!(
        &report.repository.upstream,
        UpstreamState::Tracking {
            ahead: 0,
            behind: 0,
            ..
        }
    ));
    assert_eq!(report.remotes.len(), 1);
    assert_eq!(report.remotes[0].name, "origin");
    // page 1개씩: 두 번째 page까지 읽고, 상태가 바뀌면 cursor는 stale입니다.
    let first = report.semantic.unwrap();
    assert_eq!(first.entries.len(), 1);
    assert_eq!(first.unstaged_total, 2);
    let cursor = first.next_cursor.unwrap();
    let second = s
        .status(&StatusRequest {
            cursor: Some(cursor.clone()),
            limit: 1,
        })
        .unwrap()
        .semantic
        .unwrap();
    assert_eq!(second.entries.len(), 1);
    assert_ne!(second.entries[0].id, first.entries[0].id);
    assert!(second.next_cursor.is_none());
    assert!(second
        .entries
        .iter()
        .all(|e| e.kind == SemanticChangeKind::SetProperty));
    s.stage(vec![GitItem::Change(first.entries[0].id.clone())], vec![])
        .unwrap();
    assert!(matches!(
        s.status(&StatusRequest {
            cursor: Some(cursor),
            limit: 1
        }),
        Err(GitError::StaleCursor)
    ));
    drop(pool);
}

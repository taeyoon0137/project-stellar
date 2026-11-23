//! V.10.7 남은 범위(ADR-0122): chunk cache가 Git 밖(`.stellar/cache/chunks/`)에 있는 Git 시나리오.
//!
//! 1. 두 branch가 각각 저장한 뒤 병합하면 conflict 후보는 텍스트 원본 파일뿐이고(cache 경로 없음), conflict 세션으로
//!    해결한 뒤 로컬 cache는 원본에서 다시 만들어집니다.
//! 2. clone 직후 cache가 없어도 원본에서 읽고, `stellar cache rebuild`와 같은 함수가 cache를 만듭니다.
//! 3. 옛 배치(`design/current` 등)를 추적하던 저장소는 열기 관찰이 cache 위치로 옮기고, `GitSession::migrate_cache`가 index에서
//!    빼 다음 commit이 옛 cache를 Git에서 지웁니다.
//! 4. cache GC는 보존 정책 밖 cache 파일만 지우고 Git 상태를 바꾸지 않습니다.

#![cfg(unix)] // Windows 경로는 보류 상태라 검증하지 않습니다(ADR-0006).

mod git_common;

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use git_common::card::*;
use git_common::*;
use stellar_git_porcelain::fixture::{FixtureError, GitFixture};
use stellar_ops::candidate::apply::apply_session;
use stellar_ops::candidate::git_port::{GatedMergeGit, MergeGit};
use stellar_ops::candidate::session::{ConflictSession, UnitKind};
use stellar_ops::git::GitSession;
use stellar_ops::git_cache;
use stellar_ops::lock::LockPolicy;
use stellar_ops::snapshot::open_snapshot;
use stellar_project::cache::Rebuild;
use stellar_project::document_io::session::OpenPlan;
use stellar_semantic_merge::candidate::Resolution;
use stellar_storage::gc::retention::RetentionPolicy;
use stellar_storage::generation::current::current_pointer_path;
use stellar_storage::location::{cache_dir, is_cache_path, is_legacy_cache_path};

fn text(fx: &GitFixture, dir: &Path, args: &[&str]) -> String {
    String::from_utf8(fx.git_in(dir, args.iter().copied()).unwrap().stdout).unwrap()
}

/// Git이 추적하는 파일에 cache 경로(새 위치·옛 배치)가 없는지.
fn assert_no_cache_tracked(fx: &GitFixture, dir: &Path) {
    let tracked = text(fx, dir, &["ls-files"]);
    assert!(
        tracked
            .lines()
            .all(|l| !is_cache_path(l.as_bytes()) && !is_legacy_cache_path(l.as_bytes())),
        "{tracked}"
    );
    assert!(
        !tracked.lines().any(|l| l.starts_with(".stellar/")),
        "{tracked}"
    );
}

/// 문서 하나의 node 이름(원본 기준으로 엶).
fn name_of(root: &Path, node: u64) -> String {
    let snapshot = open_snapshot(root).unwrap();
    let id = snapshot.documents()[0].id;
    snapshot
        .document_records(id)
        .unwrap()
        .nodes
        .into_iter()
        .find(|n| n.id == nid(node))
        .unwrap()
        .display_name
}

/// base를 commit·push하고, peer clone의 `theirs`와 work의 `main`이 같은 아이콘 이름을 다르게 바꿔 각각 저장·commit한 뒤
/// work에서 `origin/theirs`를 병합합니다.
fn diverged(tag: &str) -> Option<(GitFixture, PathBuf)> {
    let fx = git_fixture(tag)?;
    let work = fx.work().to_path_buf();
    write_manifest(&work, "Cache out");
    let card = card_document();
    save_documents(&work, &[&card]);
    fx.commit_all("base").unwrap();
    fx.push("main").unwrap();
    assert_no_cache_tracked(&fx, &work);
    // 저장은 로컬 cache를 만들었지만 Git은 보지 않습니다.
    assert!(current_pointer_path(&work).is_file());
    assert_eq!(
        text(
            &fx,
            &work,
            &["status", "--porcelain", "--untracked-files=all"]
        ),
        ""
    );

    let peer = fx.clone_bare("peer").unwrap();
    fx.git_in(&peer, ["switch", "-q", "-c", "theirs"]).unwrap();
    // clone에는 cache가 없습니다. 원본에서 읽습니다.
    assert!(!cache_dir(&peer).exists());
    save_documents(
        &peer,
        &[&edit(&card, |r| {
            rename(r, ICON, "아이콘 (theirs)");
            node_mut(r, DIVIDER).visible = false;
        })],
    );
    fx.commit_all_in(&peer, "theirs").unwrap();
    fx.git_in(&peer, ["push", "-q", "origin", "theirs"])
        .unwrap();

    save_documents(
        &work,
        &[&edit(&card, |r| {
            rename(r, ICON, "아이콘 (ours)");
            resize_frame(r, CARD, 400.0, 200.0);
        })],
    );
    fx.commit_all("ours").unwrap();
    assert_no_cache_tracked(&fx, &work);
    fx.git(["fetch", "-q", "origin"]).unwrap();
    Some((fx, work))
}

#[test]
fn two_branch_saves_merge_with_only_text_sources_as_conflict_candidates() {
    let Some((fx, work)) = diverged("cache-out-merge") else {
        return;
    };
    let merged = fx.git(["merge", "--no-edit", "origin/theirs"]);
    assert!(
        matches!(merged, Err(FixtureError::Git { .. })),
        "같은 이름 줄을 다르게 바꿨으므로 conflict: {merged:?}"
    );
    let port = GatedMergeGit::new(fx.gated());
    let unmerged: Vec<PathBuf> = port
        .unmerged(&work)
        .unwrap()
        .into_iter()
        .map(|e| e.path)
        .collect();
    // conflict 후보는 텍스트 원본 파일뿐입니다(옛 `design/current` conflict 없음).
    assert!(!unmerged.is_empty());
    assert!(
        unmerged.iter().all(|p| p.starts_with("design/")
            && p.to_string_lossy().ends_with(".stellar.yaml")
            && !is_legacy_cache_path(p.to_string_lossy().as_bytes())),
        "{unmerged:?}"
    );
    let status = text(
        &fx,
        &work,
        &["status", "--porcelain", "--untracked-files=all"],
    );
    assert!(!status.contains(" .stellar/"), "{status}");

    let mut session = ConflictSession::open(&port, &work).unwrap();
    assert!(
        session.unsupported().is_empty(),
        "{:?}",
        session.unsupported()
    );
    let kinds: Vec<UnitKind> = session.units().iter().map(|u| u.kind()).collect();
    assert_eq!(kinds, vec![UnitKind::TextSource]);
    let conflicts: Vec<usize> = session.conflicts().iter().map(|c| c.index).collect();
    assert_eq!(conflicts.len(), 1, "아이콘 이름 하나");
    session
        .resolve(conflicts[0], Resolution::ChooseTheirs)
        .unwrap();
    let report = apply_session(&port, &session).unwrap();
    assert_eq!(report.remaining, Some(Vec::new()));
    assert!(report.written.iter().all(|p| !p.starts_with(".stellar")));

    // 병합 뒤 로컬 cache는 원본과 짝이 아니라 원본에서 읽고, 다시 만들면 짝이 됩니다.
    assert_eq!(
        open_snapshot(&work).unwrap().open_plan(),
        OpenPlan::FromText
    );
    assert_eq!(name_of(&work, ICON), "아이콘 (theirs)");
    let Rebuild::Rebuilt { .. } = git_cache::rebuild(&work, &LockPolicy::None).unwrap() else {
        panic!("원본에서 cache를 다시 만들어야 함");
    };
    assert_eq!(open_snapshot(&work).unwrap().open_plan(), OpenPlan::Cache);
    assert_eq!(name_of(&work, ICON), "아이콘 (theirs)");
    // 한쪽만 바꾼 크기·숨김은 함께 들어왔습니다.
    let snapshot = open_snapshot(&work).unwrap();
    let records = snapshot
        .document_records(snapshot.documents()[0].id)
        .unwrap();
    assert!(
        !records
            .nodes
            .iter()
            .find(|n| n.id == nid(DIVIDER))
            .unwrap()
            .visible
    );
    fx.git(["commit", "--no-edit", "-q"]).unwrap();
    assert_no_cache_tracked(&fx, &work);
    assert_eq!(
        text(
            &fx,
            &work,
            &["status", "--porcelain", "--untracked-files=all"]
        ),
        ""
    );
}

#[test]
fn clone_without_cache_reads_the_text_source_and_rebuilds_the_cache() {
    let Some((fx, work)) = diverged("cache-out-clone") else {
        return;
    };
    fx.push("main").unwrap();
    let clone = fx.clone_bare("fresh").unwrap();
    assert!(!clone.join(".stellar").exists(), "cache는 clone되지 않음");
    let status = git_cache::status(&clone).unwrap();
    assert_eq!(status.plan, "from-text");
    assert_eq!(status.current, None);
    assert_eq!(name_of(&clone, ICON), name_of(&work, ICON));
    // 읽기는 cache를 만들지 않습니다.
    assert!(!current_pointer_path(&clone).exists());
    let Rebuild::Rebuilt { generation, .. } =
        git_cache::rebuild(&clone, &LockPolicy::None).unwrap()
    else {
        panic!("clone 직후 cache를 원본에서 만들어야 함");
    };
    assert_eq!(generation, 1);
    assert_eq!(git_cache::status(&clone).unwrap().plan, "cache");
    assert_eq!(name_of(&clone, ICON), name_of(&work, ICON));
    // cache는 Git 밖이라 clone의 작업 트리는 깨끗합니다.
    assert_eq!(
        text(
            &fx,
            &clone,
            &["status", "--porcelain", "--untracked-files=all"]
        ),
        ""
    );
}

#[test]
fn legacy_tracked_cache_is_moved_on_open_and_untracked_before_commit() {
    let Some(fx) = git_fixture("cache-out-legacy") else {
        return;
    };
    let work = fx.work().to_path_buf();
    write_manifest(&work, "Legacy");
    save_documents(&work, &[&card_document()]);
    // ADR-0122 이전 배치: cache 파일을 `design/` 아래로 옮기고 Git이 추적하게 합니다.
    let cache = cache_dir(&work);
    for name in ["current", "generations", "objects"] {
        std::fs::rename(cache.join(name), work.join("design").join(name)).unwrap();
    }
    fx.commit_all("legacy layout").unwrap();
    let tracked = text(&fx, &work, &["ls-files"]);
    assert!(tracked.lines().any(|l| l == "design/current"), "{tracked}");

    // 열기 관찰이 작업 트리의 옛 배치를 cache 위치로 옮깁니다(의미는 같음).
    let before = name_of(&work, ICON);
    assert!(!work.join("design/current").exists());
    // 원본이 있는 작업 트리에서 옛 pointer는 cache pointer로 쓰지 않습니다(ADR-0131 결정 4). 원본으로 읽습니다.
    assert!(!current_pointer_path(&work).is_file());
    assert_eq!(before, "아이콘");

    // stage·commit 전 정리: 원본이 있으므로 옛 경로를 index에서 뺍니다.
    let session = GitSession::open(fx.gated(), &work).unwrap();
    let untracked = session.migrate_cache().unwrap();
    assert!(
        untracked.iter().any(|p| p == "design/current"),
        "{untracked:?}"
    );
    assert!(untracked.iter().all(|p| is_legacy_cache_path(p.as_bytes())));
    let staged = text(&fx, &work, &["diff", "--cached", "--name-status"]);
    assert!(
        staged
            .lines()
            .all(|l| l.starts_with('D') && l.contains("design/")),
        "{staged}"
    );
    fx.git(["commit", "-q", "-m", "cache out of git"]).unwrap();
    assert_no_cache_tracked(&fx, &work);
    assert_eq!(
        text(
            &fx,
            &work,
            &["status", "--porcelain", "--untracked-files=all"]
        ),
        ""
    );
    assert_eq!(name_of(&work, ICON), "아이콘");
    // 두 번째 정리는 할 일이 없습니다.
    assert!(session.migrate_cache().unwrap().is_empty());
}

#[test]
fn cache_gc_removes_old_generations_without_touching_git() {
    let Some(fx) = git_fixture("cache-out-gc") else {
        return;
    };
    let work = fx.work().to_path_buf();
    write_manifest(&work, "GC");
    let card = card_document();
    save_documents(&work, &[&card]);
    for n in 0..3 {
        save_documents(
            &work,
            &[&edit(&card, |r| rename(r, ICON, &format!("아이콘 {n}")))],
        );
    }
    fx.commit_all("edits").unwrap();
    let status_before = text(
        &fx,
        &work,
        &["status", "--porcelain", "--untracked-files=all"],
    );
    let generations = || {
        std::fs::read_dir(cache_dir(&work).join("generations"))
            .unwrap()
            .count()
    };
    assert_eq!(generations(), 4);
    let policy = RetentionPolicy {
        keep_recent_generations: 1,
        grace_period_seconds: 0,
    };
    let report = stellar_project::cache::collect_garbage(
        &work,
        &policy,
        SystemTime::now() + Duration::from_secs(1),
    )
    .unwrap();
    assert!(report.deleted > 0, "{report:?}");
    assert_eq!(generations(), 1);
    assert_eq!(name_of(&work, ICON), "아이콘 2");
    assert_eq!(
        text(
            &fx,
            &work,
            &["status", "--porcelain", "--untracked-files=all"]
        ),
        status_before
    );
}

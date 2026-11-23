//! XIV.14.4: 시스템 Git으로 branch merge conflict를 만든 뒤 Semantic Conflict 해결·`git add`까지 통합 실행합니다.
//!
//! 판정 기준(계획 XIV.14.4): unmerged entries 0, merge state 유지(MERGE_HEAD가 남고 commit·continue는 하지 않음).
//!
//! 경로(ADR-0120 결정 5):
//!
//! - 저장소는 Git fixture harness(XIV.14.1)의 local bare 원격 + working 저장소 + peer clone입니다(네트워크 없음,
//!   hermetic 설정, 고정 author·시각). 두 branch의 편집은 실제 저장 파이프라인으로 generation을 씁니다.
//! - peer가 `theirs` branch를 push하고, work는 `main`에서 따로 편집한 뒤 `origin/theirs`를 fetch해 시스템 Git
//!   `git merge`로 병합합니다. merge driver를 등록하지 않아 `design/current` pointer가 텍스트 conflict로 남습니다.
//! - 해결은 `stellar merge-driver resolve`와 같은 conflict 세션(`ConflictSession` + `GatedMergeGit`)이고, 적용
//!   (`apply_session`)이 결과를 한 거래로 쓰고 `git add`합니다.
//!
//! 같은 node 이름을 두 branch가 다르게 바꾼 곳 하나만 semantic conflict이고, 한쪽만 바꾼 크기·숨김은 자동 병합되어야
//! 합니다. 해결 결과는 HEAD·MERGE_HEAD를 그대로 두고 index만 바꿉니다.

#![cfg(unix)] // Windows 경로는 보류 상태라 검증하지 않습니다(ADR-0006).

mod git_common;

use std::path::{Path, PathBuf};

use git_common::card::*;
use git_common::*;
use stellar_document::records::NodePayload;
use stellar_git_porcelain::fixture::{FixtureError, GitFixture};
use stellar_git_porcelain::object::ObjectId;
use stellar_ops::candidate::apply::apply_session;
use stellar_ops::candidate::git_port::{GatedMergeGit, GitOperation, MergeGit};
use stellar_ops::candidate::session::{ConflictSession, UnitKind};
use stellar_semantic_merge::candidate::Resolution;
use stellar_semantic_merge::model::ConflictTarget;

/// 진행 중인 병합.
struct Merging {
    fx: GitFixture,
    work: PathBuf,
    ours: ObjectId,
    theirs: ObjectId,
}

/// `git <args>` stdout(성공 강제).
fn out(fx: &GitFixture, dir: &Path, args: &[&str]) -> String {
    let output = fx.git_in(dir, args.iter().copied()).unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// base를 commit·push하고 peer의 `theirs`와 work의 `main`을 갈라 저장한 뒤 work에서 `origin/theirs`를 병합합니다.
fn start_merge(tag: &str) -> Option<Merging> {
    let fx = git_fixture(tag)?;
    let work = fx.work().to_path_buf();
    let card = card_document();
    write_manifest(&work, "Branch conflict");
    save_documents(&work, &[&card]);
    fx.commit_all("base").unwrap();
    fx.push("main").unwrap();

    // theirs: 아이콘 이름(충돌) + 구분선 숨김.
    let peer = fx.clone_bare("peer").unwrap();
    fx.git_in(&peer, ["switch", "-q", "-c", "theirs"]).unwrap();
    let theirs_card = edit(&card, |r| {
        rename(r, ICON, "아이콘 (theirs)");
        node_mut(r, DIVIDER).visible = false;
    });
    save_documents(&peer, &[&theirs_card]);
    let theirs = fx.commit_all_in(&peer, "theirs edit").unwrap();
    fx.git_in(&peer, ["push", "-q", "origin", "theirs"])
        .unwrap();

    // ours: 아이콘 이름(충돌) + 카드 크기.
    let ours_card = edit(&card, |r| {
        rename(r, ICON, "아이콘 (ours)");
        resize_frame(r, CARD, 400.0, 200.0);
    });
    save_documents(&work, &[&ours_card]);
    let ours = fx.commit_all("ours edit").unwrap();
    fx.git(["fetch", "-q", "origin"]).unwrap();

    // 시스템 Git 병합: 두 branch가 모두 새 generation을 저장해 pointer가 conflict입니다(종료 코드 1).
    let merged = fx.git(["merge", "--no-edit", "origin/theirs"]);
    assert!(
        matches!(merged, Err(FixtureError::Git { .. })),
        "design/current conflict가 나야 함: {merged:?}"
    );
    Some(Merging {
        fx,
        work,
        ours,
        theirs,
    })
}

#[test]
fn system_git_branch_conflict_is_resolved_semantically_and_added_keeping_merge_state() {
    let Some(m) = start_merge("branch-conflict-resolve") else {
        return;
    };
    let port = GatedMergeGit::new(m.fx.gated());
    let unmerged: Vec<PathBuf> = port
        .unmerged(&m.work)
        .unwrap()
        .into_iter()
        .map(|e| e.path)
        .collect();
    assert_eq!(unmerged, vec![PathBuf::from("design/current")]);
    assert_eq!(port.operation(&m.work).unwrap(), GitOperation::Merge);

    // conflict 세션: generation 단위 하나, raw·제외 경로 없음, semantic conflict는 아이콘 이름 하나.
    let mut session = ConflictSession::open(&port, &m.work).expect("conflict 세션");
    assert_eq!(session.operation(), GitOperation::Merge);
    assert!(
        session.raw_conflicts().is_empty(),
        "{:?}",
        session.raw_conflicts()
    );
    assert!(
        session.unsupported().is_empty(),
        "{:?}",
        session.unsupported()
    );
    let kinds: Vec<UnitKind> = session.units().iter().map(|u| u.kind()).collect();
    assert_eq!(kinds, vec![UnitKind::Generation]);
    let conflicts: Vec<(usize, ConflictTarget, stellar_core_types::ids::StableId)> = session
        .conflicts()
        .iter()
        .map(|c| (c.index, c.conflict.target.clone(), c.conflict.object))
        .collect();
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(conflicts[0].2, nid(ICON));
    assert_eq!(
        session.paths_for(nid(ICON)),
        vec![Path::new("design/current")]
    );

    // 미해결이면 아무것도 쓰거나 add하지 않습니다.
    assert!(apply_session(&port, &session).is_err());
    assert_eq!(
        port.unmerged(&m.work).unwrap().len(),
        1,
        "미해결 적용 뒤 unmerged 그대로"
    );

    // 해결: theirs 이름. 적용은 결과를 쓰고 git add합니다.
    session
        .resolve(conflicts[0].0, Resolution::ChooseTheirs)
        .unwrap();
    assert!(session.units().iter().all(|u| u.status().writable()));
    let report = apply_session(&port, &session).expect("해결 적용");
    assert_eq!(
        report.operation,
        Some(GitOperation::Merge),
        "merge 진행 중 그대로"
    );
    assert_eq!(report.remaining, Some(Vec::new()), "unmerged entries 0");
    assert!(report.added.contains(&PathBuf::from("design/current")));

    // Git 상태: unmerged 0(시스템 Git으로도 확인), MERGE_HEAD = theirs, HEAD = ours, commit 없음.
    assert_eq!(out(&m.fx, &m.work, &["ls-files", "-u"]), "");
    assert_eq!(
        out(&m.fx, &m.work, &["rev-parse", "MERGE_HEAD"]).trim(),
        m.theirs.as_str()
    );
    assert_eq!(m.fx.rev_parse("HEAD").unwrap(), m.ours);
    assert!(
        out(&m.fx, &m.work, &["status"]).contains("All conflicts fixed but you are still merging"),
        "merge state 유지"
    );
    // 작업 트리 = index(해결 결과가 모두 add됨).
    assert_eq!(out(&m.fx, &m.work, &["diff", "--name-only"]), "");

    // 결과 의미: 아이콘 = theirs, 카드 크기 = ours, 구분선 숨김 = theirs.
    let project = open_project(&m.work);
    let id = project.document_ids().unwrap()[0];
    let records = project
        .load_document(id)
        .unwrap()
        .model
        .to_records()
        .unwrap();
    let node = |n: u64| records.nodes.iter().find(|r| r.id == nid(n)).unwrap();
    assert_eq!(node(ICON).display_name, "아이콘 (theirs)");
    assert!(!node(DIVIDER).visible);
    match &node(CARD).payload {
        NodePayload::Frame(f) => assert_eq!((f.size.width(), f.size.height()), (400.0, 200.0)),
        other => panic!("Frame이 아님: {other:?}"),
    }
    assert_eq!(
        stellar_ops::validate::validate_project(&m.work)
            .unwrap()
            .error_count(),
        0
    );
}

// ───────────────────────────── 텍스트 원본 프로젝트 (ADR-0088 기본 원본, ADR-0127 결정 5) ─────────────────────────────

/// 문서만 담은 텍스트 원본을 씁니다.
fn write_text_source(root: &Path, document: stellar_document::model::DocumentModel) {
    write_text_project(
        root,
        &stellar_project::document_io::source_v4::project::TextProject {
            document,
            components: Default::default(),
            token_sets: Vec::new(),
            images: Default::default(),
        },
    );
}

/// 텍스트 원본 프로젝트에서 두 branch를 갈라 저장하고 work에서 `origin/theirs`를 병합합니다.
fn start_text_merge(tag: &str) -> Option<Merging> {
    let fx = git_fixture(tag)?;
    let work = fx.work().to_path_buf();
    let card = card_document();
    write_manifest(&work, "Branch conflict text");
    write_text_source(&work, card_document());
    fx.commit_all("base").unwrap();
    fx.push("main").unwrap();

    // theirs: 아이콘 이름(충돌) + 구분선 숨김.
    let peer = fx.clone_bare("peer").unwrap();
    fx.git_in(&peer, ["switch", "-q", "-c", "theirs"]).unwrap();
    write_text_source(
        &peer,
        edit(&card, |r| {
            rename(r, ICON, "아이콘 (theirs)");
            node_mut(r, DIVIDER).visible = false;
        }),
    );
    let theirs = fx.commit_all_in(&peer, "theirs edit").unwrap();
    fx.git_in(&peer, ["push", "-q", "origin", "theirs"])
        .unwrap();

    // ours: 아이콘 이름(충돌) + 카드 크기.
    write_text_source(
        &work,
        edit(&card, |r| {
            rename(r, ICON, "아이콘 (ours)");
            resize_frame(r, CARD, 400.0, 200.0);
        }),
    );
    let ours = fx.commit_all("ours edit").unwrap();
    fx.git(["fetch", "-q", "origin"]).unwrap();

    // 시스템 Git 병합: merge driver가 없어 두 쪽이 같은 줄(아이콘 이름)을 바꾼 View 원본이 텍스트 conflict입니다.
    let merged = fx.git(["merge", "--no-edit", "origin/theirs"]);
    assert!(
        matches!(merged, Err(FixtureError::Git { .. })),
        "View 원본 conflict가 나야 함: {merged:?}"
    );
    Some(Merging {
        fx,
        work,
        ours,
        theirs,
    })
}

#[test]
fn text_source_branch_conflict_is_resolved_semantically_and_added_keeping_merge_state() {
    let Some(m) = start_text_merge("branch-conflict-resolve-text") else {
        return;
    };
    let port = GatedMergeGit::new(m.fx.gated());
    let unmerged: Vec<PathBuf> = port
        .unmerged(&m.work)
        .unwrap()
        .into_iter()
        .map(|e| e.path)
        .collect();
    assert!(!unmerged.is_empty());
    assert!(
        unmerged.iter().all(
            |p| p.starts_with("design/pages") && p.to_string_lossy().ends_with(".stellar.yaml")
        ),
        "View 원본만 conflict: {unmerged:?}"
    );
    assert_eq!(port.operation(&m.work).unwrap(), GitOperation::Merge);

    // conflict 세션: 텍스트 원본 단위 하나, raw·제외 경로 없음, semantic conflict는 아이콘 이름 하나.
    let mut session = ConflictSession::open(&port, &m.work).expect("conflict 세션");
    assert!(
        session.raw_conflicts().is_empty(),
        "{:?}",
        session.raw_conflicts()
    );
    assert!(
        session.unsupported().is_empty(),
        "텍스트 원본을 Stellar 밖 경로로 제외하지 않음: {:?}",
        session.unsupported()
    );
    let kinds: Vec<UnitKind> = session.units().iter().map(|u| u.kind()).collect();
    assert_eq!(kinds, vec![UnitKind::TextSource]);
    let conflicts: Vec<(usize, stellar_core_types::ids::StableId)> = session
        .conflicts()
        .iter()
        .map(|c| (c.index, c.conflict.object))
        .collect();
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    assert_eq!(conflicts[0].1, nid(ICON));

    // 미해결이면 아무것도 쓰거나 add하지 않습니다.
    assert!(apply_session(&port, &session).is_err());
    assert_eq!(port.unmerged(&m.work).unwrap().len(), unmerged.len());

    session
        .resolve(conflicts[0].0, Resolution::ChooseTheirs)
        .unwrap();
    let report = apply_session(&port, &session).expect("해결 적용");
    assert_eq!(report.operation, Some(GitOperation::Merge));
    assert_eq!(report.remaining, Some(Vec::new()), "unmerged entries 0");
    for path in &unmerged {
        assert!(report.added.contains(path), "{path:?} add");
    }

    // Git 상태: unmerged 0, MERGE_HEAD = theirs, HEAD = ours, commit 없음, 작업 트리 = index.
    assert_eq!(out(&m.fx, &m.work, &["ls-files", "-u"]), "");
    assert_eq!(
        out(&m.fx, &m.work, &["rev-parse", "MERGE_HEAD"]).trim(),
        m.theirs.as_str()
    );
    assert_eq!(m.fx.rev_parse("HEAD").unwrap(), m.ours);
    assert!(
        out(&m.fx, &m.work, &["status"]).contains("All conflicts fixed but you are still merging"),
        "merge state 유지"
    );
    assert_eq!(out(&m.fx, &m.work, &["diff", "--name-only"]), "");
    for path in &unmerged {
        let text = std::fs::read_to_string(m.work.join(path)).unwrap();
        assert!(
            !text.contains("<<<<<<<") && !text.contains(">>>>>>>"),
            "conflict marker 없음: {path:?}"
        );
    }

    // 결과 의미(텍스트 원본을 제품 reader로 다시 읽음): 아이콘 = theirs, 카드 크기 = ours, 구분선 숨김 = theirs.
    let source = stellar_project::document_io::session::read_source(&m.work)
        .unwrap()
        .expect("텍스트 원본");
    let decoded = stellar_project::document_io::session::decode_source(&source).unwrap();
    let records = decoded.document.to_records().unwrap();
    let node = |n: u64| records.nodes.iter().find(|r| r.id == nid(n)).unwrap();
    assert_eq!(node(ICON).display_name, "아이콘 (theirs)");
    assert!(!node(DIVIDER).visible);
    match &node(CARD).payload {
        NodePayload::Frame(f) => assert_eq!((f.size.width(), f.size.height()), (400.0, 200.0)),
        other => panic!("Frame이 아님: {other:?}"),
    }
    assert_eq!(
        stellar_ops::validate::validate_project(&m.work)
            .unwrap()
            .error_count(),
        0
    );
}

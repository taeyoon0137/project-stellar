//! Git snapshot 원천(XIV.2.x) → semantic diff·apply·inverse(XIV.3.x) 종단 시나리오.
//!
//! 실제 저장 파이프라인(V.5)으로 저장한 프로젝트를 fixture 저장소(local bare 원격)에 commit하고, staged
//! 편집과 unstaged 편집을 차례로 디스크에 저장합니다. HEAD·Commit·Tree·Index는 `.git` 없는 virtual
//! project로 materialize하고 WorkingTree는 디스크 loader로 읽어, 모두 실제 프로젝트 loader(V.4.4)로
//! 엽니다. 각 원천의 문서는 VI.10.1 validator를 통과하고 메모리 기대 문서와 의미상 같아야 하며, 원천 사이
//! semantic diff는 편집한 대상만 담고 apply·inverse로 정확히 왕복해야 합니다.

#![cfg(unix)] // Windows 경로는 보류 상태라 검증하지 않습니다(ADR-0006).

mod git_common;

use std::collections::BTreeSet;

use git_common::card::*;
use git_common::*;
use stellar_core_types::ids::StableId;
use stellar_core_types::property::PropertyPath;
use stellar_git_porcelain::snapshot::tree::ResolvedTree;
use stellar_git_porcelain::snapshot::working_tree::SnapshotBasis;
use stellar_git_porcelain::snapshot::{read_tree_snapshot, SnapshotSource};
use stellar_project::saved_project::read_working_tree_project;
use stellar_semantic_diff::apply::apply;
use stellar_semantic_diff::change::SemanticChangeKind;
use stellar_semantic_diff::codec::{snapshot_from_bytes, snapshot_to_bytes};
use stellar_semantic_diff::diff::{diff, SemanticDiff};
use stellar_semantic_diff::keys;
use stellar_semantic_diff::snapshot::SemanticSnapshot;

/// random 문서 seed.
const SEED: u64 = 7;

/// 프로젝트 이름.
const NAME: &str = "Snapshot";

/// diff의 속성 변경 대상 `(객체, 경로)`. 속성 변경이 아닌 변경이 있으면 실패합니다.
fn property_targets(diff: &SemanticDiff) -> BTreeSet<(StableId, PropertyPath)> {
    diff.iter()
        .map(|c| {
            assert!(!c.kind().is_structural(), "속성 변경만 기대: {c:?}");
            (
                c.object_id(),
                c.property_path().cloned().expect("속성 경로"),
            )
        })
        .collect()
}

fn path(n: u64, key: stellar_core_types::property::PropertyKey) -> (StableId, PropertyPath) {
    (nid(n), PropertyPath::with_keys(nid(n), vec![key]))
}

/// forward·inverse 적용과 canonical bytes가 정확히 왕복하는지.
fn assert_round_trip(from: &SemanticSnapshot, to: &SemanticSnapshot, label: &str) -> SemanticDiff {
    let forward = diff(from, to);
    assert_eq!(apply(from, &forward).as_ref(), Ok(to), "{label}: apply");
    let backward = forward.inverse();
    assert_eq!(apply(to, &backward).as_ref(), Ok(from), "{label}: inverse");
    assert_eq!(backward, diff(to, from), "{label}: inverse = 역방향 diff");
    assert_eq!(backward.inverse(), forward, "{label}: inverse 두 번");
    assert_eq!(
        SemanticDiff::from_canonical_bytes(&forward.to_canonical_bytes()).as_ref(),
        Ok(&forward),
        "{label}: diff canonical bytes"
    );
    forward
}

#[test]
fn head_index_and_working_tree_snapshots_diff_and_round_trip() {
    let Some(fx) = git_fixture("e2e-snapshot") else {
        return;
    };
    let gate = fx.gated();
    let work = fx.work().to_path_buf();
    let parent = scratch_dir(&fx, "snapshots");

    // 1) base 프로젝트를 commit하고 local bare 원격에 push.
    let card = card_document();
    let other = seeded_document(SEED);
    write_manifest(&work, NAME);
    save_documents(&work, &[&card, &other]);
    let base_commit = fx.commit_all("base project").unwrap();
    fx.push("main").unwrap();
    assert_eq!(
        fx.rev_parse_in(fx.bare(), "refs/heads/main").unwrap(),
        base_commit
    );

    // 2) staged 편집: 카드 이름 + 구분선 끝점.
    let staged_card = edit(&card, |r| {
        rename(r, CARD, "카드 (staged)");
        move_line_end(r, DIVIDER, 240.0, 0.0);
    });
    save_documents(&work, &[&staged_card, &other]);
    fx.git(["add", "-A"]).unwrap();

    // 3) unstaged 편집: 잎 삭제 + 그룹에 새 사각형 + random 문서 seed 편집.
    let wt_card = edit(&staged_card, |r| {
        remove_leaf(r, LEAF);
        append_rectangle(r, WORKTREE_NEW, GROUP, "새 잎");
    });
    let (wt_other, other_target) = seeded_edit(&other, SEED);
    save_documents(&work, &[&wt_card, &wt_other]);

    let expected_head = ProjectState::expected(NAME, &[&card, &other]);
    let expected_index = ProjectState::expected(NAME, &[&staged_card, &other]);
    let expected_wt = ProjectState::expected(NAME, &[&wt_card, &wt_other]);

    // 4) 원천별 snapshot을 실제 loader로 엶.
    let head_view = materialize_source(&gate, &work, &SnapshotSource::Head, &parent);
    let head_project = open_project(head_view.root());
    assert_eq!(head_project.generation_number(), 1);
    let head = ProjectState::load(&head_project);
    assert_same_state(&head, &expected_head, "HEAD");

    // 같은 commit을 Commit·Tree 원천으로 읽어도 같음(불변 원천).
    let resolved = read_tree_snapshot(&gate, &work, &SnapshotSource::Head)
        .unwrap()
        .resolved;
    let ResolvedTree::Tree {
        commit: Some(head_commit),
        tree,
    } = resolved
    else {
        panic!("HEAD는 commit tree여야 함: {resolved:?}");
    };
    assert_eq!(head_commit, base_commit);
    for source in [
        commit_source(&base_commit),
        SnapshotSource::Tree(tree.clone()),
    ] {
        assert!(source.is_immutable());
        let state = source_state(&gate, &work, &source, &parent);
        assert_same_state(&state, &expected_head, &source.to_spec());
    }

    let index_view = materialize_source(&gate, &work, &SnapshotSource::Index, &parent);
    let index_project = open_project(index_view.root());
    assert_eq!(index_project.generation_number(), 2);
    let index = ProjectState::load(&index_project);
    assert_same_state(&index, &expected_index, "Index");

    let wt_snapshot = read_working_tree_project(&gate, &work).unwrap();
    assert_eq!(wt_snapshot.basis, SnapshotBasis::SavedFilesOnDisk);
    let wt_project = wt_snapshot.project.as_ref().expect("작업 트리 프로젝트");
    assert_eq!(wt_project.generation_number(), 3);
    let wt = ProjectState::load(wt_project);
    assert_same_state(&wt, &expected_wt, "WorkingTree");

    // 5) 원천 사이 semantic diff는 편집 대상만 담고 apply·inverse로 왕복.
    let (h, i, w) = (head.semantic(), index.semantic(), wt.semantic());
    for (snapshot, label) in [(&h, "HEAD"), (&i, "Index"), (&w, "WorkingTree")] {
        assert_snapshot_structure(snapshot, label);
        assert_eq!(
            snapshot_from_bytes(&snapshot_to_bytes(snapshot)).as_ref(),
            Ok(snapshot),
            "{label}: snapshot canonical bytes"
        );
        assert!(diff(snapshot, snapshot).is_empty(), "{label}: 자기 diff");
    }

    let staged = assert_round_trip(&h, &i, "HEAD→Index");
    assert_eq!(
        property_targets(&staged),
        BTreeSet::from([
            path(CARD, keys::DISPLAY_NAME),
            // 구분선 끝점은 longhand `x2` 한 경로입니다(IX.11.3, 끝점 x만 바뀜).
            path(DIVIDER, stellar_document::payload_metadata::shape_keys::X2)
        ]),
        "staged 변경은 카드 이름·구분선 끝점뿐"
    );

    let unstaged = assert_round_trip(&i, &w, "Index→WorkingTree");
    assert_eq!(
        touched_objects(&unstaged),
        BTreeSet::from([nid(LEAF), nid(WORKTREE_NEW), other_target])
    );
    for change in unstaged.iter() {
        let expected_kind = match change.object_id() {
            id if id == nid(LEAF) => Some(SemanticChangeKind::RemoveObject),
            id if id == nid(WORKTREE_NEW) => Some(SemanticChangeKind::AddObject),
            _ => None,
        };
        match expected_kind {
            Some(kind) => assert_eq!(change.kind(), kind, "{change:?}"),
            None => assert!(!change.kind().is_structural(), "{change:?}"),
        }
    }
    let other_paths: BTreeSet<PropertyPath> = unstaged
        .iter()
        .filter(|c| c.object_id() == other_target)
        .filter_map(|c| c.property_path().cloned())
        .collect();
    assert_eq!(
        other_paths,
        BTreeSet::from([
            PropertyPath::with_keys(other_target, vec![keys::VISIBLE]),
            PropertyPath::with_keys(other_target, vec![keys::DISPLAY_NAME]),
        ]),
        "seed 편집은 이름·visible 두 속성"
    );

    // HEAD→WorkingTree는 두 단계의 합성과 같음.
    let total = assert_round_trip(&h, &w, "HEAD→WorkingTree");
    let composed = apply(&apply(&h, &staged).unwrap(), &unstaged).unwrap();
    assert_eq!(composed, w, "staged 다음 unstaged 적용 = WorkingTree");
    let both: BTreeSet<StableId> = touched_objects(&staged)
        .union(&touched_objects(&unstaged))
        .copied()
        .collect();
    assert_eq!(touched_objects(&total), both);

    // 6) staging 전환: unstage하면 Index = HEAD, 전부 stage하면 Index = WorkingTree.
    drop((head_view, index_view));
    fx.git(["reset", "-q"]).unwrap();
    let unstaged_index = source_state(&gate, &work, &SnapshotSource::Index, &parent);
    assert_same_state(&unstaged_index, &expected_head, "reset 뒤 Index");
    fx.git(["add", "-A"]).unwrap();
    let staged_all = source_state(&gate, &work, &SnapshotSource::Index, &parent);
    assert_same_state(&staged_all, &expected_wt, "add -A 뒤 Index");

    // 7) commit·push하면 HEAD와 원격 branch가 WorkingTree 상태, 이전 commit과의 diff = total.
    let next = fx.commit_all("edits").unwrap();
    fx.push("main").unwrap();
    assert_eq!(fx.rev_parse_in(fx.bare(), "refs/heads/main").unwrap(), next);
    let committed = source_state(&gate, &work, &SnapshotSource::Head, &parent);
    assert_same_state(&committed, &expected_wt, "commit 뒤 HEAD");
    let previous = source_state(&gate, &work, &commit_source(&base_commit), &parent);
    assert_eq!(diff(&previous.semantic(), &committed.semantic()), total);

    // materialize한 임시 프로젝트는 모두 정리됨.
    assert_eq!(entry_count(&parent), 0, "materialize 디렉터리 정리");
}

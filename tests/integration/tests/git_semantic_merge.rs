//! 두 branch 편집 → 3-way Semantic Merge(XIV.10.x) 종단 시나리오.
//!
//! work 저장소가 base를 `main`에 commit·push하고, local bare 원격을 clone한 peer가 `theirs` branch에서
//! 편집해 push합니다. work는 `main`에서 따로 편집한 뒤 `origin/theirs`를 fetch해 병합합니다. 두 branch가 모두
//! 저장했으므로 Git 수준에서는 `design/current` pointer가 conflict입니다.
//!
//! semantic 입력은 두 경로로 만들어 서로 대조합니다.
//!
//! 1. base·ours·theirs commit을 각각 materialize(XIV.2.4·XIV.2.5·XIV.2.6)해 실제 loader로 연 snapshot.
//! 2. 병합 중 unmerged index의 stage 1·2·3을 virtual project로 연 snapshot(XIV.11.2).
//!
//! 병합 결과 candidate는 메모리에서 두 편집을 모두 적용해 VI.10.1 validator를 통과한 기대 문서의 snapshot과
//! 같아야 합니다. conflict는 base·ours·theirs 상태를 모두 보존하고 candidate는 그 대상의 base 상태를
//! 유지합니다. 마지막으로 해결 결과를 저장·commit해 merge commit의 HEAD snapshot이 기대와 같은지 봅니다.

#![cfg(unix)] // Windows 경로는 보류 상태라 검증하지 않습니다(ADR-0006).

mod git_common;

use std::collections::BTreeSet;
use std::path::PathBuf;

use git_common::card::*;
use git_common::*;
use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::property::PropertyPath;
use stellar_document::model::DocumentModel;
use stellar_document::records::DocumentRecords;
use stellar_git_porcelain::fixture::{FixtureError, GitFixture};
use stellar_git_porcelain::object::ObjectId;
use stellar_git_porcelain::snapshot::SnapshotSource;
use stellar_project::conflict_open::{open_conflict_projects, ConflictProjects};
use stellar_project::saved_project::read_working_tree_project;
use stellar_semantic_diff::apply::apply;
use stellar_semantic_diff::diff::diff;
use stellar_semantic_diff::keys;
use stellar_semantic_diff::snapshot::SemanticSnapshot;
use stellar_semantic_diff::value::SemanticValue;
use stellar_semantic_merge::merge::merge;
use stellar_semantic_merge::model::{
    ConflictKind, ConflictTarget, MergeInput, MergeResult, SemanticConflict, SideState,
};

/// base 프로젝트 이름.
const BASE_NAME: &str = "Merge";

/// 갈라진 두 branch와 진행 중인 병합.
struct Diverged {
    fx: GitFixture,
    work: PathBuf,
    base: ObjectId,
    ours: ObjectId,
    theirs: ObjectId,
}

/// 한 branch의 편집(문서 목록 + 선택적 프로젝트 이름 변경).
struct Side<'a> {
    documents: &'a [&'a DocumentModel],
    rename_project: Option<&'a str>,
}

/// base를 commit·push하고, peer clone의 `theirs`와 work의 `main`(ours)을 각각 commit·push한 뒤 work에서
/// `origin/theirs` 병합을 시작합니다. `design/current` Git conflict가 나야 합니다.
fn diverge(
    tag: &str,
    base: &[&DocumentModel],
    ours: Side<'_>,
    theirs: Side<'_>,
) -> Option<Diverged> {
    let fx = git_fixture(tag)?;
    let work = fx.work().to_path_buf();
    write_manifest(&work, BASE_NAME);
    save_documents(&work, base);
    let base_commit = fx.commit_all("base").unwrap();
    fx.push("main").unwrap();

    // peer: local bare 원격을 clone해 theirs branch에서 편집하고 push.
    let peer = fx.clone_bare("peer").unwrap();
    fx.git_in(&peer, ["switch", "-q", "-c", "theirs"]).unwrap();
    if let Some(name) = theirs.rename_project {
        write_manifest(&peer, name);
    }
    save_documents(&peer, theirs.documents);
    let theirs_commit = fx.commit_all_in(&peer, "theirs edit").unwrap();
    fx.git_in(&peer, ["push", "-q", "origin", "theirs"])
        .unwrap();

    // work: main에서 ours 편집 후 push, theirs fetch.
    if let Some(name) = ours.rename_project {
        write_manifest(&work, name);
    }
    save_documents(&work, ours.documents);
    let ours_commit = fx.commit_all("ours edit").unwrap();
    fx.push("main").unwrap();
    fx.git(["fetch", "-q", "origin"]).unwrap();
    assert_eq!(
        fx.rev_parse("refs/remotes/origin/theirs").unwrap(),
        theirs_commit
    );
    let merge_base = fx.git(["merge-base", "HEAD", "origin/theirs"]).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&merge_base.stdout).trim(),
        base_commit.as_str()
    );

    // 두 branch 모두 새 generation을 저장했으므로 pointer가 Git conflict.
    let merged = fx.git(["merge", "--no-edit", "origin/theirs"]);
    assert!(
        matches!(merged, Err(FixtureError::Git { .. })),
        "design/current conflict가 나야 함: {merged:?}"
    );
    Some(Diverged {
        fx,
        work,
        base: base_commit,
        ours: ours_commit,
        theirs: theirs_commit,
    })
}

impl Diverged {
    /// base·ours·theirs commit을 materialize해 연 상태.
    fn commit_states(&self) -> [ProjectState; 3] {
        let gate = self.fx.gated();
        let parent = scratch_dir(&self.fx, "commit-snapshots");
        [&self.base, &self.ours, &self.theirs]
            .map(|oid| source_state(&gate, &self.work, &commit_source(oid), &parent))
    }

    /// unmerged index의 stage 1·2·3 virtual project(XIV.11.2).
    fn conflict_projects(&self) -> ConflictProjects {
        let gate = self.fx.gated();
        let parent = scratch_dir(&self.fx, "conflict-snapshots");
        let projects = open_conflict_projects(&gate, &self.work, &parent).unwrap();
        let stellar: Vec<&[u8]> = projects
            .stellar_conflicts()
            .iter()
            .map(|p| p.as_bytes())
            .collect();
        assert_eq!(
            stellar,
            vec![&b"design/current"[..]],
            "Stellar conflict 경로"
        );
        assert!(projects.raw_conflicts().is_empty());
        projects
    }

    /// 병합 해결: ours pointer를 기준으로 해결 문서를 저장하고 merge commit을 만들어 push합니다.
    fn conclude(&self, resolved: &[&DocumentModel]) -> ObjectId {
        self.fx
            .git(["checkout", "--ours", "--", "design/current"])
            .unwrap();
        save_documents(&self.work, resolved);
        let merge_commit = self.fx.commit_all("semantic merge").unwrap();
        let parents = self
            .fx
            .git(["rev-list", "--parents", "-n", "1", "HEAD"])
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&parents.stdout).trim(),
            format!("{merge_commit} {} {}", self.ours, self.theirs),
            "merge commit parent = ours, theirs"
        );
        self.fx.push("main").unwrap();
        assert_eq!(
            self.fx
                .rev_parse_in(self.fx.bare(), "refs/heads/main")
                .unwrap(),
            merge_commit
        );
        merge_commit
    }
}

fn input(base: &ProjectState, ours: &ProjectState, theirs: &ProjectState) -> MergeInput {
    MergeInput {
        base: base.semantic(),
        ours: ours.semantic(),
        theirs: theirs.semantic(),
    }
}

/// 결과 candidate의 공통 불변식: 구조, base → candidate 변경 왕복, canonical bytes.
fn assert_result_invariants(input: &MergeInput, result: &MergeResult, label: &str) {
    assert_snapshot_structure(&result.merged, label);
    assert_definition_coherent(&result.merged, label);
    assert_eq!(
        diff(&input.base, &result.merged),
        result.changes,
        "{label}: changes = diff(base, merged)"
    );
    assert_eq!(
        apply(&input.base, &result.changes).as_ref(),
        Ok(&result.merged),
        "{label}: apply(base, changes) = merged"
    );
    assert_eq!(
        MergeResult::from_canonical_bytes(&result.to_canonical_bytes()).as_ref(),
        Ok(result),
        "{label}: MergeResult canonical bytes"
    );
    assert_eq!(
        MergeInput::from_canonical_bytes(&input.to_canonical_bytes()).as_ref(),
        Ok(input),
        "{label}: MergeInput canonical bytes"
    );
}

/// ours·theirs를 바꿔 merge한 결과가 같은 candidate와 좌우가 바뀐 conflict를 내는지.
fn assert_symmetric(input: &MergeInput, result: &MergeResult) {
    let swapped = merge(&MergeInput {
        base: input.base.clone(),
        ours: input.theirs.clone(),
        theirs: input.ours.clone(),
    })
    .unwrap();
    assert_eq!(swapped.merged, result.merged, "좌우 교환 candidate");
    let mirror = |c: &SemanticConflict| SemanticConflict {
        ours: c.theirs.clone(),
        theirs: c.ours.clone(),
        ..c.clone()
    };
    let expected: Vec<SemanticConflict> = result.conflicts.iter().map(mirror).collect();
    assert_eq!(
        sorted_conflicts(&swapped.conflicts),
        sorted_conflicts(&expected),
        "좌우 교환 conflict"
    );
}

fn sorted_conflicts(conflicts: &[SemanticConflict]) -> Vec<SemanticConflict> {
    let mut out = conflicts.to_vec();
    out.sort_by(|a, b| (a.object, &a.target).cmp(&(b.object, &b.target)));
    out
}

/// conflict side 상태의 문서는 각 commit 문서와 같고, 프로젝트 이름은 Git이 이미 병합한 stage 0 값입니다.
fn assert_conflict_sides_match_commits(
    projects: &ConflictProjects,
    commits: &[ProjectState; 3],
    merged_name: &str,
) -> [ProjectState; 3] {
    use stellar_git_porcelain::conflict_snapshot::ConflictSide;
    let sides = ConflictSide::ALL.map(|side| {
        let project = projects
            .side(side)
            .as_ref()
            .unwrap_or_else(|e| panic!("{side:?} 열기: {e}"));
        ProjectState::load(project)
    });
    for ((side, state), commit) in ConflictSide::ALL.iter().zip(&sides).zip(commits) {
        assert_eq!(
            state.document_json(),
            commit.document_json(),
            "{side:?}: conflict side 문서 = commit 문서"
        );
        // 충돌하지 않은 `stellar.yaml`은 세 side 모두 Git 병합 결과(stage 0)입니다(XIV.11.2).
        assert_eq!(state.name, merged_name, "{side:?}: stage 0 매니페스트");
    }
    sides
}

#[test]
fn non_conflicting_branch_edits_merge_cleanly_and_the_merge_commit_matches() {
    const SEED: u64 = 11;
    const THEIRS_NAME: &str = "Merge (theirs)";
    let card = card_document();
    let other = seeded_document(SEED);
    // ours: 카드 크기 + 배경 숨김 + 그룹 끝에 새 node + random 문서 seed 편집.
    let ours_card = edit(&card, |r| {
        resize_frame(r, CARD, 360.0, 200.0);
        node_mut(r, BACKGROUND).visible = false;
        append_rectangle(r, OURS_NEW, GROUP, "ours 잎");
    });
    let (ours_other, _) = seeded_edit(&other, SEED);
    // theirs: 아이콘 이름 + 구분선 끝점 + 그룹 끝에 새 node(같은 간격) + 프로젝트 이름.
    let theirs_card = edit(&card, |r| {
        rename(r, ICON, "아이콘 (theirs)");
        move_line_end(r, DIVIDER, 280.0, 4.0);
        append_rectangle(r, THEIRS_NEW, GROUP, "theirs 잎");
    });
    // 두 삽입은 같은 이웃 사이라 같은 OrderKey를 받습니다(XIV.10.6 재배치 대상).
    let shared_key = order_key_of(&ours_card.to_records().unwrap(), OURS_NEW);
    assert_eq!(
        order_key_of(&theirs_card.to_records().unwrap(), THEIRS_NEW),
        shared_key
    );
    assert!(nid(OURS_NEW) < nid(THEIRS_NEW));

    let Some(d) = diverge(
        "e2e-merge-clean",
        &[&card, &other],
        Side {
            documents: &[&ours_card, &ours_other],
            rename_project: None,
        },
        Side {
            documents: &[&theirs_card, &other],
            rename_project: Some(THEIRS_NAME),
        },
    ) else {
        return;
    };

    // 기대 병합 문서: 두 편집을 모두 적용하고, 같은 key를 받은 두 삽입 중 StableId가 큰 쪽만 뒤로 재배치.
    let merged_card = edit(&ours_card, |r| {
        rename(r, ICON, "아이콘 (theirs)");
        move_line_end(r, DIVIDER, 280.0, 4.0);
        append_rectangle(r, THEIRS_NEW, GROUP, "theirs 잎");
        node_mut(r, THEIRS_NEW).order_key = OrderKey::between(Some(&shared_key), None);
    });
    let expected = ProjectState::expected(THEIRS_NAME, &[&merged_card, &ours_other]);

    // 1) commit snapshot 입력.
    let commits = d.commit_states();
    assert_same_state(
        &commits[0],
        &ProjectState::expected(BASE_NAME, &[&card, &other]),
        "base commit",
    );
    assert_same_state(
        &commits[1],
        &ProjectState::expected(BASE_NAME, &[&ours_card, &ours_other]),
        "ours commit",
    );
    assert_same_state(
        &commits[2],
        &ProjectState::expected(THEIRS_NAME, &[&theirs_card, &other]),
        "theirs commit",
    );
    let commit_input = input(&commits[0], &commits[1], &commits[2]);
    let result = merge(&commit_input).unwrap();
    assert!(result.is_clean(), "conflict 없음: {:#?}", result.conflicts);
    assert_eq!(
        result.merged,
        expected.semantic(),
        "candidate = validator를 통과한 기대 문서"
    );
    assert_result_invariants(&commit_input, &result, "commit 입력");
    assert_symmetric(&commit_input, &result);
    let rekeyed = result.merged.get(nid(THEIRS_NEW)).unwrap();
    assert_eq!(rekeyed.parent(), Some(nid(GROUP)));
    assert_ne!(rekeyed.order_key(), Some(&shared_key), "충돌 key 재배치");
    assert_eq!(
        result.merged.get(nid(OURS_NEW)).unwrap().order_key(),
        Some(&shared_key),
        "작은 StableId는 key 유지"
    );

    // 2) conflict stage 입력(XIV.11.2)도 같은 candidate를 냄.
    let projects = d.conflict_projects();
    let sides = assert_conflict_sides_match_commits(&projects, &commits, THEIRS_NAME);
    let stage_input = input(&sides[0], &sides[1], &sides[2]);
    let stage_result = merge(&stage_input).unwrap();
    assert!(stage_result.is_clean());
    assert_eq!(stage_result.merged, result.merged, "두 경로의 candidate");
    assert_result_invariants(&stage_input, &stage_result, "stage 입력");
    drop(projects);

    // 3) 해결 결과를 저장·commit: merge commit의 HEAD snapshot = candidate.
    let merge_commit = d.conclude(&[&merged_card, &ours_other]);
    let gate = d.fx.gated();
    let parent = scratch_dir(&d.fx, "after-merge");
    let head = source_state(&gate, &d.work, &SnapshotSource::Head, &parent);
    assert_same_state(&head, &expected, "merge commit HEAD");
    assert_eq!(head.semantic(), result.merged);
    let pushed = source_state(&gate, &d.work, &commit_source(&merge_commit), &parent);
    assert_eq!(pushed.semantic(), result.merged);
    let wt = read_working_tree_project(&gate, &d.work).unwrap();
    assert!(wt.missing_tracked.is_empty());
    assert_same_state(
        &ProjectState::load(wt.project.as_ref().unwrap()),
        &expected,
        "merge 뒤 WorkingTree",
    );
}

#[test]
fn conflicting_branch_edits_preserve_base_ours_theirs_and_resolve() {
    const SEED: u64 = 23;
    let card = card_document();
    let other = seeded_document(SEED);
    // ours: 아이콘 이름(충돌) + 카드 크기 + 잎 삭제(충돌) + 배경 잠금(양쪽 동일).
    let ours_card = edit(&card, |r| {
        rename(r, ICON, "아이콘 (ours)");
        resize_frame(r, CARD, 400.0, 200.0);
        remove_leaf(r, LEAF);
        node_mut(r, BACKGROUND).locked = true;
    });
    // theirs: 아이콘 이름(충돌) + 구분선 숨김 + 잎 이름(삭제 대 수정) + 배경 잠금(양쪽 동일).
    let theirs_card = edit(&card, |r| {
        rename(r, ICON, "아이콘 (theirs)");
        node_mut(r, DIVIDER).visible = false;
        rename(r, LEAF, "잎 (theirs)");
        node_mut(r, BACKGROUND).locked = true;
    });

    let Some(d) = diverge(
        "e2e-merge-conflict",
        &[&card, &other],
        Side {
            documents: &[&ours_card, &other],
            rename_project: None,
        },
        Side {
            documents: &[&theirs_card, &other],
            rename_project: None,
        },
    ) else {
        return;
    };

    let commits = d.commit_states();
    let commit_input = input(&commits[0], &commits[1], &commits[2]);
    let result = merge(&commit_input).unwrap();
    assert_result_invariants(&commit_input, &result, "commit 입력");
    assert_symmetric(&commit_input, &result);

    // 1) conflict 두 개: 같은 속성의 다른 값, 한쪽 삭제 대 다른 쪽 수정.
    let conflicts = sorted_conflicts(&result.conflicts);
    assert_eq!(conflicts.len(), 2, "{conflicts:#?}");
    let name_path = PropertyPath::with_keys(nid(ICON), vec![keys::DISPLAY_NAME]);
    let name_conflict = &conflicts[0];
    assert_eq!(name_conflict.object, nid(ICON));
    assert_eq!(name_conflict.kind, ConflictKind::PropertyValue);
    assert_eq!(
        name_conflict.target,
        ConflictTarget::Property(name_path.clone())
    );
    assert_eq!(name_conflict.document(), Some(nid(DOCUMENT)));
    let values = name_conflict.property_values().expect("속성 conflict 값");
    let text = |s: &str| SemanticValue::Text(s.to_string());
    assert_eq!(values.path, &name_path);
    assert_eq!(values.base, Some(&text("아이콘")));
    assert_eq!(values.ours, Some(&text("아이콘 (ours)")));
    assert_eq!(values.theirs, Some(&text("아이콘 (theirs)")));

    let leaf_conflict = &conflicts[1];
    assert_eq!(leaf_conflict.object, nid(LEAF));
    assert_eq!(leaf_conflict.kind, ConflictKind::DeleteVsModify);
    assert_eq!(leaf_conflict.target, ConflictTarget::Object);
    assert_eq!(leaf_conflict.document(), Some(nid(DOCUMENT)));
    let object_of = |snapshot: &SemanticSnapshot| {
        SideState::Object(Box::new(snapshot.get(nid(LEAF)).unwrap().clone()))
    };
    assert_eq!(leaf_conflict.base, object_of(&commit_input.base));
    assert_eq!(leaf_conflict.ours, SideState::Missing);
    assert_eq!(leaf_conflict.theirs, object_of(&commit_input.theirs));
    assert_ne!(leaf_conflict.base, leaf_conflict.theirs, "theirs 수정 보존");

    // 2) candidate: 겹치지 않는 변경(카드 크기·구분선 숨김·양쪽 같은 배경 잠금)은 적용, conflict 대상은 base.
    let candidate_card = edit(&card, |r| {
        resize_frame(r, CARD, 400.0, 200.0);
        node_mut(r, DIVIDER).visible = false;
        node_mut(r, BACKGROUND).locked = true;
    });
    let candidate = ProjectState::expected(BASE_NAME, &[&candidate_card, &other]);
    assert_eq!(
        result.merged,
        candidate.semantic(),
        "candidate = validator를 통과한 기대 문서"
    );
    assert_eq!(
        result.merged.get(nid(ICON)).unwrap(),
        commit_input.base.get(nid(ICON)).unwrap(),
        "속성 conflict 대상은 base 상태"
    );
    assert_eq!(
        result.merged.get(nid(LEAF)).unwrap(),
        commit_input.base.get(nid(LEAF)).unwrap(),
        "삭제 대 수정 대상은 base 상태"
    );
    let conflict_objects = BTreeSet::from([nid(ICON), nid(LEAF)]);
    assert!(
        touched_objects(&result.changes).is_disjoint(&conflict_objects),
        "자동 병합 변경은 conflict 대상을 건드리지 않음"
    );
    assert_eq!(
        touched_objects(&result.changes),
        BTreeSet::from([nid(CARD), nid(DIVIDER), nid(BACKGROUND)])
    );

    // 3) conflict stage 입력(XIV.11.2)도 같은 candidate와 conflict를 냄.
    let projects = d.conflict_projects();
    let sides = assert_conflict_sides_match_commits(&projects, &commits, BASE_NAME);
    let stage_input = input(&sides[0], &sides[1], &sides[2]);
    let stage_result = merge(&stage_input).unwrap();
    assert_eq!(stage_result.merged, result.merged, "두 경로의 candidate");
    assert_eq!(
        sorted_conflicts(&stage_result.conflicts),
        conflicts,
        "두 경로의 conflict"
    );
    drop(projects);

    // 4) 해결: 아이콘 이름은 ours, 잎은 theirs 수정 유지. 해결 문서는 validator를 통과하고 candidate와는
    //    conflict 대상만 다름.
    let resolved_card = edit(&candidate_card, |r| {
        rename(r, ICON, "아이콘 (ours)");
        rename(r, LEAF, "잎 (theirs)");
    });
    let resolved = ProjectState::expected(BASE_NAME, &[&resolved_card, &other]);
    let resolution = diff(&result.merged, &resolved.semantic());
    let touched: BTreeSet<StableId> = touched_objects(&resolution);
    assert_eq!(touched, conflict_objects, "해결은 conflict 대상만 바꿈");
    for change in resolution.iter() {
        assert_eq!(
            change.property_path().map(|p| p.keys().to_vec()),
            Some(vec![keys::DISPLAY_NAME]),
            "{change:?}"
        );
    }

    // 5) 해결 결과를 저장·commit: merge commit의 HEAD snapshot = 해결 문서.
    d.conclude(&[&resolved_card, &other]);
    let gate = d.fx.gated();
    let parent = scratch_dir(&d.fx, "after-merge");
    let head = source_state(&gate, &d.work, &SnapshotSource::Head, &parent);
    assert_same_state(&head, &resolved, "merge commit HEAD");
    assert_eq!(
        diff(&commit_input.base, &head.semantic()),
        diff(&commit_input.base, &resolved.semantic())
    );
}

#[test]
fn deleted_parent_with_added_child_keeps_parent_and_candidate_validates() {
    const SEED: u64 = 37;
    let card = card_document();
    let other = seeded_document(SEED);
    // ours: 그룹(과 그 잎)을 삭제하고 카드 이름 변경.
    let ours_card = edit(&card, |r| {
        remove_leaf(r, LEAF);
        remove_leaf(r, GROUP);
        rename(r, CARD, "카드 (ours)");
    });
    // theirs: 같은 그룹에 자식을 추가하고 구분선 숨김.
    let theirs_card = edit(&card, |r| {
        append_rectangle(r, THEIRS_NEW, GROUP, "theirs 잎");
        node_mut(r, DIVIDER).visible = false;
    });
    let Some(d) = diverge(
        "e2e-merge-orphan",
        &[&card, &other],
        Side {
            documents: &[&ours_card, &other],
            rename_project: None,
        },
        Side {
            documents: &[&theirs_card, &other],
            rename_project: None,
        },
    ) else {
        return;
    };

    let commits = d.commit_states();
    let commit_input = input(&commits[0], &commits[1], &commits[2]);
    let result = merge(&commit_input).unwrap();
    assert_result_invariants(&commit_input, &result, "commit 입력");
    assert_symmetric(&commit_input, &result);

    // 그룹 삭제는 고아를 만들므로 받아들이지 않고 삭제 대 수정 conflict(base·ours·theirs 보존).
    assert_eq!(result.conflicts.len(), 1, "{:#?}", result.conflicts);
    let conflict = &result.conflicts[0];
    assert_eq!(conflict.kind, ConflictKind::DeleteVsModify);
    assert_eq!(conflict.object, nid(GROUP));
    assert_eq!(conflict.target, ConflictTarget::Object);
    let group = |snapshot: &SemanticSnapshot| {
        SideState::Object(Box::new(snapshot.get(nid(GROUP)).unwrap().clone()))
    };
    assert_eq!(conflict.base, group(&commit_input.base));
    assert_eq!(conflict.ours, SideState::Missing);
    assert_eq!(conflict.theirs, group(&commit_input.theirs));

    // candidate: theirs(그룹 유지 + 자식 추가 + 구분선 숨김) 위에 ours의 잎 삭제·카드 이름. theirs 자식은
    // theirs가 준 OrderKey를 그대로 가집니다.
    let candidate_card = edit(&theirs_card, |r| {
        remove_leaf(r, LEAF);
        rename(r, CARD, "카드 (ours)");
    });
    let candidate = ProjectState::expected(BASE_NAME, &[&candidate_card, &other]);
    assert_eq!(
        result.merged,
        candidate.semantic(),
        "candidate = validator를 통과한 기대 문서"
    );

    // 그룹을 유지하는 해결(= candidate)을 commit하면 HEAD가 candidate.
    d.conclude(&[&candidate_card, &other]);
    let gate = d.fx.gated();
    let parent = scratch_dir(&d.fx, "after-merge");
    let head = source_state(&gate, &d.work, &SnapshotSource::Head, &parent);
    assert_same_state(&head, &candidate, "merge commit HEAD");
}

/// 레코드 편집 함수.
type RecordEdit = fn(&mut DocumentRecords);

fn ours_page(r: &mut DocumentRecords) {
    let [page, view, root] = OURS_PAGE;
    append_page(r, page, view, root, "ours 페이지");
}

fn theirs_page(r: &mut DocumentRecords) {
    let [page, view, root] = THEIRS_PAGE;
    append_page(r, page, view, root, "theirs 페이지");
}

fn ours_view(r: &mut DocumentRecords) {
    let [view, root] = OURS_VIEW;
    append_view(r, view, root, "ours View");
}

fn theirs_view(r: &mut DocumentRecords) {
    let [view, root] = THEIRS_VIEW;
    append_view(r, view, root, "theirs View");
}

/// 버그(stellar-semantic-merge, `crates/semantic-merge/src/merge.rs`): 페이지·View 순서는 부모 정의 객체의
/// `DOCUMENT_PAGES`·`PAGE_VIEWS` Ids 속성 하나로 표현되는데, merge가 이 목록을 원자 값으로 비교합니다. 양쪽이
/// 서로 다른 페이지·View를 독립적으로 추가하면(아키텍처 §27.1 "충돌하지 않는 자식 삽입" = 자동 병합) 목록
/// PropertyValue conflict가 나고, 동시에 새 Page·View·root 객체의 AddObject는 받아들여 candidate의 정의 목록과
/// 객체가 어긋납니다(목록에 없는 Page·View, 어떤 View 목록에도 닿지 않는 root Frame — VI.10.1 root count로
/// 되돌릴 수 없는 상태). 결함 기록은 `docs/task-reports/XIV.10.6.md` "알려진 결함 SM-1"입니다. 수정되면
/// `#[ignore]`를 지웁니다.
#[test]
#[ignore = "버그: semantic-merge가 독립 페이지·View 삽입을 DOCUMENT_PAGES·PAGE_VIEWS 목록 conflict로 만들고 candidate 정의 목록이 객체와 어긋남"]
fn independent_page_and_view_insertions_merge_cleanly() {
    let card = card_document();
    let other = seeded_document(41);
    let ours_card = edit(&card, |r| {
        ours_page(r);
        ours_view(r);
    });
    let theirs_card = edit(&card, |r| {
        theirs_page(r);
        theirs_view(r);
    });
    let Some(d) = diverge(
        "e2e-merge-pages",
        &[&card, &other],
        Side {
            documents: &[&ours_card, &other],
            rename_project: None,
        },
        Side {
            documents: &[&theirs_card, &other],
            rename_project: None,
        },
    ) else {
        return;
    };

    let commits = d.commit_states();
    let commit_input = input(&commits[0], &commits[1], &commits[2]);
    let result = merge(&commit_input).unwrap();
    // conflict 여부와 무관하게 candidate 정의 목록은 객체와 맞아야 합니다(현재 여기서 실패).
    assert_definition_coherent(&result.merged, "candidate");
    assert!(
        result.is_clean(),
        "독립 페이지·View 삽입은 conflict 없이 병합: {:#?}",
        result
            .conflicts
            .iter()
            .map(|c| (c.kind, c.object, &c.target))
            .collect::<Vec<_>>()
    );
    assert_result_invariants(&commit_input, &result, "commit 입력");

    // 같은 끝 위치 삽입의 상대 순서는 정하지 않습니다(ours 먼저·theirs 먼저 모두 허용).
    let orders: [(RecordEdit, RecordEdit); 2] =
        [(ours_page, theirs_page), (theirs_page, ours_page)];
    let view_orders: [(RecordEdit, RecordEdit); 2] =
        [(ours_view, theirs_view), (theirs_view, ours_view)];
    let mut candidates = Vec::new();
    for (first_page, second_page) in orders {
        for (first_view, second_view) in view_orders {
            candidates.push(edit(&card, |r| {
                first_page(r);
                second_page(r);
                first_view(r);
                second_view(r);
            }));
        }
    }
    let merged_card = candidates
        .iter()
        .find(|model| {
            ProjectState::expected(BASE_NAME, &[model, &other]).semantic() == result.merged
        })
        .expect("candidate = 두 삽입을 모두 담은 validator 통과 문서");

    d.conclude(&[merged_card, &other]);
    let gate = d.fx.gated();
    let parent = scratch_dir(&d.fx, "after-merge");
    let head = source_state(&gate, &d.work, &SnapshotSource::Head, &parent);
    assert_eq!(head.semantic(), result.merged, "merge commit HEAD");
}

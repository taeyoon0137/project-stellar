//! 텍스트 원본 merge의 Component Context 병합 test (ADR-0123 결정 2).
//!
//! Context는 창 reconciliation과 같은 semantic snapshot 경로로 병합됩니다. 파일 통째 3-way였던 옛 규칙이 거부하던
//! "두 쪽이 서로 다른 Context 항목을 바꿈"이 병합되고, 같은 항목을 다르게 바꾸면 엔진 conflict입니다.

use stellar_document::component::definition::ComponentLibrary;
use stellar_document::context::fixtures::{literal, rgb, sid, SceneBuilder};
use stellar_document::context::key::{ContextKey, ContextScope};
use stellar_document::context::provision::{ContextOwner, ContextProvision};
use stellar_document::context::set::ContextSet;
use stellar_document::expressions::types::{ExpressionType, TypeSpec};
use stellar_project::document_io::source_v4::project::{decode, encode, TextProject};

use super::*;

const CONTEXT: &str = "context.stellar.yaml";

fn key(id: u64, name: &str) -> ContextKey {
    ContextKey::new(
        sid(id),
        name,
        ContextScope::Document(sid(1)),
        TypeSpec::required(ExpressionType::Color),
        literal(id + 1, rgb(1.0, 1.0, 1.0)),
    )
    .unwrap()
}

fn provision(value: f32, seed: u64) -> ContextProvision {
    ContextProvision::new(
        sid(910),
        ContextOwner::View(sid(3)),
        sid(11),
        sid(900),
        literal(seed, rgb(value, 0.0, 1.0)),
    )
    .unwrap()
}

fn base_context() -> ContextSet {
    let mut set = ContextSet::new();
    set.insert_key(key(900, "theme")).unwrap();
    set.insert_provision(provision(0.0, 911)).unwrap();
    set.add_requirement(sid(950), sid(900));
    set
}

fn files(context: &ContextSet) -> Files {
    let mut document = SceneBuilder::new(sid(1), sid(2), sid(3), sid(10))
        .frame(sid(11), sid(10))
        .build();
    document.replace_context(context.clone());
    encode(&TextProject {
        document,
        components: ComponentLibrary::new(),
        token_sets: Vec::new(),
        images: Default::default(),
    })
    .unwrap()
    .files
}

fn context_of(files: &Files) -> ContextSet {
    let project = decode(files).unwrap();
    project.document.context().live(&project.document)
}

fn renamed(set: &ContextSet, name: &str) -> ContextSet {
    let mut out = set.clone();
    let _ = out.remove_key(sid(900));
    out.insert_key(key(900, name)).unwrap();
    out
}

#[test]
fn different_context_items_from_both_sides_merge_through_the_snapshot() {
    // 검증 기준(ADR-0123 결정 2): 옛 파일 통째 3-way가 거부하던 서로 다른 항목 변경이 병합됩니다.
    let base = base_context();
    // ours: 새 key와 Slot 공개 목록, theirs: provision 값 변경과 다른 컴포넌트 requirement.
    let mut ours = base.clone();
    ours.insert_key(key(920, "gap")).unwrap();
    ours.add_slot_key(sid(960), sid(900));
    let mut theirs = base.clone();
    let _ = theirs.remove_provision(sid(910));
    theirs.insert_provision(provision(1.0, 912)).unwrap();
    theirs.add_requirement(sid(970), sid(900));
    let sides = TextSides {
        base: files(&base),
        ours: files(&ours),
        theirs: files(&theirs),
    };
    let TextMerge::Merged(result) = merge_text(&sides).unwrap() else {
        panic!("병합되지 않음");
    };
    let mut expected = theirs.clone();
    expected.insert_key(key(920, "gap")).unwrap();
    expected.add_slot_key(sid(960), sid(900));
    assert_eq!(context_of(&result), expected);
    // driver 한 경로(Context 파일)도 같은 결과입니다.
    let outcome = merge_text_file(
        &sides.ours,
        CONTEXT,
        sides.base[CONTEXT].as_bytes(),
        sides.ours[CONTEXT].as_bytes(),
        sides.theirs[CONTEXT].as_bytes(),
    )
    .unwrap();
    assert_eq!(
        outcome,
        FileMergeOutcome::Merged(result[CONTEXT].clone().into_bytes())
    );
}

#[test]
fn the_same_context_item_changed_differently_is_an_engine_conflict() {
    let base = base_context();
    let TextMerge::Conflicts(conflicts) = merge_text(&TextSides {
        base: files(&base),
        ours: files(&renamed(&base, "brand")),
        theirs: files(&renamed(&base, "accent")),
    })
    .unwrap() else {
        panic!("conflict가 아님");
    };
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].object, sid(900));
}

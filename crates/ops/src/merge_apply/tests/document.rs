use stellar_document::context::set::ContextSet;
use stellar_document::records::DocumentRecords;

use crate::core_types::ids::StableId;
use crate::diff::DocumentSource;
use crate::merge_apply::document::{merge_document, DocumentMerge, DocumentMergeError};
use crate::test_support::random_valid_document;

fn records(seed: u64) -> (DocumentSource, Vec<StableId>) {
    let mut records = random_valid_document(seed).to_records().unwrap();
    records.sort_canonical();
    let children = records
        .nodes
        .iter()
        .filter(|n| n.parent.is_some())
        .map(|n| n.id)
        .collect();
    (
        DocumentSource {
            records,
            context: ContextSet::new(),
        },
        children,
    )
}

fn renamed(document: &DocumentSource, node: StableId, name: &str) -> DocumentSource {
    let mut out = document.clone();
    out.records
        .nodes
        .iter_mut()
        .find(|n| n.id == node)
        .unwrap()
        .display_name = name.into();
    out
}

fn name_of(records: &DocumentRecords, node: StableId) -> &str {
    &records
        .nodes
        .iter()
        .find(|n| n.id == node)
        .unwrap()
        .display_name
}

#[test]
fn non_overlapping_edits_merge_into_one_document() {
    let (base, children) = records(7);
    assert!(children.len() >= 2);
    let ours = renamed(&base, children[0], "ours");
    let theirs = renamed(&base, children[1], "theirs");
    let merged = merge_document(&base, &ours, &theirs).unwrap();
    let DocumentMerge::Merged { decision, document } = merged else {
        panic!("병합되지 않음: {merged:?}");
    };
    assert_eq!(decision, "merged");
    assert_eq!(name_of(&document.records, children[0]), "ours");
    assert_eq!(name_of(&document.records, children[1]), "theirs");
    // 같은 입력이면 같은 결과입니다.
    assert_eq!(
        merge_document(&base, &ours, &theirs).unwrap(),
        DocumentMerge::Merged {
            decision: "merged",
            document
        }
    );
}

#[test]
fn one_sided_changes_keep_that_side() {
    let (base, children) = records(7);
    let ours = renamed(&base, children[0], "ours");
    assert_eq!(
        merge_document(&base, &ours, &base).unwrap(),
        DocumentMerge::Merged {
            decision: "keep-ours",
            document: Box::new(ours.clone())
        }
    );
    assert_eq!(
        merge_document(&base, &base, &ours).unwrap(),
        DocumentMerge::Merged {
            decision: "take-theirs",
            document: Box::new(ours)
        }
    );
}

#[test]
fn overlapping_edit_is_a_conflict_with_all_three_values() {
    let (base, children) = records(7);
    let ours = renamed(&base, children[0], "ours");
    let theirs = renamed(&base, children[0], "theirs");
    let DocumentMerge::Conflicted {
        conflicts,
        violations,
    } = merge_document(&base, &ours, &theirs).unwrap()
    else {
        panic!("conflict가 아님");
    };
    assert_eq!(violations, 0);
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].object, children[0]);
    assert_eq!(conflicts[0].kind, "property-value");
}

#[test]
fn different_documents_are_rejected() {
    let (base, _) = records(7);
    let (other, _) = records(8);
    assert!(matches!(
        merge_document(&base, &base, &other),
        Err(DocumentMergeError::DocumentMismatch { side: "theirs", .. })
    ));
}

/// 문서 Context에 key 하나(이름 `name`, default `value`)를 더합니다.
fn with_key(document: &DocumentSource, key: u64, name: &str, value: f32) -> DocumentSource {
    use stellar_document::context::key::{ContextKey, ContextScope};
    use stellar_document::expressions::ast::{ExprNode, ExprNodeId, ExpressionAst};
    use stellar_document::expressions::types::{ExpressionType, TypeSpec};
    let id = StableId::new_v7(key, 0, key);
    let root = ExprNodeId::new(StableId::new_v7(key + 1, 0, key + 1));
    let default = ExpressionAst::new(
        root,
        vec![ExprNode::literal(
            root,
            crate::core_types::typed_value::TypedValue::Number(
                crate::core_types::value::FiniteF32::new(value).unwrap(),
            ),
        )],
    )
    .unwrap();
    let mut out = document.clone();
    let _ = out.context.remove_key(id);
    out.context
        .insert_key(
            ContextKey::new(
                id,
                name.to_owned(),
                ContextScope::Document(out.records.definition.id),
                TypeSpec::required(ExpressionType::Number),
                default,
            )
            .unwrap(),
        )
        .unwrap();
    out
}

#[test]
fn context_changes_merge_and_conflict_like_other_objects() {
    // 검증 기준(XXX.7.10 남은 범위, ADR-0114 결정 2): 병합 결과가 ContextSet에 반영됩니다.
    let (plain, children) = records(7);
    let base = with_key(&plain, 9000, "theme", 1.0);
    let ours = renamed(&with_key(&base, 9000, "brand", 1.0), children[0], "ours");
    let theirs = with_key(&base, 9100, "gap", 4.0);
    let DocumentMerge::Merged { decision, document } =
        merge_document(&base, &ours, &theirs).unwrap()
    else {
        panic!("병합되지 않음");
    };
    assert_eq!(decision, "merged");
    assert_eq!(name_of(&document.records, children[0]), "ours");
    let names: Vec<&str> = document.context.keys().map(|k| k.name()).collect();
    assert_eq!(names, ["brand", "gap"]);
    // 같은 key 이름을 서로 다르게 바꾸면 conflict입니다.
    let other = with_key(&base, 9000, "accent", 1.0);
    assert!(matches!(
        merge_document(&base, &ours, &other).unwrap(),
        DocumentMerge::Conflicted { .. }
    ));
}

#[test]
fn requirement_and_slot_lists_merge_in_document_scope() {
    // 검증 기준(ADR-0123 결정 1): 컴포넌트 library를 투영하지 않는 문서 범위에서도 requirement·Slot 공개 목록이 있는
    // 문서를 거부하지 않고, 목록 변경이 다른 객체 변경처럼 병합·충돌합니다.
    let (plain, children) = records(7);
    let theme = StableId::new_v7(9000, 0, 9000);
    let gap = StableId::new_v7(9100, 0, 9100);
    let card = StableId::new_v7(9200, 0, 9200);
    let button = StableId::new_v7(9300, 0, 9300);
    let slot = StableId::new_v7(9400, 0, 9400);
    let mut base = with_key(&with_key(&plain, 9000, "theme", 1.0), 9100, "gap", 4.0);
    base.context.add_requirement(card, theme);
    // 세 쪽이 같으면 그대로 돌아옵니다(무손실 왕복).
    let DocumentMerge::Merged { document, .. } = merge_document(&base, &base, &base).unwrap()
    else {
        panic!("병합되지 않음");
    };
    assert_eq!(document.context, base.context);
    // 한쪽은 다른 컴포넌트 requirement, 다른 쪽은 Slot 공개 목록과 node 이름: 모두 받아들입니다.
    let mut ours = renamed(&base, children[0], "ours");
    ours.context.add_requirement(button, gap);
    let mut theirs = base.clone();
    theirs.context.add_slot_key(slot, theme);
    let DocumentMerge::Merged { decision, document } =
        merge_document(&base, &ours, &theirs).unwrap()
    else {
        panic!("병합되지 않음");
    };
    assert_eq!(decision, "merged");
    assert_eq!(name_of(&document.records, children[0]), "ours");
    let mut expected = base.context.clone();
    expected.add_requirement(button, gap);
    expected.add_slot_key(slot, theme);
    assert_eq!(document.context, expected);
    // 같은 컴포넌트의 requirement 목록을 두 쪽이 다르게 바꾸면 conflict입니다.
    let mut left = base.clone();
    left.context.add_requirement(card, gap);
    let mut right = base.clone();
    right.context.remove_requirement(card, theme);
    let DocumentMerge::Conflicted { conflicts, .. } = merge_document(&base, &left, &right).unwrap()
    else {
        panic!("conflict가 아님");
    };
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].object, base.records.definition.id);
}

#[test]
fn adding_a_requirement_is_not_a_new_consumption_of_a_removed_provision() {
    // requirement는 공개 interface 선언이라 provision 삭제 대 소비 추가 conflict(XXX.7.10)를 만들지 않습니다(ADR-0123
    // 결정 1). 목록이 Document 객체에 붙어도 같은 문서 안 "위치 모르는 소비"로 보지 않습니다.
    use stellar_document::context::provision::{ContextOwner, ContextProvision};
    let (plain, children) = records(7);
    let theme = StableId::new_v7(9000, 0, 9000);
    let provision = StableId::new_v7(9500, 0, 9500);
    let mut base = with_key(&plain, 9000, "theme", 1.0);
    let view = base
        .records
        .definition
        .pages
        .iter()
        .flat_map(|page| page.views.iter())
        .next()
        .unwrap()
        .id;
    let value = with_key(&plain, 9600, "value", 2.0)
        .context
        .key(StableId::new_v7(9600, 0, 9600))
        .unwrap()
        .default_expression()
        .ast()
        .clone();
    base.context
        .insert_provision(
            ContextProvision::new(
                provision,
                ContextOwner::View(view),
                children[0],
                theme,
                value,
            )
            .unwrap(),
        )
        .unwrap();
    let mut ours = base.clone();
    let _ = ours.context.remove_provision(provision);
    let mut theirs = base.clone();
    theirs
        .context
        .add_requirement(StableId::new_v7(9200, 0, 9200), theme);
    let DocumentMerge::Merged { document, .. } = merge_document(&base, &ours, &theirs).unwrap()
    else {
        panic!("병합되지 않음");
    };
    assert!(document.context.provision(provision).is_none());
    assert_eq!(document.context.all_requirements().count(), 1);
}

use stellar_document::records::DocumentRecords;

use crate::core_types::ids::StableId;
use crate::merge_apply::document::{merge_document, DocumentMerge, DocumentMergeError};
use crate::test_support::random_valid_document;

fn records(seed: u64) -> (DocumentRecords, Vec<StableId>) {
    let mut records = random_valid_document(seed).to_records().unwrap();
    records.sort_canonical();
    let children = records
        .nodes
        .iter()
        .filter(|n| n.parent.is_some())
        .map(|n| n.id)
        .collect();
    (records, children)
}

fn renamed(records: &DocumentRecords, node: StableId, name: &str) -> DocumentRecords {
    let mut out = records.clone();
    out.nodes
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
    let DocumentMerge::Merged { decision, records } = merged else {
        panic!("병합되지 않음: {merged:?}");
    };
    assert_eq!(decision, "merged");
    assert_eq!(name_of(&records, children[0]), "ours");
    assert_eq!(name_of(&records, children[1]), "theirs");
    // 같은 입력이면 같은 결과입니다.
    assert_eq!(
        merge_document(&base, &ours, &theirs).unwrap(),
        DocumentMerge::Merged {
            decision: "merged",
            records
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
            records: Box::new(ours.clone())
        }
    );
    assert_eq!(
        merge_document(&base, &base, &ours).unwrap(),
        DocumentMerge::Merged {
            decision: "take-theirs",
            records: Box::new(ours)
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

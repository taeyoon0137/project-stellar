//! 외부 변경 겹침·분할 test (XIX.7.1, XIX.7.3).

use super::*;
use crate::command::test_fixtures::{sid, small_model};
use crate::lock::lease::Lease;

fn table(model: &DocumentModel) -> LeaseTable {
    let mut table = LeaseTable::new();
    table
        .insert(Lease {
            id: LeaseId(sid(900)),
            subtrees: vec![SubtreeLock::capture(model, sid(13)).unwrap()],
            externals: Vec::new(),
        })
        .unwrap();
    table
}

#[test]
fn overlap_is_lease_subtree_or_moved_ancestor() {
    let model = small_model();
    let table = table(&model);
    let none = overlapping_leases(&table, &model, &model, [ExternalTarget::Node(sid(12))]);
    assert!(none.is_empty());
    let inside = overlapping_leases(&table, &model, &model, [ExternalTarget::Node(sid(13))]);
    assert_eq!(inside[&LeaseId(sid(900))], BTreeSet::from([sid(13)]));
    // 11은 lease root 13의 진조상입니다: 속성 변경은 겹치지 않고 위치 변경은 겹칩니다.
    assert!(overlapping_leases(&table, &model, &model, [ExternalTarget::Node(sid(11))]).is_empty());
    assert!(
        !overlapping_leases(&table, &model, &model, [ExternalTarget::Moved(sid(11))]).is_empty()
    );
    assert!(!overlapping_leases(
        &table,
        &model,
        &model,
        [ExternalTarget::View {
            root_frame: sid(10)
        }]
    )
    .is_empty());
}

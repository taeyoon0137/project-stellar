//! 외부 변경 batch 겹침 판정과 반영 test (XIX.7.1~XIX.7.3).

use super::*;
use crate::agent_history::SessionOrigin;
use crate::provenance_fixture::world::{World, L};
use crate::provenance_fixture::*;
use stellar_ops::commands::command::Command;
use stellar_ops::commands::request::CommandRequest;

/// 원본 문서에 이름 변경을 적용한 외부 원본.
fn external(renames: &[(u64, &str)]) -> DocumentRecords {
    let mut state = state();
    for (node, name) in renames {
        state
            .execute(&CommandRequest::at(
                &state,
                Command::rename_node(sid(*node), *name),
            ))
            .unwrap();
    }
    state.model().to_records().unwrap()
}

#[test]
fn external_batch_semantic_diff_and_lease_overlap() {
    let world = World::new(SessionOrigin::ExternalClient);
    let base = state().model().to_records().unwrap();
    let table = world.table();
    let outside = analyze_external(
        world.history.doc.model(),
        &base,
        &external(&[(12, "x")]),
        &table,
    )
    .unwrap();
    assert_eq!(outside.diff.entries.len(), 1);
    assert!(!outside.overlaps_any(), "non-overlap");
    let inside = analyze_external(
        world.history.doc.model(),
        &base,
        &external(&[(14, "x")]),
        &table,
    )
    .unwrap();
    assert_eq!(
        inside.overlaps.get(&lease_id(L)),
        Some(&BTreeSet::from([sid(14)])),
        "lease root 아래 자손도 subtree 안"
    );
}

//! command 검증 index 시험 (XVI.4.7, ADR-0108 결정 1).
//!
//! command 검증은 [`DocumentModel`]이 편집과 함께 증분 유지하는 자식·inbound 참조·영속 id index를 읽습니다
//! ([`crate::tree`]). 이 모듈은 random command sequence(undo 포함) 뒤 그 증분 index가 문서 전체에서 다시 만든
//! index(test oracle)와 같은지 확인하고, release 측정용 시험(`#[ignore]`)을 둡니다.
#![cfg(test)]

use std::collections::BTreeSet;

use stellar_core_types::ids::StableId;
use stellar_core_types::order_key::OrderKey;
use stellar_document::model::DocumentModel;
use stellar_document::test_support::random_document::{random_valid_document, SplitMix64};

use crate::command::test_fixtures::frame;
use crate::command::Command;
use crate::convert::RestoreNodeRecords;
use crate::request::CommandRequest;
use crate::state::DocumentState;
use crate::test_support::{random_valid_request, FreshIds};
use crate::tree::{id_owners, id_use, node_ids, reference_index, ChildIndex};

/// 증분 index와 전체 재구성 index가 같은지 확인합니다. `seen`은 지금까지 문서에 나타난 id(사라진 id가 `None`인지
/// 확인용)이고, 문서에 없는 fresh id도 함께 확인합니다.
fn assert_indexes_match(model: &DocumentModel, seen: &mut BTreeSet<StableId>, probe: StableId) {
    let owners = id_owners(model);
    seen.extend(owners.keys().copied());
    for &id in seen.iter().chain(std::iter::once(&probe)) {
        assert_eq!(
            id_use(model, id),
            owners.get(&id).map(|owner| owner.id_use()),
            "id {id:?}의 종류"
        );
    }
    let rebuilt = ChildIndex::rebuild(model);
    let index = ChildIndex::build(model);
    for id in node_ids(model) {
        assert_eq!(
            index.children(id),
            rebuilt.get(&id).cloned().unwrap_or_default(),
            "node {id:?}의 자식"
        );
    }
    for parent in rebuilt.keys() {
        assert!(model.contains(*parent), "자식 표의 부모는 live node");
    }
    assert_eq!(model.references(), &reference_index(model), "참조 index");
}

#[test]
fn incremental_indexes_match_full_rebuilds_after_random_command_sequences() {
    let mut applied = 0;
    let mut undone = 0;
    for seed in 0..40u64 {
        let mut state = DocumentState::new(random_valid_document(seed));
        let mut rng = SplitMix64::new(seed ^ 0x1D3E_7A11);
        let mut ids = FreshIds::new(seed ^ 0x55);
        let mut seen = BTreeSet::new();
        assert_indexes_match(state.model(), &mut seen, ids.fresh());
        let mut inverses = Vec::new();
        for _ in 0..40 {
            // 셋 중 하나는 직전 command를 되돌립니다(undo 경로의 역 command도 같은 index를 갱신해야 함).
            if rng.one_in(3) {
                if let Some(inverse) = inverses.pop() {
                    let request = CommandRequest::at(&state, inverse);
                    state.execute(&request).expect("역 command 적용");
                    undone += 1;
                    assert_indexes_match(state.model(), &mut seen, ids.fresh());
                    continue;
                }
            }
            let Some(request) = random_valid_request(&state, &mut rng, &mut ids, 24) else {
                continue;
            };
            let outcome = state.execute(&request).expect("유효 요청 적용");
            inverses.push(outcome.inverse);
            applied += 1;
            assert_indexes_match(state.model(), &mut seen, ids.fresh());
        }
    }
    assert!(applied >= 900, "충분한 command 적용: {applied}");
    assert!(undone >= 200, "충분한 역 command 적용: {undone}");
}

#[test]
fn typography_ids_occupy_the_id_space_like_the_document_validator() {
    // 문서 validator(VI.10.1)는 typography resource id도 영속 id 공간에 넣습니다. command 검증 조회도 같습니다.
    let typography = crate::command::test_fixtures::sid(77);
    let mut definition = crate::command::test_fixtures::definition();
    definition.text_styles.typography.insert(
        typography,
        stellar_core_types::typed_value::TypographyValue::new(vec![0]),
    );
    let mut model = DocumentModel::new(definition);
    model.insert_node(frame(10, None, 1)).expect("root");
    assert_eq!(id_use(&model, typography), Some(crate::tree::IdUse::Other));
    assert_eq!(
        id_owners(&model)
            .get(&typography)
            .map(|owner| owner.id_use()),
        Some(crate::tree::IdUse::Other)
    );
    // 그 id를 clip 대상으로 두는 속성 변경은 validator와 같이 참조 종류 불일치로 거부합니다.
    let clip = Command::SetProperty(crate::property::SetProperty {
        node: crate::command::test_fixtures::sid(10),
        key: stellar_document::frame::keys::CLIPPING,
        value: crate::property::PropertyValue::Clipping(stellar_document::frame::Clipping::Path(
            stellar_core_types::reference::NodeRef::new(typography),
        )),
    });
    assert_eq!(
        clip.validate(&model),
        Err(crate::command::CommandError::ReferenceTypeMismatch {
            owner: crate::command::test_fixtures::sid(10),
            target: typography,
        })
    );
}

/// root ← group 100개 ← leaf Frame `leaf_count`개 문서(VII.2.7 사후 수정 측정과 같은 모양).
fn grouped_document(leaf_count: u64) -> DocumentModel {
    let mut model = DocumentModel::new(crate::command::test_fixtures::definition());
    model.insert_node(frame(10, None, 1)).expect("root");
    let per_group = leaf_count / 100;
    let keys = OrderKey::evenly_spaced(per_group as usize + 2);
    for group in 0..100 {
        let parent = 100 + group;
        model
            .insert_node(frame(parent, Some(10), group as u8 + 1))
            .expect("group");
        for offset in 0..per_group {
            let mut leaf = frame(1_000 + group * per_group + offset, Some(parent), 1);
            leaf.order_key = keys[offset as usize].clone();
            model.insert_node(leaf).expect("leaf");
        }
    }
    model
}

#[test]
#[ignore = "release 측정: cargo test -p stellar-commands --release -- --ignored measure_restore_node_records_validate --nocapture"]
fn measure_restore_node_records_validate() {
    for leaf_count in [1_000u64, 10_000, 50_000] {
        let model = grouped_document(leaf_count);
        let target = crate::command::test_fixtures::sid(1_000 + leaf_count / 2);
        let mut record = model.record(target).expect("leaf");
        record.display_name = "이름만 바꿈".into();
        let command = Command::RestoreNodeRecords(RestoreNodeRecords {
            records: vec![record],
        });
        command.validate(&model).expect("유효");
        let rounds = 2_000u32;
        let mut samples = Vec::with_capacity(rounds as usize);
        for _ in 0..rounds {
            let started = std::time::Instant::now();
            command.validate(&model).expect("유효");
            samples.push(started.elapsed());
        }
        samples.sort_unstable();
        let total: std::time::Duration = samples.iter().sum();
        let mean = total / rounds;
        let p99 = samples[(rounds as usize * 99) / 100];
        println!(
            "RestoreNodeRecords validate, leaf {leaf_count} (node {}): 평균 {mean:?}, p50 {:?}, p99 {p99:?}, 최대 {:?}",
            model.node_count(),
            samples[rounds as usize / 2],
            samples[rounds as usize - 1]
        );
        assert!(
            mean <= std::time::Duration::from_millis(1),
            "50,000 node까지 1 ms 이하"
        );
    }
}

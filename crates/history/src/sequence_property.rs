//! random command sequence undo/redo property test (VII.6.2).
//!
//! seed 0..1000마다 random valid document(VI.10.5)에서 유효한 command·선택 event를 무작위로 이어
//! 기록한 뒤, **전체 undo**가 초기 상태로, 이어서 **전체 redo**가 최종 상태로 돌아오는지 확인합니다.
//! 상태 비교는 semantic snapshot(VI.10.4)과 창 선택입니다. command 생성기는 commands crate의 테스트
//! 전용 `test_support`(VII.6.1)를 씁니다.
#![cfg(test)]

use stellar_commands::document::test_support::random_document::{
    random_valid_document, SplitMix64,
};
use stellar_commands::document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_commands::id::{CommandId, CommandOrigin, WindowSessionId};
use stellar_commands::selection::{Selection, SetSelection};
use stellar_commands::state::DocumentState;
use stellar_commands::test_support::{random_valid_request, FreshIds};
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::ids::StableId;

use crate::window::WindowHistory;

/// seed corpus 크기.
const SEEDS: u64 = 1_000;
/// seed당 기록 수(문서 편집 + 선택).
const STEPS: usize = 6;

fn snapshot(state: &DocumentState) -> String {
    model_snapshot_json(state.model()).expect("snapshot")
}

#[test]
fn smoke_thousand_seed_undo_redo_sequences() {
    // 검증 기준(VII.6.2): seed corpus 전부 통과.
    let mut total_document = 0usize;
    let mut total_selection = 0usize;
    for seed in 0..SEEDS {
        let mut state = DocumentState::new(random_valid_document(seed));
        let mut history = WindowHistory::new(WindowSessionId::from_stable_id(StableId::new_v7(
            9_000 + seed,
            0,
            seed,
        )));
        let mut rng = SplitMix64::new(seed ^ 0x6_2000);
        let mut ids = FreshIds::new(seed ^ 0x6_2001);
        let initial = snapshot(&state);
        let initial_selection = history.view().selection().clone();
        let mut recorded = 0usize;
        for step in 0..STEPS {
            if rng.one_in(4) {
                let nodes: Vec<StableId> = state
                    .model()
                    .columns()
                    .iter_live()
                    .filter_map(|h| state.model().stable_id(h))
                    .collect();
                if nodes.is_empty() {
                    continue;
                }
                let picked: Vec<StableId> =
                    (0..1 + rng.below(3)).map(|_| *rng.pick(&nodes)).collect();
                let event = SetSelection::new(Selection::new(picked, None));
                if history
                    .record_selection(&state, &event)
                    .expect("선택 기록")
                    .is_some()
                {
                    recorded += 1;
                    total_selection += 1;
                }
                continue;
            }
            let Some(request) = random_valid_request(&state, &mut rng, &mut ids, 32) else {
                continue;
            };
            let transaction = CommandTransaction::single(
                CommandId::from_stable_id(StableId::new_v7(10_000 + seed, step as u16, seed)),
                CommandOrigin::User,
                request,
            );
            history
                .record_document(&mut state, &transaction, None)
                .unwrap_or_else(|e| panic!("seed {seed} step {step}: 기록 실패 {e}"));
            recorded += 1;
            total_document += 1;
        }
        assert_eq!(history.history().len(), recorded, "seed {seed}");
        let final_snapshot = snapshot(&state);
        let final_selection = history.view().selection().clone();

        // 전체 undo → 초기 상태.
        for _ in 0..recorded {
            history
                .undo(&mut state)
                .unwrap_or_else(|e| panic!("seed {seed}: undo 실패 {e}"))
                .expect("undo 항목");
        }
        assert!(history.undo(&mut state).unwrap().is_none());
        assert_eq!(snapshot(&state), initial, "seed {seed}: 전체 undo ≠ 초기");
        assert_eq!(
            history.view().selection(),
            &initial_selection,
            "seed {seed}: 선택"
        );

        // 전체 redo → 최종 상태.
        for _ in 0..recorded {
            history
                .redo(&mut state)
                .unwrap_or_else(|e| panic!("seed {seed}: redo 실패 {e}"))
                .expect("redo 항목");
        }
        assert!(history.redo(&mut state).unwrap().is_none());
        assert_eq!(
            snapshot(&state),
            final_snapshot,
            "seed {seed}: 전체 redo ≠ 최종"
        );
        assert_eq!(
            history.view().selection(),
            &final_selection,
            "seed {seed}: 선택"
        );
    }
    // 분포 회귀 방지: 문서 편집과 선택이 충분히 섞였는지.
    assert!(total_document > 3_000, "문서 편집 {total_document}");
    assert!(total_selection > 500, "선택 {total_selection}");
}

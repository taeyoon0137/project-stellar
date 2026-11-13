//! command 왕복 property test (VII.6.1).
//!
//! [`round_trip_property!`] macro가 command 종류마다 test를 만듭니다. 각 test는 seed별 random valid
//! document(VI.10.5)에 그 종류의 command를 생성해 `apply(forward)` 뒤 `apply(inverse)`를 실행하고,
//! 결과 semantic snapshot이 원본과 같은지 확인합니다. forward 적용 뒤 문서는 전체 validator(VI.10.1)를
//! 통과해야 합니다.
//!
//! **등록 검사**: macro가 등록한 종류 목록이 [`CommandKind::ALL`]과 정확히 같아야 하며, 선택 event
//! (VII.3)도 같은 macro로 등록합니다. 새 command 종류를 추가하면 생성기(`test_support::generate`의
//! wildcard 없는 match)와 이 등록이 함께 갱신되어야 합니다.

use std::collections::BTreeSet;

use stellar_document::test_support::random_document::{random_valid_document, SplitMix64};
use stellar_document::test_support::semantic_snapshot::model_snapshot_json;
use stellar_document::validate::validate_document;

use crate::id::WindowSessionId;
use crate::kind::CommandKind;
use crate::request::CommandRequest;
use crate::selection::{Selection, SelectionPolicy, SetSelection, WindowViewState};
use crate::state::DocumentState;
use crate::test_support::{generate, FreshIds};

/// 종류별 seed 수.
const SEEDS: u64 = 160;

/// 등록 대상: 문서 command 종류 또는 선택 event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Registered {
    Command(CommandKind),
    Selection,
}

/// `kind` command의 forward/inverse 왕복을 seed마다 검사하고 성공 횟수를 반환합니다.
fn command_round_trip(kind: CommandKind) -> usize {
    let mut applied = 0;
    for seed in 0..SEEDS {
        let mut state = DocumentState::new(random_valid_document(seed));
        let mut rng = SplitMix64::new(seed.wrapping_mul(0x9E37) ^ u64::from(kind.as_u16()));
        let mut ids = FreshIds::new(seed);
        // 문서마다 여러 후보를 시도해 적용 가능한 command를 찾습니다.
        for _ in 0..8 {
            let Some(command) = generate(kind, state.model(), &mut rng, &mut ids) else {
                break;
            };
            if command.validate(state.model()).is_err() {
                continue;
            }
            let before = model_snapshot_json(state.model()).expect("snapshot");
            let request = CommandRequest::at(&state, command);
            let outcome = state
                .execute(&request)
                .unwrap_or_else(|e| panic!("{kind} seed {seed}: validate 통과 후 실행 실패: {e}"));
            validate_document(&state.model().to_records().expect("records"))
                .unwrap_or_else(|e| panic!("{kind} seed {seed}: forward 후 문서 무효: {e}"));
            let inverse = CommandRequest::at(&state, outcome.inverse);
            state
                .execute(&inverse)
                .unwrap_or_else(|e| panic!("{kind} seed {seed}: inverse 실패: {e}"));
            assert_eq!(
                model_snapshot_json(state.model()).expect("snapshot"),
                before,
                "{kind} seed {seed}: apply(forward); apply(inverse) ≠ 원본"
            );
            applied += 1;
            break;
        }
    }
    applied
}

/// 선택 event의 forward/inverse 왕복.
fn selection_round_trip() -> usize {
    let mut applied = 0;
    for seed in 0..SEEDS {
        let state = DocumentState::new(random_valid_document(seed));
        let mut rng = SplitMix64::new(seed ^ 0x5E1E);
        let ids = crate::tree::node_ids(state.model());
        if ids.is_empty() {
            continue;
        }
        let mut view = WindowViewState::new(WindowSessionId::from_stable_id(ids[0]));
        let policy = SelectionPolicy {
            include_hidden: rng.one_in(2),
        };
        let first: Vec<_> = (0..rng.below(4)).map(|_| *rng.pick(&ids)).collect();
        SetSelection::new(Selection::new(first, None)).apply(&mut view, state.model(), policy);
        let before = view.clone();
        let next: Vec<_> = (0..1 + rng.below(5)).map(|_| *rng.pick(&ids)).collect();
        let applied_event =
            SetSelection::new(Selection::new(next, None)).apply(&mut view, state.model(), policy);
        applied_event
            .inverse
            .apply(&mut view, state.model(), policy);
        assert_eq!(view, before, "seed {seed}: 선택 forward/inverse 왕복");
        applied += 1;
    }
    applied
}

/// 종류마다 왕복 test를 만들고 등록 목록을 남깁니다.
macro_rules! round_trip_property {
    ($( $name:ident => $target:expr, min $min:expr; )*) => {
        $(
            #[test]
            fn $name() {
                let applied = match $target {
                    Registered::Command(kind) => command_round_trip(kind),
                    Registered::Selection => selection_round_trip(),
                };
                assert!(
                    applied >= $min,
                    "{}: 적용 {applied}건 < 최소 {}",
                    stringify!($name),
                    $min
                );
            }
        )*

        /// macro로 등록한 대상.
        const REGISTERED: &[Registered] = &[$($target),*];
    };
}

round_trip_property! {
    insert_node_round_trip => Registered::Command(CommandKind::InsertNode), min 120;
    delete_subtree_round_trip => Registered::Command(CommandKind::DeleteSubtree), min 80;
    move_node_round_trip => Registered::Command(CommandKind::MoveNode), min 80;
    set_property_round_trip => Registered::Command(CommandKind::SetProperty), min 120;
    insert_modifier_round_trip => Registered::Command(CommandKind::InsertModifier), min 100;
    remove_modifier_round_trip => Registered::Command(CommandKind::RemoveModifier), min 60;
    move_modifier_round_trip => Registered::Command(CommandKind::MoveModifier), min 60;
    set_modifier_property_round_trip => Registered::Command(CommandKind::SetModifierProperty), min 60;
    convert_fragment_to_frame_round_trip => Registered::Command(CommandKind::ConvertFragmentToFrame), min 10;
    convert_shape_to_path_round_trip => Registered::Command(CommandKind::ConvertShapeToPath), min 40;
    restore_node_records_round_trip => Registered::Command(CommandKind::RestoreNodeRecords), min 120;
    duplicate_subtree_round_trip => Registered::Command(CommandKind::DuplicateSubtree), min 80;
    set_selection_round_trip => Registered::Selection, min 120;
}

#[test]
fn every_phase_vii_command_is_registered() {
    // 검증 기준(VII.6.1): 모든 Phase VII command 등록 검사 통과.
    let registered: BTreeSet<Registered> = REGISTERED.iter().copied().collect();
    assert_eq!(registered.len(), REGISTERED.len(), "중복 등록 금지");
    let mut expected: BTreeSet<Registered> = CommandKind::ALL
        .iter()
        .copied()
        .map(Registered::Command)
        .collect();
    expected.insert(Registered::Selection);
    assert_eq!(
        registered, expected,
        "등록 목록 = CommandKind::ALL + 선택 event"
    );
}

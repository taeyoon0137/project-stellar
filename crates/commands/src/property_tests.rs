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
#![cfg(test)]

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
        if matches!(
            kind,
            CommandKind::SwapInstance | CommandKind::DetachInstance | CommandKind::PlaceSlotContent
        ) {
            // random document는 인스턴스를 만들지 않으므로 대상 인스턴스를 하나 넣습니다.
            seed_instance(&mut state, &mut ids);
        }
        if kind == CommandKind::UnsetProperty {
            // random document에는 명시 layout longhand가 없으므로 하나 둡니다(ADR-0086).
            seed_layout(&mut state, &mut rng, &mut ids);
        }
        // 문서마다 여러 후보를 시도해 적용 가능한 command를 찾습니다.
        for _ in 0..8 {
            let Some(command) = generate(kind, state.model(), &mut rng, &mut ids) else {
                break;
            };
            if command.validate(state.model()).is_err() {
                continue;
            }
            let before = model_snapshot_json(state.model()).expect("snapshot");
            // semantic snapshot은 layout record와 활성 기본값 세트를 담지 않으므로 따로 비교합니다(ADR-0086).
            let before_layout = layout_state(&state);
            // 가이드 기록과 View 변수 model도 semantic snapshot JSON 밖이라 따로 비교합니다(ADR-0126).
            let before_guides = guides_state(&state);
            let request = CommandRequest::at(&state, command);
            let outcome = state
                .execute(&request)
                .unwrap_or_else(|e| panic!("{kind} seed {seed}: validate 통과 후 실행 실패: {e}"));
            validate_document(&state.model().to_records().expect("records"))
                .unwrap_or_else(|e| panic!("{kind} seed {seed}: forward 후 문서 무효: {e}"));
            if kind == CommandKind::EditViewVariants {
                // 생성기는 변수·Variant 생성·삭제만 만들므로 forward는 언제나 View model을 바꿉니다.
                assert_ne!(
                    guides_state(&state),
                    before_guides,
                    "{kind} seed {seed}: forward가 View 변수 model을 바꾸지 않았습니다"
                );
            }
            if LAYOUT_KINDS.contains(&kind) {
                assert_ne!(
                    layout_state(&state),
                    before_layout,
                    "{kind} seed {seed}: forward가 layout record·세트를 바꾸지 않았습니다"
                );
            }
            let inverse = CommandRequest::at(&state, outcome.inverse);
            state
                .execute(&inverse)
                .unwrap_or_else(|e| panic!("{kind} seed {seed}: inverse 실패: {e}"));
            assert_eq!(
                model_snapshot_json(state.model()).expect("snapshot"),
                before,
                "{kind} seed {seed}: apply(forward); apply(inverse) ≠ 원본"
            );
            assert_eq!(
                layout_state(&state),
                before_layout,
                "{kind} seed {seed}: apply(forward); apply(inverse)의 layout record·세트 ≠ 원본"
            );
            assert_eq!(
                guides_state(&state),
                before_guides,
                "{kind} seed {seed}: apply(forward); apply(inverse)의 가이드·View 변수 ≠ 원본"
            );
            applied += 1;
            break;
        }
    }
    applied
}

/// 명시 layout longhand·기본값 세트만 바꾸는 종류(forward가 반드시 그 상태를 바꿔야 함).
const LAYOUT_KINDS: [CommandKind; 3] = [
    CommandKind::SetStyleDefaults,
    CommandKind::UnsetProperty,
    CommandKind::SetLayoutProperty,
];

/// semantic snapshot 밖의 상태: 영속 layout record와 활성 기본값 세트 이름.
fn layout_state(
    state: &DocumentState,
) -> (Vec<stellar_document::layout_record::LayoutRecord>, String) {
    (
        state.model().to_records().expect("records").layout_records,
        state.model().style_defaults().to_string(),
    )
}

/// semantic snapshot JSON 밖의 상태: 영속 가이드 기록과 View 변수·Variant model(ADR-0126).
fn guides_state(
    state: &DocumentState,
) -> (
    Vec<stellar_document::guides::NodeGuides>,
    Vec<stellar_document::view_variant::commands::ViewVariantModel>,
) {
    let records = state.model().to_records().expect("records");
    (records.node_guides, records.view_variants)
}

/// root 아래에 override 하나를 가진 인스턴스를 넣습니다(SwapInstance 왕복용).
fn seed_instance(state: &mut DocumentState, ids: &mut FreshIds) {
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::reference::ComponentRef;
    use stellar_core_types::typed_value::TypedValue;
    use stellar_document::component::definition::ComponentPropertyId;
    use stellar_document::component::instance::{ComponentInstancePayload, OverrideKey};
    use stellar_document::records::{NodePayload, NodeRecord};
    let Some(root) = state
        .model()
        .definition()
        .pages
        .iter()
        .flat_map(|p| p.views.iter())
        .map(|v| v.root_frame)
        .find(|root| state.model().contains(*root))
    else {
        return;
    };
    let mut payload = ComponentInstancePayload::new(ComponentRef::new(ids.fresh()));
    payload.set_property_override(
        OverrideKey::direct(ComponentPropertyId::new(ids.fresh())),
        TypedValue::Boolean(true),
    );
    let last = state
        .model()
        .children_in_order(root)
        .last()
        .and_then(|child| state.model().record(*child))
        .map(|record| record.order_key);
    let record = NodeRecord {
        id: ids.fresh(),
        parent: Some(root),
        order_key: OrderKey::between(last.as_ref(), None),
        visible: true,
        locked: false,
        display_name: "인스턴스".into(),
        payload: NodePayload::ComponentInstance(payload),
    };
    let command = crate::command::Command::InsertNode(crate::node_ops::InsertNode::single(record));
    state
        .execute(&CommandRequest::at(state, command))
        .expect("인스턴스 삽입");
}

/// 명시 layout longhand 하나를 둡니다(UnsetProperty 왕복용).
fn seed_layout(state: &mut DocumentState, rng: &mut SplitMix64, ids: &mut FreshIds) {
    for _ in 0..8 {
        let Some(command) = generate(CommandKind::SetLayoutProperty, state.model(), rng, ids)
        else {
            return;
        };
        if command.validate(state.model()).is_ok() {
            state
                .execute(&CommandRequest::at(state, command))
                .expect("layout longhand 두기");
            return;
        }
    }
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
    replace_text_round_trip => Registered::Command(CommandKind::ReplaceText), min 60;
    set_text_range_style_round_trip => Registered::Command(CommandKind::SetTextRangeStyle), min 100;
    add_page_round_trip => Registered::Command(CommandKind::AddPage), min 120;
    remove_page_round_trip => Registered::Command(CommandKind::RemovePage), min 120;
    rename_page_round_trip => Registered::Command(CommandKind::RenamePage), min 120;
    add_view_round_trip => Registered::Command(CommandKind::AddView), min 120;
    remove_view_round_trip => Registered::Command(CommandKind::RemoveView), min 100;
    rename_view_round_trip => Registered::Command(CommandKind::RenameView), min 120;
    apply_external_snapshot_round_trip => Registered::Command(CommandKind::ApplyExternalSnapshot), min 120;
    edit_path_point_round_trip => Registered::Command(CommandKind::EditPathPoint), min 60;
    swap_instance_round_trip => Registered::Command(CommandKind::SwapInstance), min 120;
    set_style_defaults_round_trip => Registered::Command(CommandKind::SetStyleDefaults), min 120;
    unset_property_round_trip => Registered::Command(CommandKind::UnsetProperty), min 120;
    set_layout_property_round_trip => Registered::Command(CommandKind::SetLayoutProperty), min 120;
    edit_context_round_trip => Registered::Command(CommandKind::EditContext), min 120;
    set_node_guides_round_trip => Registered::Command(CommandKind::SetNodeGuides), min 120;
    edit_view_variants_round_trip => Registered::Command(CommandKind::EditViewVariants), min 120;
    detach_instance_round_trip => Registered::Command(CommandKind::DetachInstance), min 120;
    place_slot_content_round_trip => Registered::Command(CommandKind::PlaceSlotContent), min 60;
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

/// 합성 key의 longhand 분할(IX.11.22): 옛 묶음 key bytes는 그대로 decode·실행되고, 결과는 longhand 묶음과 같습니다.
mod key_split {
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::value::{FiniteF32, Vec2};
    use stellar_document::css::bundle::bundle_longhands;
    use stellar_document::frame::keys as frame_keys;
    use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierKind, ModifierPayload};
    use stellar_document::modifier_metadata::keys as modifier_keys;

    use crate::codec::{decode_command, encode_command};
    use crate::command::test_fixtures::{sid, small_model};
    use crate::command::Command;
    use crate::modifier_ops::{InsertModifier, SetModifierProperty};
    use crate::property::{node_property_keys, PropertyValue};
    use crate::request::CommandRequest;
    use crate::state::DocumentState;

    fn with_shadow() -> DocumentState {
        let mut state = DocumentState::new(small_model());
        let black = Color::new(ColorSpace::Srgb, 0., 0., 0., 1.).unwrap();
        let insert = Command::InsertModifier(InsertModifier {
            node: sid(11),
            entry: ModifierEntry {
                id: ModifierId::new(sid(61)),
                order: OrderKey::from_bytes(vec![0x20]).unwrap(),
                kind: ModifierKind::Shadow,
                enabled: true,
                payload: ModifierPayload::Shadow {
                    color: black,
                    offset: Vec2::new(1., 1.).unwrap(),
                    blur: FiniteF32::ZERO,
                },
            },
        });
        state.execute(&CommandRequest::at(&state, insert)).unwrap();
        state
    }

    fn shadow_offset(state: &DocumentState) -> Vec2 {
        match &state.model().modifier_stack(sid(11)).unwrap().entries()[0].payload {
            ModifierPayload::Shadow { offset, .. } => *offset,
            other => panic!("Shadow가 아님: {other:?}"),
        }
    }

    #[test]
    fn legacy_shadow_offset_bytes_apply_like_both_longhands() {
        // SetModifierProperty(0x0023), 길이 45, node, modifier, offset 묶음 key(0x1092), Offset tag(15), x, y.
        let mut bytes = vec![0x23, 0, 45, 0, 0, 0];
        bytes.extend_from_slice(&sid(11).to_bytes());
        bytes.extend_from_slice(&sid(61).to_bytes());
        bytes.extend_from_slice(&[0x92, 0x10, 0, 0, 15]);
        bytes.extend_from_slice(&6.5f32.to_le_bytes());
        bytes.extend_from_slice(&(-3.0f32).to_le_bytes());
        let legacy = decode_command(&bytes).unwrap();
        assert_eq!(encode_command(&legacy), bytes, "옛 bytes는 그대로 왕복");
        let mut old = with_shadow();
        old.execute(&CommandRequest::at(&old, legacy)).unwrap();

        let mut split = with_shadow();
        for (key, value) in modifier_keys::SHADOW_OFFSET_LONGHANDS
            .into_iter()
            .zip([6.5f32, -3.0])
        {
            let command = Command::SetModifierProperty(SetModifierProperty {
                node: sid(11),
                modifier: ModifierId::new(sid(61)),
                key,
                value: PropertyValue::Number(FiniteF32::new(value).unwrap()),
            });
            assert_eq!(decode_command(&encode_command(&command)).unwrap(), command);
            split.execute(&CommandRequest::at(&split, command)).unwrap();
        }
        assert_eq!(shadow_offset(&old), shadow_offset(&split));
        assert_eq!(shadow_offset(&split), Vec2::new(6.5, -3.0).unwrap());
    }

    #[test]
    fn one_offset_longhand_keeps_the_other_axis_and_undoes() {
        let mut state = with_shadow();
        let command = Command::SetModifierProperty(SetModifierProperty {
            node: sid(11),
            modifier: ModifierId::new(sid(61)),
            key: modifier_keys::SHADOW_OFFSET_Y,
            value: PropertyValue::Number(FiniteF32::new(9.).unwrap()),
        });
        let outcome = state.execute(&CommandRequest::at(&state, command)).unwrap();
        assert_eq!(shadow_offset(&state), Vec2::new(1., 9.).unwrap());
        state
            .execute(&CommandRequest::at(&state, outcome.inverse))
            .unwrap();
        assert_eq!(shadow_offset(&state), Vec2::new(1., 1.).unwrap());
    }

    #[test]
    fn new_edit_key_lists_never_offer_bundle_keys() {
        let model = small_model();
        let keys = node_property_keys(model.payload(sid(11)).unwrap());
        assert!(keys.iter().all(|k| bundle_longhands(*k).is_none()));
        assert!(keys.contains(&frame_keys::WIDTH));
        assert!(!keys.contains(&frame_keys::SIZE));
        assert!(!keys.contains(&frame_keys::CORNER_RADIUS));
    }
}

/// registry 일관성(IX.11.3): command 공통 key와 문서 registry, 속성 패널 행 문법과 registry 문법.
mod registry_consistency {
    use stellar_document::css::grammar::CssGrammar;
    use stellar_document::css::registry::{entries, PropertyScope};
    use stellar_document::payload_metadata::node_keys as document_node_keys;

    use crate::change::node_keys;
    use crate::inspector::{InspectorProperty, NodeClass};

    #[test]
    fn common_node_keys_match_the_document_registry() {
        assert_eq!(node_keys::VISIBLE, document_node_keys::VISIBLE);
        assert_eq!(node_keys::LOCKED, document_node_keys::LOCKED);
        assert_eq!(node_keys::DISPLAY_NAME, document_node_keys::DISPLAY_NAME);
    }

    #[test]
    fn inspector_rows_with_one_registry_key_use_the_registry_name_and_grammar() {
        let scope_of = |class: NodeClass| match class {
            NodeClass::Frame => Some(PropertyScope::Frame),
            NodeClass::Rectangle | NodeClass::Ellipse | NodeClass::Line | NodeClass::Polygon => {
                Some(PropertyScope::Shape)
            }
            NodeClass::Image => Some(PropertyScope::Image),
            _ => None,
        };
        let classes = [
            NodeClass::Frame,
            NodeClass::Rectangle,
            NodeClass::Ellipse,
            NodeClass::Line,
            NodeClass::Polygon,
            NodeClass::Image,
        ];
        let mut checked = 0;
        for property in InspectorProperty::all() {
            // 위치(transform 이동 성분), 칠하기·선 색(없음을 none으로 다루는 행), 묶음 행은 registry key 하나와
            // 1:1이 아닙니다.
            if matches!(
                property,
                InspectorProperty::Left
                    | InspectorProperty::Top
                    | InspectorProperty::Fill
                    | InspectorProperty::Stroke
                    | InspectorProperty::Color
                    | InspectorProperty::BorderRadius
            ) {
                continue;
            }
            for class in classes {
                if !property.applies(class) {
                    continue;
                }
                let key = property.key(class);
                let scopes = [
                    scope_of(class),
                    Some(PropertyScope::Node),
                    Some(PropertyScope::Modifier),
                ];
                let Some(entry) = entries()
                    .into_iter()
                    .find(|e| e.key == key && scopes.contains(&Some(e.scope)))
                else {
                    panic!(
                        "{} {class:?}: registry에 key 0x{:04x} 없음",
                        property.name(),
                        key.0
                    );
                };
                let row = property.grammar();
                if entry.name == "clipping" {
                    // Frame clipping은 행에서 overflow keyword로 고르고 registry는 변형 규칙으로 냅니다.
                    assert_eq!(property.name(), "overflow");
                    assert!(matches!(row, CssGrammar::Keyword(_)));
                } else {
                    assert_eq!(entry.name, property.name(), "{class:?}");
                    let registry = entry.css.grammar.expect("행이 있는 속성은 값 문법이 있음");
                    assert_eq!(row, registry, "{} {class:?}", property.name());
                }
                checked += 1;
            }
        }
        assert!(checked >= 20, "확인한 행 {checked}");
    }
}

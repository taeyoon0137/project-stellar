//! 활성 View layout의 Text 측정 hook과 컴포넌트 정의 연결 test (ADR-0134 결정 1).
#![cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]

use std::path::Path;

use stellar_commands::command::Command;
use stellar_commands::id::{CommandId, CommandOrigin};
use stellar_commands::property::{PropertyValue, SetProperty};
use stellar_commands::request::CommandRequest;
use stellar_commands::state::DocumentState;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::order_key::OrderKey;
use stellar_core_types::reference::{ComponentRef, NodeRef};
use stellar_core_types::text::UserText;
use stellar_core_types::value::{FiniteF32, Size2};
use stellar_document::component::definition::{
    ComponentDefinition, ComponentLibrary, DefinitionNode, DefinitionTree,
};
use stellar_document::component::instance::ComponentInstancePayload;
use stellar_document::layout_record::{LayoutLonghand, LayoutRecord};
use stellar_document::modifier::{ModifierEntry, ModifierId, ModifierKind, ModifierPayload};
use stellar_document::node_kind::NodeKind;
use stellar_document::payload_metadata::shape_keys;
use stellar_document::record_codec::encode_payload;
use stellar_document::records::NodeRecord;
use stellar_document::shape::{RectangleParams, ShapeGeometry};
use stellar_document::text::{Paragraph, TextPayload};
use stellar_text::incremental::IncrementalTextLayout;

use super::*;
use crate::canvas::scene_layout::CanvasLayout;
use crate::canvas::transform::fixtures::{add_shape, at, canvas_document, frame, open};
use crate::canvas::CanvasCache;
use crate::window_session::test_fixtures::sid;

fn set_layout(model: &mut DocumentModel, id: u64, values: &[(&str, &str)]) {
    let mut record = model
        .layout_record(sid(id))
        .cloned()
        .unwrap_or_else(|| LayoutRecord::new(sid(id)));
    for (name, value) in values {
        let longhand = LayoutLonghand::from_name(name).unwrap();
        record
            .set(longhand, longhand.parse(value).unwrap())
            .unwrap();
    }
    model.set_layout_record(record).unwrap();
}

fn text(content: &str) -> NodePayload {
    NodePayload::Text(
        TextPayload::new(UserText::new(content), vec![Paragraph { style: 0 }], vec![]).unwrap(),
    )
}

/// Frame 11(10, 20, 100×50)을 가로 flex로 두고 맨 앞에 Text 29, 그 뒤 Rectangle 30(20×20)을 둡니다.
fn flex_text_document(content: &str) -> DocumentModel {
    let mut doc = canvas_document();
    // 새 프로젝트의 CSS initial style(family 없음, medium): 기본 글꼴 chain으로 배치합니다.
    doc.replace_text_styles(stellar_document::text_styles::DocumentTextStyles::initial(
        sid(900),
    ));
    set_layout(
        &mut doc,
        11,
        &[("display", "flex"), ("align-items", "flex-start")],
    );
    add_shape(
        &mut doc,
        30,
        11,
        ShapeGeometry::Rectangle(RectangleParams {
            size: Size2::new(20., 20.).unwrap(),
            corner_radius: Default::default(),
        }),
        at(0., 0.),
    );
    let first = doc.children_in_order(sid(11))[0];
    let key = doc
        .record(first)
        .map(|r| OrderKey::between(None, Some(&r.order_key)))
        .unwrap();
    doc.insert_node(NodeRecord {
        id: sid(29),
        parent: Some(sid(11)),
        order_key: key,
        visible: true,
        locked: false,
        display_name: "label".to_string(),
        payload: text(content),
    })
    .unwrap();
    doc
}

/// 같은 Text를 창 글꼴 cache로 줄바꿈 없이 배치한 크기(캔버스 행 목록과 같은 함수).
fn unwrapped_size(model: &DocumentModel) -> Size2 {
    let NodePayload::Text(payload) = model.payload(sid(29)).unwrap() else {
        unreachable!()
    };
    let mut fonts = FontCache::default();
    fonts
        .layout_input(
            Path::new("."),
            sid(29),
            payload,
            &model.definition().text_styles,
            model.modifier_stack(sid(29)),
            &mut IncrementalTextLayout::new(),
        )
        .expect("기본 글꼴 chain으로 배치")
        .1
}

/// 기본 글꼴 chain은 macOS 시스템 글꼴입니다.
#[cfg(target_os = "macos")]
#[test]
fn current_lays_out_text_flow_children_with_the_window_fonts() {
    let doc = flex_text_document("ab");
    let natural = unwrapped_size(&doc);
    assert!(natural.width() > 0.0 && natural.height() > 0.0);
    let (_dir, w) = open("layout-text", &doc);
    let mut cache = CanvasCache::default();
    let layout = CanvasLayout::current(&w, &mut cache).unwrap();
    assert!(
        layout.source().skipped.is_empty(),
        "{:?}",
        layout.source().skipped
    );
    // Text는 흐름 첫 자리(컨테이너 원점 10, 20)에 줄바꿈 없는 크기로 놓이고, Rectangle은 그 너비만큼 밀립니다.
    let text_box = layout.world_box(sid(29)).expect("Text 상자");
    assert_eq!(text_box, [10., 20., natural.width(), natural.height()]);
    let rect = layout.world_box(sid(30)).unwrap();
    assert_eq!(rect, [10. + natural.width(), 20., 20., 20.]);
    // Text hook 없이 변환하면 Text는 상자 없이 빠집니다(이전 `current`의 모양).
    let bare = CanvasLayout::for_model(&doc, sid(10), None).unwrap();
    assert_eq!(bare.world_box(sid(29)), None);
}

/// 기본 글꼴 chain은 macOS 시스템 글꼴입니다.
#[cfg(target_os = "macos")]
#[test]
fn narrow_flow_containers_wrap_text_with_the_same_measurement_as_drawing() {
    // 컨테이너 content box(100)보다 긴 Text는 flex 축소로 줄바꿈되어 여러 줄 높이가 됩니다. layout이 잰 크기와 scene이
    // 그리는 Text 입력 크기가 같습니다(측정·그리기가 같은 함수, 같은 줄 너비).
    let doc = flex_text_document("alpha beta gamma delta epsilon zeta eta theta");
    let natural = unwrapped_size(&doc);
    assert!(natural.width() > 100.0, "{natural:?}");
    let mut fonts = FontCache::default();
    let hook = CanvasText::new(&mut fonts, Path::new("."), &doc);
    let layout = CanvasLayout::with_inputs(
        &doc,
        sid(10),
        CanvasLayoutInputs {
            library: None,
            text: Some(&hook),
        },
    )
    .unwrap();
    let text_box = layout.world_box(sid(29)).unwrap();
    assert!(text_box[2] <= 100.0, "{text_box:?}");
    assert!(text_box[3] > natural.height() * 1.5, "{text_box:?}");
    let layout_box = layout.layout().layout_box(sid(29)).unwrap();
    let rect = layout_box.content_rect();
    match &layout
        .source()
        .source
        .get(stellar_scene::source::SceneKey::node(sid(29)))
        .unwrap()
        .kind
    {
        stellar_scene::source::SourceKind::Text(input) => {
            assert_eq!(input.size.height(), rect.height().to_px_f32().get());
            assert!(input.size.width() <= rect.width().to_px_f32().get());
        }
        other => panic!("{other:?}"),
    }
}

fn execute(state: &mut DocumentState, command: Command) -> stellar_commands::change::ChangeSet {
    let tx = CommandTransaction::new(CommandId::from_stable_id(sid(9_000)), CommandOrigin::User)
        .with_request(CommandRequest::new(state.revision(), command));
    state.execute_transaction(&tx).unwrap().changes
}

/// 기본 글꼴 chain은 macOS 시스템 글꼴입니다.
#[cfg(target_os = "macos")]
#[test]
fn apply_keeps_text_measurement_and_matches_a_fresh_conversion() {
    let mut state = DocumentState::new(flex_text_document("ab"));
    let mut fonts = FontCache::default();
    let mut layout = {
        let hook = CanvasText::new(&mut fonts, Path::new("."), state.model());
        CanvasLayout::with_inputs(
            state.model(),
            sid(10),
            CanvasLayoutInputs {
                library: None,
                text: Some(&hook),
            },
        )
        .unwrap()
    };
    let changes = execute(
        &mut state,
        Command::SetProperty(SetProperty {
            node: sid(30),
            key: shape_keys::WIDTH,
            value: PropertyValue::NonNegative(FiniteF32::new(35.0).unwrap()),
        }),
    );
    let edits = {
        let hook = CanvasText::new(&mut fonts, Path::new("."), state.model());
        layout
            .apply_with(
                state.model(),
                &changes,
                CanvasLayoutInputs {
                    library: None,
                    text: Some(&hook),
                },
            )
            .unwrap()
    };
    assert!(!edits.is_empty());
    // 같은 측정 hook으로 다시 계산한 것이 아니라 증분 갱신입니다.
    assert_eq!(layout.layout().full_rebuilds(), 0);
    let hook = CanvasText::new(&mut fonts, Path::new("."), state.model());
    let fresh = CanvasLayout::with_inputs(
        state.model(),
        sid(10),
        CanvasLayoutInputs {
            library: None,
            text: Some(&hook),
        },
    )
    .unwrap();
    assert_eq!(layout.source().source, fresh.source().source);
    assert_eq!(layout.world_box(sid(30)).unwrap()[2], 35.0);
}

/// 정의 root 501(60×30) 하나인 컴포넌트.
fn library() -> (ComponentLibrary, ComponentRef) {
    let component = ComponentRef::new(sid(500));
    let mut tree = DefinitionTree::new(component);
    tree.insert(DefinitionNode {
        id: sid(501),
        kind: NodeKind::Frame,
        parent: None,
        order_key: OrderKey::from_bytes(vec![0x40]).unwrap(),
        visible: true,
        locked: false,
        display_name: "root".to_string(),
        payload: encode_payload(&frame(60., 30., at(999., 999.))),
    })
    .unwrap();
    let mut library = ComponentLibrary::new();
    library
        .insert(
            ComponentDefinition::new(sid(500), "카드".to_string(), NodeRef::new(sid(501))),
            tree,
        )
        .unwrap();
    (library, component)
}

#[test]
fn flow_child_instances_get_boxes_from_the_component_library() {
    let (library, component) = library();
    let mut doc = canvas_document();
    set_layout(
        &mut doc,
        11,
        &[("display", "flex"), ("align-items", "flex-start")],
    );
    doc.insert_node(NodeRecord {
        id: sid(40),
        parent: Some(sid(11)),
        order_key: doc.next_child_order_key(sid(11)),
        visible: true,
        locked: false,
        display_name: "card".to_string(),
        payload: NodePayload::ComponentInstance(ComponentInstancePayload::new(component)),
    })
    .unwrap();
    let mut stack = stellar_document::modifier::ModifierStack::new(sid(40));
    stack
        .insert(ModifierEntry {
            id: ModifierId::new(sid(140)),
            order: OrderKey::from_bytes(vec![0x40]).unwrap(),
            kind: ModifierKind::Transform,
            enabled: true,
            payload: ModifierPayload::Transform(at(0., 0.)),
        })
        .unwrap();
    doc.set_modifier_stack(stack).unwrap();
    let layout = CanvasLayout::with_inputs(
        &doc,
        sid(10),
        CanvasLayoutInputs {
            library: Some(&library),
            text: None,
        },
    )
    .unwrap();
    // 흐름 첫 자리(컨테이너 원점 10, 20)에 정의 root 크기로 놓입니다.
    assert_eq!(layout.world_box(sid(40)), Some([10., 20., 60., 30.]));
    // 정의 집합 없이는 인스턴스가 빠지고 상자가 없습니다(이전 `current`의 모양).
    let bare = CanvasLayout::for_model(&doc, sid(10), None).unwrap();
    assert_eq!(bare.world_box(sid(40)), None);
}

/// 기본 글꼴 chain은 macOS 시스템 글꼴입니다.
#[cfg(target_os = "macos")]
#[test]
fn reorder_preview_measures_and_draws_text_with_the_same_inputs() {
    // 흐름 drag preview도 Text를 다시 그립니다. 그렇지 않으면 preview에서 Text scene 입력이 빠져 옛 자리에 남습니다.
    let doc = flex_text_document("ab");
    let natural = unwrapped_size(&doc);
    let mut fonts = FontCache::default();
    let hook = CanvasText::new(&mut fonts, Path::new("."), &doc);
    let inputs = CanvasLayoutInputs {
        library: None,
        text: Some(&hook),
    };
    let layout = CanvasLayout::with_inputs(&doc, sid(10), inputs).unwrap();
    let mut order = doc.children_in_order(sid(11)).to_vec();
    let text_at = order.iter().position(|id| *id == sid(29)).unwrap();
    let rect_at = order.iter().position(|id| *id == sid(30)).unwrap();
    order.swap(text_at, rect_at);
    // 창 자료로 다시 재는 preview와 이미 그린 Text 크기를 쓰는 preview가 같은 자리입니다.
    for preview in [
        layout
            .reorder_preview_with(&doc, sid(11), &order, inputs)
            .unwrap(),
        layout.reorder_preview(&doc, sid(11), &order).unwrap(),
    ] {
        assert_eq!(
            layout.preview_box(&preview, sid(30)),
            Some([10., 20., 20., 20.])
        );
        assert_eq!(
            layout.preview_box(&preview, sid(29)),
            Some([30., 20., natural.width(), natural.height()])
        );
    }
}

/// 기본 글꼴 chain은 macOS 시스템 글꼴입니다.
#[cfg(target_os = "macos")]
#[test]
fn min_content_width_is_the_widest_unbreakable_word() {
    // CSS min-content(`overflow-wrap: normal`): 줄바꿈 기회마다 끊은 가장 긴 단위입니다. 단어 안에서 끊지 않습니다.
    let doc = flex_text_document("alpha be");
    let word = unwrapped_size(&flex_text_document("alpha"));
    let mut fonts = FontCache::default();
    let hook = CanvasText::new(&mut fonts, Path::new("."), &doc);
    let intrinsic = TextIntrinsic {
        words: vec![],
        space: stellar_core_types::value::Length::new(0.0).unwrap(),
        line_height: stellar_core_types::value::Length::new(0.0).unwrap(),
        ascent: stellar_core_types::value::Length::new(0.0).unwrap(),
    };
    let min = hook.min_content_width(sid(29), &intrinsic).unwrap();
    assert_eq!(min.to_px_f32().get(), word.width());
    // min-content 너비로 배치하면 단어를 끊지 않고 기회에서만 줄을 나눕니다.
    let at_min = hook.layout(sid(29), &intrinsic, Some(min)).unwrap();
    assert_eq!(at_min.width.to_px_f32().get(), word.width());
}

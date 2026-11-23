//! 캔버스 Text 편집기 fixture (XII.10.33).

use super::*;
use crate::window_session::test_fixtures::{sample_document, sid, window};
use stellar_core_types::clock::ManualClock;
use stellar_core_types::text::UserText;
use stellar_document::text::{Paragraph, StyleRun, TextStyle};
use stellar_document::text_styles::DocumentTextStyles;

const TEXT: u64 = 30;

fn id(n: u64) -> CommandId {
    CommandId::from_stable_id(sid(70_000 + n))
}

fn v(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y).unwrap()
}

/// 문서 1의 View root 10 아래 Frame 11·12, 기본 style 0과 문자 style 1, (선택) Text 30 "abcd".
fn rig(with_text: bool) -> (WindowHistory, DocumentState) {
    let mut model = sample_document();
    let mut styles = DocumentTextStyles::initial(sid(900));
    styles
        .runs
        .insert(1, TextStyle::new(Some(sid(900)), None, vec![], vec![]));
    model.replace_text_styles(styles);
    if with_text {
        model
            .insert_node(NodeRecord {
                id: sid(TEXT),
                parent: Some(sid(10)),
                order_key: model.next_child_order_key(sid(10)),
                visible: true,
                locked: false,
                display_name: "Text".into(),
                payload: NodePayload::Text(
                    TextPayload::new(UserText::new("abcd"), vec![Paragraph { style: 0 }], vec![])
                        .unwrap(),
                ),
            })
            .unwrap();
    }
    (WindowHistory::new(window(1)), DocumentState::new(model))
}

fn entries(history: &WindowHistory) -> usize {
    history.history().entries().len()
}

fn content(doc: &DocumentState, node: StableId) -> Option<String> {
    text_payload(doc.model(), node).map(|t| t.content().to_owned())
}

fn placement(width: TextWidth) -> TextPlacement {
    TextPlacement {
        parent: sid(10),
        origin: v(40., 50.),
        width,
    }
}

#[test]
fn text_tool_click_is_auto_width_and_drag_is_fixed_width() {
    let click = plan_text_tool(sid(10), v(5., 5.), v(7., 6.), v(40., 50.), v(42., 51.)).unwrap();
    assert_eq!(click, placement(TextWidth::Auto));
    let drag = plan_text_tool(sid(10), v(5., 5.), v(105., 9.), v(140., 50.), v(40., 54.)).unwrap();
    assert_eq!(drag.width, TextWidth::Fixed(100.));
    assert_eq!(drag.origin, v(40., 50.));
    assert_eq!(TextWidth::Auto.value(), LayoutValue::Keyword("max-content"));
}

#[test]
fn hangul_composition_on_new_text_records_once_on_commit() {
    let (mut history, mut doc) = rig(false);
    let clock = ManualClock::new(0);
    let node = sid(31);
    let mut editor =
        CanvasTextEditor::begin_new(&doc, placement(TextWidth::Auto), node, sid(32), 7).unwrap();
    let before = doc.revision();
    for (i, step) in ["ㅎ", "하", "한"].into_iter().enumerate() {
        editor
            .set_marked(&doc, step, ScalarRange::caret(1))
            .unwrap();
        let preview = editor.preview(&doc).unwrap();
        assert_eq!(preview.payload.content(), step);
        assert_eq!(preview.key, (7, i as u64 + 1));
        assert_eq!(preview.node, node);
        assert_eq!(preview.new_in.as_ref().unwrap().0, sid(10));
    }
    // 조합 중에는 문서·history가 그대로입니다.
    assert_eq!(doc.revision(), before);
    assert_eq!(entries(&history), 0);
    assert!(!doc.model().contains(node));
    // 입력기가 "한"을 확정: 새 Text를 만들고(자동 폭) 선택합니다.
    assert!(editor
        .insert(&mut history, &mut doc, "한", id(1), &clock)
        .unwrap());
    assert_eq!(content(&doc, node).as_deref(), Some("한"));
    assert_eq!(
        doc.model()
            .layout_record(node)
            .and_then(|r| r.get(LayoutLonghand::Width)),
        Some(&AUTO_WIDTH)
    );
    assert_eq!(history.view().selection().ids(), [node]);
    editor
        .set_marked(&doc, "그", ScalarRange::caret(1))
        .unwrap();
    assert_eq!(editor.preview(&doc).unwrap().payload.content(), "한그");
    editor
        .set_marked(&doc, "글", ScalarRange::caret(1))
        .unwrap();
    assert!(editor
        .commit_composition(&mut history, &mut doc, id(2), &clock)
        .unwrap());
    assert_eq!(content(&doc, node).as_deref(), Some("한글"));
    // 생성과 연속 입력이 history 한 항목입니다.
    assert_eq!(entries(&history), 1);
    let end = editor
        .finish(&mut history, &mut doc, id(3), &clock)
        .unwrap();
    assert_eq!(
        end,
        EditEnd::Selected {
            node,
            selection: None
        }
    );
    history.undo(&mut doc).unwrap();
    assert!(!doc.model().contains(node));
    assert!(history.view().selection().is_empty());
}

#[test]
fn new_text_ended_without_input_records_nothing() {
    let (mut history, mut doc) = rig(false);
    let clock = ManualClock::new(0);
    let before = doc.model().to_records().unwrap();
    let mut editor =
        CanvasTextEditor::begin_new(&doc, placement(TextWidth::Fixed(80.)), sid(31), sid(32), 1)
            .unwrap();
    editor
        .set_marked(&doc, "ㅎ", ScalarRange::caret(1))
        .unwrap();
    // 조합 중 Escape는 조합만 버리고, 다음 Escape가 편집을 끝냅니다.
    assert_eq!(editor.escape(), EscapeStep::CompositionCancelled);
    assert_eq!(editor.preview(&doc), None);
    assert_eq!(editor.escape(), EscapeStep::End);
    // 지우기도 아무것도 만들지 않습니다.
    assert!(!editor
        .delete_backward(&mut history, &mut doc, id(1), &clock)
        .unwrap());
    let end = editor
        .finish(&mut history, &mut doc, id(2), &clock)
        .unwrap();
    assert_eq!(end, EditEnd::CreationCancelled);
    assert_eq!(entries(&history), 0);
    assert_eq!(doc.model().to_records().unwrap(), before);
}

#[test]
fn fixed_width_creation_and_edge_drag_turns_auto_into_fixed() {
    let (mut history, mut doc) = rig(false);
    let clock = ManualClock::new(0);
    let mut editor =
        CanvasTextEditor::begin_new(&doc, placement(TextWidth::Fixed(80.)), sid(31), sid(32), 1)
            .unwrap();
    editor
        .insert(&mut history, &mut doc, "abc", id(1), &clock)
        .unwrap();
    assert_eq!(
        text_width_mode(doc.model(), sid(31)),
        Some(TextWidthMode::Fixed(80.))
    );
    let mut auto =
        CanvasTextEditor::begin_new(&doc, placement(TextWidth::Auto), sid(41), sid(42), 2).unwrap();
    auto.insert(&mut history, &mut doc, "x", id(2), &clock)
        .unwrap();
    assert_eq!(
        text_width_mode(doc.model(), sid(41)),
        Some(TextWidthMode::Auto)
    );
    let command = edge_width_command(doc.model(), sid(41), 120.).unwrap();
    let tx =
        CommandTransaction::new(id(3), CommandOrigin::User).with_command(doc.revision(), command);
    history.record_document(&mut doc, &tx, None).unwrap();
    assert_eq!(
        text_width_mode(doc.model(), sid(41)),
        Some(TextWidthMode::Fixed(120.))
    );
    // Text가 아니면 거부, 기본값(`auto`)만 있는 Text도 자동 폭입니다.
    assert_eq!(
        edge_width_command(doc.model(), sid(11), 10.),
        Err(TextEditError::NotText)
    );
    let (_, plain) = rig(true);
    assert_eq!(
        text_width_mode(plain.model(), sid(TEXT)),
        Some(TextWidthMode::Auto)
    );
}

#[test]
fn japanese_conversion_on_existing_text_is_one_replace() {
    let (mut history, mut doc) = rig(true);
    let clock = ManualClock::new(0);
    let selection = EditSelection {
        anchor: 1,
        focus: 3,
    };
    let mut editor = CanvasTextEditor::begin_existing(&doc, sid(TEXT), Some(selection), 5).unwrap();
    for step in ["か", "かん", "かんじ"] {
        editor
            .set_marked(&doc, step, ScalarRange::caret(1))
            .unwrap();
    }
    editor
        .set_marked(&doc, "漢字", ScalarRange::new(0, 2).unwrap())
        .unwrap();
    let preview = editor.preview(&doc).unwrap();
    assert_eq!(preview.payload.content(), "a漢字d");
    assert!(preview.new_in.is_none());
    assert_eq!(entries(&history), 0);
    assert_eq!(content(&doc, sid(TEXT)).as_deref(), Some("abcd"));
    // 바깥 클릭: 조합을 확정하고 끝냅니다. 선택이 비어 있었으므로 Text 선택을 기록합니다.
    let end = editor
        .finish(&mut history, &mut doc, id(1), &clock)
        .unwrap();
    assert_eq!(content(&doc, sid(TEXT)).as_deref(), Some("a漢字d"));
    let EditEnd::Selected { node, selection } = end else {
        panic!("{end:?}")
    };
    assert_eq!(node, sid(TEXT));
    assert!(selection.is_some());
    assert_eq!(history.view().selection().ids(), [sid(TEXT)]);
    // 문서 항목 하나(교체) + 선택 항목 하나.
    assert_eq!(entries(&history), 2);
}

#[test]
fn typing_and_backspace_group_and_follow_undo() {
    let (mut history, mut doc) = rig(true);
    let clock = ManualClock::new(0);
    let mut editor = CanvasTextEditor::begin_existing(&doc, sid(TEXT), None, 5).unwrap();
    for (i, c) in ["e", "f", "g"].into_iter().enumerate() {
        editor
            .insert(&mut history, &mut doc, c, id(i as u64), &clock)
            .unwrap();
    }
    assert_eq!(entries(&history), 1);
    editor
        .delete_backward(&mut history, &mut doc, id(10), &clock)
        .unwrap();
    assert_eq!(content(&doc, sid(TEXT)).as_deref(), Some("abcdef"));
    assert_eq!(entries(&history), 2);
    // 다른 경로의 undo 뒤에도 편집기는 문서 본문을 다시 읽어 이어 씁니다.
    history.undo(&mut doc).unwrap();
    assert_eq!(content(&doc, sid(TEXT)).as_deref(), Some("abcdefg"));
    editor
        .insert(&mut history, &mut doc, "!", id(11), &clock)
        .unwrap();
    assert_eq!(content(&doc, sid(TEXT)).as_deref(), Some("abcdefg!"));
}

#[test]
fn typography_change_during_editing_is_one_transaction() {
    let (mut history, mut doc) = rig(true);
    let clock = ManualClock::new(0);
    let runs = |doc: &DocumentState| {
        text_payload(doc.model(), sid(TEXT))
            .unwrap()
            .style_runs()
            .to_vec()
    };
    let run = |start, end, style| StyleRun {
        range: TextRange::new(start, end).unwrap(),
        style,
    };
    // 1) 선택 범위: SetTextRangeStyle 하나.
    let mut editor = CanvasTextEditor::begin_existing(
        &doc,
        sid(TEXT),
        Some(EditSelection {
            anchor: 1,
            focus: 3,
        }),
        5,
    )
    .unwrap();
    assert_eq!(
        editor
            .apply_style(&mut history, &mut doc, Some(1), id(1))
            .unwrap(),
        StyleOutcome::Recorded
    );
    assert_eq!(runs(&doc), vec![run(1, 3, 1)]);
    assert_eq!(entries(&history), 1);
    // 2) 조합 중: 조합 확정과 그 범위 style이 한 transaction(Undo 한 번에 둘 다).
    editor
        .move_caret(&doc, CaretMove::ParagraphEnd, false)
        .unwrap();
    editor
        .set_marked(&doc, "xy", ScalarRange::caret(2))
        .unwrap();
    editor
        .apply_style(&mut history, &mut doc, Some(1), id(2))
        .unwrap();
    assert_eq!(content(&doc, sid(TEXT)).as_deref(), Some("abcdxy"));
    assert_eq!(runs(&doc), vec![run(1, 3, 1), run(4, 6, 1)]);
    assert_eq!(entries(&history), 2);
    history.undo(&mut doc).unwrap();
    assert_eq!(content(&doc, sid(TEXT)).as_deref(), Some("abcd"));
    assert_eq!(runs(&doc), vec![run(1, 3, 1)]);
    // 3) caret: 다음 확정 입력과 같은 transaction.
    assert_eq!(
        editor
            .apply_style(&mut history, &mut doc, Some(1), id(3))
            .unwrap(),
        StyleOutcome::Pending
    );
    let before = entries(&history);
    editor
        .insert(&mut history, &mut doc, "z", id(4), &clock)
        .unwrap();
    assert_eq!(entries(&history), before + 1);
    assert_eq!(content(&doc, sid(TEXT)).as_deref(), Some("abcdz"));
    assert_eq!(runs(&doc), vec![run(1, 3, 1), run(4, 5, 1)]);
    history.undo(&mut doc).unwrap();
    assert_eq!(runs(&doc), vec![run(1, 3, 1)]);
    // 표에 없는 style 번호는 거부하고 편집 상태를 되돌립니다.
    editor
        .move_caret(&doc, CaretMove::SelectAll, false)
        .unwrap();
    assert!(editor
        .apply_style(&mut history, &mut doc, Some(9), id(5))
        .is_err());
    assert_eq!(
        editor.buffer().selection().range(),
        ScalarRange::new(0, 4).unwrap()
    );
}

#[test]
fn style_before_creation_is_part_of_the_creating_entry() {
    let (mut history, mut doc) = rig(false);
    let clock = ManualClock::new(0);
    let mut editor =
        CanvasTextEditor::begin_new(&doc, placement(TextWidth::Auto), sid(31), sid(32), 1).unwrap();
    assert_eq!(
        editor
            .apply_style(&mut history, &mut doc, Some(1), id(1))
            .unwrap(),
        StyleOutcome::Pending
    );
    editor
        .insert(&mut history, &mut doc, "ab", id(2), &clock)
        .unwrap();
    assert_eq!(entries(&history), 1);
    assert_eq!(
        text_payload(doc.model(), sid(31)).unwrap().style_runs(),
        [StyleRun {
            range: TextRange::new(0, 2).unwrap(),
            style: 1
        }]
    );
}

#[test]
fn locked_text_and_locked_parent_are_not_editable() {
    let (_, doc) = rig(true);
    let mut model = doc.into_model();
    let mut record = model.record(sid(TEXT)).unwrap();
    record.locked = true;
    model.remove_leaf(sid(TEXT)).unwrap();
    model.insert_node(record).unwrap();
    let doc = DocumentState::new(model);
    assert_eq!(
        CanvasTextEditor::begin_existing(&doc, sid(TEXT), None, 1).err(),
        Some(TextEditError::NotEditable)
    );
    assert_eq!(
        edge_width_command(doc.model(), sid(TEXT), 10.),
        Err(TextEditError::NotEditable)
    );
    assert_eq!(
        CanvasTextEditor::begin_existing(&doc, sid(11), None, 1).err(),
        Some(TextEditError::NotText)
    );
}

/// 편집 중 조합 본문은 scene의 공통 text 엔진(같은 글꼴 cache·layout)으로 그려집니다.
#[test]
fn composition_preview_glyphs_come_from_the_common_text_engine() {
    use crate::canvas::text::{document, saved, tests::project_font_styles};
    use crate::canvas::CanvasScene;
    use crate::window_session::test_fixtures::TempDir;
    use stellar_core_types::color::{Color, ColorSpace};
    let project = TempDir::new("text-edit-preview");
    let (_, doc) = rig(true);
    let mut model = doc.into_model();
    let black = Color::new(ColorSpace::Srgb, 0., 0., 0., 1.).unwrap();
    let (styles, _) = project_font_styles(project.path(), [sid(701), sid(702)], 20 * 64, black);
    model.replace_text_styles(styles);
    let doc = DocumentState::new(model);
    let mut fonts = document::FontCache::default();
    let mut saved = saved::SavedTextLayouts::default();
    let mut scene = |preview: Option<&TextPreview>| {
        let table = &doc.model().definition().text_styles;
        fonts.sync_registrations(table);
        saved.begin_view(table);
        let scene = CanvasScene::with_library(
            doc.model(),
            sid(10),
            None,
            Some((project.path(), &mut fonts, &mut saved)),
            preview,
        );
        saved.end_view();
        scene
    };
    let glyphs = |scene: &CanvasScene, node| {
        let row = scene.frames.iter().position(|f| f.id == node).unwrap() as u32;
        scene.text[&row].glyphs.len()
    };
    let mut editor = CanvasTextEditor::begin_existing(
        &doc,
        sid(TEXT),
        Some(EditSelection {
            anchor: 0,
            focus: 4,
        }),
        9,
    )
    .unwrap();
    // 시험 글꼴은 "A"만 잉크가 있습니다.
    editor
        .set_marked(&doc, "AAA", ScalarRange::caret(3))
        .unwrap();
    let preview = editor.preview(&doc).unwrap();
    assert_eq!(glyphs(&scene(Some(&preview)), sid(TEXT)), 3);
    editor.set_marked(&doc, "A", ScalarRange::caret(1)).unwrap();
    let preview = editor.preview(&doc).unwrap();
    assert_eq!(glyphs(&scene(Some(&preview)), sid(TEXT)), 1);
}

#[test]
fn style_on_a_composition_equal_to_the_replaced_text_writes_only_the_style() {
    let (mut history, mut doc) = rig(true);
    let mut editor = CanvasTextEditor::begin_existing(
        &doc,
        sid(TEXT),
        Some(EditSelection {
            anchor: 0,
            focus: 2,
        }),
        5,
    )
    .unwrap();
    editor
        .set_marked(&doc, "ab", ScalarRange::new(0, 2).unwrap())
        .unwrap();
    assert_eq!(
        editor
            .apply_style(&mut history, &mut doc, Some(1), id(1))
            .unwrap(),
        StyleOutcome::Recorded
    );
    assert_eq!(content(&doc, sid(TEXT)).as_deref(), Some("abcd"));
    assert_eq!(
        text_payload(doc.model(), sid(TEXT)).unwrap().style_runs(),
        [StyleRun {
            range: TextRange::new(0, 2).unwrap(),
            style: 1
        }]
    );
    assert_eq!(entries(&history), 1);
    assert!(editor.buffer().composition().is_none());
}

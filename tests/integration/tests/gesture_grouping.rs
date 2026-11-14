//! 종단 시나리오: pointer gesture preview·scrub 취소·키보드 반복 grouping(VII.4)이 만든 history가 저장·재열기와
//! 섞여도 문서 의미가 맞는지.
//!
//! - preview는 마지막 transaction만 문서에 반영하고 끝날 때 항목 하나가 됩니다.
//! - 취소한 scrub은 문서와 history를 바꾸지 않습니다.
//! - 간격 안의 키보드 반복은 항목 하나로 합쳐지고 undo 한 번이 묶음 전체를 되돌립니다.
//! - 저장은 history와 독립입니다: 저장 뒤 undo한 문서를 다시 저장하면 디스크도 그 상태가 됩니다.

mod common;

use common::builders::{color, frame, new_document};
use common::{assert_same_document, key, save_documents, sid, Editor, TempDir};
use stellar_commands::command::Command;
use stellar_commands::node_ops::InsertNode;
use stellar_commands::property::{PropertyValue, SetProperty};
use stellar_core_types::clock::ManualClock;
use stellar_core_types::ids::StableId;
use stellar_core_types::value::Transform2D;
use stellar_document::frame::keys as frame_keys;
use stellar_document::model::DocumentModel;
use stellar_document::records::NodePayload;
use stellar_history::entry::{GroupingKey, HistoryError};
use stellar_project::identity::parse_project_id;
use stellar_project::saved_project::SavedProjectLoader;
use stellar_project::scaffold::write_default_scaffold;

const SPACE: u64 = 71;
const PROJECT_ID: &str = "0195c530-3acd-7bc1-8414-c213f61cb96b";
const NANOS_PER_MS: u64 = 1_000_000;

fn id(n: u64) -> StableId {
    sid(SPACE, n)
}

fn translate(x: f32, y: f32) -> Command {
    Command::SetProperty(SetProperty {
        node: id(11),
        key: frame_keys::TRANSFORM,
        value: PropertyValue::Transform(Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y).expect("변환")),
    })
}

fn offset(model: &DocumentModel) -> (f32, f32) {
    match model.payload(id(11)) {
        Some(NodePayload::Frame(frame)) => {
            let (_, _, _, _, tx, ty) = frame.transform.components();
            (tx, ty)
        }
        other => panic!("Frame이어야 함: {other:?}"),
    }
}

fn reopen(root: &std::path::Path) -> DocumentModel {
    SavedProjectLoader::new()
        .open(root)
        .expect("열기")
        .load_document(id(1))
        .expect("로드")
        .model
}

#[test]
fn gesture_preview_scrub_and_keyboard_grouping_round_trip_through_disk() {
    let sandbox = TempDir::new("gestures");
    let root = sandbox.path.join("project");
    std::fs::create_dir_all(&root).expect("프로젝트 폴더");
    write_default_scaffold(&root, "제스처", PROJECT_ID).expect("scaffold");
    let project_id = parse_project_id(PROJECT_ID).expect("project id");

    let mut editor = Editor::new(
        new_document(
            id(1),
            "제스처 문서",
            &[(id(2), "페이지", vec![(id(3), "홈", id(10))])],
        ),
        SPACE,
    );
    editor.apply(
        vec![Command::InsertNode(InsertNode::single(frame(
            id(11),
            id(10),
            key(0x10),
            "상자",
            (0.0, 0.0),
            (100.0, 100.0),
        )))],
        "상자",
    );
    let inserted = editor.snapshot();
    let first = save_documents(&root, project_id, &[editor.model()], &[]);

    // 1. drag gesture: 문서에는 마지막 preview만, history에는 끝날 때 항목 하나.
    editor
        .window
        .begin_pointer_gesture(1)
        .expect("gesture 시작");
    for step in 1..=5 {
        let transaction =
            editor.transaction(vec![translate(step as f32 * 10.0, step as f32 * 5.0)]);
        editor
            .window
            .update_preview(&mut editor.state, &transaction)
            .expect("preview");
        assert_eq!(
            offset(editor.model()),
            (step as f32 * 10.0, step as f32 * 5.0)
        );
        assert_eq!(editor.window.history().len(), 1, "preview 중 항목 없음");
    }
    // preview 중 다른 문서 편집은 거부됩니다.
    assert!(matches!(
        editor.try_apply(vec![Command::rename_node(id(11), "x")]),
        Err(HistoryError::PreviewActive)
    ));
    assert!(editor.window.end_preview().expect("gesture 끝").is_some());
    assert_eq!(editor.window.history().len(), 2);
    let dragged = editor.snapshot();

    // 2. scrub 취소: 문서·history 무변경.
    editor.window.begin_scrub(2).expect("scrub 시작");
    for shade in [0.2, 0.5, 0.8] {
        let transaction = editor.transaction(vec![Command::SetProperty(SetProperty {
            node: id(11),
            key: frame_keys::BACKGROUND,
            value: PropertyValue::OptionalColor(Some(color(shade, shade, shade))),
        })]);
        editor
            .window
            .update_preview(&mut editor.state, &transaction)
            .expect("scrub preview");
    }
    editor
        .window
        .cancel_preview(&mut editor.state)
        .expect("취소");
    assert_eq!(editor.snapshot(), dragged, "취소는 preview 전 상태");
    assert_eq!(editor.window.history().len(), 2);

    // 3. 키보드 반복: 간격 안 4번은 한 항목, 간격 밖 1번은 새 항목.
    let mut clock = ManualClock::new(1_000 * NANOS_PER_MS);
    for step in 1..=4 {
        let transaction = editor.transaction(vec![translate(50.0 + step as f32, 25.0)]);
        let key = GroupingKey::keyboard(&transaction).expect("반복 signature");
        editor
            .window
            .record_grouped(&mut editor.state, &transaction, key, &clock)
            .expect("반복 기록");
        clock.advance(100 * NANOS_PER_MS);
    }
    assert_eq!(editor.window.history().len(), 3, "반복 4번 = 항목 1");
    let nudged = editor.snapshot();
    clock.advance(2_000 * NANOS_PER_MS);
    let transaction = editor.transaction(vec![translate(80.0, 25.0)]);
    let key = GroupingKey::keyboard(&transaction).expect("반복 signature");
    editor
        .window
        .record_grouped(&mut editor.state, &transaction, key, &clock)
        .expect("반복 기록");
    assert_eq!(editor.window.history().len(), 4, "간격 밖 입력은 새 항목");
    assert_eq!(offset(editor.model()), (80.0, 25.0));

    // 4. 저장 → 재열기.
    let second = save_documents(&root, project_id, &[editor.model()], &first.documents);
    assert_same_document(editor.model(), &reopen(&root), "편집 저장 재열기");

    // 5. 저장 뒤에도 session history로 undo: 반복 묶음 → gesture 순서로 되돌립니다.
    assert!(editor.undo());
    assert_eq!(editor.snapshot(), nudged);
    assert!(editor.undo());
    assert_eq!(editor.snapshot(), dragged, "undo 한 번이 반복 묶음 전체");
    assert!(editor.undo());
    assert_eq!(editor.snapshot(), inserted, "undo 한 번이 gesture 전체");
    let third = save_documents(&root, project_id, &[editor.model()], &second.documents);
    assert_eq!(
        third.plan.manifest.chunk_pointers, first.plan.manifest.chunk_pointers,
        "undo 뒤 저장 = 처음 저장 bytes"
    );
    assert_same_document(editor.model(), &reopen(&root), "undo 저장 재열기");

    // 6. redo 전체 → 마지막 편집 상태.
    while editor.redo() {}
    assert_eq!(offset(editor.model()), (80.0, 25.0));
}

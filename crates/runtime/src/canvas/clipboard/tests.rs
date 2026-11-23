//! 캔버스 클립보드 fixture: 같은 창·다른 창·다른 프로젝트 paste, 중복 StableId 0, 일반 텍스트, Cut, 봉투 검사.

use super::*;
use crate::canvas::transform::fixtures::{
    canvas_document, history_len, model, open, scene, select,
};
use crate::window_session::test_fixtures::{
    identity, other_document, saved_project_with, sid, window,
};
use stellar_core_types::text::UserText;
use stellar_document::text::{Paragraph, TextPayload};

const VISIBLE: [f32; 4] = [0.0, 0.0, 400.0, 300.0];

fn yes(_: StableId) -> bool {
    true
}

fn with_text() -> DocumentModel {
    let mut doc = canvas_document();
    for (id, parent, text) in [(60u64, 11u64, "첫 줄"), (61, 11, "둘째")] {
        let mut record = doc.record(sid(12)).unwrap();
        record.id = sid(id);
        record.parent = Some(sid(parent));
        record.order_key = doc.next_child_order_key(sid(parent));
        record.payload = NodePayload::Text(
            TextPayload::new(UserText::new(text), vec![Paragraph { style: 0 }], vec![]).unwrap(),
        );
        doc.insert_node(record).unwrap();
    }
    doc
}

/// 문서 전체의 StableId(node·modifier)에 중복이 없는지.
fn no_duplicate_ids(model: &DocumentModel) {
    let records = model.to_records().unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for node in &records.nodes {
        assert!(seen.insert(node.id), "중복 node id {}", node.id);
    }
    for stack in &records.modifier_stacks {
        for entry in stack.entries() {
            assert!(seen.insert(entry.id.stable_id()), "중복 modifier id");
        }
    }
}

#[test]
fn copy_writes_both_formats_and_the_envelope_round_trips() {
    let (_dir, mut w) = open("clip-copy", &with_text());
    select(&mut w, &[11, 13]);
    let s = scene(&w);
    let copied = copy(&w, &s).unwrap();
    assert_eq!(copied.text.as_deref(), Some("첫 줄\n둘째"));
    let clipboard = CanvasClipboard::decode(&copied.envelope).unwrap();
    assert_eq!(clipboard.roots.len(), 2);
    assert_eq!(clipboard.document, model(&w).document_id());
    assert_eq!(clipboard.roots[0].parent, sid(10));
    assert!(clipboard.roots[0].bounds.is_some());
    assert_eq!(clipboard.encode(), copied.envelope);
    // 형식 검사: 잘린 bytes·다른 header·남는 bytes 거부.
    assert!(CanvasClipboard::decode(&copied.envelope[..copied.envelope.len() - 1]).is_err());
    assert!(CanvasClipboard::decode(b"STELLAR-CLIP\x01").is_err());
    let mut longer = copied.envelope.clone();
    longer.push(0);
    assert!(CanvasClipboard::decode(&longer).is_err());
    // Text가 없는 선택은 일반 텍스트가 없습니다.
    select(&mut w, &[13]);
    assert_eq!(copy(&w, &scene(&w)).unwrap().text, None);
    assert_eq!(STELLAR_PASTEBOARD_TYPE, "app.stellar.canvas-subtree.v1");
}

#[test]
fn same_window_paste_places_by_rule_with_fresh_ids() {
    // 검증 기준(XII.10.38): 같은 창 paste, 중복 StableId 0.
    let (_dir, mut w) = open("clip-same", &with_text());
    select(&mut w, &[11]);
    let s = scene(&w);
    let copied = copy(&w, &s).unwrap();
    // 1) 선택이 Frame 12 하나 → 그 안.
    select(&mut w, &[12]);
    let before = history_len(&w);
    let pasted = paste(&mut w, &s, &copied.envelope, VISIBLE, &yes, &yes).unwrap();
    assert_eq!(pasted.placement, PastePlacement::InsideSelection(sid(12)));
    assert_eq!(history_len(&w), before + 1, "붙여넣기 한 항목");
    assert_eq!(w.view().selection().ids(), pasted.roots.as_slice());
    assert!(model(&w)
        .children_in_order(sid(12))
        .contains(&pasted.roots[0]));
    no_duplicate_ids(model(&w));
    // 2) Rectangle 선택(컨테이너 아님), 원본이 보임 → 같은 부모·같은 위치.
    select(&mut w, &[13]);
    let s = scene(&w);
    let same = paste(&mut w, &s, &copied.envelope, VISIBLE, &yes, &yes).unwrap();
    assert_eq!(same.placement, PastePlacement::SamePlace);
    let node = same.roots[0];
    assert_eq!(parent_of(model(&w), node), Some(sid(10)));
    let t = stellar_commands::arrange::place::local_transform(model(&w), node).unwrap();
    assert_eq!((t.components().4, t.components().5), (10.0, 20.0));
    // 자손도 새 id입니다(원본 Text 60·61과 다름).
    let children = model(&w).children_in_order(node);
    assert_eq!(children.len(), 2);
    assert!(!children.contains(&sid(60)));
    no_duplicate_ids(model(&w));
    // 3) 컨테이너가 아닌 선택, 원본이 화면 밖 → 화면 가운데.
    select(&mut w, &[13]);
    let s = scene(&w);
    let far = [5_000.0, 5_000.0, 400.0, 300.0];
    let center = paste(&mut w, &s, &copied.envelope, far, &yes, &yes).unwrap();
    assert_eq!(center.placement, PastePlacement::ViewCenter);
    let s = scene(&w);
    let b = node_geometry(&s, center.roots[0]).unwrap().world_bounds;
    assert!((b[0] + b[2] / 2.0 - 5_200.0).abs() < 1e-3);
    assert!((b[1] + b[3] / 2.0 - 5_150.0).abs() < 1e-3);
    no_duplicate_ids(model(&w));
    // undo 한 번이 붙여넣기 하나를 되돌립니다.
    let count = model(&w).children_in_order(sid(10)).len();
    assert!(w.undo().unwrap());
    assert_eq!(model(&w).children_in_order(sid(10)).len(), count - 1);
}

#[test]
fn paste_right_after_copy_does_not_nest_the_frame_in_itself() {
    // Frame 11을 선택한 채 Copy·Paste: 자기 안이 아니라 같은 부모·같은 위치.
    let (_dir, mut w) = open("clip-self", &with_text());
    select(&mut w, &[11]);
    let s = scene(&w);
    let copied = copy(&w, &s).unwrap();
    let pasted = paste(&mut w, &s, &copied.envelope, VISIBLE, &yes, &yes).unwrap();
    assert_eq!(pasted.placement, PastePlacement::SamePlace);
    assert_eq!(parent_of(model(&w), pasted.roots[0]), Some(sid(10)));
    assert!(!model(&w)
        .children_in_order(sid(11))
        .contains(&pasted.roots[0]));
    no_duplicate_ids(model(&w));
}

#[test]
fn paste_falls_back_to_the_view_center_when_an_original_parent_is_gone() {
    // Frame 12 안의 Rectangle을 복사한 뒤 Frame 12를 지우면 원래 부모 local 좌표를 쓸 곳이 없어 화면 가운데입니다.
    let mut doc = with_text();
    let mut record = doc.record(sid(13)).unwrap();
    record.id = sid(70);
    record.parent = Some(sid(12));
    record.order_key = doc.next_child_order_key(sid(12));
    doc.insert_node(record).unwrap();
    let (_dir, mut w) = open("clip-gone", &doc);
    select(&mut w, &[70]);
    let copied = copy(&w, &scene(&w)).unwrap();
    let delete = vec![Command::DeleteSubtree(DeleteSubtree { root: sid(12) })];
    crate::canvas::transform::commit(&mut w, delete, 0).unwrap();
    select(&mut w, &[13]);
    let s = scene(&w);
    let pasted = paste(&mut w, &s, &copied.envelope, VISIBLE, &yes, &yes).unwrap();
    assert_eq!(pasted.placement, PastePlacement::ViewCenter);
    assert_eq!(parent_of(model(&w), pasted.roots[0]), Some(sid(10)));
    no_duplicate_ids(model(&w));
}

#[test]
fn other_window_of_the_same_project_pastes_and_other_project_is_rejected() {
    // 검증 기준(XII.10.38): 다른 창·다른 프로젝트 paste fixture.
    let doc = with_text();
    let dir = saved_project_with("clip-windows", &[&doc]);
    let mut a = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
    let mut b = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
    select(&mut a, &[13]);
    let copied = copy(&a, &scene(&a)).unwrap();
    let sb = scene(&b);
    let pasted = paste(&mut b, &sb, &copied.envelope, VISIBLE, &yes, &yes).unwrap();
    assert_eq!(pasted.placement, PastePlacement::SamePlace);
    no_duplicate_ids(model(&b));
    assert_eq!(history_len(&a), 1, "복사한 창 history는 선택 하나뿐");

    // 다른 프로젝트(다른 문서): 진단과 함께 거부, 문서·history 불변.
    let other = other_document();
    let other_dir = saved_project_with("clip-other", &[&other]);
    let mut c = ProjectWindowSession::load(identity(other_dir.path()), window(3)).unwrap();
    let before = (history_len(&c), c.revision());
    let missing = |id: StableId| id != sid(900);
    let sc = scene(&c);
    let error = paste(&mut c, &sc, &copied.envelope, VISIBLE, &missing, &yes).unwrap_err();
    assert_eq!(
        error,
        ClipboardError::Rejected(vec![PasteDiagnostic::CrossDocument])
    );
    assert_eq!((history_len(&c), c.revision()), before);

    // 참조 진단: 대상에 없는 Component·asset.
    let mut clipboard = CanvasClipboard::decode(&copied.envelope).unwrap();
    clipboard.components = vec![sid(900), sid(901)];
    clipboard.assets = vec![sid(950)];
    let diagnostics = clipboard.diagnostics(model(&c), &missing, &|_| false);
    assert_eq!(
        diagnostics,
        vec![
            PasteDiagnostic::CrossDocument,
            PasteDiagnostic::ComponentMissing(sid(900)),
            PasteDiagnostic::AssetMissing(sid(950)),
        ]
    );
    assert_eq!(
        diagnostics
            .iter()
            .map(PasteDiagnostic::code)
            .collect::<Vec<_>>(),
        vec!["cross-document", "component-missing", "asset-missing"]
    );
}

#[test]
fn cut_deletes_roots_in_one_entry_and_refuses_locked_roots() {
    let (_dir, mut w) = open("clip-cut", &with_text());
    select(&mut w, &[13, 14]);
    let s = scene(&w);
    let before = history_len(&w);
    let copied = cut(&mut w, &s).unwrap();
    assert_eq!(history_len(&w), before + 1);
    assert!(!model(&w).contains(sid(13)) && !model(&w).contains(sid(14)));
    // 잘라낸 것을 다시 붙여 넣으면 화면에 보이는 원래 자리입니다.
    let sw = scene(&w);
    let pasted = paste(&mut w, &sw, &copied.envelope, VISIBLE, &yes, &yes).unwrap();
    assert_eq!(pasted.roots.len(), 2);
    // 잠긴 root는 Cut 전체를 거부합니다.
    let mut doc = with_text();
    crate::canvas::select_fixtures::set_flags(&mut doc, 13, true, true);
    let (_d2, mut locked) = open("clip-cut-locked", &doc);
    select(&mut locked, &[13]);
    let before = history_len(&locked);
    let sl = scene(&locked);
    assert_eq!(
        cut(&mut locked, &sl).unwrap_err(),
        ClipboardError::Locked(sid(13))
    );
    assert_eq!(history_len(&locked), before);
    // 잠긴 node도 복사는 됩니다.
    assert!(copy(&locked, &scene(&locked)).is_ok());
}

#[test]
#[ignore = "release 빌드에서 대규모 paste 시간을 측정"]
fn measure_large_paste() {
    // 검증 기준(XII.10.38): 대규모 paste 최초 시각 반응 10ms 이하. 1,000 node subtree를 붙여 넣는 코어 처리(봉투 해석,
    // ID 재발급, 기록, 새 선택)를 잽니다.
    let mut doc = canvas_document();
    let mut record = doc.record(sid(12)).unwrap();
    for n in 0..1_000u64 {
        record.id = sid(20_000 + n);
        record.parent = Some(sid(11));
        record.order_key = doc.next_child_order_key(sid(11));
        doc.insert_node(record.clone()).unwrap();
    }
    let (_dir, mut w) = open("clip-large", &doc);
    select(&mut w, &[11]);
    let copied = copy(&w, &scene(&w)).unwrap();
    select(&mut w, &[12]);
    let s = scene(&w);
    let mut samples = Vec::new();
    for _ in 0..20 {
        let start = std::time::Instant::now();
        paste(&mut w, &s, &copied.envelope, VISIBLE, &yes, &yes).unwrap();
        samples.push(start.elapsed());
        w.undo().unwrap();
    }
    samples.sort_unstable();
    println!(
        "paste 1,001 nodes, 20 samples: p50 {:?}, max {:?}",
        samples[10], samples[19]
    );
    assert!(
        samples[19] <= std::time::Duration::from_millis(10),
        "최대 {:?}",
        samples[19]
    );
}

//! 결정론적 자동 배치 (XXX.2.2).

mod support;

use stellar_document::prototype::layout_hint::{HintPoint, LayoutHints};
use stellar_prototype::auto_layout::layout;
use stellar_prototype::edit::PrototypeDocument;
use stellar_prototype::graph::{ProjectionInput, ProjectionScope, PrototypeGraphView};
use support::*;

fn project(doc: &PrototypeDocument) -> PrototypeGraphView {
    PrototypeGraphView::project(
        &ProjectionScope::Component(toggle()),
        ProjectionInput {
            library: &doc.library,
            bindings: &doc.bindings,
            motions: &doc.motions,
        },
    )
}

#[test]
fn auto_layout_is_deterministic_and_keeps_hints() {
    // 검증 기준(XXX.2.2): 같은 입력 같은 좌표, hint node 좌표 불변.
    let doc = document();
    let view = project(&doc);
    let empty = LayoutHints::new();
    let first = layout(&view, &empty);
    assert_eq!(first, layout(&project(&document()), &empty));
    assert_eq!(first.len(), view.nodes().len());
    let pinned = view.nodes()[3].id;
    let mut hints = LayoutHints::new();
    hints.pin(pinned.id, HintPoint::new(-500.0, 777.0).expect("좌표"));
    let with_hint = layout(&view, &hints);
    let placed = with_hint[&pinned];
    assert!(placed.pinned);
    assert_eq!((placed.x, placed.y), (-500.0, 777.0));
    assert_eq!(with_hint, layout(&view, &hints));
}

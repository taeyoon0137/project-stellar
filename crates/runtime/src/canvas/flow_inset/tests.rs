//! inset 이동 fixture: 시작·끝·늘이기·백분율 anchor, 흐름 밖이 아닌 node 거부, gesture당 history 1건.

use super::*;
use crate::canvas::transform::fixtures::{
    add_shape, at, canvas_document, history_len, model, open, select,
};
use crate::window_session::test_fixtures::sid;
use stellar_core_types::value::Size2;
use stellar_document::shape::{RectangleParams, ShapeGeometry};

/// Frame 11(10,20 100×50) 아래 20×20 Rectangle 30(저장 transform 0,0).
fn inset_window(
    tag: &str,
    values: &[(&str, &str)],
) -> (
    crate::window_session::test_fixtures::TempDir,
    ProjectWindowSession,
) {
    let mut doc = canvas_document();
    add_shape(
        &mut doc,
        30,
        11,
        ShapeGeometry::Rectangle(RectangleParams {
            size: Size2::new(20., 20.).unwrap(),
            corner_radius: Default::default(),
        }),
        at(0.0, 0.0),
    );
    let (dir, mut w) = open(tag, &doc);
    let commands = values
        .iter()
        .map(|(name, value)| {
            let longhand = LayoutLonghand::from_name(name).unwrap();
            Command::SetLayoutProperty(SetLayoutProperty {
                node: sid(30),
                longhand,
                value: longhand.parse(value).unwrap(),
            })
        })
        .collect();
    crate::canvas::transform::commit(&mut w, commands, 0).unwrap();
    (dir, w)
}

fn record_value(w: &ProjectWindowSession, name: &str) -> Option<LayoutValue> {
    let longhand = LayoutLonghand::from_name(name).unwrap();
    model(w)
        .layout_record(sid(30))
        .and_then(|r| r.get(longhand).cloned())
}

#[test]
fn start_anchor_inset_moves_with_one_history_entry() {
    let (_dir, mut w) = inset_window(
        "inset-start",
        &[("position", "absolute"), ("left", "15px"), ("top", "5px")],
    );
    select(&mut w, &[30]);
    let layout = CanvasLayout::current(&w, &mut crate::canvas::CanvasCache::default()).unwrap();
    // layout 결과 위치: 부모 원점(10, 20) + inset(15, 5).
    assert_eq!(layout.world_box(sid(30)), Some([25., 25., 20., 20.]));
    let drag = InsetDrag::begin(&w, &layout).unwrap();
    // 끄는 동안: preview만, 문서·history 불변.
    let before = history_len(&w);
    let preview = drag.preview(&layout, [10., 4.]);
    assert_eq!(preview.len(), 1);
    assert_eq!(history_len(&w), before);
    assert!(drag.commit(&mut w, [10., 4.]).unwrap());
    assert_eq!(history_len(&w), before + 1);
    assert_eq!(record_value(&w, "left"), Some(LayoutValue::Length(25.)));
    assert_eq!(record_value(&w, "top"), Some(LayoutValue::Length(9.)));
    assert_eq!(record_value(&w, "right"), None);
    let after = CanvasLayout::current(&w, &mut crate::canvas::CanvasCache::default()).unwrap();
    assert_eq!(after.world_box(sid(30)), Some([35., 29., 20., 20.]));
    // 저장 transform은 바꾸지 않았습니다(이동은 inset만).
    let stack = model(&w).modifier_stack(sid(30)).unwrap();
    assert!(stack
        .entries()
        .iter()
        .any(|e| e.payload == stellar_document::modifier::ModifierPayload::Transform(at(0., 0.))));
    // 이동이 없으면 기록하지 않습니다.
    select(&mut w, &[30]);
    let drag = InsetDrag::begin(&w, &after).unwrap();
    assert!(!drag.commit(&mut w, [0., 0.]).unwrap());
}

#[test]
fn end_stretch_and_percentage_anchors_follow_css_directions() {
    // 끝 anchor: right는 반대 방향, top·bottom 모두 auto면 static position에서 top px.
    let (_dir, mut w) = inset_window("inset-end", &[("position", "absolute"), ("right", "10px")]);
    select(&mut w, &[30]);
    let layout = CanvasLayout::current(&w, &mut crate::canvas::CanvasCache::default()).unwrap();
    let drag = InsetDrag::begin(&w, &layout).unwrap();
    assert!(drag.commit(&mut w, [5., 3.]).unwrap());
    assert_eq!(record_value(&w, "right"), Some(LayoutValue::Length(5.)));
    assert_eq!(record_value(&w, "left"), None);
    assert_eq!(record_value(&w, "top"), Some(LayoutValue::Length(3.)));
    // 늘이기(left·right 모두 명시)는 두 값을 같이 옮깁니다. 백분율은 containing block(100×50) 기준입니다.
    let (_dir, mut w) = inset_window(
        "inset-stretch",
        &[
            ("position", "absolute"),
            ("left", "10%"),
            ("right", "20px"),
            ("top", "0px"),
        ],
    );
    select(&mut w, &[30]);
    let layout = CanvasLayout::current(&w, &mut crate::canvas::CanvasCache::default()).unwrap();
    let drag = InsetDrag::begin(&w, &layout).unwrap();
    assert!(drag.commit(&mut w, [10., 0.]).unwrap());
    assert_eq!(record_value(&w, "left"), Some(LayoutValue::Percentage(20.)));
    assert_eq!(record_value(&w, "right"), Some(LayoutValue::Length(10.)));
}

#[test]
fn nodes_without_explicit_absolute_position_are_not_inset_drags() {
    let (_dir, mut w) = inset_window("inset-free", &[("left", "5px")]);
    select(&mut w, &[30]);
    let layout = CanvasLayout::current(&w, &mut crate::canvas::CanvasCache::default()).unwrap();
    assert_eq!(
        InsetDrag::begin(&w, &layout).unwrap_err(),
        TransformError::Disabled("not-absolute")
    );
}

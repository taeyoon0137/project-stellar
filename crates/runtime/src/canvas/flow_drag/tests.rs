//! 흐름 drag fixture: flex 가로·세로·wrap, Grid 자동 배치, 끌어내기, 인스턴스 거부, Grid 명시 배치(C6).

use super::*;
use crate::canvas::transform::fixtures::{
    add_shape, at, canvas_document, history_len, model, open, scene, select,
};
use crate::window_session::test_fixtures::sid;
use stellar_commands::style_defaults::SetLayoutProperty;
use stellar_core_types::value::Size2;
use stellar_document::shape::{RectangleParams, ShapeGeometry};

const ROW: FlowAxes = FlowAxes {
    main: 0,
    reverse: false,
};
const COLUMN: FlowAxes = FlowAxes {
    main: 1,
    reverse: false,
};

fn b(x: f32, y: f32) -> Box4 {
    [x, y, 20.0, 20.0]
}

#[test]
fn flex_row_column_and_reverse_insertion_indices() {
    // 검증 기준(XII.10.24): flex 가로·세로에서 삽입 index 정확.
    let row = [b(0., 0.), b(30., 0.), b(60., 0.)];
    assert_eq!(insertion_index(&row, ROW, [-5., 10.]), 0);
    assert_eq!(insertion_index(&row, ROW, [15., 10.]), 1);
    assert_eq!(insertion_index(&row, ROW, [45., 10.]), 2);
    assert_eq!(insertion_index(&row, ROW, [500., 10.]), 3);
    // 줄 밖(교차축 아래)이어도 가장 가까운 줄의 주축 위치로 셉니다.
    assert_eq!(insertion_index(&row, ROW, [45., 300.]), 2);
    let column = [b(0., 0.), b(0., 30.), b(0., 60.)];
    assert_eq!(insertion_index(&column, COLUMN, [10., 35.]), 1);
    assert_eq!(insertion_index(&column, COLUMN, [10., 45.]), 2);
    // row-reverse: 흐름 순서 첫 항목이 화면 오른쪽입니다.
    let reverse = FlowAxes {
        main: 0,
        reverse: true,
    };
    let reversed = [b(60., 0.), b(30., 0.), b(0., 0.)];
    assert_eq!(insertion_index(&reversed, reverse, [80., 10.]), 0);
    assert_eq!(insertion_index(&reversed, reverse, [45., 10.]), 1);
    assert_eq!(insertion_index(&reversed, reverse, [-10., 10.]), 3);
    assert_eq!(insertion_index(&[], ROW, [0., 0.]), 0);
}

#[test]
fn wrap_and_grid_auto_placement_use_reading_lines() {
    // 검증 기준(XII.10.24): flex wrap과 Grid 자동 배치에서 삽입 index 정확.
    // 두 줄: [0,1,2] y=0, [3,4] y=30.
    let wrapped = [b(0., 0.), b(30., 0.), b(60., 0.), b(0., 30.), b(30., 30.)];
    assert_eq!(insertion_index(&wrapped, ROW, [45., 5.]), 2);
    assert_eq!(insertion_index(&wrapped, ROW, [5., 40.]), 3);
    assert_eq!(insertion_index(&wrapped, ROW, [45., 40.]), 5);
    assert_eq!(insertion_index(&wrapped, ROW, [15., 40.]), 4);
    // Grid 3열 자동 배치(읽는 순서), 크기가 다른 칸.
    let grid = FlowAxes::of(FlowParent::Grid).unwrap();
    let cells = [
        [0., 0., 40., 20.],
        [50., 0., 40., 20.],
        [100., 0., 40., 20.],
        [0., 30., 40., 50.],
        [50., 30., 40., 50.],
    ];
    assert_eq!(insertion_index(&cells, grid, [95., 10.]), 2);
    assert_eq!(insertion_index(&cells, grid, [60., 70.]), 4);
    assert_eq!(insertion_index(&cells, grid, [120., 70.]), 5);
}

#[test]
fn reflow_preview_moves_siblings_out_of_the_way() {
    // 같은 크기 항목: 0번을 맨 끝으로 옮기면 1·2가 한 칸씩 앞으로, 0이 마지막 칸에 옵니다.
    let row = [b(0., 0.), b(30., 0.), b(60., 0.)];
    let preview = reflow_preview(&row, ROW, 0, 2);
    assert_eq!(
        preview,
        vec![(1, b(0., 0.)), (2, b(30., 0.)), (0, b(60., 0.))]
    );
    // wrap: 줄마다 원래 항목 수를 지킵니다.
    let wrapped = [b(0., 0.), b(30., 0.), b(0., 30.)];
    let preview = reflow_preview(&wrapped, ROW, 2, 0);
    assert_eq!(
        preview,
        vec![(2, b(0., 0.)), (0, b(30., 0.)), (1, b(0., 30.))]
    );
}

/// Frame 11(10,20 100×50)을 `display` 컨테이너로, 그 아래 20×20 Rectangle 30·31·32를 x 0·30·60에 둡니다.
fn flow_window(
    tag: &str,
    display: &'static str,
) -> (
    crate::window_session::test_fixtures::TempDir,
    ProjectWindowSession,
) {
    let mut doc = canvas_document();
    for (id, x) in [(30u64, 0.0f32), (31, 30.0), (32, 60.0)] {
        add_shape(
            &mut doc,
            id,
            11,
            ShapeGeometry::Rectangle(RectangleParams {
                size: Size2::new(20., 20.).unwrap(),
                corner_radius: Default::default(),
            }),
            at(x, 0.0),
        );
    }
    let (dir, mut w) = open(tag, &doc);
    let layout = vec![Command::SetLayoutProperty(SetLayoutProperty {
        node: sid(11),
        longhand: LayoutLonghand::Display,
        value: LayoutValue::Keyword(display),
    })];
    crate::canvas::transform::commit(&mut w, layout, 0).unwrap();
    (dir, w)
}

#[test]
fn dropping_inside_reorders_with_one_move_node() {
    // 검증 기준(XII.10.24): gesture당 history 1건, OrderKey만 바뀜.
    let (_dir, mut w) = flow_window("flow-reorder", "flex");
    select(&mut w, &[30]);
    let s = scene(&w);
    let before = history_len(&w);
    let drag = FlowDrag::begin(&w, &s, [20., 30.], None).unwrap();
    // 끄는 동안: 삽입 표시와 preview, 문서·history 불변.
    let revision = w.revision();
    for step in 0..50 {
        let x = 20.0 + step as f32;
        let _ = drag.update(&w, &s, [x, 30.]);
    }
    let FlowUpdate::Inside {
        index,
        marker,
        preview,
    } = drag.update(&w, &s, [95., 30.])
    else {
        panic!("컨테이너 안")
    };
    assert_eq!(index, 2);
    assert_eq!(marker[2], 0.0, "세로 표시 선");
    assert_eq!(preview.last().unwrap().0, sid(30));
    assert_eq!(w.revision(), revision);
    assert_eq!(history_len(&w), before);
    // 놓기: 맨 뒤로.
    assert!(drag.commit(&mut w, &s, [95., 30.], RequestId(1)).unwrap());
    assert_eq!(
        model(&w).children_in_order(sid(11)),
        vec![sid(31), sid(32), sid(30)]
    );
    assert_eq!(history_len(&w), before + 1);
    // 같은 자리에 놓으면 기록하지 않습니다.
    select(&mut w, &[31]);
    let s = scene(&w);
    let after_select = history_len(&w);
    let drag = FlowDrag::begin(&w, &s, [20., 30.], None).unwrap();
    assert!(!drag.commit(&mut w, &s, [15., 30.], RequestId(2)).unwrap());
    assert_eq!(history_len(&w), after_select);
}

#[test]
fn dragging_out_reparents_to_a_free_frame_and_rejects_instances() {
    // 검증 기준(XII.10.24): 끌어내기·instance 거부 fixture.
    let (_dir, mut w) = flow_window("flow-out", "flex");
    select(&mut w, &[31]);
    let s = scene(&w);
    let drag = FlowDrag::begin(&w, &s, [50., 30.], None).unwrap();
    // Frame 12(200,50 60×40)는 자유 배치 Frame입니다.
    assert_eq!(
        drag.update(&w, &s, [220., 60.]),
        FlowUpdate::Outside(DropTarget::Parent(sid(12)))
    );
    // 인스턴스 투영 행 위: Frame 12 행을 인스턴스 투영 행으로 바꾼 scene.
    let mut instance = (*s).clone();
    for row in instance.frames.iter_mut().filter(|r| r.id == sid(12)) {
        row.instance_frame = true;
    }
    assert_eq!(
        drag.update(&w, &instance, [220., 60.]),
        FlowUpdate::Outside(DropTarget::Rejected("instance-slot"))
    );
    let before = history_len(&w);
    assert!(!drag
        .commit(&mut w, &instance, [220., 60.], RequestId(3))
        .unwrap());
    assert_eq!(history_len(&w), before);
    // 자유 Frame으로 끌어내기: 부모가 12로 바뀌고 history 1건.
    assert!(drag.commit(&mut w, &s, [220., 60.], RequestId(4)).unwrap());
    assert_eq!(history_len(&w), before + 1);
    assert!(model(&w).children_in_order(sid(12)).contains(&sid(31)));
    assert_eq!(model(&w).children_in_order(sid(11)), vec![sid(30), sid(32)]);
}

#[test]
fn grid_explicit_placement_children_are_rejected_and_skipped() {
    // 검증 기준(XII.10.24): C6 Grid 명시 배치 자식 drag가 고른 규칙(거부, 자동 형제 index에서 제외)과 일치.
    let (_dir, mut w) = flow_window("flow-grid-explicit", "grid");
    let explicit = vec![Command::SetLayoutProperty(SetLayoutProperty {
        node: sid(31),
        longhand: LayoutLonghand::GridColumnStart,
        value: LayoutValue::Line(3),
    })];
    crate::canvas::transform::commit(&mut w, explicit, 0).unwrap();
    select(&mut w, &[31]);
    let s = scene(&w);
    assert_eq!(
        FlowDrag::begin(&w, &s, [50., 30.], None).unwrap_err(),
        TransformError::Disabled("grid-explicit-placement")
    );
    // 자동 배치 30을 끌면 명시 배치 31은 index 계산에서 빠집니다: 자동 형제는 32 하나.
    select(&mut w, &[30]);
    let s = scene(&w);
    let drag = FlowDrag::begin(&w, &s, [20., 30.], None).unwrap();
    let FlowUpdate::Inside { index, .. } = drag.update(&w, &s, [95., 30.]) else {
        panic!("컨테이너 안")
    };
    assert_eq!(index, 1);
    assert!(drag.commit(&mut w, &s, [95., 30.], RequestId(5)).unwrap());
    let order = model(&w).children_in_order(sid(11));
    let position = |id| order.iter().position(|&n| n == sid(id)).unwrap();
    assert!(
        position(30) > position(32),
        "30이 자동 형제 32 뒤: {order:?}"
    );
}

#[test]
fn free_children_are_not_flow_drags() {
    let (_dir, mut w) = open("flow-free", &canvas_document());
    select(&mut w, &[13]);
    let s = scene(&w);
    assert_eq!(
        FlowDrag::begin(&w, &s, [40., 110.], None).unwrap_err(),
        TransformError::Disabled("not-flow-child")
    );
}

//! 메뉴 명령: 정렬·분배·행열 정리(XII.10.21), 순서·묶음(XII.10.23), 비례 확대(XII.10.12), 뒤집기(XII.10.44).
//!
//! 명령 하나가 CommandTransaction 하나(Undo 한 번)입니다. 계획은 `stellar_commands::arrange::plan`(CLI·MCP와 같은 한
//! 곳)이고, 이 module은 게시된 scene을 상자 원천([`SceneGeometry`], 회전 반영 world 경계·Text layout 결과)으로 넘깁니다.
//! 비활성 조합은 문서를 바꾸지 않고 [`TransformError::Disabled`]로 이유 코드를 돌려줍니다(도움말).

use stellar_commands::arrange::align::{model_style_set, Align, Distribute};
pub use stellar_commands::arrange::plan::ScaleOrigin;
use stellar_commands::arrange::plan::{self, Geometry, PlanError};
use stellar_commands::flip::FlipAxis;
use stellar_commands::scale::Skipped;
use stellar_core_types::ids::StableId;
use stellar_core_types::value::Transform2D;

use super::{selection_roots, FreshIds, TransformError};
use crate::canvas::CanvasScene;
use crate::window_session::{history_label, ProjectWindowSession};

/// 게시된 캔버스 scene을 상자 원천으로 씁니다.
pub struct SceneGeometry<'a> {
    scene: &'a CanvasScene,
}

impl<'a> SceneGeometry<'a> {
    /// scene 위의 상자 원천.
    pub fn new(scene: &'a CanvasScene) -> SceneGeometry<'a> {
        SceneGeometry { scene }
    }
}

impl Geometry for SceneGeometry<'_> {
    fn world_bounds(&self, node: StableId) -> Option<[f32; 4]> {
        super::node_geometry(self.scene, node).map(|g| g.world_bounds)
    }

    fn parent_world(&self, node: StableId) -> Option<Transform2D> {
        super::node_geometry(self.scene, node).map(|g| g.parent_world)
    }
}

impl From<PlanError> for TransformError {
    fn from(error: PlanError) -> TransformError {
        match error {
            PlanError::Empty | PlanError::NoGeometry(_) => TransformError::Stale,
            PlanError::Disabled(code) => TransformError::Disabled(code),
            PlanError::Place(place) => TransformError::Place(place),
        }
    }
}

/// 정렬 기준 선택.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignTo {
    /// 단일 선택은 부모, 다중 선택은 선택 경계.
    Auto,
    /// 다중 선택에서 primary selection을 기준 객체로 씁니다(없으면 선택 경계).
    Primary,
}

/// 정렬 6종 하나. 흐름 자식은 `align-self`·`justify-self`만 바꾸고, 주축 정렬이나 옛 layout 자식이 섞이면 비활성입니다.
pub fn align(
    window: &mut ProjectWindowSession,
    scene: &CanvasScene,
    align: Align,
    to: AlignTo,
) -> Result<bool, TransformError> {
    let (model, _root, roots) = selection_roots(window)?;
    let key = match to {
        AlignTo::Auto => None,
        AlignTo::Primary => window.view().selection().primary(),
    };
    let set = model_style_set(model);
    let mut ids = FreshIds::default();
    let commands = plan::align(
        model,
        &set,
        &SceneGeometry::new(scene),
        &roots,
        align,
        key,
        &mut || ids.fresh(),
    )?;
    ids.finish()?;
    super::commit(window, commands, history_label::MOVE)
}

/// 분배 2종 하나(3개 이상, 가장 바깥 두 객체 고정). 흐름 자식이 있으면 비활성입니다.
pub fn distribute(
    window: &mut ProjectWindowSession,
    scene: &CanvasScene,
    distribute: Distribute,
) -> Result<bool, TransformError> {
    let (model, _root, roots) = selection_roots(window)?;
    let set = model_style_set(model);
    let mut ids = FreshIds::default();
    let commands = plan::distribute(
        model,
        &set,
        &SceneGeometry::new(scene),
        &roots,
        distribute,
        &mut || ids.fresh(),
    )?;
    ids.finish()?;
    super::commit(window, commands, history_label::MOVE)
}

/// 행열 정리(Arrange in Rows and Columns). 흐름 컨테이너를 만들지 않고 위치만 바꿉니다.
pub fn arrange_grid(
    window: &mut ProjectWindowSession,
    scene: &CanvasScene,
    gap: f32,
) -> Result<bool, TransformError> {
    let (model, _root, roots) = selection_roots(window)?;
    let set = model_style_set(model);
    let mut ids = FreshIds::default();
    let commands = plan::arrange_grid(
        model,
        &set,
        &SceneGeometry::new(scene),
        &roots,
        gap,
        &mut || ids.fresh(),
    )?;
    ids.finish()?;
    super::commit(window, commands, history_label::MOVE)
}

/// Scale: 선택을 `factor`배 확대합니다. 쓰지 못한 길이 값(`border-width`·`font-size` 등)을 돌려줍니다.
pub fn scale(
    window: &mut ProjectWindowSession,
    scene: &CanvasScene,
    factor: f32,
    origin: ScaleOrigin,
) -> Result<Vec<Skipped>, TransformError> {
    let (model, _root, roots) = selection_roots(window)?;
    let mut ids = FreshIds::default();
    let plan = plan::scale(
        model,
        &SceneGeometry::new(scene),
        &roots,
        factor,
        origin,
        &mut || ids.fresh(),
    )?;
    ids.finish()?;
    super::commit(window, plan.commands, history_label::RESIZE)?;
    Ok(plan.skipped)
}

/// 뒤집기 명령을 받을 수 있는지(메뉴 활성 판정).
pub fn can_flip(window: &ProjectWindowSession) -> bool {
    selection_roots(window).is_ok_and(|(model, _, roots)| {
        !roots.is_empty()
            && roots
                .iter()
                .all(|&n| stellar_commands::flip::can_flip(model, n))
    })
}

/// Flip Horizontal·Vertical: 선택 world 경계 중심을 지나는 world 축의 거울상입니다(ADR-0105 결정 5).
pub fn flip(
    window: &mut ProjectWindowSession,
    scene: &CanvasScene,
    axis: FlipAxis,
) -> Result<bool, TransformError> {
    let (model, _root, roots) = selection_roots(window)?;
    let mut ids = FreshIds::default();
    let commands = plan::flip(model, &SceneGeometry::new(scene), &roots, axis, &mut || {
        ids.fresh()
    })?;
    ids.finish()?;
    super::commit(window, commands, history_label::CHANGE_PROPERTY)
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::*;
    use super::*;
    use crate::window_session::test_fixtures::sid;
    use stellar_commands::command::Command;
    use stellar_commands::style_defaults::SetLayoutProperty;
    use stellar_document::layout_record::{LayoutLonghand, LayoutValue};
    use stellar_document::records::NodePayload;
    use stellar_document::shape::{RectangleParams, ShapeGeometry};

    fn bounds(window: &ProjectWindowSession, node: u64) -> [f32; 4] {
        super::super::node_geometry(&scene(window), sid(node))
            .unwrap()
            .world_bounds
    }

    fn near(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
    }

    /// `canvas/align-distribute-matrix`: 정렬 6종 × (단일·다중·기준 객체), 분배 2종, 행열 정리.
    #[test]
    fn align_distribute_matrix() {
        // 단일 선택은 부모(root 0..400, 0..300) 기준.
        let single = [
            (Align::Left, [0.0, 100.0]),
            (Align::CenterX, [180.0, 100.0]),
            (Align::Right, [360.0, 100.0]),
            (Align::Top, [30.0, 0.0]),
            (Align::CenterY, [30.0, 140.0]),
            (Align::Bottom, [30.0, 280.0]),
        ];
        for (align, origin) in single {
            let (_dir, mut w) = open("align-single", &canvas_document());
            select(&mut w, &[13]);
            let s = scene(&w);
            let before = history_len(&w);
            assert!(super::align(&mut w, &s, align, AlignTo::Auto).unwrap());
            assert_eq!(history_len(&w), before + 1);
            assert!(
                near(bounds(&w, 13), [origin[0], origin[1], 40.0, 20.0]),
                "{align:?} {:?}",
                bounds(&w, 13)
            );
            // undo 1회 원복.
            assert!(w.undo().unwrap());
            assert!(near(bounds(&w, 13), [30.0, 100.0, 40.0, 20.0]));
        }
        // 다중 선택: 13(30..70, 100..120), 14(100..140, 100..120), 11(10..110, 20..70).
        let multi = [
            (Align::Left, [10.0, 10.0, 10.0]),
            (Align::Right, [100.0, 40.0, 100.0]),
            (Align::CenterX, [55.0, 25.0, 55.0]),
        ];
        for (align, xs) in multi {
            let (_dir, mut w) = open("align-multi", &canvas_document());
            select(&mut w, &[13, 14, 11]);
            let s = scene(&w);
            super::align(&mut w, &s, align, AlignTo::Auto).unwrap();
            let got = [bounds(&w, 13)[0], bounds(&w, 11)[0], bounds(&w, 14)[0]];
            assert!(
                got.iter().zip(xs).all(|(a, b)| (a - b).abs() < 1e-3),
                "{align:?} {got:?}"
            );
        }
        // 기준 객체(primary = 마지막 선택 11)는 움직이지 않습니다.
        let (_dir, mut w) = open("align-key", &canvas_document());
        select(&mut w, &[13, 14, 11]);
        let s = scene(&w);
        super::align(&mut w, &s, Align::Bottom, AlignTo::Primary).unwrap();
        assert!(near(bounds(&w, 11), [10.0, 20.0, 100.0, 50.0]));
        assert!((bounds(&w, 13)[1] - 50.0).abs() < 1e-3);
        // 분배: 13, 14, 15(150..180) 가로 → 첫·끝 고정, 가운데 14가 같은 간격.
        let (_dir, mut w) = open("distribute", &canvas_document());
        select(&mut w, &[13, 14, 15]);
        let s = scene(&w);
        assert!(distribute(&mut w, &s, Distribute::Horizontal).unwrap());
        let (a, b, c) = (bounds(&w, 13), bounds(&w, 14), bounds(&w, 15));
        assert!(((b[0] - (a[0] + a[2])) - (c[0] - (b[0] + b[2]))).abs() < 1e-3);
        assert!(near(a, [30.0, 100.0, 40.0, 20.0]) && (c[0] - 150.0).abs() < 1e-3);
        // 행열 정리: 같은 간격 격자, 흐름 컨테이너를 만들지 않음.
        let (_dir, mut w) = open("grid", &canvas_document());
        select(&mut w, &[13, 14, 15, 16]);
        let s = scene(&w);
        let nodes_before = model(&w).node_count();
        assert!(arrange_grid(&mut w, &s, 10.0).unwrap());
        assert_eq!(model(&w).node_count(), nodes_before);
        let (a, b) = (bounds(&w, 13), bounds(&w, 14));
        assert!((a[1] - b[1]).abs() < 1e-3 && (b[0] - (a[0] + 40.0) - 10.0).abs() < 1e-3);
    }

    #[test]
    fn rotated_nodes_align_by_world_bounds() {
        let (_dir, mut w) = open("align-rotated", &canvas_document());
        select(&mut w, &[13]);
        let s = scene(&w);
        let gesture = super::super::rotate::RotateGesture::begin(&w, &s, [100.0, 110.0]).unwrap();
        gesture.commit(&mut w, 45.0).unwrap();
        let s = scene(&w);
        super::align(&mut w, &s, Align::Left, AlignTo::Auto).unwrap();
        // 회전한 상자의 world 경계 왼쪽이 부모 왼쪽(0)에 닿습니다.
        assert!(bounds(&w, 13)[0].abs() < 1e-3, "{:?}", bounds(&w, 13));
    }

    fn flow_document(
        display: &'static str,
    ) -> (
        crate::window_session::test_fixtures::TempDir,
        ProjectWindowSession,
    ) {
        let mut doc = canvas_document();
        for (id, x) in [(30u64, 0.0f32), (31, 20.0), (32, 40.0)] {
            add_shape(
                &mut doc,
                id,
                11,
                ShapeGeometry::Rectangle(RectangleParams {
                    size: stellar_core_types::value::Size2::new(10., 10.).unwrap(),
                    corner_radius: Default::default(),
                }),
                at(x, 0.0),
            );
        }
        let (dir, mut w) = open("align-flow", &doc);
        let layout = vec![Command::SetLayoutProperty(SetLayoutProperty {
            node: sid(11),
            longhand: LayoutLonghand::Display,
            value: LayoutValue::Keyword(display),
        })];
        super::super::commit(&mut w, layout, 0).unwrap();
        (dir, w)
    }

    #[test]
    fn flex_cross_axis_and_grid_alignment_change_only_self_alignment() {
        let (_dir, mut w) = flow_document("flex");
        select(&mut w, &[30]);
        let s = scene(&w);
        let before = matrix(&w, 30);
        super::align(&mut w, &s, Align::CenterY, AlignTo::Auto).unwrap();
        assert_eq!(matrix(&w, 30), before);
        let record = model(&w).layout_record(sid(30)).unwrap().clone();
        assert_eq!(record.iter().count(), 1);
        assert_eq!(
            record.get(LayoutLonghand::AlignSelf),
            Some(&LayoutValue::Keyword("center"))
        );
        let (_dir, mut w) = flow_document("grid");
        select(&mut w, &[30]);
        let s = scene(&w);
        super::align(&mut w, &s, Align::Right, AlignTo::Auto).unwrap();
        let record = model(&w).layout_record(sid(30)).unwrap().clone();
        assert_eq!(
            record.iter().collect::<Vec<_>>(),
            vec![(LayoutLonghand::JustifySelf, &LayoutValue::Keyword("end"))]
        );
    }

    #[test]
    fn disabled_combinations_leave_the_revision_unchanged() {
        let (_dir, mut w) = flow_document("flex");
        let s = scene(&w);
        select(&mut w, &[30]);
        let revision = w.revision();
        assert_eq!(
            super::align(&mut w, &s, Align::Left, AlignTo::Auto),
            Err(TransformError::Disabled("justify-content"))
        );
        select(&mut w, &[30, 31, 32]);
        assert_eq!(
            distribute(&mut w, &s, Distribute::Horizontal),
            Err(TransformError::Disabled("flow-child"))
        );
        assert_eq!(
            arrange_grid(&mut w, &s, 10.0),
            Err(TransformError::Disabled("flow-child"))
        );
        assert_eq!(w.revision(), revision);
    }

    #[test]
    fn scale_two_hundred_percent_from_the_top_left_in_one_undo() {
        let (_dir, mut w) = open("scale", &canvas_document());
        select(&mut w, &[13, 14]);
        let s = scene(&w);
        let before = history_len(&w);
        let skipped = scale(&mut w, &s, 2.0, ScaleOrigin::TopLeft).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(history_len(&w), before + 1);
        // 경계 왼쪽 위 (30, 100) 고정: 13 (30, 100, 80×40), 14 (170, 100, 80×40).
        assert!(near(bounds(&w, 13), [30.0, 100.0, 80.0, 40.0]));
        assert!(near(bounds(&w, 14), [170.0, 100.0, 80.0, 40.0]));
        assert!(w.undo().unwrap());
        assert!(near(bounds(&w, 14), [100.0, 100.0, 40.0, 20.0]));
    }

    #[test]
    fn flip_keeps_the_selection_center_and_twice_restores_the_geometry() {
        for nodes in [vec![13u64], vec![13, 14, 11]] {
            let (_dir, mut w) = open("flip", &canvas_document());
            select(&mut w, &nodes);
            let union_of = |w: &ProjectWindowSession| {
                super::super::union(nodes.iter().map(|&n| bounds(w, n))).unwrap()
            };
            let start = union_of(&w);
            let original: Vec<_> = nodes.iter().map(|&n| matrix(&w, n)).collect();
            let s = scene(&w);
            assert!(can_flip(&w));
            flip(&mut w, &s, FlipAxis::Horizontal).unwrap();
            assert!(near(union_of(&w), start));
            let s = scene(&w);
            flip(&mut w, &s, FlipAxis::Horizontal).unwrap();
            for (n, m) in nodes.iter().zip(&original) {
                assert!(close(matrix(&w, *n), *m));
            }
            assert!(w.undo().unwrap());
        }
    }

    #[test]
    fn flipping_a_45_degree_node_mirrors_it_on_the_world_axis() {
        let (_dir, mut w) = open("flip-rotated", &canvas_document());
        select(&mut w, &[13]);
        let s = scene(&w);
        super::super::rotate::RotateGesture::begin(&w, &s, [100.0, 110.0])
            .unwrap()
            .commit(&mut w, 45.0)
            .unwrap();
        let before = bounds(&w, 13);
        let s = scene(&w);
        flip(&mut w, &s, FlipAxis::Horizontal).unwrap();
        // world 경계는 그대로, local x축은 135도(= -45도의 좌우 거울).
        assert!(near(bounds(&w, 13), before));
        let (a, b, c, d, _, _) = matrix(&w, 13);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        assert!(
            (a + h).abs() < 1e-4
                && (b - h).abs() < 1e-4
                && (c - h).abs() < 1e-4
                && (d - h).abs() < 1e-4
        );
    }
}

//! select-through와 겹친 레이어 목록 (XII.10.5, ADR-0104 결정 3.6).
//!
//! - pointer down이 현재 선택 bounds(선택 node world bounds의 합) 안이면 위에 다른 node가 있어도 선택을 그대로 둡니다.
//!   끌면 선택을 옮기고, 움직이지 않고 놓으면 그 점을 클릭 규칙(XII.10.3)으로 고릅니다([`PressPlan::KeepSelection`]).
//! - 밖이면서 고를 node가 있으면 누름에서 선택을 바꾸고(기록은 제스처 끝, XII.10.40) 끌면 그 선택을 옮깁니다
//!   ([`PressPlan::SelectAndDrag`]).
//! - 고를 node가 없으면(빈 캔버스, View root 빈 영역) 영역 선택입니다([`PressPlan::Marquee`], XII.10.6).
//! - 잠긴·숨긴 node는 캔버스 hit에서 통과합니다. 겹친 레이어 목록([`overlapping_layers`])은 잠긴 node까지 위에 그린
//!   것부터 돌려줍니다(문맥 메뉴의 Select Overlapping, XII.10.39).

use stellar_commands::selection::Selection;
use stellar_core_types::ids::StableId;
use stellar_core_types::value::Vec2;

use super::hit::{CanvasGeometry, PointStack};
use super::select::{click_pick, resolve_click, EditingScope, SelectTree, SelectionChange};
use super::tool::Modifiers;

/// Move 도구 pointer down의 처리 계획.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PressPlan {
    /// 선택 bounds 안: 선택을 그대로 둡니다. 끌면 선택을 옮기고, 움직이지 않고 놓으면 `on_click`을 적용합니다.
    KeepSelection {
        /// 움직이지 않고 놓았을 때의 클릭 결과.
        on_click: SelectionChange,
    },
    /// 누름에서 이 선택으로 바꾸고, 끌면 그 선택을 옮깁니다. 움직이지 않고 놓아도 이 선택입니다.
    SelectAndDrag(SelectionChange),
    /// 영역 선택을 시작합니다.
    Marquee,
}

/// pointer down의 계획. `world`는 누른 world 점입니다.
pub fn plan_press(
    tree: &SelectTree<'_>,
    scope: EditingScope,
    current: &Selection,
    stack: &PointStack,
    geometry: &CanvasGeometry,
    world: Vec2,
    modifiers: Modifiers,
) -> PressPlan {
    let on_click = resolve_click(tree, scope, current, stack, modifiers);
    let pick = click_pick(tree, scope, stack, modifiers.command);
    let inside = !current.is_empty()
        && (geometry
            .selection_bounds(tree, current.ids())
            .is_some_and(|bounds| bounds.contains_point(world))
            || pick.is_some_and(|pick| current.contains(pick.target)));
    if inside {
        return PressPlan::KeepSelection { on_click };
    }
    match pick {
        Some(_) if !on_click.selection.is_empty() => PressPlan::SelectAndDrag(on_click),
        _ => PressPlan::Marquee,
    }
}

/// 겹친 레이어 목록의 항목.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlapEntry {
    /// 문서 node.
    pub node: StableId,
    /// 잠겨 캔버스에서 고를 수 없음(목록에서는 고를 수 있음).
    pub locked: bool,
}

/// 커서 아래 겹친 node 목록(잠금 포함, 위에 그린 것부터, View root 제외).
pub fn overlapping_layers(tree: &SelectTree<'_>, stack: &PointStack) -> Vec<OverlapEntry> {
    stack
        .entries()
        .iter()
        .filter(|entry| entry.node != tree.root() && tree.contains(entry.node))
        .map(|entry| OverlapEntry {
            node: entry.node,
            locked: entry.locked,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::hit::point_stack;
    use crate::canvas::select_fixtures::{
        add_rectangle, frame_payload, scene_of, set_flags, sid, view_point,
    };
    use crate::window_session::test_fixtures::sample_document;
    use stellar_scene::narrow_phase::{LayoutBoxesOnly, NarrowPhaseOptions};

    /// root 10(800×600) 아래 Rectangle 40 (0,0)–(200,200), 41 (50,50)–(250,250), 잠긴 42 (100,100)–(300,300).
    fn overlap() -> stellar_document::model::DocumentModel {
        let mut model = sample_document();
        model.remove_leaf(sid(11)).unwrap();
        model.remove_leaf(sid(12)).unwrap();
        model
            .set_payload(sid(10), frame_payload(0., 0., 800., 600.))
            .unwrap();
        add_rectangle(&mut model, 40, 10, 0x10, (0., 0., 200., 200.));
        add_rectangle(&mut model, 41, 10, 0x20, (50., 50., 200., 200.));
        add_rectangle(&mut model, 42, 10, 0x30, (100., 100., 200., 200.));
        set_flags(&mut model, 42, true, true);
        model
    }

    #[test]
    fn three_layer_overlap_drag_click_and_list() {
        // 검증 기준(XII.10.5): 3단 겹침 fixture에서 drag·click·목록 결과, 잠금 node는 클릭으로 선택되지 않고 목록에 존재.
        let model = overlap();
        let tree = SelectTree::new(&model, sid(10));
        let scene = scene_of(&model, sid(10));
        let geometry = CanvasGeometry::from_snapshot(&scene.snapshot);
        let top = EditingScope::top(&tree);
        let at = |x: f32, y: f32| {
            point_stack(
                &scene.index,
                &scene.viewport,
                view_point(x, y),
                &NarrowPhaseOptions::EXACT,
                &LayoutBoxesOnly,
            )
        };
        let selected = Selection::new([sid(40)], Some(sid(40)));
        let plan = |x: f32, y: f32, current: &Selection| {
            plan_press(
                &tree,
                top,
                current,
                &at(x, y),
                &geometry,
                view_point(x, y),
                Modifiers::NONE,
            )
        };
        // (150,150): 42(잠김) > 41 > 40. 선택 40의 bounds 안이라 선택을 그대로 두고, 놓으면 클릭 규칙으로 41.
        let PressPlan::KeepSelection { on_click } = plan(150., 150., &selected) else {
            panic!("선택 bounds 안은 선택 유지");
        };
        assert_eq!(on_click.selection.ids(), &[sid(41)]);
        // (220,220): 선택 bounds 밖, 맞는 node는 41 → 누름에서 41 선택 후 끌기.
        let PressPlan::SelectAndDrag(change) = plan(220., 220., &selected) else {
            panic!("선택 밖 node는 즉시 선택 후 끌기");
        };
        assert_eq!(change.selection.ids(), &[sid(41)]);
        // (280,280): 잠긴 42만 있음 → 클릭이 통과해 root 빈 영역 → 마퀴.
        assert_eq!(plan(280., 280., &selected), PressPlan::Marquee);
        assert_eq!(plan(700., 500., &Selection::empty()), PressPlan::Marquee);
        // 선택이 없으면 맨 위 잠기지 않은 node를 즉시 선택.
        let PressPlan::SelectAndDrag(change) = plan(150., 150., &Selection::empty()) else {
            panic!("즉시 선택");
        };
        assert_eq!(change.selection.ids(), &[sid(41)]);
        // 겹친 레이어 목록: 잠금 포함, 위에 그린 것부터, root 제외.
        let list: Vec<(StableId, bool)> = overlapping_layers(&tree, &at(150., 150.))
            .into_iter()
            .map(|e| (e.node, e.locked))
            .collect();
        assert_eq!(
            list,
            vec![(sid(42), true), (sid(41), false), (sid(40), false)]
        );
        // 잠긴 42는 어떤 클릭으로도 선택되지 않습니다.
        for modifiers in [Modifiers::NONE, Modifiers::COMMAND, Modifiers::SHIFT] {
            let change = resolve_click(&tree, top, &Selection::empty(), &at(280., 280.), modifiers);
            assert!(!change.selection.contains(sid(42)), "{modifiers:?}");
        }
    }
}

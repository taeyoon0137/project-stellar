//! 선택 엔진의 hit 입력: 커서 아래 node 목록과 node world 상자 (XII.10.3, XII.10.5, ADR-0104 결정 3.2).
//!
//! - [`PointStack`]: 커서 아래 node를 위에 그린 것부터 담습니다. 숨김 node는 없고, 잠긴 node(scene
//!   `pointer-events: none`)는 [`StackEntry::locked`]로 표시해 겹친 레이어 목록에만 씁니다. 잠기지 않은 node는
//!   narrow-phase hit test(XI.3.4) 결과 그대로이고, 잠긴 node는 같은 공간 인덱스 후보를 local hit bounds와 clip으로
//!   판정합니다(broad-phase 정밀도). 같은 문서 node의 scene node가 여럿이면(인스턴스 펼침) 가장 위 하나만 남깁니다.
//! - [`CanvasGeometry`]: 문서 node별 world 상자(scene node의 layout 상자와 world transform). Fragment처럼 scene node가
//!   없는 node의 bounds는 자손 상자의 합입니다.

use std::collections::{BTreeMap, BTreeSet};

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{Rect, Transform2D, Vec2};
use stellar_scene::bounds::Aabb;
use stellar_scene::hit_test::clip_stack_contains;
use stellar_scene::narrow_phase::{hit_test_view_narrow, NarrowPhaseOptions, TextHitGeometry};
use stellar_scene::node::{PointerEvents, Visibility};
use stellar_scene::snapshot::SceneSnapshot;
use stellar_scene::spatial::SceneSpatialIndex;
use stellar_scene::visibility::Viewport;

use super::select::SelectTree;

/// 커서 아래 node 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackEntry {
    /// 문서 node.
    pub node: StableId,
    /// 자신이나 조상이 잠겨 캔버스 클릭이 통과하는 node.
    pub locked: bool,
}

/// 커서 아래 node 목록(위에 그린 것부터, 숨김 제외).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PointStack {
    entries: Vec<StackEntry>,
}

impl PointStack {
    /// 위에 그린 것부터 정렬된 항목으로 만듭니다. 같은 node가 다시 나오면 처음 것만 남깁니다.
    pub fn new(entries: impl IntoIterator<Item = StackEntry>) -> PointStack {
        let mut seen = BTreeSet::new();
        PointStack {
            entries: entries
                .into_iter()
                .filter(|entry| seen.insert(entry.node))
                .collect(),
        }
    }

    /// 빈 목록(아무것도 맞지 않음).
    pub fn empty() -> PointStack {
        PointStack::default()
    }

    /// 항목(위에 그린 것부터).
    pub fn entries(&self) -> &[StackEntry] {
        &self.entries
    }

    /// 캔버스 클릭이 맞는 가장 위 node(잠긴 node는 통과).
    pub fn leaf(&self) -> Option<StableId> {
        self.entries
            .iter()
            .find(|entry| !entry.locked)
            .map(|entry| entry.node)
    }

    /// 아무것도 없으면 참입니다.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// view 점 아래의 [`PointStack`]. `options.tolerance`는 view 단위입니다(XI.3.4와 같음).
pub fn point_stack(
    index: &SceneSpatialIndex,
    viewport: &Viewport,
    view_point: Vec2,
    options: &NarrowPhaseOptions,
    text: &dyn TextHitGeometry,
) -> PointStack {
    let mut entries: Vec<(u64, StackEntry)> =
        hit_test_view_narrow(index, viewport, view_point, options, text)
            .hits
            .iter()
            .map(|hit| {
                (
                    hit.z.0,
                    StackEntry {
                        node: hit.source,
                        locked: false,
                    },
                )
            })
            .collect();
    if let Some(world) = viewport
        .view()
        .inverse()
        .and_then(|inverse| inverse.apply(view_point).ok())
    {
        let mut candidates = Vec::new();
        index.query_point(world, &mut candidates);
        for z in candidates {
            let Some(node) = index.node(z) else {
                continue;
            };
            let hit = node.hit();
            if hit.visibility == Visibility::Hidden || hit.pointer_events != PointerEvents::None {
                continue;
            }
            let inside = node
                .world_transform()
                .inverse()
                .and_then(|inverse| inverse.apply(world).ok())
                .is_some_and(|local| node.local_hit_bounds().contains_point(local));
            if inside && clip_stack_contains(node.clip(), world) {
                entries.push((
                    z.0,
                    StackEntry {
                        node: node.source(),
                        locked: true,
                    },
                ));
            }
        }
    }
    entries.sort_by_key(|(z, _)| std::cmp::Reverse(*z));
    PointStack::new(entries.into_iter().map(|(_, entry)| entry))
}

/// scene node 하나의 world 상자.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeBox {
    /// local → world.
    pub world: Transform2D,
    /// local layout 상자.
    pub rect: Rect,
}

impl NodeBox {
    /// world 꼭짓점 `[top_left, top_right, bottom_right, bottom_left]`. 비유한이면 `None`.
    pub fn quad(&self) -> Option<[Vec2; 4]> {
        let (x, y) = (self.rect.origin().x(), self.rect.origin().y());
        let (w, h) = (self.rect.size().width(), self.rect.size().height());
        let corner = |cx: f32, cy: f32| {
            Vec2::new(cx, cy)
                .ok()
                .and_then(|p| self.world.apply(p).ok())
        };
        Some([
            corner(x, y)?,
            corner(x + w, y)?,
            corner(x + w, y + h)?,
            corner(x, y + h)?,
        ])
    }

    /// world 축 정렬 bounds.
    pub fn bounds(&self) -> Option<Aabb> {
        Aabb::from_rect(self.rect).transformed(&self.world)
    }
}

/// 문서 node별 world 상자.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CanvasGeometry {
    boxes: BTreeMap<StableId, Vec<NodeBox>>,
}

impl CanvasGeometry {
    /// 빈 표.
    pub fn new() -> CanvasGeometry {
        CanvasGeometry::default()
    }

    /// scene snapshot의 모든 node 상자(pre-order). 같은 source의 상자는 함께 모읍니다.
    pub fn from_snapshot(snapshot: &SceneSnapshot) -> CanvasGeometry {
        let mut geometry = CanvasGeometry::new();
        for node in snapshot.iter_preorder() {
            geometry.insert(
                node.source(),
                NodeBox {
                    world: node.world_transform(),
                    rect: node.layout_rect(),
                },
            );
        }
        geometry
    }

    /// 상자 하나를 더합니다.
    pub fn insert(&mut self, node: StableId, node_box: NodeBox) {
        self.boxes.entry(node).or_default().push(node_box);
    }

    /// node 자신의 상자들(scene node가 없으면 빈 목록).
    pub fn boxes(&self, node: StableId) -> &[NodeBox] {
        self.boxes.get(&node).map_or(&[], Vec::as_slice)
    }

    /// node의 world 사각형. scene 상자가 하나면 그 상자(회전 포함), 아니면 bounds의 사각형입니다.
    pub fn quad(&self, tree: &SelectTree<'_>, node: StableId) -> Option<[Vec2; 4]> {
        match self.boxes(node) {
            [single] => single.quad(),
            _ => {
                let b = self.bounds(tree, node)?;
                let p = |x: f32, y: f32| Vec2::new(x, y).ok();
                Some([
                    p(b.min_x(), b.min_y())?,
                    p(b.max_x(), b.min_y())?,
                    p(b.max_x(), b.max_y())?,
                    p(b.min_x(), b.max_y())?,
                ])
            }
        }
    }

    /// node의 world bounds. 자기 상자가 없으면(Fragment) 자손 bounds의 합입니다.
    pub fn bounds(&self, tree: &SelectTree<'_>, node: StableId) -> Option<Aabb> {
        let own = self
            .boxes(node)
            .iter()
            .filter_map(NodeBox::bounds)
            .reduce(|a, b| a.union(&b));
        if own.is_some() {
            return own;
        }
        tree.children(node)
            .into_iter()
            .filter_map(|child| self.bounds(tree, child))
            .reduce(|a, b| a.union(&b))
    }

    /// 선택 bounds(선택 node world bounds의 합).
    pub fn selection_bounds(&self, tree: &SelectTree<'_>, ids: &[StableId]) -> Option<Aabb> {
        ids.iter()
            .filter_map(|id| self.bounds(tree, *id))
            .reduce(|a, b| a.union(&b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::select_fixtures::{
        add_frame, add_rectangle, nested, scene_of, set_flags, sid, view_point,
    };
    use stellar_scene::narrow_phase::LayoutBoxesOnly;

    #[test]
    fn point_stack_lists_locked_nodes_topmost_first_and_skips_hidden() {
        let fixture = nested();
        let scene = scene_of(&fixture.model, fixture.root);
        // (130, 130): Root 10 > Frame 11 > Rectangle 13(그 위 잠긴 Rectangle 15, 숨긴 Rectangle 16).
        let stack = point_stack(
            &scene.index,
            &scene.viewport,
            view_point(130.0, 130.0),
            &NarrowPhaseOptions::EXACT,
            &LayoutBoxesOnly,
        );
        let ids: Vec<(StableId, bool)> =
            stack.entries().iter().map(|e| (e.node, e.locked)).collect();
        assert_eq!(
            ids,
            vec![
                (sid(15), true),
                (sid(13), false),
                (sid(11), false),
                (sid(10), false)
            ]
        );
        assert_eq!(stack.leaf(), Some(sid(13)), "잠긴 node는 클릭이 통과");
        let geometry = CanvasGeometry::from_snapshot(&scene.snapshot);
        let tree = SelectTree::new(&fixture.model, fixture.root);
        // Fragment 12는 scene node가 없어 자손 bounds의 합입니다.
        let fragment = geometry.bounds(&tree, sid(12)).unwrap();
        assert_eq!(
            (
                fragment.min_x(),
                fragment.min_y(),
                fragment.max_x(),
                fragment.max_y()
            ),
            (300.0, 20.0, 420.0, 140.0)
        );
        assert!(geometry.boxes(sid(12)).is_empty());
    }

    #[test]
    fn children_of_a_locked_container_pass_through_like_the_container() {
        // 잠긴 Frame 22의 자식 23도 잠김으로 표시되어 클릭이 아래 13까지 통과합니다.
        let mut fixture = nested();
        add_frame(&mut fixture.model, 22, 11, 0x50, (10., 10., 100., 100.));
        set_flags(&mut fixture.model, 22, true, true);
        add_rectangle(&mut fixture.model, 23, 22, 0x10, (0., 0., 80., 80.));
        let scene = scene_of(&fixture.model, fixture.root);
        let tree = SelectTree::new(&fixture.model, fixture.root);
        let stack = point_stack(
            &scene.index,
            &scene.viewport,
            view_point(130.0, 130.0),
            &NarrowPhaseOptions::EXACT,
            &LayoutBoxesOnly,
        );
        let ids: Vec<(StableId, bool)> =
            stack.entries().iter().map(|e| (e.node, e.locked)).collect();
        assert_eq!(
            ids,
            vec![
                (sid(23), true),
                (sid(22), true),
                (sid(15), true),
                (sid(13), false),
                (sid(11), false),
                (sid(10), false)
            ]
        );
        assert_eq!(stack.leaf(), Some(sid(13)), "잠긴 컨테이너의 자식도 통과");
        let change = crate::canvas::select::resolve_click(
            &tree,
            crate::canvas::select::EditingScope::at(&tree, sid(11)),
            &stellar_commands::selection::Selection::empty(),
            &stack,
            crate::canvas::tool::Modifiers::NONE,
        );
        assert_eq!(change.selection.ids(), &[sid(13)]);
    }
}

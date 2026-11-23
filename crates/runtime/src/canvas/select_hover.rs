//! hover 미리보기 (XII.10.7, ADR-0104 결정 3.8).
//!
//! hover 대상은 같은 입력으로 지금 클릭하면 고를 node입니다. 클릭 규칙의 고를 node 계산([`click_pick`])을 그대로 쓰므로
//! 두 결과가 갈라질 수 없습니다. ⌘는 깊은 선택이고, Shift·Option은 고를 node를 바꾸지 않습니다(토글·primary 지정은 클릭
//! 결과의 선택 집합만 바꿈).
//!
//! 캔버스 hover와 사이드바 레이어 행 hover는 같은 [`HoverState`] 하나를 바꿉니다. hover 윤곽([`hover_outline`])은 world
//! 사각형 transient 값이며 history·저장·scene snapshot에 들어가지 않습니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::value::Vec2;

use super::hit::{CanvasGeometry, PointStack};
use super::select::{click_pick, EditingScope, SelectTree};
use super::tool::Modifiers;

/// hover를 정한 곳.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HoverSource {
    /// 캔버스 pointer.
    Canvas,
    /// 사이드바 레이어 행.
    Sidebar,
}

/// 창의 hover 상태(캔버스·사이드바 공유). history·저장 대상이 아닙니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HoverState {
    hovered: Option<(StableId, HoverSource)>,
}

impl HoverState {
    /// hover 없음.
    pub const fn new() -> HoverState {
        HoverState { hovered: None }
    }

    /// hover node.
    pub fn node(&self) -> Option<StableId> {
        self.hovered.map(|(node, _)| node)
    }

    /// hover를 정한 곳.
    pub fn source(&self) -> Option<HoverSource> {
        self.hovered.map(|(_, source)| source)
    }

    fn set(&mut self, next: Option<(StableId, HoverSource)>) -> bool {
        let changed = self.hovered != next;
        self.hovered = next;
        changed
    }

    /// 캔버스 pointer 이동: 클릭이 고를 node를 hover로 둡니다. 바뀌면 참입니다.
    pub fn update_canvas(
        &mut self,
        tree: &SelectTree<'_>,
        scope: EditingScope,
        stack: &PointStack,
        modifiers: Modifiers,
    ) -> bool {
        let next = hover_target(tree, scope, stack, modifiers).map(|n| (n, HoverSource::Canvas));
        self.set(next)
    }

    /// 사이드바 행 hover. View root 아래 node만 받습니다. 바뀌면 참입니다.
    pub fn update_sidebar(&mut self, tree: &SelectTree<'_>, row: Option<StableId>) -> bool {
        let next = row
            .filter(|node| tree.contains(*node))
            .map(|n| (n, HoverSource::Sidebar));
        self.set(next)
    }

    /// pointer가 캔버스·사이드바를 떠남. 바뀌면 참입니다.
    pub fn clear(&mut self) -> bool {
        self.set(None)
    }
}

/// 지금 클릭하면 고를 node(누른 수식키 반영).
pub fn hover_target(
    tree: &SelectTree<'_>,
    scope: EditingScope,
    stack: &PointStack,
    modifiers: Modifiers,
) -> Option<StableId> {
    click_pick(tree, scope, stack, modifiers.command).map(|pick| pick.target)
}

/// hover 윤곽: node의 world 사각형(회전 포함, Fragment는 자손 bounds).
pub fn hover_outline(
    tree: &SelectTree<'_>,
    geometry: &CanvasGeometry,
    hover: &HoverState,
) -> Option<[Vec2; 4]> {
    geometry.quad(tree, hover.node()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::hit::{point_stack, StackEntry};
    use crate::canvas::select::resolve_click;
    use crate::canvas::select_fixtures::{
        add_frame, frame_payload, nested, record, save_bytes, scene_of, sid, view_point,
    };
    use crate::window_session::test_fixtures::{sample_document, window};
    use stellar_commands::selection::Selection;
    use stellar_commands::state::DocumentState;
    use stellar_document::fragment::FragmentPayload;
    use stellar_document::model::DocumentModel;
    use stellar_document::records::NodePayload;
    use stellar_history::window::WindowHistory;
    use stellar_scene::narrow_phase::{LayoutBoxesOnly, NarrowPhaseOptions};

    /// 결정론적 난수(SplitMix64).
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n.max(1)
        }
    }

    /// root 10 아래 무작위 tree(Frame·Fragment, 일부 잠김·숨김). node id는 100부터.
    fn random_tree(rng: &mut Rng, count: u64) -> (DocumentModel, Vec<StableId>) {
        let mut model = sample_document();
        model.remove_leaf(sid(11)).unwrap();
        model.remove_leaf(sid(12)).unwrap();
        let mut numbers = vec![10u64];
        for i in 0..count {
            let n = 100 + i;
            let parent = numbers[rng.below(numbers.len() as u64) as usize];
            let key = (i % 250) as u8 + 1;
            let payload = if rng.below(4) == 0 {
                NodePayload::Fragment(FragmentPayload)
            } else {
                frame_payload(0., 0., 10., 10.)
            };
            let mut node = record(n, parent, key, payload);
            node.order_key =
                stellar_core_types::order_key::OrderKey::from_bytes(vec![key, (i / 250) as u8 + 1])
                    .unwrap();
            match rng.below(10) {
                0 => node.locked = true,
                1 => node.visible = false,
                _ => {}
            }
            model.insert_node(node).unwrap();
            numbers.push(n);
        }
        (model, numbers.into_iter().map(sid).collect())
    }

    #[test]
    fn hover_target_matches_the_click_result_for_every_modifier_combination() {
        // 검증 기준(XII.10.7): hover 대상과 클릭 선택 결과가 수식키 조합마다 같다는 property test.
        let mut rng = Rng(0x5EED_0007);
        for _ in 0..40 {
            let (model, nodes) = random_tree(&mut rng, 60);
            let tree = SelectTree::new(&model, sid(10));
            for _ in 0..40 {
                // 무작위 leaf의 조상 사슬(위에 그린 것부터: leaf → root)과 잠금 표시.
                let leaf = nodes[rng.below(nodes.len() as u64) as usize];
                let mut chain = tree.ancestry(leaf).unwrap();
                if rng.below(3) == 0 {
                    chain.insert(0, nodes[rng.below(nodes.len() as u64) as usize]);
                }
                let stack = PointStack::new(chain.into_iter().map(|node| StackEntry {
                    node,
                    locked:
                        stellar_commands::lock::NodeLock::of(&model, node).is_some_and(|l| l.any()),
                }));
                let scope_node = nodes[rng.below(nodes.len() as u64) as usize];
                let scope = EditingScope::at(&tree, scope_node);
                let current = Selection::new(
                    (0..rng.below(4))
                        .map(|_| nodes[rng.below(nodes.len() as u64) as usize])
                        .filter(|n| tree.selectable(*n)),
                    None,
                );
                for bits in 0..16u8 {
                    let modifiers = Modifiers {
                        shift: bits & 1 != 0,
                        option: bits & 2 != 0,
                        command: bits & 4 != 0,
                        control: bits & 8 != 0,
                    };
                    let hover = hover_target(&tree, scope, &stack, modifiers);
                    let click = resolve_click(&tree, scope, &current, &stack, modifiers);
                    match hover {
                        None if modifiers.shift => assert_eq!(click.selection, current),
                        None => assert!(click.selection.is_empty()),
                        Some(target)
                            if modifiers.option
                                && current.len() > 1
                                && current.contains(target) =>
                        {
                            assert_eq!(click.selection.primary(), Some(target));
                        }
                        Some(target) if modifiers.shift => {
                            assert_eq!(click.selection.contains(target), !current.contains(target));
                        }
                        Some(target) => {
                            assert_eq!(click.selection.ids(), &[target], "{modifiers:?}");
                        }
                    }
                    if let Some(target) = hover {
                        assert!(tree.selectable(target), "hover는 선택 가능한 node만");
                    }
                }
            }
        }
    }

    #[test]
    fn canvas_and_sidebar_share_one_hover_without_history_or_save() {
        let fixture = nested();
        let root = fixture.root;
        let doc = DocumentState::new(fixture.model);
        let before = save_bytes(&doc);
        let history = WindowHistory::new(window(7));
        let tree = SelectTree::new(doc.model(), root);
        let scene = scene_of(doc.model(), root);
        let geometry = CanvasGeometry::from_snapshot(&scene.snapshot);
        let scene_hash = scene.snapshot.semantic_hash();
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
        let mut hover = HoverState::new();
        assert!(hover.update_canvas(&tree, top, &at(220., 220.), Modifiers::NONE));
        assert_eq!(
            (hover.node(), hover.source()),
            (Some(sid(11)), Some(HoverSource::Canvas))
        );
        assert!(
            !hover.update_canvas(&tree, top, &at(221., 221.), Modifiers::NONE),
            "같으면 바뀌지 않음"
        );
        // ⌘를 누르면 깊은 node.
        assert!(hover.update_canvas(&tree, top, &at(220., 220.), Modifiers::COMMAND));
        assert_eq!(hover.node(), Some(sid(17)));
        let outline = hover_outline(&tree, &geometry, &hover).unwrap();
        assert_eq!(
            (
                outline[0].x(),
                outline[0].y(),
                outline[2].x(),
                outline[2].y()
            ),
            (210., 210., 240., 240.)
        );
        // 사이드바 행 hover가 같은 상태를 바꿉니다.
        assert!(hover.update_sidebar(&tree, Some(sid(12))));
        assert_eq!(
            (hover.node(), hover.source()),
            (Some(sid(12)), Some(HoverSource::Sidebar))
        );
        let fragment = hover_outline(&tree, &geometry, &hover).unwrap();
        assert_eq!((fragment[0].x(), fragment[2].x()), (300., 420.));
        // View root 밖 행은 hover가 아닙니다. 떠나면 지웁니다.
        assert!(hover.update_sidebar(&tree, Some(sid(999))));
        assert_eq!(hover.node(), None);
        assert!(hover.update_sidebar(&tree, Some(sid(13))));
        assert!(hover.clear());
        assert!(!hover.clear());
        assert_eq!(hover.node(), None);
        // hover는 history·저장·scene과 무관합니다.
        assert_eq!(history.history().len(), 0);
        assert_eq!(save_bytes(&doc), before);
        assert_eq!(scene.snapshot.semantic_hash(), scene_hash);
    }

    #[test]
    #[ignore = "release 빌드에서 2만 가시 node hover 시간을 측정"]
    fn measure_hover_20k() {
        // 검증 기준(XII.10.7): 2만 가시 node hover p99 8ms 이하(목표 5ms). hit test와 대상 계산을 함께 잽니다.
        let mut model = sample_document();
        model.remove_leaf(sid(11)).unwrap();
        model.remove_leaf(sid(12)).unwrap();
        model
            .set_payload(sid(10), frame_payload(0., 0., 20_000., 10_000.))
            .unwrap();
        let keys = stellar_core_types::order_key::OrderKey::evenly_spaced(200);
        // 200 Frame × 자식 99 = 2만 node.
        for g in 0..200u64 {
            let n = 1_000 + g;
            let (x, y) = ((g % 20) as f32 * 1_000., (g / 20) as f32 * 1_000.);
            add_frame(&mut model, n, 10, 0x10, (x, y, 990., 990.));
            let mut frame = model.record(sid(n)).unwrap();
            frame.order_key = keys[g as usize].clone();
            model.remove_leaf(sid(n)).unwrap();
            model.insert_node(frame).unwrap();
            for c in 0..99u64 {
                let child = 100_000 + g * 100 + c;
                let (cx, cy) = ((c % 10) as f32 * 99., (c / 10) as f32 * 99.);
                add_frame(&mut model, child, n, 0x10, (cx, cy, 90., 90.));
                let mut record = model.record(sid(child)).unwrap();
                record.order_key = keys[c as usize].clone();
                model.remove_leaf(sid(child)).unwrap();
                model.insert_node(record).unwrap();
            }
        }
        assert_eq!(model.node_count(), 1 + 1 + 200 + 200 * 99);
        let scene = scene_of(&model, sid(10));
        let tree = SelectTree::new(&model, sid(10));
        let top = EditingScope::top(&tree);
        let mut hover = HoverState::new();
        let mut rng = Rng(42);
        let mut samples = Vec::with_capacity(1_000);
        for _ in 0..1_000 {
            let point = view_point(rng.below(20_000) as f32, rng.below(10_000) as f32);
            let start = std::time::Instant::now();
            let stack = point_stack(
                &scene.index,
                &scene.viewport,
                point,
                &NarrowPhaseOptions::EXACT,
                &LayoutBoxesOnly,
            );
            hover.update_canvas(&tree, top, &stack, Modifiers::NONE);
            samples.push(start.elapsed());
        }
        samples.sort_unstable();
        println!(
            "hover, 20,000 nodes, 1,000 samples: p50 {:?}, p99 {:?}, max {:?}",
            samples[500], samples[989], samples[999]
        );
        assert!(samples[989] <= std::time::Duration::from_millis(8));
    }
}

//! 영역 선택(마퀴) (XII.10.6, ADR-0104 결정 3.7).
//!
//! - 시작: 빈 캔버스나 View root 빈 영역의 누름([`super::select_through::PressPlan::Marquee`]).
//! - 후보: 시작 때 한 번 만듭니다. 기본은 편집 범위의 선택 가능 직계 자식, ⌘는 범위 아래 선택 가능 자식이 없는 가장 깊은
//!   node입니다. 후보마다 world 사각형과 bounds를 미리 계산해 update는 후보 수에 선형입니다.
//! - 판정: 기본은 영역과 겹침(회전 node는 world 사각형으로 분리축 판정), Option은 완전 포함입니다.
//! - Shift는 시작 전 선택에 더하기(`AreaSelectionMode::Add`), 아니면 교체입니다.
//! - 진행 중 결과는 `WindowViewState` 영역 선택 preview이고(history 없음), pointer up에서 `SetSelection` 하나만 만듭니다.
//!   취소는 아무 event도 만들지 않습니다.

use stellar_commands::selection::{
    AreaSelectionError, AreaSelectionMode, SetSelection, WindowViewState,
};
use stellar_core_types::ids::StableId;
use stellar_core_types::value::Vec2;
use stellar_scene::bounds::Aabb;

use super::hit::CanvasGeometry;
use super::select::{EditingScope, SelectTree};
use super::tool::Modifiers;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Candidate {
    node: StableId,
    quad: [Vec2; 4],
    bounds: Aabb,
}

/// 진행 중인 영역 선택.
#[derive(Debug, Clone, PartialEq)]
pub struct Marquee {
    candidates: Vec<Candidate>,
    contain: bool,
    start: Vec2,
    scope: EditingScope,
    /// 마지막으로 preview에 넘긴 hit. 같으면 preview를 다시 만들지 않습니다.
    last: Option<Vec<StableId>>,
}

/// 영역 선택을 끝낸 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarqueeResult {
    /// 기록할 선택 event(확정 선택과 같으면 없음).
    pub event: Option<SetSelection>,
    /// 새 편집 범위(선택에 맞춤).
    pub scope: EditingScope,
}

impl Marquee {
    /// pointer down(world `start`): 후보를 만들고 `view`의 영역 선택 preview를 시작합니다.
    pub fn begin(
        tree: &SelectTree<'_>,
        scope: EditingScope,
        geometry: &CanvasGeometry,
        view: &mut WindowViewState,
        start: Vec2,
        modifiers: Modifiers,
    ) -> Result<Marquee, AreaSelectionError> {
        let mode = if modifiers.shift {
            AreaSelectionMode::Add
        } else {
            AreaSelectionMode::Replace
        };
        view.begin_area_selection(mode)?;
        let scope_node = scope.node(tree);
        let mut nodes = Vec::new();
        if modifiers.command {
            deepest(tree, scope_node, &mut nodes);
        } else {
            nodes = tree.selectable_children(scope_node);
        }
        let candidates = nodes
            .into_iter()
            .filter_map(|node| {
                let quad = geometry.quad(tree, node)?;
                let bounds = Aabb::from_points(quad)?;
                Some(Candidate { node, quad, bounds })
            })
            .collect();
        Ok(Marquee {
            candidates,
            contain: modifiers.option,
            start,
            scope: EditingScope::at(tree, scope_node),
            last: None,
        })
    }

    /// 후보 수.
    pub fn candidate_count(&self) -> usize {
        self.candidates.len()
    }

    /// 시작점과 `to`가 만드는 영역에 걸린 후보(후보 순서).
    pub fn hits(&self, to: Vec2) -> Vec<StableId> {
        let Some(area) = Aabb::new(
            self.start.x().min(to.x()),
            self.start.y().min(to.y()),
            self.start.x().max(to.x()),
            self.start.y().max(to.y()),
        ) else {
            return Vec::new();
        };
        self.candidates
            .iter()
            .filter(|candidate| {
                if self.contain {
                    contains(&area, &candidate.bounds)
                } else {
                    candidate.bounds.intersects(&area) && quad_overlaps(&candidate.quad, &area)
                }
            })
            .map(|candidate| candidate.node)
            .collect()
    }

    /// pointer move: preview만 바꿉니다(history 없음). hit가 직전과 같으면 preview를 그대로 둡니다.
    pub fn update(
        &mut self,
        view: &mut WindowViewState,
        to: Vec2,
    ) -> Result<(), AreaSelectionError> {
        if !view.area_selection_active() {
            return Err(AreaSelectionError::NotActive);
        }
        let hits = self.hits(to);
        if self.last.as_ref() == Some(&hits) {
            return Ok(());
        }
        view.update_area_selection(hits.iter().copied())?;
        self.last = Some(hits);
        Ok(())
    }

    /// pointer up: preview를 끝내고 기록할 event 하나와 새 범위를 돌려줍니다.
    pub fn finish(
        self,
        tree: &SelectTree<'_>,
        view: &mut WindowViewState,
    ) -> Result<MarqueeResult, AreaSelectionError> {
        let event = view.finish_area_selection()?;
        let selection = event
            .as_ref()
            .map_or_else(|| view.selection().clone(), |e| e.selection.clone());
        Ok(MarqueeResult {
            scope: self.scope.refit(tree, &selection),
            event,
        })
    }

    /// 취소: preview를 버리고 event를 만들지 않습니다.
    pub fn cancel(self, view: &mut WindowViewState) -> Result<(), AreaSelectionError> {
        view.cancel_area_selection()
    }
}

/// `node` 아래 선택 가능 자식이 없는 가장 깊은 선택 가능 node(OrderKey 깊이 우선 순서).
fn deepest(tree: &SelectTree<'_>, node: StableId, out: &mut Vec<StableId>) {
    for child in tree.selectable_children(node) {
        if tree.is_container(child) && !tree.selectable_children(child).is_empty() {
            deepest(tree, child, out);
        } else {
            out.push(child);
        }
    }
}

fn contains(area: &Aabb, inner: &Aabb) -> bool {
    inner.min_x() >= area.min_x()
        && inner.min_y() >= area.min_y()
        && inner.max_x() <= area.max_x()
        && inner.max_y() <= area.max_y()
}

/// 볼록 사각형과 축 정렬 영역의 분리축 판정. AABB 축은 호출자가 먼저 봅니다.
fn quad_overlaps(quad: &[Vec2; 4], area: &Aabb) -> bool {
    let rect = [
        (area.min_x(), area.min_y()),
        (area.max_x(), area.min_y()),
        (area.max_x(), area.max_y()),
        (area.min_x(), area.max_y()),
    ];
    for i in 0..4 {
        let (a, b) = (quad[i], quad[(i + 1) % 4]);
        let (nx, ny) = (a.y() - b.y(), b.x() - a.x());
        if nx == 0.0 && ny == 0.0 {
            continue;
        }
        let project = |x: f32, y: f32| x * nx + y * ny;
        let (mut q_min, mut q_max) = (f32::INFINITY, f32::NEG_INFINITY);
        for p in quad {
            let v = project(p.x(), p.y());
            q_min = q_min.min(v);
            q_max = q_max.max(v);
        }
        let (mut r_min, mut r_max) = (f32::INFINITY, f32::NEG_INFINITY);
        for (x, y) in rect {
            let v = project(x, y);
            r_min = r_min.min(v);
            r_max = r_max.max(v);
        }
        if q_max < r_min || r_max < q_min {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::hit::NodeBox;
    use crate::canvas::select_fixtures::{
        add_frame, frame_payload, nested, scene_of, sid, view_point,
    };
    use crate::window_session::test_fixtures::{sample_document, window};
    use stellar_commands::state::DocumentState;
    use stellar_core_types::value::{Rect, Size2, Transform2D};
    use stellar_history::window::WindowHistory;

    fn p(x: f32, y: f32) -> Vec2 {
        view_point(x, y)
    }

    /// 수식키 조합으로 (0,0)→`to` 마퀴를 끝까지 하고 확정 선택을 돌려줍니다.
    fn run(
        history: &mut WindowHistory,
        doc: &DocumentState,
        tree: &SelectTree<'_>,
        scope: EditingScope,
        geometry: &CanvasGeometry,
        (from, to): (Vec2, Vec2),
        modifiers: Modifiers,
    ) -> (Vec<StableId>, StableId) {
        let mut marquee =
            Marquee::begin(tree, scope, geometry, history.view_mut(), from, modifiers).unwrap();
        marquee.update(history.view_mut(), to).unwrap();
        let result = marquee.finish(tree, history.view_mut()).unwrap();
        if let Some(event) = &result.event {
            history.record_selection(doc, event).unwrap();
        }
        (
            history.view().selection().ids().to_vec(),
            result.scope.node(tree),
        )
    }

    #[test]
    fn modifier_matrix_on_the_nested_fixture() {
        // 검증 기준(XII.10.6): 수식키 4종 matrix(기본 겹침, Option 완전 포함, ⌘ 가장 깊은 node, Shift 더하기).
        let fixture = nested();
        let root = fixture.root;
        let doc = DocumentState::new(fixture.model);
        let tree = SelectTree::new(doc.model(), root);
        let scene = scene_of(doc.model(), root);
        let geometry = CanvasGeometry::from_snapshot(&scene.snapshot);
        let top = EditingScope::top(&tree);
        let mut history = WindowHistory::new(window(3));
        // (90,90)→(310,40)… 아래 영역 (90,10)–(310,250): 11(100–300)은 걸침, Fragment 12(300–420, 20–140)는 걸침.
        let area = (p(90., 10.), p(310., 250.));
        let none = run(
            &mut history,
            &doc,
            &tree,
            top,
            &geometry,
            area,
            Modifiers::NONE,
        );
        assert_eq!(none, (vec![sid(11), sid(12)], sid(10)));
        // Option: 완전 포함만. 11은 300까지라 (90,10)–(310,310) 안, 12는 420까지라 밖.
        let contain = run(
            &mut history,
            &doc,
            &tree,
            top,
            &geometry,
            (p(90., 10.), p(310., 310.)),
            Modifiers::OPTION,
        );
        assert_eq!(contain, (vec![sid(11)], sid(10)));
        // ⌘: 가장 깊은 node. 영역 (110,110)–(250,250)에 13·17 걸침(잠긴 15, 숨긴 16 제외), 범위는 공통 조상 11.
        let deep = run(
            &mut history,
            &doc,
            &tree,
            top,
            &geometry,
            (p(110., 110.), p(250., 250.)),
            Modifiers::COMMAND,
        );
        assert_eq!(deep, (vec![sid(13), sid(17)], sid(11)));
        // Shift: 시작 전 선택(13, 17)에 영역 (350,10)–(450,200)의 18·19를 더함. 범위는 공통 조상.
        let add = run(
            &mut history,
            &doc,
            &tree,
            EditingScope::at(&tree, sid(12)),
            &geometry,
            (p(350., 10.), p(450., 200.)),
            Modifiers::SHIFT,
        );
        assert_eq!(add, (vec![sid(13), sid(17), sid(18), sid(19)], sid(10)));
        assert_eq!(history.history().len(), 4, "마퀴마다 history 1건");
        // 같은 선택으로 끝나면 기록 없음, 취소도 기록 없음.
        let same = run(
            &mut history,
            &doc,
            &tree,
            top,
            &geometry,
            area,
            Modifiers::NONE,
        );
        assert_eq!(same.0, vec![sid(11), sid(12)]);
        let len = history.history().len();
        let mut again = Marquee::begin(
            &tree,
            top,
            &geometry,
            history.view_mut(),
            area.0,
            Modifiers::NONE,
        )
        .unwrap();
        again.update(history.view_mut(), area.1).unwrap();
        let result = again.finish(&tree, history.view_mut()).unwrap();
        assert_eq!(result.event, None);
        let mut cancelled = Marquee::begin(
            &tree,
            top,
            &geometry,
            history.view_mut(),
            p(0., 0.),
            Modifiers::NONE,
        )
        .unwrap();
        cancelled.update(history.view_mut(), p(800., 600.)).unwrap();
        assert!(history.view().area_selection_active());
        cancelled.cancel(history.view_mut()).unwrap();
        assert_eq!(history.history().len(), len);
        assert_eq!(history.view().selection().ids(), &[sid(11), sid(12)]);
    }

    #[test]
    fn rotated_nodes_use_their_world_quad() {
        // 45도 회전한 100×100 상자: AABB는 영역과 겹쳐도 실제 사각형은 겹치지 않는 모서리 영역.
        let model = sample_document();
        let tree = SelectTree::new(&model, sid(10));
        let mut geometry = CanvasGeometry::new();
        let (s, c) = std::f32::consts::FRAC_PI_4.sin_cos();
        geometry.insert(
            sid(11),
            NodeBox {
                world: Transform2D::new(c, s, -s, c, 100., 0.).unwrap(),
                rect: Rect::new(Vec2::ZERO, Size2::new(100., 100.).unwrap()),
            },
        );
        let mut view = WindowViewState::new(window(4));
        let top = EditingScope::top(&tree);
        let marquee =
            Marquee::begin(&tree, top, &geometry, &mut view, p(0., 0.), Modifiers::NONE).unwrap();
        // 사각형 꼭짓점: (100,0), (170.7,70.7), (100,141.4), (29.3,70.7). 영역 (0,0)–(40,20)은 AABB 안이지만 사각형 밖.
        assert!(marquee.hits(p(40., 20.)).is_empty());
        assert_eq!(marquee.hits(p(80., 60.)), vec![sid(11)]);
        marquee.cancel(&mut view).unwrap();
    }

    fn wide_document(count: u64) -> (DocumentState, CanvasGeometry) {
        let mut model = sample_document();
        model.remove_leaf(sid(11)).unwrap();
        model.remove_leaf(sid(12)).unwrap();
        model
            .set_payload(sid(10), frame_payload(0., 0., 20_000., 20_000.))
            .unwrap();
        let keys = stellar_core_types::order_key::OrderKey::evenly_spaced(count as usize);
        let mut geometry = CanvasGeometry::new();
        for (i, key) in keys.into_iter().enumerate() {
            let n = 1_000 + i as u64;
            let (x, y) = ((i % 200) as f32 * 100., (i / 200) as f32 * 100.);
            add_frame(&mut model, n, 10, 0x10, (x, y, 80., 80.));
            let mut record = model.record(sid(n)).unwrap();
            record.order_key = key;
            model.remove_leaf(sid(n)).unwrap();
            model.insert_node(record).unwrap();
            geometry.insert(
                sid(n),
                NodeBox {
                    world: Transform2D::new(1., 0., 0., 1., x, y).unwrap(),
                    rect: Rect::new(Vec2::ZERO, Size2::new(80., 80.).unwrap()),
                },
            );
        }
        (DocumentState::new(model), geometry)
    }

    #[test]
    fn thousand_moves_make_one_history_entry() {
        // 검증 기준(XII.10.6): 1,000 move event에서 history 1건.
        let (doc, geometry) = wide_document(2_000);
        let tree = SelectTree::new(doc.model(), sid(10));
        let mut history = WindowHistory::new(window(5));
        let mut marquee = Marquee::begin(
            &tree,
            EditingScope::top(&tree),
            &geometry,
            history.view_mut(),
            p(0., 0.),
            Modifiers::NONE,
        )
        .unwrap();
        assert_eq!(marquee.candidate_count(), 2_000);
        for step in 0..1_000 {
            let t = step as f32 * 2.;
            marquee.update(history.view_mut(), p(t, t)).unwrap();
            assert_eq!(history.history().len(), 0, "진행 중 기록 없음");
        }
        let result = marquee.finish(&tree, history.view_mut()).unwrap();
        history
            .record_selection(&doc, &result.event.unwrap())
            .unwrap();
        assert_eq!(history.history().len(), 1);
        assert!(!history.view().selection().is_empty());
    }

    #[test]
    #[ignore = "release 빌드에서 2만 가시 node 마퀴 갱신 시간을 측정"]
    fn measure_marquee_update_20k() {
        // 검증 기준(XII.10.6): 2만 가시 node fixture에서 갱신 p99 8ms 이하.
        let (doc, geometry) = wide_document(20_000);
        let tree = SelectTree::new(doc.model(), sid(10));
        let mut view = WindowViewState::new(window(6));
        let mut marquee = Marquee::begin(
            &tree,
            EditingScope::top(&tree),
            &geometry,
            &mut view,
            p(0., 0.),
            Modifiers::NONE,
        )
        .unwrap();
        // 후보 판정(이 모듈)과 preview 갱신 전체(WindowViewState 포함)를 따로 잽니다.
        let mut hits = Vec::with_capacity(1_000);
        let mut updates = Vec::with_capacity(1_000);
        for step in 0..1_000 {
            let to = p(20. + step as f32 * 10., 20. + step as f32 * 10.);
            let start = std::time::Instant::now();
            std::hint::black_box(marquee.hits(to));
            hits.push(start.elapsed());
            let start = std::time::Instant::now();
            marquee.update(&mut view, to).unwrap();
            updates.push(start.elapsed());
        }
        hits.sort_unstable();
        updates.sort_unstable();
        println!(
            "marquee hits, 20,000 nodes, 1,000 samples: p50 {:?}, p99 {:?}, max {:?}",
            hits[500], hits[989], hits[999]
        );
        println!(
            "marquee update(preview 포함), 20,000 nodes, 1,000 samples: p50 {:?}, p99 {:?}, max {:?}",
            updates[500], updates[989], updates[999]
        );
        assert!(
            hits[989] <= std::time::Duration::from_millis(8),
            "후보 판정 p99 {:?}",
            hits[989]
        );
        // 선택 중복 제거가 hash 기반이 된 뒤에는 preview 갱신 전체도 같은 기준 안이어야 합니다(ADR-0104 Performance).
        assert!(
            updates[989] <= std::time::Duration::from_millis(8),
            "preview 갱신 p99 {:?}",
            updates[989]
        );
    }
}

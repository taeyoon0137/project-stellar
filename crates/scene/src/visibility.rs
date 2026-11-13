//! viewport·clip 기반 가시 후보 계산 (XI.3.2).
//!
//! 2D canvas의 frustum은 화면 사각형을 world로 되돌린 영역입니다. [`visible_candidates`]는
//!
//! 1. [`Viewport::world_bounds`]로 공간 인덱스(XI.3.1)를 질의해 후보를 얻고,
//! 2. 숨김 node([`Visibility::Hidden`])를 빼고,
//! 3. node world 그리기 영역 ∩ viewport ∩ clip stack world bounds가 비면 뺀 뒤,
//! 4. 남은 node를 z-order 오름차순(paint 순서)의 [`DrawList`]로 돌려줍니다.
//!
//! clip bounds는 clip마다 world AABB(rect clip은 rect, path clip은 control point 껍질)를 구해
//! 교집합한 **보수적** 값입니다. 회전된 clip·곡선 path clip은 실제 clip보다 넓을 수 있어 일부 화면 밖
//! 조각이 후보에 남을 수 있지만, 화면 안에 그려질 node를 빼지는 않습니다. 정확한 잘라내기는
//! render graph clip 계획(XI.4.3)이 합니다.

use std::fmt;
use std::sync::Arc;

use stellar_core_types::value::{Size2, Transform2D, Vec2};

use crate::bounds::{path_control_bounds, Aabb};
use crate::node::{SceneNode, Visibility, ZOrder};
use crate::spatial::SceneSpatialIndex;
use crate::stack::{ClipShape, ClipStackRef};

/// viewport 생성 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportError {
    /// world→view 변환이 역변환 불가입니다(축척 0 등).
    NotInvertible,
    /// 화면 영역을 world로 되돌린 값이 유한하지 않습니다.
    NonFiniteWorldBounds,
}

impl fmt::Display for ViewportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ViewportError::NotInvertible => f.write_str("viewport 변환을 역변환할 수 없습니다"),
            ViewportError::NonFiniteWorldBounds => {
                f.write_str("viewport world 영역이 유한하지 않습니다")
            }
        }
    }
}

impl std::error::Error for ViewportError {}

/// 화면 영역과 world → view 변환(pan·zoom).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    size: Size2,
    view: Transform2D,
    world: Aabb,
}

impl Viewport {
    /// 화면 크기(view 좌표 `(0,0)`~`size`)와 world → view 변환으로 viewport를 만듭니다.
    pub fn new(size: Size2, view: Transform2D) -> Result<Viewport, ViewportError> {
        let inverse = view.inverse().ok_or(ViewportError::NotInvertible)?;
        let screen = Aabb::new(0.0, 0.0, size.width(), size.height())
            .ok_or(ViewportError::NonFiniteWorldBounds)?;
        let world = screen
            .transformed(&inverse)
            .ok_or(ViewportError::NonFiniteWorldBounds)?;
        Ok(Viewport { size, view, world })
    }

    /// 화면 크기.
    pub const fn size(&self) -> Size2 {
        self.size
    }

    /// world → view 변환.
    pub const fn view(&self) -> Transform2D {
        self.view
    }

    /// 화면 영역을 world로 되돌린 AABB(2D frustum).
    pub const fn world_bounds(&self) -> Aabb {
        self.world
    }
}

/// clip stack의 world bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClipBounds {
    /// clip 없음.
    Unclipped,
    /// 모든 clip world AABB의 교집합(보수적).
    Bounded(Aabb),
    /// 교집합이 비었거나 clip 변환이 비유한이라 아무것도 보이지 않습니다.
    Empty,
}

/// clip 하나의 world AABB. 비유한이면 `None`.
pub fn clip_shape_world_bounds(clip: &ClipShape) -> Option<Aabb> {
    match clip {
        ClipShape::Rect {
            rect, transform, ..
        } => Aabb::from_rect(*rect).transformed(transform),
        ClipShape::Path {
            geometry,
            transform,
            ..
        } => path_control_bounds(geometry)?.transformed(transform),
    }
}

/// clip stack 전체의 world bounds(보수적 교집합).
pub fn clip_world_bounds(clip: &ClipStackRef) -> ClipBounds {
    let mut acc: Option<Aabb> = None;
    for shape in clip.iter_inner_first() {
        let Some(bounds) = clip_shape_world_bounds(shape) else {
            return ClipBounds::Empty;
        };
        acc = match acc {
            None => Some(bounds),
            Some(current) => match current.intersection(&bounds) {
                Some(inter) => Some(inter),
                None => return ClipBounds::Empty,
            },
        };
    }
    acc.map_or(ClipBounds::Unclipped, ClipBounds::Bounded)
}

/// draw list 항목.
#[derive(Debug, Clone)]
pub struct DrawItem {
    /// paint 순서 key.
    pub z: ZOrder,
    /// scene node.
    pub node: Arc<SceneNode>,
    /// world 그리기 영역 ∩ viewport ∩ clip bounds(보수적).
    pub visible_bounds: Aabb,
}

/// culling 통계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CullStats {
    /// 공간 인덱스가 돌려준 후보 수.
    pub candidates: usize,
    /// 숨김이라 뺀 수.
    pub hidden: usize,
    /// clip 밖이라 뺀 수.
    pub clipped: usize,
    /// viewport 밖이라 뺀 수(인덱스 후보 중 정밀 검사에서 빠진 것).
    pub outside: usize,
}

/// z-order 오름차순 가시 node 목록.
#[derive(Debug, Clone, Default)]
pub struct DrawList {
    /// paint 순서 항목.
    pub items: Vec<DrawItem>,
    /// culling 통계.
    pub stats: CullStats,
}

impl DrawList {
    /// 항목들의 z-order.
    pub fn z_orders(&self) -> Vec<ZOrder> {
        self.items.iter().map(|item| item.z).collect()
    }
}

/// viewport와 clip bounds로 가시 후보를 계산합니다.
pub fn visible_candidates(index: &SceneSpatialIndex, viewport: &Viewport) -> DrawList {
    let area = viewport.world_bounds();
    let mut candidates = Vec::new();
    index.query_rect(&area, &mut candidates);
    let mut list = DrawList {
        items: Vec::with_capacity(candidates.len()),
        stats: CullStats {
            candidates: candidates.len(),
            ..CullStats::default()
        },
    };
    for z in candidates {
        let Some(node) = index.node(z) else {
            continue;
        };
        if node.hit().visibility == Visibility::Hidden {
            list.stats.hidden += 1;
            continue;
        }
        let Some(on_screen) = node
            .world_render_bounds()
            .and_then(|bounds| bounds.intersection(&area))
        else {
            list.stats.outside += 1;
            continue;
        };
        let visible = match clip_world_bounds(node.clip()) {
            ClipBounds::Unclipped => Some(on_screen),
            ClipBounds::Bounded(clip) => on_screen.intersection(&clip),
            ClipBounds::Empty => None,
        };
        match visible {
            Some(visible_bounds) => list.items.push(DrawItem {
                z,
                node: Arc::clone(node),
                visible_bounds,
            }),
            None => list.stats.clipped += 1,
        }
    }
    list.items.sort_unstable_by_key(|item| item.z);
    list
}

/// world 점을 view(화면) 좌표로 옮깁니다(입력·overlay 계산 보조).
pub fn world_to_view(viewport: &Viewport, point: Vec2) -> Option<Vec2> {
    viewport.view().apply(point).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{
        FrameClipInput, FrameInput, NodeInput, PathInput, SceneBuilder, SceneScope,
    };
    use crate::fixtures::{self, id, p, rect, translate, FixtureConfig, SplitMix64};
    use crate::geometry::{FillRule, PathCommand, PathGeometry};
    use crate::node::CornerRadii;
    use crate::snapshot::SceneSnapshot;
    use stellar_core_types::revision::{DocumentRevision, SceneRevision};

    fn size(w: f32, h: f32) -> Size2 {
        Size2::new(w, h).unwrap()
    }

    fn frame(source: u64, x: f32, y: f32, w: f32, h: f32, clip: FrameClipInput) -> FrameInput {
        FrameInput {
            node: NodeInput::new(id(source), translate(x, y)),
            size: size(w, h),
            corner_radii: CornerRadii::ZERO,
            background: None,
            border: None,
            clip,
        }
    }

    fn square_path(
        builder: &mut SceneBuilder,
        scope: &mut SceneScope,
        source: u64,
        x: f32,
        y: f32,
    ) {
        builder
            .build_path(
                scope,
                &PathInput {
                    node: NodeInput::new(id(source), translate(x, y)),
                    layout_rect: rect(0.0, 0.0, 10.0, 10.0),
                    geometry: PathGeometry::new(vec![
                        PathCommand::MoveTo(p(0.0, 0.0)),
                        PathCommand::LineTo(p(10.0, 0.0)),
                        PathCommand::LineTo(p(10.0, 10.0)),
                        PathCommand::Close,
                    ])
                    .unwrap(),
                    fill_rule: FillRule::NonZero,
                },
            )
            .unwrap();
    }

    /// viewport (0,0)~(800,600) 기준 배치.
    fn scene() -> SceneSnapshot {
        let mut builder = SceneBuilder::new();
        let mut root = builder.root_scope();
        // z1: 화면 안 frame(clip 있음), 자식 z2 안, z3 frame clip 밖(화면 안이지만 잘림).
        let mut inside = builder
            .begin_frame(
                &root,
                &frame(1, 100.0, 100.0, 200.0, 200.0, FrameClipInput::Bounds),
            )
            .unwrap();
        square_path(&mut builder, inside.scope_mut(), 2, 10.0, 10.0);
        square_path(&mut builder, inside.scope_mut(), 3, 250.0, 10.0);
        builder.finish_frame(&mut root, inside).unwrap();
        // z4: 완전히 화면 오른쪽 밖.
        let offscreen = builder
            .begin_frame(
                &root,
                &frame(4, 900.0, 100.0, 50.0, 50.0, FrameClipInput::None),
            )
            .unwrap();
        builder.finish_frame(&mut root, offscreen).unwrap();
        // z5: 화면 왼쪽 경계에 걸침.
        let partial = builder
            .begin_frame(
                &root,
                &frame(5, -40.0, 300.0, 80.0, 80.0, FrameClipInput::None),
            )
            .unwrap();
        builder.finish_frame(&mut root, partial).unwrap();
        // z6: 화면 안이지만 숨김.
        let mut hidden = frame(6, 400.0, 400.0, 50.0, 50.0, FrameClipInput::None);
        hidden.node.visible = false;
        let hidden = builder.begin_frame(&root, &hidden).unwrap();
        builder.finish_frame(&mut root, hidden).unwrap();
        // z7: 화면 밖 clip frame, 자식 z8은 자기 bounds로는 화면 안이지만 부모 clip이 화면 밖.
        let mut far = builder
            .begin_frame(
                &root,
                &frame(7, 2_000.0, 0.0, 100.0, 100.0, FrameClipInput::Bounds),
            )
            .unwrap();
        square_path(&mut builder, far.scope_mut(), 8, -1_900.0, 50.0);
        builder.finish_frame(&mut root, far).unwrap();
        SceneSnapshot::new(DocumentRevision(1), SceneRevision(1), builder.finish(root)).unwrap()
    }

    #[test]
    fn offscreen_nodes_are_excluded_from_draw_list() {
        // 검증 기준(XI.3.2): 화면 밖 node draw list 제외.
        let snapshot = scene();
        let index = SceneSpatialIndex::build(&snapshot);
        let viewport = Viewport::new(size(800.0, 600.0), Transform2D::identity()).unwrap();
        let list = visible_candidates(&index, &viewport);
        assert_eq!(list.z_orders(), vec![ZOrder(1), ZOrder(2), ZOrder(5)]);
        assert_eq!(list.stats.hidden, 1);
        // z3(frame clip 밖), z8(부모 clip이 화면 밖)은 clip으로 빠집니다.
        assert_eq!(list.stats.clipped, 2);
        // 화면에 걸친 node의 가시 영역은 화면 안으로 잘립니다.
        let partial = list.items.iter().find(|item| item.z == ZOrder(5)).unwrap();
        assert_eq!(
            partial.visible_bounds,
            Aabb::new(0.0, 300.0, 40.0, 380.0).unwrap()
        );
    }

    #[test]
    fn pan_and_zoom_move_the_world_frustum() {
        let snapshot = scene();
        let index = SceneSpatialIndex::build(&snapshot);
        // world (850, 50)이 화면 원점, 2배 확대: world 영역 (850,50)~(1250,350).
        let view = Transform2D::new(2.0, 0.0, 0.0, 2.0, -1_700.0, -100.0).unwrap();
        let viewport = Viewport::new(size(800.0, 600.0), view).unwrap();
        assert_eq!(
            viewport.world_bounds(),
            Aabb::new(850.0, 50.0, 1_250.0, 350.0).unwrap()
        );
        let list = visible_candidates(&index, &viewport);
        assert_eq!(list.z_orders(), vec![ZOrder(4)]);
        assert_eq!(
            world_to_view(&viewport, p(900.0, 100.0)),
            Some(p(100.0, 100.0))
        );
        assert_eq!(
            Viewport::new(
                size(1.0, 1.0),
                Transform2D::new(0.0, 0.0, 0.0, 0.0, 0.0, 0.0).unwrap()
            ),
            Err(ViewportError::NotInvertible)
        );
    }

    #[test]
    fn draw_list_matches_brute_force_on_random_fixture() {
        let snapshot = fixtures::generate(FixtureConfig {
            seed: 0x32,
            roots: 6,
            children: 5,
            depth: 3,
        });
        let index = SceneSpatialIndex::build(&snapshot);
        let mut rng = SplitMix64::new(0x32);
        for _ in 0..60 {
            let scale = rng.range(0.25, 4.0);
            let view = Transform2D::new(
                scale,
                0.0,
                0.0,
                scale,
                rng.range(-800.0, 800.0),
                rng.range(-800.0, 800.0),
            )
            .unwrap();
            let viewport = Viewport::new(size(1_280.0, 800.0), view).unwrap();
            let area = viewport.world_bounds();
            let list = visible_candidates(&index, &viewport);
            let mut expected: Vec<ZOrder> = snapshot
                .iter_preorder()
                .filter(|node| node.hit().visibility == Visibility::Visible)
                .filter(|node| {
                    let Some(on_screen) = node
                        .world_render_bounds()
                        .and_then(|bounds| bounds.intersection(&area))
                    else {
                        return false;
                    };
                    match clip_world_bounds(node.clip()) {
                        ClipBounds::Unclipped => true,
                        ClipBounds::Bounded(clip) => on_screen.intersects(&clip),
                        ClipBounds::Empty => false,
                    }
                })
                .map(|node| node.z())
                .collect();
            expected.sort_unstable();
            assert_eq!(list.z_orders(), expected);
            // draw list는 paint 순서(z 오름차순)입니다.
            assert!(list.items.windows(2).all(|w| w[0].z < w[1].z));
        }
    }

    #[test]
    fn clip_bounds_intersect_conservatively() {
        let outer = ClipShape::Rect {
            rect: rect(0.0, 0.0, 100.0, 100.0),
            radii: CornerRadii::ZERO,
            transform: Transform2D::identity(),
        };
        let inner = ClipShape::Rect {
            rect: rect(0.0, 0.0, 100.0, 100.0),
            radii: CornerRadii::ZERO,
            transform: translate(50.0, 50.0),
        };
        let disjoint = ClipShape::Rect {
            rect: rect(0.0, 0.0, 10.0, 10.0),
            radii: CornerRadii::ZERO,
            transform: translate(500.0, 0.0),
        };
        assert_eq!(
            clip_world_bounds(&ClipStackRef::none()),
            ClipBounds::Unclipped
        );
        let stack = ClipStackRef::none().push(outer).push(inner);
        assert_eq!(
            clip_world_bounds(&stack),
            ClipBounds::Bounded(Aabb::new(50.0, 50.0, 100.0, 100.0).unwrap())
        );
        assert_eq!(clip_world_bounds(&stack.push(disjoint)), ClipBounds::Empty);
    }
}

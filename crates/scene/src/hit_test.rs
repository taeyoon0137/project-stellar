//! broad-phase hit test (XI.3.3).
//!
//! [`hit_test_broad`]는 world 점 하나에 대해 다음 순서로 후보를 거릅니다.
//!
//! 1. 공간 인덱스(XI.3.1)로 world 그리기 영역이 점을 포함하는 node를 찾습니다.
//! 2. **visibility**: 실효 [`Visibility::Hidden`] node는 대상이 아닙니다.
//! 3. **pointer-events**: 실효 [`PointerEvents::None`] node는 건너뛰고 아래 node가 대상이 됩니다
//!    (잠금 node 통과). 자식은 자기 값을 따릅니다.
//! 4. **node bounds**: 점을 node local 좌표로 되돌려(world transform 역변환) local hit bounds
//!    (geometry + stroke, XI.3.1) 안인지 봅니다. 회전·기울인 node도 방향을 고려합니다. 역변환이
//!    불가능한(축척 0) node는 면적이 없어 맞지 않습니다.
//! 5. **clip**: 점이 clip stack의 모든 clip 안이어야 합니다. rect clip은 모서리 radius까지 정확히
//!    판정하고, path clip은 broad-phase에서 clip path의 control point 껍질로 판정합니다(보수적).
//! 6. 남은 후보를 **z-order 역순**(위에 그린 것부터)으로 정렬합니다. 첫 항목이 topmost입니다.
//!
//! node content geometry(path 내부, glyph 등) 정밀 판정은 narrow-phase(XI.3.4)가 이 결과 위에서
//! 합니다.

use std::sync::Arc;

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{Transform2D, Vec2};

use crate::bounds::{path_control_bounds, Aabb};
use crate::node::{CornerRadii, PointerEvents, SceneNode, Visibility, ZOrder};
use crate::spatial::SceneSpatialIndex;
use crate::stack::{ClipShape, ClipStackRef};
use crate::visibility::Viewport;

/// hit 후보.
#[derive(Debug, Clone)]
pub struct HitCandidate {
    /// paint 순서 key.
    pub z: ZOrder,
    /// 원본 document node.
    pub source: StableId,
    /// scene node.
    pub node: Arc<SceneNode>,
    /// node local 좌표의 hit 지점.
    pub local_point: Vec2,
}

/// broad-phase 통계.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HitStats {
    /// 공간 인덱스 후보 수.
    pub candidates: usize,
    /// 숨김이라 뺀 수.
    pub hidden: usize,
    /// pointer-events None이라 통과한 수.
    pub pass_through: usize,
    /// node bounds 밖이라 뺀 수.
    pub outside: usize,
    /// clip 밖이라 뺀 수.
    pub clipped: usize,
}

/// broad-phase 결과(topmost 우선).
#[derive(Debug, Clone, Default)]
pub struct HitResult {
    /// z-order 내림차순 후보.
    pub hits: Vec<HitCandidate>,
    /// 통계.
    pub stats: HitStats,
}

impl HitResult {
    /// 가장 위 후보.
    pub fn topmost(&self) -> Option<&HitCandidate> {
        self.hits.first()
    }

    /// 후보 z-order(topmost 우선).
    pub fn z_orders(&self) -> Vec<ZOrder> {
        self.hits.iter().map(|hit| hit.z).collect()
    }
}

/// `point`를 `transform`의 local 좌표로 되돌립니다. 역변환 불가·비유한이면 `None`.
fn to_local(transform: &Transform2D, point: Vec2) -> Option<Vec2> {
    transform.inverse()?.apply(point).ok()
}

/// 모서리 둥근 사각형 안(경계 포함)이면 참입니다. radius는 실효 값(변 길이 이하)입니다.
pub fn rounded_rect_contains(bounds: &Aabb, radii: &CornerRadii, point: Vec2) -> bool {
    if !bounds.contains_point(point) {
        return false;
    }
    let (tl, tr, br, bl) = radii.components();
    let (x, y) = (point.x(), point.y());
    let corner = |cx: f32, cy: f32, r: f32| {
        let (dx, dy) = (x - cx, y - cy);
        dx * dx + dy * dy <= r * r
    };
    let (x0, y0, x1, y1) = (
        bounds.min_x(),
        bounds.min_y(),
        bounds.max_x(),
        bounds.max_y(),
    );
    if tl > 0.0 && x < x0 + tl && y < y0 + tl {
        return corner(x0 + tl, y0 + tl, tl);
    }
    if tr > 0.0 && x > x1 - tr && y < y0 + tr {
        return corner(x1 - tr, y0 + tr, tr);
    }
    if br > 0.0 && x > x1 - br && y > y1 - br {
        return corner(x1 - br, y1 - br, br);
    }
    if bl > 0.0 && x < x0 + bl && y > y1 - bl {
        return corner(x0 + bl, y1 - bl, bl);
    }
    true
}

/// world 점이 clip 하나 안인지(broad-phase). 비유한·역변환 불가 clip은 아무것도 통과시키지 않습니다.
pub fn clip_contains(clip: &ClipShape, point: Vec2) -> bool {
    match clip {
        ClipShape::Rect {
            rect,
            radii,
            transform,
        } => to_local(transform, point)
            .is_some_and(|local| rounded_rect_contains(&Aabb::from_rect(*rect), radii, local)),
        ClipShape::Path {
            geometry,
            transform,
            ..
        } => match (to_local(transform, point), path_control_bounds(geometry)) {
            (Some(local), Some(bounds)) => bounds.contains_point(local),
            _ => false,
        },
    }
}

/// world 점이 clip stack 전체 안인지(broad-phase).
pub fn clip_stack_contains(clip: &ClipStackRef, point: Vec2) -> bool {
    clip.iter_inner_first()
        .all(|shape| clip_contains(shape, point))
}

/// world 점 `point`의 broad-phase hit test.
pub fn hit_test_broad(index: &SceneSpatialIndex, point: Vec2) -> HitResult {
    let mut candidates = Vec::new();
    index.query_point(point, &mut candidates);
    let mut result = HitResult {
        hits: Vec::with_capacity(candidates.len()),
        stats: HitStats {
            candidates: candidates.len(),
            ..HitStats::default()
        },
    };
    for z in candidates {
        let Some(node) = index.node(z) else {
            continue;
        };
        let hit = node.hit();
        if hit.visibility == Visibility::Hidden {
            result.stats.hidden += 1;
            continue;
        }
        if hit.pointer_events == PointerEvents::None {
            result.stats.pass_through += 1;
            continue;
        }
        let Some(local) = to_local(&node.world_transform(), point)
            .filter(|local| node.local_hit_bounds().contains_point(*local))
        else {
            result.stats.outside += 1;
            continue;
        };
        if !clip_stack_contains(node.clip(), point) {
            result.stats.clipped += 1;
            continue;
        }
        result.hits.push(HitCandidate {
            z,
            source: node.source(),
            node: Arc::clone(node),
            local_point: local,
        });
    }
    result
        .hits
        .sort_unstable_by_key(|hit| std::cmp::Reverse(hit.z));
    result
}

/// view(화면) 좌표 점의 broad-phase hit test. viewport 역변환이 비유한이면 빈 결과입니다.
pub fn hit_test_view(
    index: &SceneSpatialIndex,
    viewport: &Viewport,
    view_point: Vec2,
) -> HitResult {
    match to_local(&viewport.view(), view_point) {
        Some(world) => hit_test_broad(index, world),
        None => HitResult::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::{FrameClipInput, FrameInput, NodeInput, SceneBuilder};
    use crate::fixtures::{self, id, p, rect, translate, FixtureConfig, SplitMix64};
    use crate::geometry::{FillRule, PathCommand, PathGeometry};
    use crate::snapshot::SceneSnapshot;
    use stellar_core_types::revision::{DocumentRevision, SceneRevision};
    use stellar_core_types::value::Size2;

    fn frame(source: u64, transform: Transform2D, w: f32, h: f32) -> FrameInput {
        FrameInput {
            node: NodeInput::new(id(source), transform),
            size: Size2::new(w, h).unwrap(),
            corner_radii: CornerRadii::ZERO,
            background: None,
            border: None,
            clip: FrameClipInput::Bounds,
        }
    }

    /// overlap fixture.
    ///
    /// - A(z1) (0,0)~(100,100), 자식 A1(z2) (50,50)~(250,250) — A clip에 잘림.
    /// - B(z3) (40,40)~(140,140).
    /// - C(z4) (60,60)~(160,160), `pointer_none`이면 잠금(통과).
    /// - D(z5) (70,70)~(170,170), `hide_d`면 숨김.
    /// - E(z6) 45° 회전 정사각형(한 변 100, 원점 (400,0)).
    /// - F(z7) (300,300)~(400,400), 모서리 radius 30 bounds clip, 자식 F1(z8)이 F 전체를 덮음.
    fn overlap(pointer_none: bool, hide_d: bool) -> SceneSnapshot {
        let mut builder = SceneBuilder::new();
        let mut root = builder.root_scope();
        let mut a = builder
            .begin_frame(&root, &frame(1, Transform2D::identity(), 100.0, 100.0))
            .unwrap();
        let a1 = builder
            .begin_frame(a.scope(), &frame(2, translate(50.0, 50.0), 200.0, 200.0))
            .unwrap();
        builder.finish_frame(a.scope_mut(), a1).unwrap();
        builder.finish_frame(&mut root, a).unwrap();
        let b = builder
            .begin_frame(&root, &frame(3, translate(40.0, 40.0), 100.0, 100.0))
            .unwrap();
        builder.finish_frame(&mut root, b).unwrap();
        let mut c_input = frame(4, translate(60.0, 60.0), 100.0, 100.0);
        c_input.node.locked = pointer_none;
        let c = builder.begin_frame(&root, &c_input).unwrap();
        builder.finish_frame(&mut root, c).unwrap();
        let mut d_input = frame(5, translate(70.0, 70.0), 100.0, 100.0);
        d_input.node.visible = !hide_d;
        let d = builder.begin_frame(&root, &d_input).unwrap();
        builder.finish_frame(&mut root, d).unwrap();
        let (s, c45) = (
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        let rotated = Transform2D::new(c45, s, -s, c45, 400.0, 0.0).unwrap();
        let e = builder
            .begin_frame(&root, &frame(6, rotated, 100.0, 100.0))
            .unwrap();
        builder.finish_frame(&mut root, e).unwrap();
        let mut f_input = frame(7, translate(300.0, 300.0), 100.0, 100.0);
        f_input.corner_radii = CornerRadii::new(30.0, 30.0, 30.0, 30.0).unwrap();
        let mut f = builder.begin_frame(&root, &f_input).unwrap();
        let f1 = builder
            .begin_frame(f.scope(), &frame(8, Transform2D::identity(), 100.0, 100.0))
            .unwrap();
        builder.finish_frame(f.scope_mut(), f1).unwrap();
        builder.finish_frame(&mut root, f).unwrap();
        SceneSnapshot::new(DocumentRevision(1), SceneRevision(1), builder.finish(root)).unwrap()
    }

    #[test]
    fn overlap_fixture_returns_topmost() {
        // 검증 기준(XI.3.3): overlap fixture에서 topmost 결과.
        let index = SceneSpatialIndex::build(&overlap(false, false));
        // (80,80)은 A·A1·B·C·D 모두 안: z 역순, topmost D.
        let result = hit_test_broad(&index, p(80.0, 80.0));
        assert_eq!(
            result.z_orders(),
            vec![ZOrder(5), ZOrder(4), ZOrder(3), ZOrder(2), ZOrder(1)]
        );
        let top = result.topmost().unwrap();
        assert_eq!(top.source, id(5));
        assert_eq!(top.local_point, p(10.0, 10.0));
        // (45,45)는 A·B만(A1은 50부터).
        assert_eq!(
            hit_test_broad(&index, p(45.0, 45.0)).z_orders(),
            vec![ZOrder(3), ZOrder(1)]
        );
    }

    #[test]
    fn pointer_none_and_hidden_pass_through_to_lower_nodes() {
        // D 숨김 + C 잠금 → (80,80) topmost는 B.
        let index = SceneSpatialIndex::build(&overlap(true, true));
        let result = hit_test_broad(&index, p(80.0, 80.0));
        assert_eq!(result.topmost().unwrap().z, ZOrder(3));
        assert_eq!(result.stats.hidden, 1);
        assert_eq!(result.stats.pass_through, 1);
    }

    #[test]
    fn clip_excludes_points_outside_ancestor_clip() {
        let index = SceneSpatialIndex::build(&overlap(false, false));
        // (200,200)은 A1 bounds 안이지만 A clip(0~100) 밖. 아래 node도 없음.
        let result = hit_test_broad(&index, p(200.0, 200.0));
        assert!(result.hits.is_empty());
        assert_eq!(result.stats.clipped, 1);
        // F 모서리(radius 30) 밖 (302,302)은 F 자체 bounds 안이지만 자식 F1은 둥근 clip에 잘립니다.
        let corner = hit_test_broad(&index, p(302.0, 302.0));
        assert_eq!(corner.z_orders(), vec![ZOrder(7)]);
        // 모서리 안쪽 (320,320)은 F1이 topmost.
        assert_eq!(
            hit_test_broad(&index, p(320.0, 320.0)).topmost().unwrap().z,
            ZOrder(8)
        );
    }

    #[test]
    fn rotated_nodes_use_oriented_bounds() {
        let index = SceneSpatialIndex::build(&overlap(false, false));
        // 45° 회전 정사각형: 꼭짓점 (400,0), (470.7,70.7), (400,141.4), (329.3,70.7).
        assert_eq!(
            hit_test_broad(&index, p(400.0, 70.0)).topmost().unwrap().z,
            ZOrder(6)
        );
        // AABB 모서리 근처 (335,5)는 AABB 안이지만 회전 사각형 밖.
        let miss = hit_test_broad(&index, p(335.0, 5.0));
        assert!(miss.hits.is_empty());
        assert_eq!(miss.stats.outside, 1);
    }

    #[test]
    fn view_space_hit_uses_viewport_inverse() {
        let index = SceneSpatialIndex::build(&overlap(false, false));
        let view = Transform2D::new(2.0, 0.0, 0.0, 2.0, -100.0, -100.0).unwrap();
        let viewport = Viewport::new(Size2::new(800.0, 600.0).unwrap(), view).unwrap();
        // 화면 (60,60) = world (80,80).
        assert_eq!(
            hit_test_view(&index, &viewport, p(60.0, 60.0))
                .topmost()
                .unwrap()
                .z,
            ZOrder(5)
        );
    }

    #[test]
    fn path_clip_uses_control_hull_in_broad_phase() {
        let clip = ClipShape::Path {
            geometry: PathGeometry::new(vec![
                PathCommand::MoveTo(p(0.0, 0.0)),
                PathCommand::LineTo(p(10.0, 0.0)),
                PathCommand::LineTo(p(0.0, 10.0)),
                PathCommand::Close,
            ])
            .unwrap(),
            fill_rule: FillRule::NonZero,
            transform: translate(100.0, 0.0),
        };
        // 삼각형 밖이지만 control 껍질 안 → broad-phase 통과(정밀 판정은 XI.3.4).
        assert!(clip_contains(&clip, p(109.0, 9.0)));
        assert!(!clip_contains(&clip, p(111.0, 5.0)));
        let degenerate = ClipShape::Rect {
            rect: rect(0.0, 0.0, 10.0, 10.0),
            radii: CornerRadii::ZERO,
            transform: Transform2D::new(0.0, 0.0, 0.0, 0.0, 0.0, 0.0).unwrap(),
        };
        assert!(!clip_contains(&degenerate, p(0.0, 0.0)));
        let bounds = Aabb::new(0.0, 0.0, 100.0, 100.0).unwrap();
        let radii = CornerRadii::new(10.0, 0.0, 20.0, 0.0).unwrap();
        assert!(!rounded_rect_contains(&bounds, &radii, p(1.0, 1.0)));
        assert!(rounded_rect_contains(&bounds, &radii, p(99.0, 1.0)));
        assert!(!rounded_rect_contains(&bounds, &radii, p(97.0, 97.0)));
        assert!(rounded_rect_contains(&bounds, &radii, p(1.0, 99.0)));
    }

    #[test]
    fn hits_match_brute_force_predicate_on_random_fixture() {
        let snapshot = fixtures::generate(FixtureConfig {
            seed: 0x33,
            roots: 5,
            children: 5,
            depth: 3,
        });
        let index = SceneSpatialIndex::build(&snapshot);
        let mut rng = SplitMix64::new(0x33);
        for _ in 0..300 {
            let point = p(rng.range(-300.0, 1_500.0), rng.range(-300.0, 1_500.0));
            let result = hit_test_broad(&index, point);
            let mut expected: Vec<ZOrder> = snapshot
                .iter_preorder()
                .filter(|node| node.hit().is_hit_target())
                .filter(|node| {
                    to_local(&node.world_transform(), point)
                        .is_some_and(|local| node.local_hit_bounds().contains_point(local))
                })
                .filter(|node| clip_stack_contains(node.clip(), point))
                .map(|node| node.z())
                .collect();
            expected.sort_unstable_by_key(|z| std::cmp::Reverse(*z));
            assert_eq!(result.z_orders(), expected);
        }
    }
}

//! selection overlay transient scene layer (XII.4.4).
//!
//! 선택 표시(선택 상자 외곽선)와 크기 조절 handle은 문서 내용이 아니라 창의 상호작용 상태(선택, VII.3.1)에서 만든 일시
//! 객체입니다. [`build_selection_overlay`]는 문서 scene snapshot·선택·camera([`Viewport`])로 **별도 transient scene
//! layer**([`SelectionOverlay`])를 만듭니다.
//!
//! - **그리기**: overlay node는 view(화면) 좌표의 scene primitive입니다. 선택된 node마다 외곽선(path, 선 `outline_width`),
//!   primary 선택 상자의 네 모서리에 크기 조절 handle(정사각형 frame, 채움 + 테두리)을 둡니다. renderer는 문서 scene을
//!   camera로 그린 뒤 이 layer를 항등 변환으로 위에 그립니다. 그래서 handle·외곽선 두께는 확대율과 관계없이 화면에서
//!   같습니다. 변 띠·회전 영역은 그리지 않고 hit test 영역으로만 둡니다(XI.3.5).
//! - **hit test**: 같은 primary 상자로 [`TransientLayer::push_selection_box`] handle을 만듭니다(XI.3.5).
//! - **저장·문서와 분리**: overlay는 문서 scene snapshot·공간 인덱스·scene codec·문서 모델 어디에도 들어가지 않습니다.
//!   overlay snapshot은 자기 z-order 공간(1부터)을 쓰고 창이 다시 만들어 버립니다. project 저장 경로에 들어갈 수 없습니다.
//! - **색·크기**: 외곽선·handle 색과 크기는 [`OverlayStyle`]로 셸이 넘깁니다(운영체제 강조색·HIG 크기). 코어는 기본
//!   모양을 정하지 않습니다.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use stellar_core_types::color::Color;
use stellar_core_types::ids::StableId;
use stellar_core_types::value::{Rect, Size2, Transform2D, Vec2};

use crate::builder::{
    BorderInput, FrameClipInput, FrameInput, ModifierPaint, NodeInput, PathInput, SceneBuildError,
    SceneBuilder, StrokeInput,
};
use crate::geometry::{FillRule, PathCommand, PathGeometry};
use crate::node::{CornerRadii, SceneNode, SceneTreeError};
use crate::snapshot::SceneSnapshot;
use crate::stack::Paint;
use crate::transient::{HandleMetrics, TransientLayer};
use crate::visibility::Viewport;

/// overlay 모양(셸이 정함).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayStyle {
    /// 선택 외곽선 색.
    pub outline: Color,
    /// 선택 외곽선 두께(view px).
    pub outline_width: f32,
    /// handle 채움 색.
    pub handle_fill: Color,
    /// handle 테두리 색.
    pub handle_stroke: Color,
    /// handle 테두리 두께(view px).
    pub handle_stroke_width: f32,
    /// handle 정사각형 반변(view px).
    pub handle_half: f32,
    /// hit test 영역 크기(XI.3.5).
    pub hit: HandleMetrics,
}

/// overlay 생성 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayError {
    /// view 좌표 계산이 유한 범위를 넘었습니다.
    NonFinite {
        /// 문제 node.
        node: StableId,
    },
    /// overlay primitive를 만들지 못했습니다(음수 두께 등).
    Build(SceneBuildError),
    /// overlay snapshot을 만들지 못했습니다.
    Tree(SceneTreeError),
}

impl fmt::Display for OverlayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OverlayError::NonFinite { node } => {
                write!(f, "node {node}의 선택 표시 좌표가 유한하지 않습니다")
            }
            OverlayError::Build(error) => write!(f, "선택 표시를 만들지 못했습니다: {error}"),
            OverlayError::Tree(error) => write!(f, "선택 표시 layer를 만들지 못했습니다: {error}"),
        }
    }
}

impl std::error::Error for OverlayError {}

impl From<SceneBuildError> for OverlayError {
    fn from(error: SceneBuildError) -> OverlayError {
        OverlayError::Build(error)
    }
}

/// 선택된 node 하나의 표시 상자.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayTarget {
    /// 선택된 document node.
    pub node: StableId,
    /// world 상자 꼭짓점 `[top_left, top_right, bottom_right, bottom_left]`.
    pub world_quad: [Vec2; 4],
    /// view 상자 꼭짓점(같은 순서).
    pub view_quad: [Vec2; 4],
    /// primary 선택인지(handle을 받는 상자).
    pub primary: bool,
}

/// selection overlay transient scene layer(모듈 문서).
#[derive(Debug, Clone)]
pub struct SelectionOverlay {
    targets: Vec<OverlayTarget>,
    layer: SceneSnapshot,
    hit: TransientLayer,
    handle_count: usize,
}

impl SelectionOverlay {
    /// scene에 있는 선택 node의 표시 상자(선택 순서). scene에 없는 선택 id는 빠집니다.
    pub fn targets(&self) -> &[OverlayTarget] {
        &self.targets
    }

    /// 그릴 overlay layer(view 좌표 scene primitive, 자기 z-order 공간).
    pub fn layer(&self) -> &SceneSnapshot {
        &self.layer
    }

    /// handle hit test layer(XI.3.5).
    pub fn hit_layer(&self) -> &TransientLayer {
        &self.hit
    }

    /// 그린 handle 수(primary가 있으면 4).
    pub const fn handle_count(&self) -> usize {
        self.handle_count
    }

    /// 표시할 것이 없는지(선택이 비었거나 scene에 없음).
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }
}

/// scene snapshot에서 source id → 첫 scene node(paint 순서) 표.
fn nodes_by_source(scene: &SceneSnapshot) -> BTreeMap<StableId, Arc<SceneNode>> {
    let mut out = BTreeMap::new();
    let mut stack: Vec<Arc<SceneNode>> = scene.roots().cloned().collect();
    stack.reverse();
    while let Some(node) = stack.pop() {
        out.entry(node.source())
            .or_insert_with(|| Arc::clone(&node));
        let mut children: Vec<Arc<SceneNode>> = node.children().cloned().collect();
        children.reverse();
        stack.extend(children);
    }
    out
}

fn quad_of(transform: &Transform2D, rect: Rect) -> Option<[Vec2; 4]> {
    let (x, y) = (rect.origin().x(), rect.origin().y());
    let (w, h) = (rect.size().width(), rect.size().height());
    let corner = |px: f32, py: f32| Vec2::new(px, py).ok().and_then(|p| transform.apply(p).ok());
    Some([
        corner(x, y)?,
        corner(x + w, y)?,
        corner(x + w, y + h)?,
        corner(x, y + h)?,
    ])
}

/// 꼭짓점의 축 정렬 bounding 상자.
fn bounds_of(quad: &[Vec2; 4]) -> Option<Rect> {
    let xs = quad.map(|p| p.x());
    let ys = quad.map(|p| p.y());
    let (min_x, max_x) = (
        xs.iter().copied().fold(f32::INFINITY, f32::min),
        xs.iter().copied().fold(f32::NEG_INFINITY, f32::max),
    );
    let (min_y, max_y) = (
        ys.iter().copied().fold(f32::INFINITY, f32::min),
        ys.iter().copied().fold(f32::NEG_INFINITY, f32::max),
    );
    let origin = Vec2::new(min_x, min_y).ok()?;
    let size = Size2::new(max_x - min_x, max_y - min_y).ok()?;
    Some(Rect::new(origin, size))
}

/// 문서 scene·선택·camera로 selection overlay를 만듭니다(모듈 문서).
///
/// `primary`가 scene에 있으면 그 상자에 handle을 둡니다. 없으면(또는 `None`) 마지막으로 찾은 선택 node가 primary입니다.
/// overlay layer의 revision은 문서 scene snapshot과 같습니다.
pub fn build_selection_overlay(
    scene: &SceneSnapshot,
    selection: &[StableId],
    primary: Option<StableId>,
    viewport: &Viewport,
    style: &OverlayStyle,
) -> Result<SelectionOverlay, OverlayError> {
    let nodes = nodes_by_source(scene);
    let view = viewport.view();
    let mut targets: Vec<OverlayTarget> = Vec::new();
    for &id in selection {
        let Some(node) = nodes.get(&id) else {
            continue;
        };
        if targets.iter().any(|target| target.node == id) {
            continue;
        }
        let world_quad = quad_of(&node.world_transform(), node.layout_rect())
            .ok_or(OverlayError::NonFinite { node: id })?;
        let view_quad = match world_quad.map(|p| view.apply(p).ok()) {
            [Some(a), Some(b), Some(c), Some(d)] => [a, b, c, d],
            _ => return Err(OverlayError::NonFinite { node: id }),
        };
        targets.push(OverlayTarget {
            node: id,
            world_quad,
            view_quad,
            primary: false,
        });
    }
    let primary_index = primary
        .and_then(|primary| targets.iter().position(|target| target.node == primary))
        .or_else(|| targets.len().checked_sub(1));
    if let Some(index) = primary_index {
        targets[index].primary = true;
    }

    let mut builder = SceneBuilder::new();
    let mut root = builder.root_scope();
    for target in &targets {
        let [a, b, c, d] = target.view_quad;
        let geometry = PathGeometry::new(vec![
            PathCommand::MoveTo(a),
            PathCommand::LineTo(b),
            PathCommand::LineTo(c),
            PathCommand::LineTo(d),
            PathCommand::Close,
        ])
        .map_err(|_| OverlayError::NonFinite { node: target.node })?;
        let layout_rect =
            bounds_of(&target.view_quad).ok_or(OverlayError::NonFinite { node: target.node })?;
        let mut node = NodeInput::new(target.node, Transform2D::identity());
        node.paint = ModifierPaint {
            strokes: vec![StrokeInput {
                paint: Paint::Solid(style.outline),
                width: style.outline_width,
            }],
            ..ModifierPaint::default()
        };
        builder.build_path(
            &mut root,
            &PathInput {
                node,
                layout_rect,
                geometry,
                fill_rule: FillRule::NonZero,
            },
        )?;
    }
    let mut hit = TransientLayer::new();
    let mut handle_count = 0;
    if let Some(index) = primary_index {
        let target = targets[index];
        let side = style.handle_half * 2.0;
        let size =
            Size2::new(side, side).map_err(|_| OverlayError::NonFinite { node: target.node })?;
        for corner in target.view_quad {
            let transform = Transform2D::new(
                1.0,
                0.0,
                0.0,
                1.0,
                corner.x() - style.handle_half,
                corner.y() - style.handle_half,
            )
            .map_err(|_| OverlayError::NonFinite { node: target.node })?;
            let frame = builder.begin_frame(
                &root,
                &FrameInput {
                    node: NodeInput::new(target.node, transform),
                    size,
                    corner_radii: CornerRadii::ZERO,
                    background: Some(style.handle_fill),
                    border: Some(BorderInput {
                        color: style.handle_stroke,
                        thickness: style.handle_stroke_width,
                    }),
                    clip: FrameClipInput::None,
                },
            )?;
            builder.finish_frame(&mut root, frame)?;
            handle_count += 1;
        }
        hit.push_selection_box(target.node, target.world_quad, style.hit);
    }
    let roots = builder.finish(root);
    let layer = SceneSnapshot::new(scene.document_revision(), scene.scene_revision(), roots)
        .map_err(OverlayError::Tree)?;
    Ok(SelectionOverlay {
        targets,
        layer,
        hit,
        handle_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::SceneContent;
    use crate::transient::{Corner, HandleKind};
    use stellar_core_types::color::ColorSpace;
    use stellar_core_types::revision::{DocumentRevision, SceneRevision};

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn color(r: f32, g: f32, b: f32) -> Color {
        Color::new(ColorSpace::Srgb, r, g, b, 1.0).unwrap()
    }

    fn style() -> OverlayStyle {
        OverlayStyle {
            outline: color(0.0, 0.4, 1.0),
            outline_width: 1.0,
            handle_fill: color(1.0, 1.0, 1.0),
            handle_stroke: color(0.0, 0.4, 1.0),
            handle_stroke_width: 1.0,
            handle_half: 4.0,
            hit: HandleMetrics::new(5.0, 4.0, 12.0).unwrap(),
        }
    }

    /// world (x, y)에 w×h Frame을 둔 문서 scene. 11 아래 자식 13.
    fn scene() -> SceneSnapshot {
        let mut builder = SceneBuilder::new();
        let mut root = builder.root_scope();
        let frame = |node: u64, x: f32, y: f32, w: f32, h: f32| FrameInput {
            node: NodeInput::new(
                sid(node),
                Transform2D::new(1.0, 0.0, 0.0, 1.0, x, y).unwrap(),
            ),
            size: Size2::new(w, h).unwrap(),
            corner_radii: CornerRadii::ZERO,
            background: Some(color(0.9, 0.9, 0.9)),
            border: None,
            clip: FrameClipInput::None,
        };
        let mut outer = builder
            .begin_frame(&root, &frame(11, 10.0, 20.0, 100.0, 50.0))
            .unwrap();
        let inner = builder
            .begin_frame(outer.scope(), &frame(13, 5.0, 5.0, 10.0, 10.0))
            .unwrap();
        builder.finish_frame(outer.scope_mut(), inner).unwrap();
        builder.finish_frame(&mut root, outer).unwrap();
        let other = builder
            .begin_frame(&root, &frame(12, 300.0, 0.0, 40.0, 40.0))
            .unwrap();
        builder.finish_frame(&mut root, other).unwrap();
        SceneSnapshot::new(DocumentRevision(3), SceneRevision(3), builder.finish(root)).unwrap()
    }

    fn viewport(zoom: f32, tx: f32, ty: f32) -> Viewport {
        Viewport::new(
            Size2::new(800.0, 600.0).unwrap(),
            Transform2D::new(zoom, 0.0, 0.0, zoom, tx, ty).unwrap(),
        )
        .unwrap()
    }

    fn p(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y).unwrap()
    }

    #[test]
    fn overlay_outlines_every_selected_node_and_handles_the_primary() {
        let scene = scene();
        let camera = viewport(2.0, 100.0, 50.0);
        let overlay = build_selection_overlay(
            &scene,
            &[sid(12), sid(13), sid(99)],
            Some(sid(13)),
            &camera,
            &style(),
        )
        .unwrap();
        // scene에 없는 99는 빠지고, 선택 순서를 지킵니다. 자식 13의 world 상자는 부모 위치를 더한 (15, 25)부터.
        let targets = overlay.targets();
        assert_eq!(targets.len(), 2);
        assert_eq!((targets[0].node, targets[0].primary), (sid(12), false));
        assert_eq!((targets[1].node, targets[1].primary), (sid(13), true));
        assert_eq!(targets[1].world_quad[0], p(15.0, 25.0));
        assert_eq!(targets[1].world_quad[2], p(25.0, 35.0));
        // view = world × 2 + (100, 50).
        assert_eq!(targets[1].view_quad[0], p(130.0, 100.0));
        assert_eq!(targets[1].view_quad[2], p(150.0, 120.0));

        // layer: 외곽선 path 2개 + primary 모서리 handle 4개, 자기 z-order 공간(1부터).
        let layer = overlay.layer();
        assert_eq!(layer.node_count(), 6);
        assert_eq!(overlay.handle_count(), 4);
        let nodes: Vec<_> = layer.roots().cloned().collect();
        assert_eq!(nodes[0].z().0, 1);
        assert!(matches!(nodes[0].content(), SceneContent::Path(_)));
        assert!(matches!(nodes[2].content(), SceneContent::Rect(_)));
        // handle은 view px 크기(반변 4)로 모서리 중심에 놓입니다.
        let first_handle = &nodes[2];
        assert_eq!(
            first_handle.layout_rect().size(),
            Size2::new(8.0, 8.0).unwrap()
        );
        assert_eq!(
            first_handle.world_transform().components(),
            (1.0, 0.0, 0.0, 1.0, 126.0, 96.0)
        );

        // hit test: primary 상자 오른쪽 아래 모서리 view 점은 크기 조절 모서리 handle.
        let hit = overlay
            .hit_layer()
            .hit(&camera, p(150.0, 120.0))
            .expect("모서리 handle");
        assert_eq!(
            hit.handle.kind,
            HandleKind::ResizeCorner(Corner::BottomRight)
        );
        assert_eq!(hit.handle.target, sid(13));
    }

    #[test]
    fn handles_keep_their_screen_size_at_any_zoom() {
        let scene = scene();
        for zoom in [0.25f32, 1.0, 8.0] {
            let overlay = build_selection_overlay(
                &scene,
                &[sid(11)],
                None,
                &viewport(zoom, 0.0, 0.0),
                &style(),
            )
            .unwrap();
            let handles: Vec<_> = overlay.layer().roots().skip(1).cloned().collect();
            assert_eq!(handles.len(), 4);
            for handle in handles {
                assert_eq!(
                    handle.layout_rect().size(),
                    Size2::new(8.0, 8.0).unwrap(),
                    "zoom {zoom}"
                );
            }
            assert!(overlay.targets()[0].primary, "primary 없으면 마지막 선택");
        }
    }

    #[test]
    fn document_scene_never_receives_overlay_nodes() {
        // 검증 기준(XII.4.4, scene 쪽): overlay는 문서 scene snapshot에 node를 더하지 않습니다.
        let scene = scene();
        let hash = scene.semantic_hash();
        let count = scene.node_count();
        let overlay = build_selection_overlay(
            &scene,
            &[sid(11), sid(12)],
            None,
            &viewport(1.0, 0.0, 0.0),
            &style(),
        )
        .unwrap();
        assert_eq!(scene.semantic_hash(), hash);
        assert_eq!(scene.node_count(), count);
        assert_eq!(overlay.layer().node_count(), 2 + 4);
        assert_ne!(overlay.layer().semantic_hash(), hash);
        // 빈 선택은 빈 layer.
        let empty =
            build_selection_overlay(&scene, &[], None, &viewport(1.0, 0.0, 0.0), &style()).unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.layer().node_count(), 0);
        assert!(empty.hit_layer().is_empty());
        // 음수 두께는 구조화 오류입니다.
        let mut bad = style();
        bad.outline_width = -1.0;
        assert!(matches!(
            build_selection_overlay(&scene, &[sid(11)], None, &viewport(1.0, 0.0, 0.0), &bad),
            Err(OverlayError::Build(_))
        ));
    }
}

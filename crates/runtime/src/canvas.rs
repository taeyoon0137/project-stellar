//! 활성 View의 고정 Free Frame과 사각형 Shape를 평가한 창별 불변 그리기 목록.

pub mod text;

use std::sync::Arc;

use stellar_commands::view_tabs::CanvasViewport;
use stellar_core_types::color::Color;
use stellar_core_types::ids::StableId;
use stellar_core_types::revision::DocumentRevision;
use stellar_core_types::value::Transform2D;
use stellar_document::frame::Clipping;
use stellar_document::model::DocumentModel;
use stellar_document::records::NodePayload;
use stellar_document::shape::RectangleParams;
use stellar_scene::bounds::Aabb;
use stellar_scene::builder::{
    BorderInput, FragmentInput, FrameClipInput, FrameInput, NodeInput, SceneBuilder,
};
use stellar_scene::node::{CornerRadii, RectPrimitive, SceneContent};

use crate::window_session::ProjectWindowSession;

pub(crate) mod shape;

/// painter 순서의 사각형 하나. parent는 투명한 Fragment를 펼친 뒤의 가장 가까운 Frame입니다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanvasFrame {
    /// Shape Rectangle이면 true입니다. Frame과 같은 사각형 painter를 사용합니다.
    pub rectangle_shape: bool,
    /// 문서 node 식별자.
    pub id: StableId,
    /// 같은 목록의 Frame 부모 index.
    pub parent: Option<u32>,
    /// local → world affine.
    pub world: Transform2D,
    /// local 사각형과 실효 모서리 반경.
    pub geometry: RectPrimitive,
    /// 단색 배경.
    pub background: Option<Color>,
    /// 안쪽 단색 테두리.
    pub border: Option<stellar_document::frame::Border>,
    /// 자식이 이 Frame의 바깥 경계에서 잘리는지.
    pub clip_children: bool,
    /// 조상의 잠금·숨김을 반영한 입력 대상 여부.
    pub hit_target: bool,
}

/// 동일한 문서 revision·View는 카메라·선택·다른 창의 변경 때 그대로 공유합니다.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanvasScene {
    /// 아래에서 위로 칠하는 Frame·사각형 Shape 목록.
    pub frames: Vec<CanvasFrame>,
    /// 해석하지 못한 subtree 수. 그 자손도 잘못된 위치·clip으로 표시하지 않습니다.
    pub skipped: u32,
}

#[derive(Debug, Default)]
pub(crate) struct CanvasCache {
    latest: Option<(DocumentRevision, StableId, Arc<CanvasScene>)>,
}

impl CanvasCache {
    pub(crate) fn current(&mut self, session: &ProjectWindowSession) -> Option<Arc<CanvasScene>> {
        let view = session.tabs().active_view()?;
        let state = session.document()?;
        if let Some((revision, id, scene)) = &self.latest {
            if *revision == state.revision() && *id == view {
                return Some(scene.clone());
            }
        }
        let model = state.model();
        let root = model
            .definition()
            .pages
            .iter()
            .flat_map(|p| &p.views)
            .find(|v| v.id == view)?
            .root_frame;
        let scene = Arc::new(CanvasScene::from_root(model, root));
        self.latest = Some((state.revision(), view, scene.clone()));
        Some(scene)
    }
}

impl CanvasScene {
    /// 최초 Frame을 창의 logical point 안에 배치합니다. 100%보다 키우지 않습니다.
    pub(crate) fn root_viewport(&self, width: f64, height: f64) -> Option<CanvasViewport> {
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return None;
        }
        let root = self.frames.first()?;
        let bounds = Aabb::from_rect(root.geometry.rect).transformed(&root.world)?;
        let w = f64::from(bounds.max_x()) - f64::from(bounds.min_x());
        let h = f64::from(bounds.max_y()) - f64::from(bounds.min_y());
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        // 좁은 창에서도 그릴 공간을 남기고 일반 창에서는 가장자리마다 24pt를 둡니다.
        let margin = 24.0_f64.min(width.min(height) * 0.1);
        let scale = ((width - 2.0 * margin) / w)
            .min((height - 2.0 * margin) / h)
            .min(1.0);
        CanvasViewport::new(
            f64::from(bounds.min_x()) + w / 2.0,
            f64::from(bounds.min_y()) + h / 2.0,
            scale,
        )
        .ok()
    }

    fn from_root(model: &DocumentModel, root: StableId) -> Self {
        let mut result = Self::default();
        let mut builder = SceneBuilder::new();
        let mut pending = vec![(root, builder.root_scope(), None)];
        while let Some((id, parent_scope, parent)) = pending.pop() {
            let Some(fields) = model.runtime_id(id).and_then(|r| model.columns().get(r)) else {
                continue;
            };
            if !fields.visible {
                continue;
            }
            let payload = model.payload(id);
            if let Some(NodePayload::Shape(stellar_document::shape::ShapeGeometry::Rectangle(
                shape,
            ))) = payload
            {
                if let Some(rectangle) = shape::rectangle(
                    model,
                    id,
                    shape,
                    &parent_scope,
                    parent,
                    fields.locked,
                    &mut builder,
                ) {
                    result.frames.push(rectangle);
                } else {
                    result.skipped += 1;
                }
                continue;
            }
            if model.modifier_stack(id).is_some() {
                result.skipped += 1;
                continue;
            }
            let (scope, next_parent) = match payload {
                Some(NodePayload::Fragment(_)) => {
                    let scope = builder.begin_fragment(
                        &parent_scope,
                        &FragmentInput {
                            source: id,
                            visible: true,
                            locked: fields.locked,
                        },
                    );
                    (scope.scope().clone(), parent)
                }
                Some(NodePayload::Frame(frame))
                    if frame.layout_mode.is_none()
                        && !matches!(frame.clipping, Clipping::Path(_)) =>
                {
                    let radius = RectangleParams {
                        size: frame.size,
                        corner_radius: frame.corner_radius,
                    }
                    .effective_corner_radius();
                    let Ok(radii) = CornerRadii::new(radius.0, radius.1, radius.2, radius.3) else {
                        result.skipped += 1;
                        continue;
                    };
                    let mut node = NodeInput::new(id, frame.transform);
                    node.locked = fields.locked;
                    let clip_children = frame.clipping == Clipping::FrameBounds;
                    let input = FrameInput {
                        node,
                        size: frame.size,
                        corner_radii: radii,
                        background: frame.background,
                        border: frame.border.map(|b| BorderInput {
                            color: b.color,
                            thickness: b.thickness(),
                        }),
                        clip: if clip_children {
                            FrameClipInput::Bounds
                        } else {
                            FrameClipInput::None
                        },
                    };
                    let Ok(scope) = builder.begin_frame(&parent_scope, &input) else {
                        result.skipped += 1;
                        continue;
                    };
                    let spec = scope.spec();
                    let SceneContent::Rect(geometry) = &spec.content else {
                        unreachable!("Frame primitive")
                    };
                    if spec.world_transform.inverse().is_none() {
                        continue;
                    }
                    if Aabb::from_rect(geometry.rect)
                        .transformed(&spec.world_transform)
                        .is_none()
                    {
                        result.skipped += 1;
                        continue;
                    }
                    let index = result.frames.len() as u32;
                    result.frames.push(CanvasFrame {
                        rectangle_shape: false,
                        id,
                        parent,
                        world: spec.world_transform,
                        geometry: *geometry,
                        background: frame.background,
                        border: frame.border,
                        clip_children,
                        hit_target: spec.hit.is_hit_target(),
                    });
                    (scope.scope().clone(), Some(index))
                }
                _ => {
                    result.skipped += 1;
                    continue;
                }
            };
            pending.extend(
                model
                    .children_in_order(id)
                    .into_iter()
                    .rev()
                    .map(|child| (child, scope.clone(), next_parent)),
            );
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_session::test_fixtures::{
        identity, sample_document, saved_project_with, sid, window,
    };
    use stellar_core_types::color::ColorSpace;
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::value::{Size2, Vec2};
    use stellar_document::fragment::FragmentPayload;
    use stellar_document::frame::{Border, CornerRadius, FramePayload};
    use stellar_document::records::NodeRecord;

    fn frame(w: f32, h: f32, x: f32, y: f32, rgb: [f32; 3]) -> FramePayload {
        FramePayload {
            size: Size2::new(w, h).unwrap(),
            transform: Transform2D::new(1., 0., 0., 1., x, y).unwrap(),
            background: Some(Color::new(ColorSpace::Srgb, rgb[0], rgb[1], rgb[2], 1.).unwrap()),
            ..FramePayload::default()
        }
    }

    #[test]
    fn initial_fit_uses_world_bounds_both_axes_and_zoom_limits() {
        let mut model = sample_document();
        let mut rotated = frame(200.0, 60.0, 0.0, 0.0, [1.0; 3]);
        rotated.transform = Transform2D::new(0.0, 1.0, -1.0, 0.0, 600.0, 50.0).unwrap();
        model
            .set_payload(sid(10), NodePayload::Frame(rotated))
            .unwrap();
        let scene = CanvasScene::from_root(&model, sid(10));
        for (width, height, expected) in
            [(100.0, 120.0, 0.5), (30.0, 400.0, 0.4), (800.0, 900.0, 1.0)]
        {
            let camera = scene.root_viewport(width, height).unwrap();
            assert_eq!((camera.center_x(), camera.center_y()), (570.0, 150.0));
            assert_eq!(camera.zoom(), expected);
        }
        assert!(scene.root_viewport(f64::NAN, 400.0).is_none());
        assert!(scene.root_viewport(0.0, 400.0).is_none());
        assert!(CanvasScene::default().root_viewport(800.0, 600.0).is_none());
    }

    #[test]
    #[ignore = "requires a Release macOS app and writes the 50,000-Frame benchmark project"]
    fn profile_saved_canvas_50000_frames() {
        let app =
            std::env::var_os("STELLAR_CANVAS_PROFILE_APP").expect("set STELLAR_CANVAS_PROFILE_APP");
        let mut model = sample_document();
        model.remove_leaf(sid(11)).unwrap();
        model.remove_leaf(sid(12)).unwrap();
        let mut root = frame(800., 600., -400., -300., [1.; 3]);
        root.clipping = Clipping::Disabled;
        model
            .set_payload(sid(10), NodePayload::Frame(root))
            .unwrap();
        let flat = std::env::var_os("STELLAR_CANVAS_PROFILE_FLAT").is_some();
        // 같은 world 좌표의 Frame을 100개 그룹 또는 한 root의 직계 자식으로 배치합니다.
        for i in 0..49_999_u32 {
            let group = i / 500;
            let member = i % 500;
            let (parent, x, y, w, h) = if member == 0 {
                (10, group as f32 * 2500., 0., 400., 300.)
            } else {
                (
                    1000 + u64::from(group * 500),
                    (member % 25) as f32 * 49.,
                    (member / 25) as f32 * 39.,
                    16.,
                    14.,
                )
            };
            let (parent, x) = if flat && member != 0 {
                (10, group as f32 * 2500. + x)
            } else {
                (parent, x)
            };
            let mut f = frame(w, h, x, y, [0.1, 0.25 + (member % 7) as f32 / 10., 0.55]);
            if member == 0 {
                f.clipping = Clipping::Disabled;
            }
            let mut key = i.to_be_bytes().to_vec();
            key.push(1);
            model
                .insert_node(NodeRecord {
                    id: sid(1000 + u64::from(i)),
                    parent: Some(sid(parent)),
                    order_key: OrderKey::from_bytes(key).unwrap(),
                    visible: true,
                    locked: false,
                    display_name: format!("Frame {i}"),
                    payload: NodePayload::Frame(f),
                })
                .unwrap();
        }
        let dir = saved_project_with("canvas-profile", &[&model]);
        let status = std::process::Command::new(app)
            .arg("--canvas-culling-check")
            .arg(dir.path())
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn saved_frames_open_in_window_painter_order() {
        let mut model = sample_document();
        let root = frame(800., 600., 0., 0., [1., 1., 1.]);
        let mut red = frame(80., 60., 10., 12., [1., 0., 0.]);
        red.corner_radius = CornerRadius::new(8., 0., 0., 4.).unwrap();
        red.border =
            Some(Border::new(Color::new(ColorSpace::Srgb, 0., 0., 0., 1.).unwrap(), 2.).unwrap());
        model
            .set_payload(sid(10), NodePayload::Frame(root))
            .unwrap();
        model.set_payload(sid(11), NodePayload::Frame(red)).unwrap();
        model
            .set_payload(
                sid(12),
                NodePayload::Frame(frame(24., 15., 55., 48., [0., 1., 0.])),
            )
            .unwrap();
        let mut outside = frame(40., 40., 1000., 20., [0.; 3]);
        outside.clipping = Clipping::Disabled;
        let mut rotated = frame(34., 18., 0., 0., [1., 1., 0.]);
        rotated.transform = Transform2D::new(0., 1., -1., 0., -850., 90.).unwrap();
        rotated.border =
            Some(Border::new(Color::new(ColorSpace::Srgb, 0., 0., 0., 1.).unwrap(), 2.).unwrap());
        let mut drag_parent = outside.clone();
        drag_parent.transform = Transform2D::new(1., 0., 0., 1., 1200., 20.).unwrap();
        for (n, parent, visible, payload) in [
            (13, 11, true, NodePayload::Fragment(FragmentPayload)),
            (
                14,
                13,
                true,
                NodePayload::Frame(frame(60., 18., 60., 8., [0., 0., 1.])),
            ),
            (
                15,
                10,
                false,
                NodePayload::Frame(frame(120., 80., 0., 0., [0., 0., 0.])),
            ),
            (16, 10, true, NodePayload::Frame(outside)),
            (17, 16, true, NodePayload::Frame(rotated)),
            (18, 10, true, NodePayload::Frame(drag_parent)),
            (
                19,
                18,
                true,
                NodePayload::Frame(frame(25., 20., 10., 120., [0., 1., 1.])),
            ),
            (
                21,
                10,
                true,
                NodePayload::Frame(frame(40., 40., 1000., 20., [0.; 3])),
            ),
            (
                22,
                21,
                true,
                NodePayload::Frame(frame(25., 20., -820., 140., [0., 1., 1.])),
            ),
        ] {
            model
                .insert_node(NodeRecord {
                    id: sid(n),
                    parent: Some(sid(parent)),
                    order_key: OrderKey::from_bytes(vec![n as u8 + 0x40]).unwrap(),
                    visible,
                    locked: false,
                    display_name: format!("n{n}"),
                    payload,
                })
                .unwrap();
        }
        let dir = saved_project_with("visible-frames", &[&model]);
        let mut session = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        let mut cache = CanvasCache::default();
        let scene = cache.current(&session).unwrap();
        assert_eq!(scene.skipped, 0);
        assert_eq!(
            scene.frames.iter().map(|r| r.id).collect::<Vec<_>>(),
            [10, 11, 14, 12, 16, 17, 18, 19, 21, 22].map(sid)
        );
        assert_eq!(scene.frames[2].parent, Some(1));
        assert_eq!(
            scene.frames[2].world.apply(Vec2::ZERO).unwrap(),
            Vec2::new(70., 20.).unwrap()
        );
        session.pan(13., -22.).unwrap();
        assert!(Arc::ptr_eq(&scene, &cache.current(&session).unwrap()));
        session.open_view(sid(4)).unwrap();
        assert_eq!(cache.current(&session).unwrap().frames[0].id, sid(20));
        if let Some(app) = std::env::var_os("STELLAR_TEST_CANVAS_APP") {
            let status = std::process::Command::new(app)
                .arg("--visible-frames-check")
                .arg(dir.path())
                .status()
                .unwrap();
            assert!(
                status.success(),
                "AppKit saved Frame bitmap check: {status}"
            );
        }
    }
}

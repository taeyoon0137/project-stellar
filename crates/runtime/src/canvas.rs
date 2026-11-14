//! 활성 View의 고정 Free Frame을 scene builder로 평가한 창별 불변 그리기 목록.

use std::sync::Arc;

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

/// painter 순서의 Frame 하나. parent는 투명한 Fragment를 펼친 뒤의 가장 가까운 Frame입니다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanvasFrame {
    /// 문서 Frame 식별자.
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
    /// 아래에서 위로 칠하는 Frame 목록.
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
            if model.modifier_stack(id).is_some() {
                result.skipped += 1;
                continue;
            }
            let (scope, next_parent) = match model.payload(id) {
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
        ] {
            model
                .insert_node(NodeRecord {
                    id: sid(n),
                    parent: Some(sid(parent)),
                    order_key: OrderKey::from_bytes(vec![n as u8]).unwrap(),
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
            [sid(10), sid(11), sid(14), sid(12)]
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

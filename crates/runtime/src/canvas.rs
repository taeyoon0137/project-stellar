//! 활성 View의 고정 Free Frame과 사각형 Shape를 평가한 창별 불변 그리기 목록.

pub mod text;

use std::sync::Arc;

pub use shape::CanvasShapePath;
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
use stellar_storage::hash::ContentHash;

use crate::window_session::ProjectWindowSession;

mod component;
mod image;
pub(crate) mod shape;

/// painter 순서의 사각형 하나. parent는 투명한 Fragment를 펼친 뒤의 가장 가까운 Frame입니다.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanvasFrame {
    /// Shape Rectangle이면 true입니다. Frame과 같은 사각형 painter를 사용합니다.
    pub rectangle_shape: bool,
    /// 불변 scene의 경로 행으로 그리는 Shape입니다.
    pub path_shape: bool,
    /// 저장한 font/style로 layout한 Text 행.
    pub text_node: bool,
    /// 실제 문서의 ComponentInstance를 표시하는 투영 행입니다.
    pub instance_frame: bool,
    /// PNG asset 참조. 픽셀은 worker에서 검증·디코드합니다.
    pub image_asset: Option<StableId>,
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
    /// 아래에서 위로 칠하는 Frame·Rectangle·ComponentInstance·Text 목록.
    pub frames: Vec<CanvasFrame>,
    /// Frame 행에는 할당하지 않는 도형별 canonical 경로입니다.
    pub shapes: std::collections::BTreeMap<u32, CanvasShapePath>,
    /// Text 행 번호별 layout 및 glyph path.
    pub text: std::collections::BTreeMap<u32, Arc<text::DocumentText>>,
    /// 해석하지 못한 subtree 수. 그 자손도 잘못된 위치·clip으로 표시하지 않습니다.
    pub skipped: u32,
}

/// 파일 이름과 원본 bytes 전체가 검증된 창별 정의 집합. worker와 장면이 같은 자료를 읽습니다.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CanvasComponentLibrary {
    pub(crate) revision: stellar_ops::revision::ProjectRevision,
    pub(crate) source_hash: ContentHash,
    pub(crate) definitions: stellar_document::component::definition::ComponentLibrary,
}

/// 게시한 scene의 재사용 조건: 문서 revision, 활성 View, 진행 중 Text 조합(token, 갱신 순번).
type SceneKey = (DocumentRevision, StableId, Option<(u64, u64)>);

#[derive(Debug)]
pub(crate) struct CanvasCache {
    fonts: text::document::FontCache,
    saved_text: text::saved::SavedTextLayouts,
    library_request: u64,
    latest: Option<(SceneKey, Arc<CanvasScene>)>,
    pub(crate) library: Option<Arc<CanvasComponentLibrary>>,
}

#[cfg(test)]
impl Default for CanvasCache {
    fn default() -> Self {
        Self::new(text::fonts::FaceLoader::inline())
    }
}

impl CanvasCache {
    pub(crate) fn new(fonts: Arc<text::fonts::FaceLoader>) -> Self {
        Self {
            fonts: text::document::FontCache::new(fonts),
            saved_text: Default::default(),
            library_request: 0,
            latest: None,
            library: None,
        }
    }

    /// 새 글꼴 face를 읽었습니다. 대체 글꼴로 만든 layout과 게시한 scene을 버립니다.
    pub(crate) fn fonts_loaded(&mut self) {
        self.latest = None;
        self.saved_text = Default::default();
    }
    pub(crate) fn update_library(
        &mut self,
        request: u64,
        next: Option<Arc<CanvasComponentLibrary>>,
    ) {
        if request < self.library_request {
            return;
        }
        self.library_request = request;
        if self.library.as_ref().map(|v| v.source_hash) != next.as_ref().map(|v| v.source_hash) {
            self.latest = None;
        }
        self.library = next;
    }

    /// `text`는 진행 중인 IME 조합의 미리보기입니다(ADR-0067 §6). 같은 revision·View·조합이면 scene을 공유합니다.
    pub(crate) fn current(
        &mut self,
        session: &ProjectWindowSession,
        text: Option<&text::TextPreview>,
    ) -> Option<Arc<CanvasScene>> {
        let view = session.tabs().active_view()?;
        let state = session.document()?;
        let key = text.map(|t| t.key);
        let scene_key = (state.revision(), view, key);
        if let Some((previous, scene)) = &self.latest {
            if *previous == scene_key {
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
        self.fonts
            .sync_registrations(&model.definition().text_styles);
        self.saved_text.begin_view(&model.definition().text_styles);
        let scene = Arc::new(CanvasScene::with_library(
            model,
            root,
            self.library.as_ref().map(|v| &v.definitions),
            Some((
                session.identity().canonical_target(),
                &mut self.fonts,
                &mut self.saved_text,
            )),
            text,
        ));
        self.saved_text.end_view();
        self.latest = Some((scene_key, scene.clone()));
        Some(scene)
    }
}

impl CanvasScene {
    /// 활성 View의 root Frame(painter 순서 첫 행)을 창의 logical point 안에 배치합니다. 100%보다 키우지 않습니다.
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

    fn push_frame(
        &mut self,
        builder: &mut SceneBuilder,
        parent_scope: &stellar_scene::builder::SceneScope,
        parent: Option<u32>,
        id: StableId,
        frame: &stellar_document::frame::FramePayload,
        locked: bool,
    ) -> Option<(stellar_scene::builder::SceneScope, u32)> {
        let radius = RectangleParams {
            size: frame.size,
            corner_radius: frame.corner_radius,
        }
        .effective_corner_radius();
        let radii = CornerRadii::new(radius.0, radius.1, radius.2, radius.3).ok()?;
        let mut node = NodeInput::new(id, frame.transform);
        node.locked = locked;
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
        let scope = builder.begin_frame(parent_scope, &input).ok()?;
        let spec = scope.spec();
        let SceneContent::Rect(geometry) = &spec.content else {
            unreachable!("Frame primitive")
        };
        spec.world_transform.inverse()?;
        Aabb::from_rect(geometry.rect).transformed(&spec.world_transform)?;
        let index = self.frames.len() as u32;
        self.frames.push(CanvasFrame {
            rectangle_shape: false,
            path_shape: false,
            instance_frame: false,
            text_node: false,
            image_asset: None,
            id,
            parent,
            world: spec.world_transform,
            geometry: *geometry,
            background: frame.background,
            border: frame.border,
            clip_children,
            hit_target: spec.hit.is_hit_target(),
        });
        Some((scope.scope().clone(), index))
    }

    #[cfg(test)]
    fn from_root(model: &DocumentModel, root: StableId) -> Self {
        Self::with_library(model, root, None, None, None)
    }

    /// 저장된 Text 또는 조합 미리보기 Text 한 행을 layout해 추가합니다. 실패하면 skipped로 셉니다.
    #[allow(clippy::too_many_arguments)]
    fn push_text(
        &mut self,
        builder: &mut SceneBuilder,
        parent_scope: &stellar_scene::builder::SceneScope,
        parent: Option<u32>,
        model: &DocumentModel,
        id: StableId,
        locked: bool,
        payload: &stellar_document::text::TextPayload,
        modifiers: Option<&stellar_document::modifier::ModifierStack>,
        fonts: &mut Option<(
            &std::path::Path,
            &mut text::document::FontCache,
            &mut text::saved::SavedTextLayouts,
        )>,
    ) {
        let prepared = fonts.as_mut().and_then(|(root, faces, saved)| {
            saved.layout(faces, root, model, id, payload, modifiers)
        });
        let row = prepared.and_then(|(text, size, transform)| {
            let mut node = NodeInput::new(id, transform);
            node.locked = locked;
            let item = builder
                .build_text(
                    &mut parent_scope.clone(),
                    &stellar_scene::builder::TextInput {
                        node,
                        layout: stellar_scene::node::TextLayoutKey::from_bytes(id.to_bytes()),
                        size,
                    },
                )
                .ok()?;
            let world = item.world_transform();
            world.inverse()?;
            let rect =
                stellar_core_types::value::Rect::new(stellar_core_types::value::Vec2::ZERO, size);
            Aabb::from_rect(rect).transformed(&world)?;
            Some((
                CanvasFrame {
                    rectangle_shape: false,
                    path_shape: false,
                    instance_frame: false,
                    text_node: true,
                    image_asset: None,
                    id,
                    parent,
                    world,
                    geometry: RectPrimitive {
                        rect,
                        radii: CornerRadii::ZERO,
                    },
                    background: Some(text.fill),
                    border: None,
                    clip_children: false,
                    hit_target: item.hit().is_hit_target(),
                },
                text,
            ))
        });
        if let Some((frame, text)) = row {
            self.text.insert(self.frames.len() as u32, text);
            self.frames.push(frame);
        } else {
            self.skipped += 1;
        }
    }

    fn with_library(
        model: &DocumentModel,
        root: StableId,
        library: Option<&stellar_document::component::definition::ComponentLibrary>,
        mut fonts: Option<(
            &std::path::Path,
            &mut text::document::FontCache,
            &mut text::saved::SavedTextLayouts,
        )>,
        text: Option<&text::TextPreview>,
    ) -> Self {
        let mut result = Self::default();
        let mut builder = SceneBuilder::new();
        let new_text =
            text.and_then(|t| t.new_in.as_ref().map(|(parent, stack)| (t, *parent, stack)));
        let mut pending = vec![(root, builder.root_scope(), None)];
        while let Some((id, parent_scope, parent)) = pending.pop() {
            if let Some((preview, _, stack)) = new_text.filter(|(t, _, _)| t.node == id) {
                // 확정 전 새 Text는 문서에 없으므로 선택·hit 대상이 아닙니다.
                result.push_text(
                    &mut builder,
                    &parent_scope,
                    parent,
                    model,
                    id,
                    true,
                    &preview.payload,
                    Some(stack),
                    &mut fonts,
                );
                continue;
            }
            let Some(fields) = model.runtime_id(id).and_then(|r| model.columns().get(r)) else {
                continue;
            };
            if !fields.visible {
                continue;
            }
            let payload = model.payload(id);
            if let Some(NodePayload::Text(stored)) = payload {
                let payload = text
                    .filter(|t| t.node == id && t.new_in.is_none())
                    .map_or(stored, |t| &t.payload);
                result.push_text(
                    &mut builder,
                    &parent_scope,
                    parent,
                    model,
                    id,
                    fields.locked,
                    payload,
                    model.modifier_stack(id),
                    &mut fonts,
                );
                continue;
            }
            if let Some(NodePayload::Shape(shape)) = payload {
                if let Some((frame, path)) = shape::project(
                    model,
                    id,
                    shape,
                    &parent_scope,
                    parent,
                    fields.locked,
                    &mut builder,
                ) {
                    if let Some(path) = path {
                        result.shapes.insert(result.frames.len() as u32, path);
                    }
                    result.frames.push(frame);
                } else {
                    result.skipped += 1;
                }
                continue;
            }
            if let Some(NodePayload::Image(payload)) = payload {
                if let Some(row) = image::project(
                    model,
                    id,
                    payload,
                    &parent_scope,
                    parent,
                    fields.locked,
                    &mut builder,
                ) {
                    result.frames.push(row);
                } else {
                    result.skipped += 1;
                }
                continue;
            }
            if let Some(NodePayload::ComponentInstance(instance)) = payload {
                let start = result.frames.len();
                let entry = library.and_then(|lib| lib.get(instance.component()));
                let projected = entry
                    .filter(|_| instance.has_no_overrides())
                    .and_then(|entry| {
                        component::project(
                            &mut result,
                            &mut builder,
                            &parent_scope,
                            parent,
                            id,
                            fields.locked,
                            model.modifier_stack(id),
                            entry,
                        )
                    });
                if projected.is_none() {
                    result.frames.truncate(start);
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
                    let Some((scope, index)) = result.push_frame(
                        &mut builder,
                        &parent_scope,
                        parent,
                        id,
                        frame,
                        fields.locked,
                    ) else {
                        result.skipped += 1;
                        continue;
                    };
                    (scope, Some(index))
                }
                _ => {
                    result.skipped += 1;
                    continue;
                }
            };
            // 새 Text 미리보기는 부모의 마지막 자식 자리(가장 위)에 그립니다.
            if let Some((preview, _, _)) = new_text.filter(|(_, p, _)| *p == id) {
                pending.push((preview.node, scope.clone(), next_parent));
            }
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
        let scene = cache.current(&session, None).unwrap();
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
        assert!(Arc::ptr_eq(&scene, &cache.current(&session, None).unwrap()));
        session.open_view(sid(4)).unwrap();
        assert_eq!(cache.current(&session, None).unwrap().frames[0].id, sid(20));
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

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
use stellar_document::image::FitMode;
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
/// 문서 → 증분 scene source 변환 (XI.2.4, XI.2.5, XII.10.30).
pub mod scene_source;
pub use image::image_placement;
/// 선택 엔진의 hit 입력: 커서 아래 node 목록과 node world 상자 (XII.10.3, XII.10.5).
pub mod hit;
/// 캔버스 클릭 선택과 편집 범위 (XII.10.3, ADR-0104).
pub mod select;
mod select_fixtures;
/// 깊은 선택과 편집 범위 이동 (XII.10.4, ADR-0104).
pub mod select_navigate;
/// select-through와 겹친 레이어 목록 (XII.10.5, ADR-0104).
pub mod select_through;
pub(crate) mod shape;
/// 캔버스 도구 10종과 도구 상태 기계 (XII.10.1, ADR-0104).
pub mod tool;
/// 캔버스 이동·크기 단위와 핸들 숨김 기준 (XII.10.2, ADR-0104).
pub mod tool_units;

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
    /// 이미지 상자 안의 CSS object-fit 배치 방식. Image 행에만 있습니다.
    pub image_fit: Option<FitMode>,
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
    /// 읽을 때 쓴 symlink 정책. 신뢰를 철회하면 외부 target을 읽은 결과를 버립니다.
    pub(crate) access: stellar_ops::component_source::ExternalAccess,
    pub(crate) definitions: stellar_document::component::definition::ComponentLibrary,
}

/// 게시한 scene의 재사용 조건: 문서 revision, 활성 View, 진행 중 Text 조합(token, 갱신 순번).
type SceneKey = (DocumentRevision, StableId, Option<(u64, u64)>);

/// 컴포넌트 원본 읽기를 시작한 순서. 늦게 끝난 이전 읽기가 새 결과를 덮지 않게 합니다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct LibraryOrder(pub(crate) u64);

#[derive(Debug)]
pub(crate) struct CanvasCache {
    fonts: text::document::FontCache,
    saved_text: text::saved::SavedTextLayouts,
    library_order: LibraryOrder,
    latest: Option<(SceneKey, Arc<CanvasScene>)>,
    pub(crate) library: Option<Arc<CanvasComponentLibrary>>,
}

#[cfg(test)]
impl Default for CanvasCache {
    fn default() -> Self {
        Self::new(text::fonts::FaceLoader::inline())
    }
}

/// 저장본과 같은 문서를 보는 창들이 함께 쓰는 표시 목록(document thread 전용).
///
/// 창 문서의 revision 번호는 창마다 따로 셉니다. 그래서 revision 번호만으로는 두 창의 내용이 같은지 알 수
/// 없습니다. 저장하지 않은 변경이 없는 창의 문서는 loaded generation과 내용이 같습니다(Save Now가 저장한
/// revision과 generation을 함께 갱신). 이 경우 같은 프로젝트·generation·문서·View·컴포넌트 원본 hash면 같은
/// 표시 목록이므로 한 번만 평가해 공유합니다. 편집 중인 창은 자기 목록을 따로 평가합니다.
#[derive(Debug, Default)]
pub(crate) struct SharedScenes(
    std::collections::HashMap<SharedSceneKey, std::sync::Weak<CanvasScene>>,
);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SharedSceneKey {
    project: std::path::PathBuf,
    manifest: ContentHash,
    document: StableId,
    view: StableId,
    library: Option<ContentHash>,
}

impl SharedScenes {
    /// 새 글꼴 face를 읽어 대체 글꼴로 평가한 목록을 더 쓰지 않습니다([`CanvasCache::fonts_loaded`]).
    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }

    fn get(&self, key: &SharedSceneKey) -> Option<Arc<CanvasScene>> {
        self.0.get(key).and_then(std::sync::Weak::upgrade)
    }

    fn insert(&mut self, key: SharedSceneKey, scene: &Arc<CanvasScene>) {
        // 닫힌 창·지난 generation의 항목은 다른 창이 가리키지 않으면 여기서 정리합니다.
        self.0.retain(|_, scene| scene.strong_count() > 0);
        self.0.insert(key, Arc::downgrade(scene));
    }
}

impl CanvasCache {
    pub(crate) fn new(fonts: Arc<text::fonts::FaceLoader>) -> Self {
        Self {
            fonts: text::document::FontCache::new(fonts),
            saved_text: Default::default(),
            library_order: LibraryOrder::default(),
            latest: None,
            library: None,
        }
    }

    /// 새 글꼴 face를 읽었습니다. 대체 글꼴로 만든 layout과 게시한 scene을 버립니다. 버린 scene이 있으면 true입니다.
    pub(crate) fn fonts_loaded(&mut self) -> bool {
        self.saved_text = Default::default();
        self.latest.take().is_some()
    }

    /// `order`에 시작한 읽기 결과로 정의 집합을 바꿉니다. 더 늦게 시작한 읽기가 이미 반영됐으면 버립니다.
    /// 원본 hash가 바뀌었으면 true입니다.
    pub(crate) fn update_library(
        &mut self,
        order: LibraryOrder,
        next: Option<Arc<CanvasComponentLibrary>>,
    ) -> bool {
        if order < self.library_order {
            return false;
        }
        self.library_order = order;
        self.replace_library(next)
    }

    /// 신뢰 철회처럼 진행 중인 읽기와 무관하게 정의 집합을 비웁니다. 외부 target을 읽은 정의를 버리고, 셸이
    /// 다시 요청한 읽기가 외부 target을 읽지 않는 정책으로 정의를 채웁니다. 그 사이 끝나는 읽기는 반영 시점의
    /// 신뢰를 다시 확인합니다.
    pub(crate) fn clear_library(&mut self) -> bool {
        self.replace_library(None)
    }

    fn replace_library(&mut self, next: Option<Arc<CanvasComponentLibrary>>) -> bool {
        let changed =
            self.library.as_ref().map(|v| v.source_hash) != next.as_ref().map(|v| v.source_hash);
        if changed {
            self.latest = None;
        }
        self.library = next;
        changed
    }

    /// `text`는 진행 중인 IME 조합의 미리보기입니다(ADR-0067 §6). 같은 revision·View·조합이면 scene을 다시 씁니다.
    /// 조합이 없고 저장하지 않은 변경이 없는 창만 같은 저장본을 보는 다른 창과 scene을 공유합니다(`shared`).
    pub(crate) fn current(
        &mut self,
        session: &ProjectWindowSession,
        text: Option<&text::TextPreview>,
        shared: &mut SharedScenes,
    ) -> Option<Arc<CanvasScene>> {
        let view = session.tabs().active_view()?;
        let state = session.document()?;
        let scene_key = (state.revision(), view, text.map(|t| t.key));
        if let Some((previous, scene)) = &self.latest {
            if *previous == scene_key {
                return Some(scene.clone());
            }
        }
        // 조합 미리보기는 그 창에만 있으므로 공유 목록에서 찾거나 넣지 않습니다.
        let key = session
            .loaded_generation()
            .filter(|_| text.is_none() && !session.has_unsaved_changes())
            .map(|loaded| SharedSceneKey {
                project: session.identity().canonical_target().to_path_buf(),
                manifest: loaded.manifest_hash,
                document: state.model().document_id(),
                view,
                library: self.library.as_ref().map(|library| library.source_hash),
            });
        if let Some(scene) = key.as_ref().and_then(|key| shared.get(key)) {
            self.latest = Some((scene_key, scene.clone()));
            return Some(scene);
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
        if let Some(key) = key {
            shared.insert(key, &scene);
        }
        self.latest = Some((scene_key, scene.clone()));
        Some(scene)
    }
}

/// 고정 Free Frame payload를 scene 입력으로 바꿉니다. 캔버스 행과 증분 scene source(ADR-0084)가 함께 씁니다.
/// Path clip은 아직 그리지 않습니다(호출자가 거릅니다).
pub(crate) fn frame_input(
    id: StableId,
    frame: &stellar_document::frame::FramePayload,
    locked: bool,
) -> Option<FrameInput> {
    let radius = RectangleParams {
        size: frame.size,
        corner_radius: frame.corner_radius,
    }
    .effective_corner_radius();
    let radii = CornerRadii::new(radius.0, radius.1, radius.2, radius.3).ok()?;
    let mut node = NodeInput::new(id, frame.transform);
    node.locked = locked;
    Some(FrameInput {
        node,
        size: frame.size,
        corner_radii: radii,
        background: frame.background,
        border: frame.border.map(|b| BorderInput {
            color: b.color,
            thickness: b.thickness(),
        }),
        clip: if frame.clipping == Clipping::FrameBounds {
            FrameClipInput::Bounds
        } else {
            FrameClipInput::None
        },
    })
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
        let input = frame_input(id, frame, locked)?;
        let clip_children = input.clip == FrameClipInput::Bounds;
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
            image_fit: None,
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
                    image_fit: None,
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
        let mut shared = SharedScenes::default();
        let scene = cache.current(&session, None, &mut shared).unwrap();
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
        assert!(Arc::ptr_eq(
            &scene,
            &cache.current(&session, None, &mut shared).unwrap()
        ));
        session.open_view(sid(4)).unwrap();
        assert_eq!(
            cache.current(&session, None, &mut shared).unwrap().frames[0].id,
            sid(20)
        );
    }

    #[test]
    fn clean_windows_of_the_same_generation_share_one_scene_until_edited() {
        use crate::window_session::test_fixtures::rename;
        let dir = saved_project_with("shared-scene", &[&sample_document()]);
        let mut a = ProjectWindowSession::load(identity(dir.path()), window(1)).unwrap();
        let b = ProjectWindowSession::load(identity(dir.path()), window(2)).unwrap();
        let mut shared = SharedScenes::default();
        let (mut cache_a, mut cache_b) = (CanvasCache::default(), CanvasCache::default());
        let first = cache_a.current(&a, None, &mut shared).unwrap();
        assert!(Arc::ptr_eq(
            &first,
            &cache_b.current(&b, None, &mut shared).unwrap()
        ));

        // 편집한 창은 자기 목록을 다시 평가하고, 저장본 그대로인 창은 공유 목록을 유지합니다.
        let tx = rename(a.document().unwrap(), 1, 11, "Edited");
        a.apply(&tx, 0).unwrap();
        assert!(a.has_unsaved_changes());
        let edited = cache_a.current(&a, None, &mut shared).unwrap();
        assert!(!Arc::ptr_eq(&first, &edited));
        assert!(Arc::ptr_eq(
            &first,
            &cache_b.current(&b, None, &mut shared).unwrap()
        ));

        // 편집 중인 창의 목록은 같은 revision 번호의 다른 창과 공유하지 않습니다.
        let mut c = ProjectWindowSession::load(identity(dir.path()), window(3)).unwrap();
        let tx = rename(c.document().unwrap(), 2, 12, "Other");
        c.apply(&tx, 0).unwrap();
        assert_eq!(c.revision(), a.revision());
        let other = CanvasCache::default()
            .current(&c, None, &mut shared)
            .unwrap();
        assert!(!Arc::ptr_eq(&edited, &other));
    }
}

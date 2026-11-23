//! 창 활성 View의 layout 반영 scene source (ADR-0125).
//!
//! [`CanvasLayout`]은 scene crate의 문서 변환([`document_scene_source`])을 그대로 씁니다. 흐름 컨테이너 자식과
//! `position`을 명시한 node가 layout 결과 위치·크기에 있는 source이고, runtime 캔버스 기능(흐름 drag 삽입 위치·preview,
//! inset 이동)은 node 상자를 이 source에서 읽습니다. layout 계산은 scene crate의 [`DocumentLayout`] 하나이고 runtime은
//! 따로 배치하지 않습니다.
//!
//! 옛 캔버스 행 목록(`CanvasScene`)은 저장 transform으로 그리는 이전 경로로 남아 있습니다. UI를 다시 설계할 때 이
//! source를 그리는 경로로 바꿉니다.

use std::collections::HashMap;

use stellar_commands::change::ChangeSet;
use stellar_core_types::ids::StableId;
use stellar_core_types::value::Size2;
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::ModifierStack;
use stellar_document::text::TextPayload;
use stellar_scene::builder::{NodeInput, TextInput};
use stellar_scene::document::inputs::shape_style;
use stellar_scene::document::layout::DocumentLayout;
use stellar_scene::document::{
    document_scene_source, scene_edits, DocumentSceneSource, FlowPreview, SceneSourceOptions,
};
use stellar_scene::node::TextLayoutKey;
use stellar_scene::source::{SceneKey, SourceEdit, SourceError};

use crate::canvas::flow_drag::Box4;
use crate::canvas::transform::TransformError;
use crate::canvas::CanvasScene;
use crate::window_session::ProjectWindowSession;

/// 활성 View 하나의 layout 반영 scene source.
#[derive(Debug, Clone)]
pub struct CanvasLayout {
    source: DocumentSceneSource,
}

impl CanvasLayout {
    /// 문서 `model`의 View root `root` subtree를 변환합니다.
    pub fn for_model(
        model: &DocumentModel,
        root: StableId,
        library: Option<&ComponentLibrary>,
    ) -> Result<CanvasLayout, SourceError> {
        let source = document_scene_source(
            model,
            root,
            SceneSourceOptions {
                library,
                ..SceneSourceOptions::default()
            },
        )?;
        Ok(CanvasLayout { source })
    }

    /// `scene`(게시한 캔버스 행 목록)이 이미 layout한 Text 크기를 Text 입력으로 써서 변환합니다. text layout을 다시
    /// 하지 않고, Text가 흐름에서 크기를 갖고 world 상자를 냅니다(Text hook이 없으면 Text는 상자 없이 빠짐). 행 목록의
    /// Text는 줄바꿈 없이 layout한 결과라 줄 너비 인자는 쓰지 않습니다.
    pub fn for_scene(
        model: &DocumentModel,
        root: StableId,
        library: Option<&ComponentLibrary>,
        scene: &CanvasScene,
    ) -> Result<CanvasLayout, SourceError> {
        let sizes: HashMap<StableId, Size2> = scene
            .frames
            .iter()
            .filter(|row| row.text_node)
            .map(|row| (row.id, row.geometry.rect.size()))
            .collect();
        let mut hook = |id: StableId,
                        _: &TextPayload,
                        stack: Option<&ModifierStack>,
                        locked: bool,
                        _: Option<f32>|
         -> Option<TextInput> {
            let size = *sizes.get(&id)?;
            let mut node = NodeInput::new(id, shape_style(stack)?.transform);
            node.locked = locked;
            Some(TextInput {
                node,
                layout: TextLayoutKey::from_bytes(id.to_bytes()),
                size,
            })
        };
        let source = document_scene_source(
            model,
            root,
            SceneSourceOptions {
                library,
                text: Some(&mut hook),
                ..SceneSourceOptions::default()
            },
        )?;
        Ok(CanvasLayout { source })
    }

    /// 창의 활성 View를 변환합니다.
    pub fn current(window: &ProjectWindowSession) -> Result<CanvasLayout, TransformError> {
        let view = window.tabs().active_view().ok_or(TransformError::Stale)?;
        let model = window.document().ok_or(TransformError::Stale)?.model();
        let root =
            crate::app::canvas_gesture::view_root(model, view).ok_or(TransformError::Stale)?;
        CanvasLayout::for_model(model, root, None).map_err(|_| TransformError::Stale)
    }

    /// command change set을 반영합니다(증분 layout과 증분 scene source 편집).
    pub fn apply(
        &mut self,
        model: &DocumentModel,
        changes: &ChangeSet,
        library: Option<&ComponentLibrary>,
    ) -> Result<Vec<SourceEdit>, SourceError> {
        scene_edits(
            model,
            &mut self.source,
            changes,
            SceneSourceOptions {
                library,
                ..SceneSourceOptions::default()
            },
        )
    }

    /// scene source.
    pub fn source(&self) -> &DocumentSceneSource {
        &self.source
    }

    /// layout 결과.
    pub fn layout(&self) -> &DocumentLayout {
        self.source.layout()
    }

    /// node의 world 상자(x, y, 너비, 높이).
    pub fn world_box(&self, node: StableId) -> Option<Box4> {
        let bounds = self.source.world_bounds(SceneKey::node(node), None)?;
        Some([
            bounds.min_x(),
            bounds.min_y(),
            bounds.width(),
            bounds.height(),
        ])
    }

    /// 흐름 컨테이너 `container`의 layout 자식을 `order` 순서로 바꾼 preview.
    pub fn reorder_preview(
        &self,
        model: &DocumentModel,
        container: StableId,
        order: &[StableId],
    ) -> Option<FlowPreview> {
        self.source
            .reorder_preview(model, container, order, SceneSourceOptions::default())
            .ok()
    }

    /// preview 대체 입력으로 센 node의 world 상자.
    pub fn preview_box(&self, preview: &FlowPreview, node: StableId) -> Option<Box4> {
        let bounds = self
            .source
            .world_bounds(SceneKey::node(node), Some(&preview.preview))?;
        Some([
            bounds.min_x(),
            bounds.min_y(),
            bounds.width(),
            bounds.height(),
        ])
    }
}

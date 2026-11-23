//! 창 활성 View의 layout 반영 scene source (ADR-0125).
//!
//! [`CanvasLayout`]은 scene crate의 문서 변환([`document_scene_source`])을 그대로 씁니다. 흐름 컨테이너 자식과
//! `position`을 명시한 node가 layout 결과 위치·크기에 있는 source이고, runtime 캔버스 기능(흐름 drag 삽입 위치·preview,
//! inset 이동)은 node 상자를 이 source에서 읽습니다. layout 계산은 scene crate의 [`DocumentLayout`] 하나이고 runtime은
//! 따로 배치하지 않습니다.
//!
//! 창 자료([`CanvasLayoutInputs`]): 컴포넌트 정의 집합이 있으면 ComponentInstance가 정의 root 크기의 상자를 갖고,
//! Text hook([`CanvasText`])이 있으면 Text가 줄바꿈 측정으로 흐름 배치에 들어가 상자를 갖습니다(ADR-0134 결정 1).
//! [`CanvasLayout::current`]·[`CanvasLayout::apply`]는 창 cache([`CanvasCache`])의 정의 집합과 글꼴로 두 값을 채웁니다.
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
use stellar_scene::source::{SceneKey, SourceEdit, SourceError, SourceKind};

use crate::canvas::flow_drag::Box4;
use crate::canvas::layout_text::CanvasLayoutInputs;
use crate::canvas::transform::TransformError;
use crate::canvas::{CanvasCache, CanvasScene};
use crate::window_session::ProjectWindowSession;

/// 측정 hook(layout 엔진이 Text를 잼).
fn measure_hook<'a>(
    inputs: &CanvasLayoutInputs<'a>,
) -> Option<&'a dyn stellar_scene::document::layout::engine::intrinsic::TextLayoutHook> {
    inputs
        .text
        .map(|t| t as &dyn stellar_scene::document::layout::engine::intrinsic::TextLayoutHook)
}

/// 게시 행 목록 Text 크기(`sizes`)로 만든 Text 입력([`CanvasLayout::for_scene`]).
fn scene_text_input(
    sizes: &HashMap<StableId, Size2>,
    id: StableId,
    stack: Option<&ModifierStack>,
    locked: bool,
) -> Option<TextInput> {
    let size = *sizes.get(&id)?;
    let mut node = NodeInput::new(id, shape_style(stack)?.transform);
    node.locked = locked;
    Some(TextInput {
        node,
        layout: TextLayoutKey::from_bytes(id.to_bytes()),
        size,
    })
}

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
        let mut hook =
            |id: StableId,
             _: &TextPayload,
             stack: Option<&ModifierStack>,
             locked: bool,
             _: Option<f32>|
             -> Option<TextInput> { scene_text_input(&sizes, id, stack, locked) };
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

    /// 문서 `model`의 View root `root` subtree를 창 자료 `inputs`(정의 집합, Text hook)로 변환합니다.
    pub fn with_inputs(
        model: &DocumentModel,
        root: StableId,
        inputs: CanvasLayoutInputs<'_>,
    ) -> Result<CanvasLayout, SourceError> {
        // 그리기 hook과 측정 hook은 같은 [`CanvasTextHook`]입니다.
        let text = inputs.text;
        let mut hook = move |id, _: &TextPayload, _: Option<&ModifierStack>, locked, width| {
            text.and_then(|t| t.text_input(id, locked, width))
        };
        let options = SceneSourceOptions {
            library: inputs.library,
            text: text.is_some().then_some(&mut hook),
            variant: None,
            text_measure: measure_hook(&inputs),
        };
        let source = document_scene_source(model, root, options)?;
        Ok(CanvasLayout { source })
    }

    /// 창의 활성 View를 창 cache(`cache`)의 컴포넌트 정의 집합과 글꼴로 변환합니다. Text는 줄바꿈 측정으로 흐름에
    /// 들어가고 ComponentInstance는 정의 root 크기의 상자를 갖습니다.
    pub fn current(
        window: &ProjectWindowSession,
        cache: &mut CanvasCache,
    ) -> Result<CanvasLayout, TransformError> {
        let view = window.tabs().active_view().ok_or(TransformError::Stale)?;
        let model = window.document().ok_or(TransformError::Stale)?.model();
        let root =
            crate::app::canvas_gesture::view_root(model, view).ok_or(TransformError::Stale)?;
        let project = window.identity().canonical_target();
        cache
            .with_layout_inputs(project, model, |inputs| {
                CanvasLayout::with_inputs(model, root, inputs)
            })
            .map_err(|_| TransformError::Stale)
    }

    /// command change set을 반영합니다(증분 layout과 증분 scene source 편집). 변환 때와 같은 창 자료를 넘깁니다(Text
    /// hook 유무가 바뀌면 layout을 처음부터 다시 계산합니다).
    pub fn apply_with(
        &mut self,
        model: &DocumentModel,
        changes: &ChangeSet,
        inputs: CanvasLayoutInputs<'_>,
    ) -> Result<Vec<SourceEdit>, SourceError> {
        let text = inputs.text;
        let mut hook = move |id, _: &TextPayload, _: Option<&ModifierStack>, locked, width| {
            text.and_then(|t| t.text_input(id, locked, width))
        };
        let options = SceneSourceOptions {
            library: inputs.library,
            text: text.is_some().then_some(&mut hook),
            variant: None,
            text_measure: measure_hook(&inputs),
        };
        scene_edits(model, &mut self.source, changes, options)
    }

    /// 창 문서의 command change set을 창 cache(`cache`)의 정의 집합과 글꼴로 반영합니다.
    pub fn apply(
        &mut self,
        window: &ProjectWindowSession,
        changes: &ChangeSet,
        cache: &mut CanvasCache,
    ) -> Result<Vec<SourceEdit>, TransformError> {
        let model = window.document().ok_or(TransformError::Stale)?.model();
        let project = window.identity().canonical_target();
        cache
            .with_layout_inputs(project, model, |inputs| {
                self.apply_with(model, changes, inputs)
            })
            .map_err(|_| TransformError::Stale)
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

    /// 흐름 컨테이너 `container`의 layout 자식을 `order` 순서로 바꾼 preview. Text는 이 source가 이미 그린 Text 입력 크기로
    /// 다시 그립니다(Text가 옛 자리에 남지 않음). 순서만 바꾸면 흐름이 Text 줄 너비를 바꾸지 않는 한 결과가 같습니다.
    /// 줄 너비까지 다시 재려면 변환 때와 같은 창 자료로 [`Self::reorder_preview_with`]를 부릅니다.
    pub fn reorder_preview(
        &self,
        model: &DocumentModel,
        container: StableId,
        order: &[StableId],
    ) -> Option<FlowPreview> {
        let source = &self.source.source;
        let mut hook = |id: StableId,
                        _: &TextPayload,
                        stack: Option<&ModifierStack>,
                        locked: bool,
                        _: Option<f32>|
         -> Option<TextInput> {
            let SourceKind::Text(drawn) = &source.get(SceneKey::node(id))?.kind else {
                return None;
            };
            let mut node = NodeInput::new(id, shape_style(stack)?.transform);
            node.locked = locked;
            Some(TextInput {
                node,
                layout: drawn.layout,
                size: drawn.size,
            })
        };
        let options = SceneSourceOptions {
            text: Some(&mut hook),
            ..SceneSourceOptions::default()
        };
        self.source
            .reorder_preview(model, container, order, options)
            .ok()
    }

    /// [`Self::reorder_preview`]를 변환 때와 같은 창 자료(`inputs`)로 계산합니다. Text는 같은 측정 hook으로 다시 재고
    /// 같은 Text hook으로 그리며, 인스턴스는 같은 정의 집합으로 펼칩니다(ADR-0134 결정 1).
    pub fn reorder_preview_with(
        &self,
        model: &DocumentModel,
        container: StableId,
        order: &[StableId],
        inputs: CanvasLayoutInputs<'_>,
    ) -> Option<FlowPreview> {
        let text = inputs.text;
        let mut hook = move |id, _: &TextPayload, _: Option<&ModifierStack>, locked, width| {
            text.and_then(|t| t.text_input(id, locked, width))
        };
        let options = SceneSourceOptions {
            library: inputs.library,
            text: text.is_some().then_some(&mut hook),
            variant: None,
            text_measure: measure_hook(&inputs),
        };
        self.source
            .reorder_preview(model, container, order, options)
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

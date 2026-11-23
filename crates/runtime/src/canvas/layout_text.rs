//! 활성 View layout의 Text 측정·그리기 hook과 컴포넌트 정의 연결 (ADR-0134 결정 1).
//!
//! [`CanvasLayout`](super::scene_layout::CanvasLayout)은 scene crate의 문서 변환으로 layout 결과 source를 만듭니다. 이
//! 모듈은 그 변환에 넘길 두 hook을 창의 글꼴 cache([`FontCache`])로 만듭니다.
//!
//! - **측정**(`SceneSourceOptions::text_measure`, `stellar_layout::intrinsic::TextLayoutHook`): layout 엔진이 Text 흐름 자식의
//!   min-content 너비와 주어진 줄 너비에서의 줄 배치(너비·높이·첫 baseline)를 묻습니다.
//! - **그리기**(`SceneSourceOptions::text`): scene이 layout content box 너비(풀린 `width`가 `max-content`면 줄바꿈 없음)로
//!   Text 입력을 만듭니다.
//!
//! 두 hook은 같은 함수([`FontCache::layout_input_width`])로 같은 문서 Text를 배치하고, 결과를 (node, 줄 너비)로
//! 공유합니다. 그래서 layout이 잰 크기와 scene이 그리는 크기가 같습니다. Text stack은 scene Text hook이 받는 stack과
//! 같은 [`content_stack`]입니다(효과·Boolean 항목 제외).
//!
//! - min-content 너비는 줄 너비 0으로 배치한 결과에서 비상 줄바꿈 조각을 줄바꿈 기회 사이 단위로 다시 이은 가장 넓은
//!   단위입니다([`DocumentText::min_content_width`](super::text::document::DocumentText::min_content_width)). text 엔진의
//!   비상 줄바꿈(`overflow-wrap: break-word`)은 CSS에서 min-content를 줄이지 않습니다.
//! - 줄 너비는 layout 단위(1/64 px)를 그대로 text 단위(1/64 px)로 옮깁니다.
//! - 글꼴을 읽는 중이면 기본 chain으로 배치합니다([`FontCache`] 규칙). 글꼴을 다 읽으면 창 cache가 게시 결과를 버리고,
//!   호출자는 `CanvasLayout`을 다시 만듭니다.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use stellar_core_types::ids::StableId;
use stellar_core_types::value::{FiniteF32, Size2, Transform2D};
use stellar_document::component::definition::ComponentLibrary;
use stellar_document::model::DocumentModel;
use stellar_document::modifier::ModifierStack;
use stellar_document::records::NodePayload;
use stellar_document::text::TextPayload;
use stellar_scene::builder::{NodeInput, TextInput};
use stellar_scene::document::content_stack;
// runtime은 layout crate에 직접 의존하지 않고 scene 재수출을 씁니다(ADR-0125 결정 1).
use stellar_scene::document::layout::engine::intrinsic::{
    MeasureError, TextIntrinsic, TextLayoutHook, TextLineLayout,
};
use stellar_scene::document::layout::engine::rounding::LayoutUnit;
use stellar_scene::node::TextLayoutKey;
use stellar_text::incremental::IncrementalTextLayout;
use stellar_text::units::TextUnit;

use super::text::document::FontCache;
use super::CanvasCache;

/// 한 줄 너비(1/64 px, 없으면 줄바꿈 없음)에서 배치한 결과.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Measured {
    size: Size2,
    baseline: Option<f32>,
    transform: Transform2D,
    /// CSS min-content 너비(px). 줄 너비 0으로 배치한 결과에만 있습니다.
    min_content: Option<f32>,
}

struct State<'a> {
    fonts: &'a mut FontCache,
    project: &'a Path,
    model: &'a DocumentModel,
    engine: IncrementalTextLayout,
    results: HashMap<(StableId, Option<i32>), Option<Measured>>,
}

/// 창 글꼴 cache로 만든 Text 측정·그리기 hook(모듈 문서). layout 계산 한 번 동안만 씁니다.
pub struct CanvasText<'a> {
    state: Mutex<State<'a>>,
}

impl std::fmt::Debug for CanvasText<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanvasText").finish_non_exhaustive()
    }
}

/// Text 측정(layout)과 그리기(scene Text 입력)를 같은 결과로 내는 hook. [`CanvasText`]가 구현하고, 시험은 글꼴 없는
/// 구현을 넘길 수 있습니다.
pub trait CanvasTextHook: TextLayoutHook {
    /// 줄 너비 `width`(px, 없으면 줄바꿈 없음)에서의 Text `id` scene 입력. 배치할 수 없으면 `None`입니다.
    fn text_input(&self, id: StableId, locked: bool, width: Option<f32>) -> Option<TextInput>;
}

/// `CanvasLayout` 변환에 넘기는 창 자료: 컴포넌트 정의 집합과 Text hook.
#[derive(Clone, Copy, Default)]
pub struct CanvasLayoutInputs<'a> {
    /// ComponentInstance 정의 집합(인스턴스가 상자를 가짐). 없으면 인스턴스는 scene에서 빠집니다.
    pub library: Option<&'a ComponentLibrary>,
    /// Text 측정·그리기 hook. 없으면 Text는 흐름 배치에 크기 없이 들어가고 scene에서 빠집니다.
    pub text: Option<&'a dyn CanvasTextHook>,
}

impl std::fmt::Debug for CanvasLayoutInputs<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanvasLayoutInputs")
            .field("library", &self.library.is_some())
            .field("text", &self.text.is_some())
            .finish()
    }
}

impl<'a> CanvasText<'a> {
    /// `project`(프로젝트 root, 프로젝트 글꼴 파일 위치)의 문서 `model` Text를 `fonts`로 배치하는 hook.
    pub(crate) fn new(
        fonts: &'a mut FontCache,
        project: &'a Path,
        model: &'a DocumentModel,
    ) -> CanvasText<'a> {
        fonts.sync_registrations(&model.definition().text_styles);
        CanvasText {
            state: Mutex::new(State {
                fonts,
                project,
                model,
                engine: IncrementalTextLayout::new(),
                results: HashMap::new(),
            }),
        }
    }

    /// node `id`를 줄 너비 `width`(1/64 px)로 배치합니다. 배치할 수 없는 Text(지원하지 않는 style·modifier, 글꼴
    /// 없음)는 `None`입니다.
    fn measure(&self, id: StableId, width: Option<i32>) -> Option<Measured> {
        let mut guard = self.state.lock().ok()?;
        let state = &mut *guard;
        if let Some(found) = state.results.get(&(id, width)) {
            return *found;
        }
        let measured = (|| {
            let model = state.model;
            let NodePayload::Text(payload) = model.payload(id)? else {
                return None;
            };
            let stack = content_stack(id, model.modifier_stack(id));
            measure_payload(state, id, payload, stack.as_ref(), width)
        })();
        state.results.insert((id, width), measured);
        measured
    }
}

impl CanvasTextHook for CanvasText<'_> {
    fn text_input(&self, id: StableId, locked: bool, width: Option<f32>) -> Option<TextInput> {
        let raw = match width {
            Some(px) => Some(LayoutUnit::from_px(FiniteF32::new(px).ok()?).ok()?.raw()),
            None => None,
        };
        let measured = self.measure(id, raw)?;
        let mut node = NodeInput::new(id, measured.transform);
        node.locked = locked;
        Some(TextInput {
            node,
            layout: TextLayoutKey::from_bytes(id.to_bytes()),
            size: measured.size,
        })
    }
}

fn measure_payload(
    state: &mut State<'_>,
    id: StableId,
    payload: &TextPayload,
    stack: Option<&ModifierStack>,
    width: Option<i32>,
) -> Option<Measured> {
    let (text, size, transform) = state.fonts.layout_input_width(
        state.project,
        id,
        payload,
        &state.model.definition().text_styles,
        stack,
        &mut state.engine,
        width.map(TextUnit::from_raw),
    )?;
    Some(Measured {
        size,
        baseline: text.first_baseline(),
        transform,
        min_content: match width {
            Some(0) => Some(text.min_content_width(payload.content())?),
            _ => None,
        },
    })
}

fn unit(px: f32) -> Result<LayoutUnit, MeasureError> {
    let px = FiniteF32::new(px).map_err(|_| MeasureError {
        reason: "Text 측정 값이 유한하지 않습니다",
    })?;
    Ok(LayoutUnit::from_px(px)?)
}

const UNAVAILABLE: MeasureError = MeasureError {
    reason: "Text를 배치할 수 없습니다(style·글꼴)",
};

impl TextLayoutHook for CanvasText<'_> {
    fn min_content_width(
        &self,
        node: StableId,
        _: &TextIntrinsic,
    ) -> Result<LayoutUnit, MeasureError> {
        let measured = self.measure(node, Some(0)).ok_or(UNAVAILABLE)?;
        unit(measured.min_content.ok_or(UNAVAILABLE)?)
    }

    fn layout(
        &self,
        node: StableId,
        _: &TextIntrinsic,
        width: Option<LayoutUnit>,
    ) -> Result<TextLineLayout, MeasureError> {
        let measured = self
            .measure(node, width.map(|w| w.raw()))
            .ok_or(UNAVAILABLE)?;
        Ok(TextLineLayout {
            width: unit(measured.size.width())?,
            height: unit(measured.size.height())?,
            first_baseline: measured.baseline.map(unit).transpose()?,
        })
    }
}

impl CanvasCache {
    /// 창 cache의 컴포넌트 정의 집합과 글꼴로 `f`에 [`CanvasLayoutInputs`]를 줍니다. `project`는 프로젝트 root입니다.
    pub fn with_layout_inputs<R>(
        &mut self,
        project: &Path,
        model: &DocumentModel,
        f: impl FnOnce(CanvasLayoutInputs<'_>) -> R,
    ) -> R {
        let library = self.library.clone();
        let text = CanvasText::new(&mut self.fonts, project, model);
        f(CanvasLayoutInputs {
            library: library.as_ref().map(|l| &l.definitions),
            text: Some(&text),
        })
    }
}

#[cfg(test)]
mod tests;

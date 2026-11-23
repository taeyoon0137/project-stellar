//! 재생 layout 단계: 문서 scene source의 layout 결과를 그대로 씁니다 (ADR-0084 결정 9, ADR-0125, ADR-0134 결정 5).
//!
//! layout 계산은 scene 문서 변환의 [`DocumentLayout`](stellar_scene::document::layout::DocumentLayout) 하나입니다.
//! `scene_edits`가 같은 change set으로 증분 layout을 갱신하고, 재생은 그 갱신에서 위치·크기가 바뀐 상자를 scene
//! 무효화(`LayoutRect`)로 옮깁니다. 재생이 scene source에서 layout 입력을 따로 만들지 않습니다(이전의 Free node →
//! absolute box 변환은 지웠습니다).

use stellar_core_types::ids::StableId;
use stellar_document::model::DocumentModel;
use stellar_scene::dirty::SceneInvalidation;
use stellar_scene::document::{DocumentSceneSource, SceneSourceOptions};

/// 마지막 `scene_edits`(또는 [`rebuild`])의 layout rect 무효화.
pub fn invalidations(source: &DocumentSceneSource) -> Vec<SceneInvalidation> {
    source.layout().rect_invalidations()
}

/// cold 측정: 이전 layout 결과를 버리고 처음부터 배치합니다. 상자가 바뀐 node를 돌려줍니다(결과는 증분과 같아
/// 보통 비어 있습니다). 계산 실패는 scene 변환과 같이 [`DocumentLayout::error`]에 남습니다.
///
/// [`DocumentLayout::error`]: stellar_scene::document::layout::DocumentLayout::error
pub fn rebuild<'a>(
    source: &mut DocumentSceneSource,
    model: &'a DocumentModel,
    options: SceneSourceOptions<'a>,
) -> Vec<StableId> {
    source.rebuild_layout(model, options).into_iter().collect()
}

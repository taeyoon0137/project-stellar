//! 상태 머신·`MotionSpec`·상호작용 상태 토큰의 Web 출력 (XXII.2.6, ADR-0106 결정 7).
//!
//! 판정은 codegen(`web::motion`, `web::state`)이 하고, 이 module은 문서 registry의 CSS longhand 이름과 문서 값
//! formatter를 넘깁니다.

use stellar_codegen::ir::IrTree;
use stellar_codegen::web::motion::{
    lower_interaction_tokens, lower_motions, EasingSampler, WebMotion,
};
use stellar_codegen::web::state::{lower_state_machines, WebState};
use stellar_codegen::web::tokens::plan_tokens;
use stellar_document::css::custom_property::literal_text;
use stellar_document::css::mapping::MappingStatus;
use stellar_document::css::registry;

use crate::core_types::property::PropertyPath;
use crate::export_web_css::DocumentCss;
use crate::export_web_tokens::web_names;

/// 속성 경로의 CSS longhand 이름. 경로가 key 하나이고 registry 매핑이 `exact`(이름 = CSS longhand)일 때만 있습니다.
pub fn css_property_name(path: &PropertyPath) -> Option<&'static str> {
    let [key] = path.keys() else {
        return None;
    };
    if path.tail().is_some() {
        return None;
    }
    registry::entries()
        .into_iter()
        .find(|e| e.key == *key && e.bundle.is_empty())
        .filter(|e| e.css.status == MappingStatus::Exact)
        .map(|e| e.name)
}

/// Web 상호작용·motion·상태 출력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebBehavior {
    /// motion 규칙(reduced motion 블록 포함).
    pub motion: WebMotion,
    /// 상호작용 상태 selector 규칙.
    pub interaction: WebMotion,
    /// `state.js`.
    pub state: WebState,
}

/// IR의 motion·상태 머신·상호작용 토큰을 Web 출력으로 낮춥니다. `sampler`는 spring 표본용 엔진 evaluator입니다.
pub fn export_behavior(ir: &IrTree, sampler: Option<&dyn EasingSampler>) -> WebBehavior {
    let names = web_names(ir);
    let css = DocumentCss { names: &names };
    let plan = plan_tokens(ir, &|value| literal_text(value).is_some());
    WebBehavior {
        motion: lower_motions(ir, &css_property_name, sampler),
        interaction: lower_interaction_tokens(ir, &plan.interaction, &css, &css_property_name),
        state: lower_state_machines(ir),
    }
}

#[cfg(test)]
mod tests;

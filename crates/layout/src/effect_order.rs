//! CSS 효과 적용 순서에서 그림자·blur가 놓이는 자리 (ADR-0134 결정 3, 속성 모델 3장·9.1절).
//!
//! CSS는 요소 하나에 paint(`box-shadow`, background, border, 내용) → `filter` 목록(적은 순서) 순서로 효과를 적용합니다.
//! 문서 modifier stack의 Shadow·Blur를 이 순서로 옮기는 규칙은 하나이고, Web 코드 생성(`stellar-codegen`의 modifier 계획)과
//! 문서 → scene 변환(`stellar-scene`의 효과 평가)이 이 함수를 같이 씁니다. 그래서 두 경로가 같은 stack을 같은 순서로
//! 읽습니다.
//!
//! - **상자 요소**(Frame): Shadow는 그 앞에 filter 효과(Blur 또는 drop-shadow)가 없을 때만 `box-shadow`(테두리 상자 모양,
//!   내용 아래)입니다. 앞에 filter 효과가 있으면 `filter: drop-shadow()`(그때까지 filter를 적용한 결과의 alpha 모양)입니다.
//! - **그 밖의 요소**(Path·Text·Image·Shape): Shadow는 언제나 `filter: drop-shadow()`입니다.
//! - Blur는 언제나 `filter: blur()`입니다.
//! - filter 목록은 stack 순서 그대로입니다(`blur() drop-shadow()`와 `drop-shadow() blur()`는 다른 결과).
//!
//! layout 계산과는 무관하지만, 코드 생성과 scene이 함께 의존하는 CSS 의미 crate가 이 crate라서 여기에 둡니다(CSS 순서
//! 합성 `transform::css_node_transform`과 같은 이유).

/// 순서 규칙이 보는 효과 종류(stack 순서로 넘김).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterEffect {
    /// Shadow modifier.
    Shadow,
    /// Blur modifier.
    Blur,
}

/// 효과 하나의 CSS 자리.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffectSlot {
    /// `box-shadow`: 테두리 상자 모양, 내용 아래, filter 목록 앞.
    BoxShadow,
    /// `filter` 목록 항목(`blur()`·`drop-shadow()`), stack 순서.
    Filter,
}

/// `effects`(활성 Shadow·Blur, stack 순서)의 CSS 자리. `boxed`는 상자 요소(Frame)인지입니다. 결과는 입력과 같은 순서·
/// 길이입니다.
pub fn effect_slots(
    effects: impl IntoIterator<Item = FilterEffect>,
    boxed: bool,
) -> Vec<EffectSlot> {
    let mut filter_seen = false;
    effects
        .into_iter()
        .map(|effect| match effect {
            FilterEffect::Shadow if boxed && !filter_seen => EffectSlot::BoxShadow,
            FilterEffect::Shadow | FilterEffect::Blur => {
                filter_seen = true;
                EffectSlot::Filter
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use EffectSlot::{BoxShadow, Filter};
    use FilterEffect::{Blur, Shadow};

    #[test]
    fn frame_shadows_are_box_shadows_until_a_filter_effect() {
        assert_eq!(effect_slots([Shadow, Blur], true), [BoxShadow, Filter]);
        assert_eq!(effect_slots([Blur, Shadow], true), [Filter, Filter]);
        assert_eq!(
            effect_slots([Shadow, Shadow, Blur, Shadow], true),
            [BoxShadow, BoxShadow, Filter, Filter]
        );
        assert!(effect_slots([], true).is_empty());
    }

    #[test]
    fn non_box_shadows_are_always_drop_shadow_filters() {
        assert_eq!(effect_slots([Shadow, Blur], false), [Filter, Filter]);
        assert_eq!(effect_slots([Blur, Shadow], false), [Filter, Filter]);
    }
}

//! 문서 효과 modifier → scene 합성 효과 (ADR-0116 결정 2).
//!
//! Shadow·Blur·Blend·Opacity modifier는 node 내용 모양(transform·fill·stroke)과 따로 node 합성 결과에 걸리는 효과입니다
//! (CSS `box-shadow`·`filter: drop-shadow()`, `filter: blur()`, `mix-blend-mode`, `opacity`). 변환은 이 종류를 stack에서
//! 떼어 내 [`NodeEffects`]로 평가하고, 나머지 stack은 기존 node 입력 함수([`super::inputs`])가 그대로 평가합니다. 그래서
//! 도형 modifier 규칙([`stellar_document::shape::evaluate_shape_modifiers`])은 바뀌지 않습니다.
//!
//! # 규칙
//!
//! - 활성 항목만 OrderKey 순서로 읽습니다. Shadow·Blur의 CSS 자리(`box-shadow` 또는 `filter` 목록 항목)는 Web 코드
//!   생성과 같은 규칙 하나(`stellar_layout::effect_order::effect_slots`)로 정해 [`NodeEffects::order`]에 싣습니다(ADR-0134
//!   결정 3). Frame은 앞에 filter 효과가 없는 Shadow만 `box-shadow`이고, 그 밖의 node Shadow는 `drop-shadow()`입니다.
//!   합성 pass의 실제 적용 순서는 renderer가 정합니다(ADR-0102 결정 5).
//! - Opacity: 활성 항목의 곱([`ModifierStack::effective_opacity`], 문서 규칙 그대로).
//! - Shadow: 활성 항목 전부, OrderKey 순서가 scene 목록 순서(앞이 아래)입니다.
//! - Blur·Blend: CSS 속성 하나에 대응하므로 활성 항목이 둘 이상이면 평가하지 않습니다([`EffectError::Repeated`]).
//! - Blend tag는 문서와 scene이 같은 값입니다(`SceneBlendMode::from_tag`).

use stellar_core_types::ids::StableId;
use stellar_core_types::shadow::{BlurRadius, Shadow};
use stellar_document::modifier::{ModifierKind, ModifierPayload, ModifierStack};

use stellar_layout::effect_order::{effect_slots, EffectSlot, FilterEffect};

use crate::builder::ModifierPaint;
use crate::stack::{Opacity, SceneBlendMode};

/// node 합성 효과 modifier 종류.
pub const EFFECT_KINDS: [ModifierKind; 4] = [
    ModifierKind::Shadow,
    ModifierKind::Blur,
    ModifierKind::Blend,
    ModifierKind::Opacity,
];

/// `kind`가 node 합성 효과 modifier이면 참입니다.
pub fn is_effect(kind: ModifierKind) -> bool {
    EFFECT_KINDS.contains(&kind)
}

/// 평가한 node 합성 효과.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeEffects {
    /// drop shadow(앞이 아래).
    pub shadows: Vec<Shadow>,
    /// 합성 불투명도.
    pub opacity: Opacity,
    /// 합성 blend mode.
    pub blend: SceneBlendMode,
    /// 합성 결과 blur.
    pub layer_blur: BlurRadius,
    /// 활성 Shadow·Blur의 stack 순서와 CSS 자리(코드 생성과 같은 규칙). Shadow 항목은 `shadows` 순서와 같습니다.
    pub order: Vec<(FilterEffect, EffectSlot)>,
}

impl Default for NodeEffects {
    fn default() -> NodeEffects {
        NodeEffects {
            shadows: Vec::new(),
            opacity: Opacity::OPAQUE,
            blend: SceneBlendMode::Normal,
            layer_blur: BlurRadius::ZERO,
            order: Vec::new(),
        }
    }
}

impl NodeEffects {
    /// scene builder 입력의 합성 효과를 이 값으로 채웁니다(fill·stroke는 그대로).
    pub fn apply(&self, paint: &mut ModifierPaint) {
        paint.shadows.clone_from(&self.shadows);
        paint.opacity = self.opacity;
        paint.blend = self.blend;
        paint.layer_blur = self.layer_blur;
    }

    /// 효과가 없으면 참입니다.
    pub fn is_empty(&self) -> bool {
        *self == NodeEffects::default()
    }

    /// `drop-shadow()`(filter 목록 항목)로 놓이는 Shadow가 있는지. Frame에서는 Blur 뒤 Shadow입니다.
    pub fn has_drop_shadow(&self) -> bool {
        self.order
            .contains(&(FilterEffect::Shadow, EffectSlot::Filter))
    }
}

/// 효과 modifier 평가 실패.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectError {
    /// CSS 속성 하나에 대응하는 종류(Blur·Blend)의 활성 항목이 둘 이상입니다.
    Repeated(ModifierKind),
    /// 값이 scene 범위를 벗어났습니다(불투명도 범위, blur·shadow 반경).
    InvalidValue(ModifierKind),
}

/// `stack`의 활성 효과 modifier를 평가합니다. `boxed`는 상자 요소(Frame)인지입니다(Shadow의 CSS 자리).
pub fn evaluate_effects(
    stack: Option<&ModifierStack>,
    boxed: bool,
) -> Result<NodeEffects, EffectError> {
    let mut effects = NodeEffects::default();
    let Some(stack) = stack else {
        return Ok(effects);
    };
    let mut blurs = Vec::new();
    let mut blends = Vec::new();
    let mut filters = Vec::new();
    for entry in stack.active_entries() {
        match entry.payload {
            ModifierPayload::Shadow {
                color,
                offset,
                blur,
            } => {
                let radius = BlurRadius::new(blur.get())
                    .map_err(|_| EffectError::InvalidValue(ModifierKind::Shadow))?;
                effects.shadows.push(Shadow::new(offset, radius, color));
                filters.push(FilterEffect::Shadow);
            }
            ModifierPayload::Blur { radius } => {
                blurs.push(radius);
                filters.push(FilterEffect::Blur);
            }
            ModifierPayload::Blend { mode } => blends.push(mode),
            _ => {}
        }
    }
    if blurs.len() > 1 {
        return Err(EffectError::Repeated(ModifierKind::Blur));
    }
    if blends.len() > 1 {
        return Err(EffectError::Repeated(ModifierKind::Blend));
    }
    let (blur, blend) = (blurs.first(), blends.first());
    effects.opacity = Opacity::new(stack.effective_opacity())
        .map_err(|_| EffectError::InvalidValue(ModifierKind::Opacity))?;
    if let Some(radius) = blur {
        effects.layer_blur = BlurRadius::new(radius.get())
            .map_err(|_| EffectError::InvalidValue(ModifierKind::Blur))?;
    }
    if let Some(mode) = blend {
        effects.blend = SceneBlendMode::from_tag(*mode as u8)
            .ok_or(EffectError::InvalidValue(ModifierKind::Blend))?;
    }
    let slots = effect_slots(filters.iter().copied(), boxed);
    effects.order = filters.into_iter().zip(slots).collect();
    Ok(effects)
}

/// `keep`이 참인 종류의 항목만 남긴 stack(항목 id·순서·활성 여부 유지). 입력이 없으면 `None`입니다.
pub fn retain_kinds(
    node: StableId,
    stack: Option<&ModifierStack>,
    keep: impl Fn(ModifierKind) -> bool,
) -> Option<ModifierStack> {
    let stack = stack?;
    let mut filtered = ModifierStack::new(node);
    for entry in stack.entries() {
        if keep(entry.kind) {
            // 원래 stack의 항목이라 id·순서가 겹치지 않습니다.
            let _ = filtered.insert(entry.clone());
        }
    }
    Some(filtered)
}

/// 효과 modifier를 뗀 stack(node 내용 입력 함수가 평가할 부분).
pub fn without_effects(node: StableId, stack: Option<&ModifierStack>) -> Option<ModifierStack> {
    retain_kinds(node, stack, |kind| !is_effect(kind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use stellar_core_types::color::{Color, ColorSpace};
    use stellar_core_types::order_key::OrderKey;
    use stellar_core_types::value::{FiniteF32, Vec2};
    use stellar_document::modifier::{ModifierEntry, ModifierId};

    fn sid(n: u64) -> StableId {
        StableId::new_v7(n, 0, n)
    }

    fn stack(payloads: Vec<ModifierPayload>) -> ModifierStack {
        let mut stack = ModifierStack::new(sid(1));
        for (i, payload) in payloads.into_iter().enumerate() {
            stack
                .insert(ModifierEntry {
                    id: ModifierId::new(sid(100 + i as u64)),
                    order: OrderKey::from_bytes(vec![0x10 + i as u8]).unwrap(),
                    kind: payload.kind(),
                    enabled: true,
                    payload,
                })
                .unwrap();
        }
        stack
    }

    fn shadow() -> ModifierPayload {
        ModifierPayload::Shadow {
            color: Color::new(ColorSpace::Srgb, 0., 0., 0., 1.).unwrap(),
            offset: Vec2::new(2., 3.).unwrap(),
            blur: FiniteF32::new(4.).unwrap(),
        }
    }

    fn blur() -> ModifierPayload {
        ModifierPayload::Blur {
            radius: FiniteF32::new(1.5).unwrap(),
        }
    }

    #[test]
    fn effect_order_follows_the_codegen_rule() {
        // ADR-0134 결정 3: Frame Shadow는 앞에 filter 효과가 없을 때만 box-shadow이고, Blur 뒤면 drop-shadow입니다.
        let before = evaluate_effects(Some(&stack(vec![shadow(), blur()])), true).unwrap();
        assert_eq!(
            before.order,
            [
                (FilterEffect::Shadow, EffectSlot::BoxShadow),
                (FilterEffect::Blur, EffectSlot::Filter)
            ]
        );
        assert!(!before.has_drop_shadow());
        let after = evaluate_effects(Some(&stack(vec![blur(), shadow()])), true).unwrap();
        assert_eq!(
            after.order,
            [
                (FilterEffect::Blur, EffectSlot::Filter),
                (FilterEffect::Shadow, EffectSlot::Filter)
            ]
        );
        assert!(after.has_drop_shadow());
        // 상자 요소가 아니면 Shadow는 언제나 drop-shadow입니다.
        let leaf = evaluate_effects(Some(&stack(vec![shadow(), blur()])), false).unwrap();
        assert!(leaf.has_drop_shadow());
        // 순서는 정해진 규칙 함수 하나의 결과와 같습니다.
        assert_eq!(
            after.order.iter().map(|e| e.1).collect::<Vec<_>>(),
            effect_slots([FilterEffect::Blur, FilterEffect::Shadow], true)
        );
        assert!(evaluate_effects(None, true).unwrap().order.is_empty());
    }
}

//! 토큰 Web 출력 계획 (XXII.2.3, ADR-0106 결정 5, 속성 모델 10장).
//!
//! CSS 이름·값 텍스트는 만들지 않습니다(문서 `custom_property`·`css::value` 함수를 쓰는 상위 ops가 조립). 이
//! 모듈은 어떤 토큰을 어디에 선언할지만 정합니다.
//!
//! - CSS 값 텍스트가 있는 토큰(호출자 판정 `has_css_text`)은 등록 custom property입니다. 기본 환경 값(또는 alias)은
//!   `:root`, 조건부 값은 `:root`를 감싼 `@media` 블록입니다.
//! - 그 밖의 토큰은 생성 상수입니다.
//! - `interaction_state`에 따라 값이 달라지는 토큰은 토큰 선언에서 빼고 [`InteractionToken`]으로 넘깁니다(XXII.2.6).
//! - media 조건: `color_scheme`(dark), `contrast`(high), `reduced_motion`(true)은 key마다 기본이 아닌 값이 하나뿐이므로
//!   조합은 key 부분집합입니다. 조건에 나온 key 집합 K의 부분집합을 (크기, key 순) 순서로 보고, 그때까지 낸 블록의
//!   cascade 결과와 값이 다를 때만 블록에 넣습니다. K의 부분집합인데 값이 없으면 기본 값이고, K 밖 key는 값을 바꾸지
//!   않습니다(평가기는 식이 읽는 key만 바꿔 평가). `viewport_*`·`density`·`locale` 등은 표현하지 않고
//!   [`TokenDiagnosticKind::EnvironmentUnsupported`]를 냅니다.

use stellar_core_types::ids::StableId;
use stellar_core_types::typed_value::TypedValue;

use crate::ir::token_env::normalize_when;
use crate::ir::{
    IrColorScheme, IrContrast, IrEnvironmentValue, IrInteractionState, IrToken, IrTree,
};

/// CSS 토큰 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssToken {
    /// 토큰.
    pub id: StableId,
    /// 기본 환경 값(`@property initial-value`).
    pub default: TypedValue,
    /// `:root` 값이 다른 토큰 참조이면 그 토큰.
    pub alias: Option<StableId>,
}

/// `:root`를 감싼 `@media` 블록 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaBlock {
    /// media query 텍스트(`(prefers-color-scheme: dark) and (...)`).
    pub query: String,
    /// (토큰, 값). 토큰 id 순.
    pub values: Vec<(StableId, TypedValue)>,
}

/// `interaction_state` 조건부 토큰(XXII.2.6이 Control 상태 selector로 냄).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionToken {
    /// 토큰.
    pub id: StableId,
    /// 기본 상태 값.
    pub default: TypedValue,
    /// (상태, 값). 상태 순.
    pub states: Vec<(IrInteractionState, TypedValue)>,
}

/// 토큰 진단 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TokenDiagnosticKind {
    /// Web 토큰 선언으로 표현하지 않는 환경 조건(값을 뺌).
    EnvironmentUnsupported,
    /// `interaction_state`와 다른 조건이 함께 있는 값(뺌).
    InteractionCombined,
    /// 상수 토큰의 조건부 값(상수는 기본 환경 값만 냄).
    ConstantVariantDropped,
}

impl TokenDiagnosticKind {
    /// 안정 code.
    pub const fn code(self) -> &'static str {
        match self {
            TokenDiagnosticKind::EnvironmentUnsupported => "token-environment-unsupported",
            TokenDiagnosticKind::InteractionCombined => "token-interaction-combined",
            TokenDiagnosticKind::ConstantVariantDropped => "token-constant-variant-dropped",
        }
    }
}

/// 토큰 진단.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TokenDiagnostic {
    /// 토큰.
    pub token: StableId,
    /// 종류.
    pub kind: TokenDiagnosticKind,
}

/// 토큰 출력 계획.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenPlan {
    /// CSS 토큰(id 순).
    pub css: Vec<CssToken>,
    /// media 블록(cascade 순).
    pub media: Vec<MediaBlock>,
    /// 생성 상수 토큰(id 순).
    pub constants: Vec<StableId>,
    /// `interaction_state` 조건부 토큰(id 순).
    pub interaction: Vec<InteractionToken>,
    /// 진단(토큰·종류 순, 중복 없음).
    pub diagnostics: Vec<TokenDiagnostic>,
}

/// media key 하나(기본이 아닌 값이 하나뿐인 key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MediaKey {
    Dark,
    HighContrast,
    ReducedMotion,
}

impl MediaKey {
    const ALL: [MediaKey; 3] = [
        MediaKey::Dark,
        MediaKey::HighContrast,
        MediaKey::ReducedMotion,
    ];

    fn bit(self) -> u8 {
        match self {
            MediaKey::Dark => 1,
            MediaKey::HighContrast => 2,
            MediaKey::ReducedMotion => 4,
        }
    }

    fn feature(self) -> &'static str {
        match self {
            MediaKey::Dark => "(prefers-color-scheme: dark)",
            MediaKey::HighContrast => "(prefers-contrast: more)",
            MediaKey::ReducedMotion => "(prefers-reduced-motion: reduce)",
        }
    }

    fn of(value: IrEnvironmentValue) -> Option<MediaKey> {
        match value {
            IrEnvironmentValue::ColorScheme(IrColorScheme::Dark) => Some(MediaKey::Dark),
            IrEnvironmentValue::Contrast(IrContrast::High) => Some(MediaKey::HighContrast),
            IrEnvironmentValue::ReducedMotion(true) => Some(MediaKey::ReducedMotion),
            _ => None,
        }
    }
}

/// 조합 부분집합의 cascade 순서(크기, 그다음 bit 값).
fn subset_order() -> Vec<u8> {
    let mut subsets: Vec<u8> = (1u8..8).collect();
    subsets.sort_by_key(|s| (s.count_ones(), *s));
    subsets
}

/// media query 텍스트.
pub fn media_query(subset: u8) -> String {
    MediaKey::ALL
        .iter()
        .filter(|k| subset & k.bit() != 0)
        .map(|k| k.feature())
        .collect::<Vec<_>>()
        .join(" and ")
}

/// 토큰 하나의 media 조합별 값과 진단.
fn media_values(token: &IrToken, diagnostics: &mut Vec<TokenDiagnostic>) -> Vec<(u8, TypedValue)> {
    let mut by_subset: Vec<(u8, TypedValue)> = Vec::new();
    for variant in &token.variants {
        let mut subset = 0u8;
        let when = normalize_when(&variant.when);
        let mut supported = when.as_ref().is_some_and(|w| !w.is_empty());
        for value in when.iter().flatten() {
            match MediaKey::of(*value) {
                Some(key) => subset |= key.bit(),
                None => supported = false,
            }
        }
        if !supported {
            diagnostics.push(TokenDiagnostic {
                token: token.id,
                kind: TokenDiagnosticKind::EnvironmentUnsupported,
            });
            continue;
        }
        by_subset.push((subset, variant.value.clone()));
    }
    // 값이 정해진 key 집합 K(조건에 나온 key). K 밖 key는 값을 바꾸지 않습니다.
    let relevant = by_subset.iter().fold(0u8, |acc, (s, _)| acc | s);
    // cascade: K의 부분집합을 순서대로 보고, 그때까지 낸 블록 중 부분집합인 마지막 블록의 값(없으면 기본)과 다를 때만
    // 냅니다. K의 부분집합인데 값이 없으면 기본 값입니다.
    let mut emitted: Vec<(u8, TypedValue)> = Vec::new();
    for subset in subset_order() {
        if subset & !relevant != 0 {
            continue;
        }
        let wanted = by_subset
            .iter()
            .find(|(s, _)| *s == subset)
            .map_or(&token.value, |(_, v)| v);
        let current = emitted
            .iter()
            .rev()
            .find(|(s, _)| s & subset == *s)
            .map_or(&token.value, |(_, v)| v);
        if wanted != current {
            emitted.push((subset, wanted.clone()));
        }
    }
    emitted
}

/// IR 토큰의 Web 출력 계획을 만듭니다. `has_css_text`는 값의 CSS 텍스트가 있는지(문서 `literal_text`) 판정합니다.
pub fn plan_tokens(ir: &IrTree, has_css_text: &dyn Fn(&TypedValue) -> bool) -> TokenPlan {
    let mut plan = TokenPlan::default();
    let mut media: Vec<(u8, StableId, TypedValue)> = Vec::new();
    for token in &ir.tokens {
        if token.depends_on_interaction() {
            let mut states = Vec::new();
            for variant in &token.variants {
                match variant.when.as_slice() {
                    [IrEnvironmentValue::InteractionState(state)] => {
                        states.push((*state, variant.value.clone()));
                    }
                    when if when
                        .iter()
                        .any(|e| matches!(e, IrEnvironmentValue::InteractionState(_))) =>
                    {
                        plan.diagnostics.push(TokenDiagnostic {
                            token: token.id,
                            kind: TokenDiagnosticKind::InteractionCombined,
                        });
                    }
                    _ => plan.diagnostics.push(TokenDiagnostic {
                        token: token.id,
                        kind: TokenDiagnosticKind::InteractionCombined,
                    }),
                }
            }
            states.sort_by_key(|(state, _)| *state);
            plan.interaction.push(InteractionToken {
                id: token.id,
                default: token.value.clone(),
                states,
            });
            continue;
        }
        if !has_css_text(&token.value) {
            if !token.variants.is_empty() {
                plan.diagnostics.push(TokenDiagnostic {
                    token: token.id,
                    kind: TokenDiagnosticKind::ConstantVariantDropped,
                });
            }
            plan.constants.push(token.id);
            continue;
        }
        plan.css.push(CssToken {
            id: token.id,
            default: token.value.clone(),
            alias: token.alias,
        });
        for (subset, value) in media_values(token, &mut plan.diagnostics) {
            media.push((subset, token.id, value));
        }
    }
    for subset in subset_order() {
        let values: Vec<(StableId, TypedValue)> = media
            .iter()
            .filter(|(s, _, _)| *s == subset)
            .map(|(_, id, value)| (*id, value.clone()))
            .collect();
        if !values.is_empty() {
            plan.media.push(MediaBlock {
                query: media_query(subset),
                values,
            });
        }
    }
    plan.diagnostics.sort();
    plan.diagnostics.dedup();
    plan
}

/// 같은 media 조합에서 cascade가 낼 값(검증용). `subset`은 dark=1, high contrast=2, reduced motion=4 bit 합입니다.
pub fn cascade_value(plan: &TokenPlan, token: StableId, subset: u8) -> Option<&TypedValue> {
    let base = plan.css.iter().find(|t| t.id == token)?;
    let mut value = &base.default;
    for order in subset_order() {
        if order & subset != order {
            continue;
        }
        let query = media_query(order);
        if let Some(block) = plan.media.iter().find(|b| b.query == query) {
            if let Some((_, v)) = block.values.iter().find(|(id, _)| *id == token) {
                value = v;
            }
        }
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{self as fx, sid, Fixture};
    use crate::ir::{IrTokenVariant, TokenDecl};

    fn has_css(value: &TypedValue) -> bool {
        matches!(
            value,
            TypedValue::Length(_)
                | TypedValue::Percentage(_)
                | TypedValue::Angle(_)
                | TypedValue::Duration(_)
                | TypedValue::Number(_)
                | TypedValue::Color(_)
        )
    }

    fn dark() -> IrEnvironmentValue {
        IrEnvironmentValue::ColorScheme(IrColorScheme::Dark)
    }

    fn high() -> IrEnvironmentValue {
        IrEnvironmentValue::Contrast(IrContrast::High)
    }

    fn red() -> TypedValue {
        fx::srgb(1.0, 0.0, 0.0, 1.0)
    }

    fn blue() -> TypedValue {
        fx::srgb(0.0, 0.0, 1.0, 1.0)
    }

    fn green() -> TypedValue {
        fx::srgb(0.0, 1.0, 0.0, 1.0)
    }

    fn with_tokens(tokens: Vec<TokenDecl>) -> crate::ir::CodegenIr {
        let mut fixture = Fixture::new();
        fixture.tokens = tokens;
        fixture.token_bindings.clear();
        fixture.machines.clear();
        fixture.contexts = crate::ir::IrContexts::default();
        fixture.ir(&fx::web_profile())
    }

    #[test]
    fn interaction_tokens_are_split_and_media_values_follow_the_cascade() {
        let fixture = Fixture::new();
        let ir = fixture.ir(&fx::web_profile());
        let plan = plan_tokens(&ir, &has_css);
        // accent는 hover 값이 있어 토큰 선언에서 빠집니다.
        assert!(plan.css.iter().all(|t| t.id != sid(fx::TOKEN_ACCENT)));
        assert_eq!(plan.interaction.len(), 1);
        assert_eq!(plan.interaction[0].states[0].0, IrInteractionState::Hover);
        // surface는 dark 블록 하나.
        assert_eq!(plan.media.len(), 1);
        assert_eq!(plan.media[0].query, "(prefers-color-scheme: dark)");
    }

    #[test]
    fn combined_conditions_emit_only_blocks_that_change_the_cascade() {
        // dark → red, high → green, dark+high → 기본(blue). dark+high 블록은 cascade(마지막 부분집합 = high → green)와
        // 달라 blue로 냅니다.
        let token = TokenDecl {
            variants: vec![
                IrTokenVariant {
                    when: vec![dark()],
                    value: red(),
                },
                IrTokenVariant {
                    when: vec![high()],
                    value: green(),
                },
            ],
            ..TokenDecl::new(sid(90), "c", blue())
        };
        let ir = with_tokens(vec![token]);
        let plan = plan_tokens(&ir, &has_css);
        let queries: Vec<&str> = plan.media.iter().map(|b| b.query.as_str()).collect();
        assert_eq!(
            queries,
            [
                "(prefers-color-scheme: dark)",
                "(prefers-contrast: more)",
                "(prefers-color-scheme: dark) and (prefers-contrast: more)"
            ]
        );
        for subset in 0u8..8 {
            let expected = match subset & 3 {
                1 => red(),
                2 => green(),
                _ => blue(),
            };
            assert_eq!(
                cascade_value(&plan, sid(90), subset),
                Some(&expected),
                "{subset}"
            );
        }
        // 같은 값이면 블록을 내지 않습니다.
        let same = TokenDecl {
            variants: vec![IrTokenVariant {
                when: vec![dark(), high()],
                value: blue(),
            }],
            ..TokenDecl::new(sid(91), "d", blue())
        };
        let plan = plan_tokens(&with_tokens(vec![same]), &has_css);
        assert!(plan.media.is_empty());
    }

    #[test]
    fn unsupported_environments_and_constants_are_diagnosed() {
        let tokens = vec![
            TokenDecl {
                variants: vec![IrTokenVariant {
                    when: vec![IrEnvironmentValue::Other(7)],
                    value: red(),
                }],
                ..TokenDecl::new(sid(92), "viewport", blue())
            },
            TokenDecl {
                variants: vec![IrTokenVariant {
                    when: vec![dark()],
                    value: TypedValue::String("밤".into()),
                }],
                ..TokenDecl::new(sid(93), "label", TypedValue::String("낮".into()))
            },
            TokenDecl::new(sid(94), "flag", TypedValue::Boolean(true)),
        ];
        let plan = plan_tokens(&with_tokens(tokens), &has_css);
        assert_eq!(plan.constants, vec![sid(93), sid(94)]);
        assert_eq!(
            plan.diagnostics,
            vec![
                TokenDiagnostic {
                    token: sid(92),
                    kind: TokenDiagnosticKind::EnvironmentUnsupported
                },
                TokenDiagnostic {
                    token: sid(93),
                    kind: TokenDiagnosticKind::ConstantVariantDropped
                },
            ]
        );
        assert!(plan.media.is_empty());
    }
}

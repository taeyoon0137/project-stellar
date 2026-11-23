//! alpha 합성과 blend mode 수식 (XI.10.2, ADR-0102 결정 3).
//!
//! 입력과 출력은 working 값([`WorkingColor`], 선형 premultiplied)입니다. backend shader와 CPU 기준 rasterizer가 같은
//! 식을 씁니다.
//!
//! - **source-over**: `Co = Cs + Cb·(1 − αs)`, `αo = αs + αb·(1 − αs)` ([`source_over`]).
//! - **blend mode**(W3C Compositing and Blending Level 1, separable):
//!   `Co = Cs·(1 − αb) + Cb·(1 − αs) + αs·αb·B(Cb/αb, Cs/αs)`, `αo`는 source-over와 같습니다([`composite`]). `B`는
//!   [`blend_channel`]입니다. 비연관 값은 alpha가 0이면 0으로 둡니다.
//!
//! | mode | `B(cb, cs)` |
//! |---|---|
//! | normal | `cs` |
//! | multiply | `cb·cs` |
//! | screen | `cb + cs − cb·cs` |
//! | overlay | `hard-light(cs, cb)`: `cb ≤ 0.5`면 `2·cb·cs`, 아니면 `screen(cs, 2·cb − 1)` |
//! | darken | `min(cb, cs)` |
//! | lighten | `max(cb, cs)` |
//!
//! ## 지원 목록과 진단
//!
//! 초기 지원 목록은 [`SceneBlendMode::ALL`](document `BlendMode`와 같은 6개)입니다. 목록 밖 tag는 [`resolve_blend_tag`]가
//! [`BlendDiagnostic`](`render.blend.unsupported_mode`)으로 돌려주며, 호출자는 진단을 남기고 Normal로 그립니다
//! ([`resolve_or_normal`]). 조용히 바꾸지 않습니다.
//!
//! ## backend program
//!
//! Normal은 고정 기능 premultiplied over로 합성하는 `LayerComposite`, 나머지는 shader가 backdrop을 읽어 위 식으로
//! 합성하는 `BlendComposite(mode)`입니다([`composite_program`]). `BlendComposite`는 shader가 합성을 끝내므로 pipeline
//! blend state가 `Replace`여야 합니다([`composite_blend_state`]).

use std::fmt;

pub use stellar_scene::stack::SceneBlendMode;

use crate::backend::{BlendState, ShaderProgram};
use crate::color::WorkingColor;

/// premultiplied source-over.
pub fn source_over(src: WorkingColor, dst: WorkingColor) -> WorkingColor {
    let k = 1.0 - src.alpha();
    WorkingColor::premultiplied(std::array::from_fn(|i| src.rgba[i] + dst.rgba[i] * k))
}

/// separable blend 함수 `B(cb, cs)`(비연관 선형 채널).
pub fn blend_channel(mode: SceneBlendMode, cb: f32, cs: f32) -> f32 {
    let screen = |b: f32, s: f32| b + s - b * s;
    match mode {
        SceneBlendMode::Normal => cs,
        SceneBlendMode::Multiply => cb * cs,
        SceneBlendMode::Screen => screen(cb, cs),
        SceneBlendMode::Overlay => {
            if cb <= 0.5 {
                2.0 * cb * cs
            } else {
                screen(cs, 2.0 * cb - 1.0)
            }
        }
        SceneBlendMode::Darken => cb.min(cs),
        SceneBlendMode::Lighten => cb.max(cs),
    }
}

/// `src`를 backdrop `dst`에 `mode`로 합성합니다(모듈 문서 일반식).
pub fn composite(mode: SceneBlendMode, src: WorkingColor, dst: WorkingColor) -> WorkingColor {
    if mode == SceneBlendMode::Normal {
        return source_over(src, dst);
    }
    let (a_s, a_b) = (src.alpha(), dst.alpha());
    let cs = src.unpremultiplied();
    let cb = dst.unpremultiplied();
    let mut out = [0.0; 4];
    for i in 0..3 {
        out[i] = src.rgba[i] * (1.0 - a_b)
            + dst.rgba[i] * (1.0 - a_s)
            + a_s * a_b * blend_channel(mode, cb[i], cs[i]);
    }
    out[3] = a_s + a_b * (1.0 - a_s);
    WorkingColor::premultiplied(out)
}

/// 지원 목록 밖 blend mode 진단.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlendDiagnostic {
    /// 받은 tag.
    pub tag: u8,
}

impl BlendDiagnostic {
    /// 정적 진단 코드.
    pub const CODE: &'static str = "render.blend.unsupported_mode";

    /// 정적 진단 코드.
    pub const fn code(&self) -> &'static str {
        BlendDiagnostic::CODE
    }
}

impl fmt::Display for BlendDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "지원하지 않는 blend mode tag {}: normal로 그립니다",
            self.tag
        )
    }
}

/// tag를 지원 blend mode로 풉니다. 목록 밖이면 진단입니다.
pub fn resolve_blend_tag(tag: u8) -> Result<SceneBlendMode, BlendDiagnostic> {
    SceneBlendMode::from_tag(tag).ok_or(BlendDiagnostic { tag })
}

/// tag를 풀고, 목록 밖이면 진단을 `diagnostics`에 남긴 뒤 Normal을 돌려줍니다.
pub fn resolve_or_normal(tag: u8, diagnostics: &mut Vec<BlendDiagnostic>) -> SceneBlendMode {
    match resolve_blend_tag(tag) {
        Ok(mode) => mode,
        Err(diagnostic) => {
            diagnostics.push(diagnostic);
            SceneBlendMode::Normal
        }
    }
}

/// layer를 `mode`로 합성하는 shader program.
pub const fn composite_program(mode: SceneBlendMode) -> ShaderProgram {
    match mode {
        SceneBlendMode::Normal => ShaderProgram::LayerComposite,
        other => ShaderProgram::BlendComposite(other),
    }
}

/// [`composite_program`]의 pipeline blend state.
pub const fn composite_blend_state(mode: SceneBlendMode) -> BlendState {
    match mode {
        SceneBlendMode::Normal => BlendState::PremultipliedOver,
        _ => BlendState::Replace,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(r: f32, g: f32, b: f32, a: f32) -> WorkingColor {
        WorkingColor::from_straight([r, g, b], a)
    }

    #[track_caller]
    fn close(a: WorkingColor, b: [f32; 4]) {
        for (x, y) in a.rgba.iter().zip(b.iter()) {
            assert!((x - y).abs() < 1e-6, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn separable_functions_match_w3c_definitions() {
        use SceneBlendMode::*;
        let (cb, cs) = (0.25, 0.6);
        assert_eq!(blend_channel(Normal, cb, cs), cs);
        assert!((blend_channel(Multiply, cb, cs) - 0.15).abs() < 1e-7);
        assert!((blend_channel(Screen, cb, cs) - 0.7).abs() < 1e-7);
        // overlay = hard-light(cs, cb): cb ≤ 0.5 → 2·cb·cs.
        assert!((blend_channel(Overlay, cb, cs) - 0.3).abs() < 1e-7);
        assert!((blend_channel(Overlay, 0.75, 0.6) - (0.6 + 0.5 - 0.6 * 0.5)).abs() < 1e-7);
        assert_eq!(blend_channel(Darken, cb, cs), cb);
        assert_eq!(blend_channel(Lighten, cb, cs), cs);
    }

    #[test]
    fn opaque_composites_reduce_to_blend_function() {
        let src = c(0.6, 0.2, 0.9, 1.0);
        let dst = c(0.25, 0.8, 0.5, 1.0);
        for mode in SceneBlendMode::ALL {
            let out = composite(mode, src, dst);
            let expected = [0, 1, 2].map(|i| blend_channel(mode, dst.rgba[i], src.rgba[i]));
            close(out, [expected[0], expected[1], expected[2], 1.0]);
        }
    }

    #[test]
    fn alpha_terms_follow_general_formula() {
        // 투명 backdrop에는 source 그대로, 투명 source는 backdrop 그대로입니다(모든 mode).
        let src = c(0.6, 0.2, 0.9, 0.5);
        let dst = c(0.25, 0.8, 0.5, 0.75);
        for mode in SceneBlendMode::ALL {
            close(composite(mode, src, WorkingColor::TRANSPARENT), src.rgba);
            close(composite(mode, WorkingColor::TRANSPARENT, dst), dst.rgba);
            assert!((composite(mode, src, dst).alpha() - (0.5 + 0.75 * 0.5)).abs() < 1e-6);
        }
        // normal은 source-over와 같습니다.
        assert_eq!(
            composite(SceneBlendMode::Normal, src, dst),
            source_over(src, dst)
        );
        // multiply 반투명 손 계산: R = 0.3·0.25 + 0.1875·0.5 + 0.5·0.75·(0.25·0.6).
        let out = composite(SceneBlendMode::Multiply, src, dst);
        let expected_r = 0.3 * 0.25 + 0.1875 * 0.5 + 0.375 * 0.15;
        assert!((out.rgba[0] - expected_r).abs() < 1e-6);
    }

    #[test]
    fn unsupported_tags_are_diagnosed_not_silently_replaced() {
        for mode in SceneBlendMode::ALL {
            assert_eq!(resolve_blend_tag(mode.tag()), Ok(mode));
        }
        let error = resolve_blend_tag(6).unwrap_err();
        assert_eq!(error, BlendDiagnostic { tag: 6 });
        assert_eq!(error.code(), "render.blend.unsupported_mode");
        let mut diagnostics = Vec::new();
        assert_eq!(
            resolve_or_normal(42, &mut diagnostics),
            SceneBlendMode::Normal
        );
        assert_eq!(
            resolve_or_normal(1, &mut diagnostics),
            SceneBlendMode::Multiply
        );
        assert_eq!(diagnostics, vec![BlendDiagnostic { tag: 42 }]);
    }

    #[test]
    fn programs_and_blend_states_per_mode() {
        assert_eq!(
            composite_program(SceneBlendMode::Normal),
            ShaderProgram::LayerComposite
        );
        assert_eq!(
            composite_blend_state(SceneBlendMode::Normal),
            BlendState::PremultipliedOver
        );
        for mode in &SceneBlendMode::ALL[1..] {
            assert_eq!(
                composite_program(*mode),
                ShaderProgram::BlendComposite(*mode)
            );
            assert_eq!(composite_blend_state(*mode), BlendState::Replace);
        }
    }
}

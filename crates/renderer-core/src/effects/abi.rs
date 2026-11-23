//! 공통 shader ABI의 byte 배치 (ADR-0102 결정 5).
//!
//! backend shader(Metal `renderer-metal` `shaders`, 후속 D3D12)는 이 배치를 따릅니다. 공통 계층과 harness는 이
//! 모듈의 함수로 vertex·uniform byte를 만듭니다.
//!
//! | 구조 | byte | 배치(little-endian `f32`) |
//! |---|---|---|
//! | `SolidVertex` | [`SOLID_VERTEX_STRIDE`] | position(x, y), color(r, g, b, a) |
//! | `TexturedVertex` | [`TEXTURED_VERTEX_STRIDE`] | position(x, y), uv(u, v) |
//! | `DrawUniforms` | [`DRAW_UNIFORMS_SIZE`] | tint(4), depth, mode, param(2) |
//! | `BlurUniforms` | [`BLUR_UNIFORMS_SIZE`] | tint(4), depth, sigma, direction(2), source_alpha, pad(3) |
//!
//! - 좌표는 attachment pixel 좌표(좌상단 원점, y 아래로)이고 색은 working 값([`crate::color`])입니다.
//! - `DrawUniforms.mode`: `LayerComposite`·`BlendComposite`에서 [`MODE_MASKED`]이면 texture slot 1의 coverage(`.r`)를
//!   곱합니다. 다른 program은 0입니다.
//! - `BlurUniforms`: `GaussianBlur` 전용. `direction`은 단위 texel 방향(가로 `(1, 0)`, 세로 `(0, 1)`), `sigma`는 표준편차
//!   (pixel), `source_alpha`가 1이면 texel alpha를 모든 채널로 퍼뜨린 뒤 blur합니다. 출력은 blur 결과 × tint입니다.

/// position·color vertex stride(byte).
pub const SOLID_VERTEX_STRIDE: u32 = 24;

/// position·uv vertex stride(byte).
pub const TEXTURED_VERTEX_STRIDE: u32 = 16;

/// `DrawUniforms` byte 크기.
pub const DRAW_UNIFORMS_SIZE: u32 = 32;

/// `BlurUniforms` byte 크기.
pub const BLUR_UNIFORMS_SIZE: u32 = 48;

/// uniform 범위 offset 정렬(byte).
pub const UNIFORM_ALIGNMENT: u64 = 16;

/// `DrawUniforms.mode`: mask 없음.
pub const MODE_PLAIN: f32 = 0.0;

/// `DrawUniforms.mode`: texture slot 1 coverage mask를 곱함.
pub const MODE_MASKED: f32 = 1.0;

/// 기본 clip z(depth 미사용 pass).
pub const DEFAULT_DEPTH: f32 = 0.5;

fn push(out: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

/// `SolidVertex` 하나를 덧붙입니다.
pub fn push_solid_vertex(out: &mut Vec<u8>, position: [f32; 2], color: [f32; 4]) {
    push(out, &position);
    push(out, &color);
}

/// `TexturedVertex` 하나를 덧붙입니다.
pub fn push_textured_vertex(out: &mut Vec<u8>, position: [f32; 2], uv: [f32; 2]) {
    push(out, &position);
    push(out, &uv);
}

/// `DrawUniforms` byte.
pub fn draw_uniforms(tint: [f32; 4], depth: f32, mode: f32, param: [f32; 2]) -> Vec<u8> {
    let mut out = Vec::with_capacity(DRAW_UNIFORMS_SIZE as usize);
    push(&mut out, &tint);
    push(&mut out, &[depth, mode, param[0], param[1]]);
    out
}

/// `BlurUniforms` byte.
pub fn blur_uniforms(
    tint: [f32; 4],
    sigma: f32,
    direction: [f32; 2],
    source_alpha: bool,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(BLUR_UNIFORMS_SIZE as usize);
    push(&mut out, &tint);
    push(
        &mut out,
        &[DEFAULT_DEPTH, sigma, direction[0], direction[1]],
    );
    push(
        &mut out,
        &[f32::from(u8::from(source_alpha)), 0.0, 0.0, 0.0],
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_have_documented_sizes() {
        let mut solid = Vec::new();
        push_solid_vertex(&mut solid, [1.0, 2.0], [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(solid.len(), SOLID_VERTEX_STRIDE as usize);
        let mut textured = Vec::new();
        push_textured_vertex(&mut textured, [1.0, 2.0], [0.5, 0.5]);
        assert_eq!(textured.len(), TEXTURED_VERTEX_STRIDE as usize);
        let draw = draw_uniforms([1.0; 4], DEFAULT_DEPTH, MODE_MASKED, [0.0; 2]);
        assert_eq!(draw.len(), DRAW_UNIFORMS_SIZE as usize);
        assert_eq!(&draw[20..24], &MODE_MASKED.to_le_bytes());
        let blur = blur_uniforms([1.0; 4], 2.0, [1.0, 0.0], true);
        assert_eq!(blur.len(), BLUR_UNIFORMS_SIZE as usize);
        assert_eq!(&blur[20..24], &2.0f32.to_le_bytes());
        assert_eq!(&blur[32..36], &1.0f32.to_le_bytes());
        assert_eq!(DRAW_UNIFORMS_SIZE as u64 % UNIFORM_ALIGNMENT, 0);
        assert_eq!(BLUR_UNIFORMS_SIZE as u64 % UNIFORM_ALIGNMENT, 0);
    }
}

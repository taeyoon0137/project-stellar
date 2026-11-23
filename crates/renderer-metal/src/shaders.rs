//! Metal shader 소스와 shader ABI.
//!
//! ## shader ABI(공통 계층 ↔ Metal backend 계약)
//!
//! | slot | 단계 | 내용 |
//! |---|---|---|
//! | buffer 0 | vertex | vertex 배열(`SetVertexBuffer`, program별 layout) |
//! | buffer 1 | vertex·fragment | [`DRAW_UNIFORMS_SIZE`] byte `DrawUniforms`(`SetUniforms`) |
//! | buffer 2 | vertex | backend가 pass마다 넣는 target 크기(`float2`, pixel) |
//! | texture·sampler 0 | fragment | sampling program의 source(`BindTexture { slot: 0 }`) |
//! | texture·sampler 1 | fragment | `LayerComposite`·`BlendComposite`의 coverage mask(`DrawUniforms.mode = 1`일 때만 읽음) |
//!
//! - 좌표는 attachment pixel 좌표(좌상단 원점, y 아래로)입니다. backend가 clip 좌표로 바꿉니다.
//! - 색은 working space 값입니다: surface primaries 선형광, premultiplied alpha
//!   (`stellar_renderer_core::color`, XI.10.1). fragment는 encode하지 않고, attachment format이 저장 방식을
//!   정합니다(`*UnormSrgb`는 하드웨어 transfer encode, 나머지는 선형 그대로).
//! - byte 배치는 공통 ABI(`stellar_renderer_core::effects::abi`)가 소유합니다.
//!   `DrawUniforms { float4 tint; float depth; float mode; float2 param; }` = 32 byte. clip z = depth.
//!
//! | program | vertex layout(stride) | fragment 출력 | texture 0 |
//! |---|---|---|---|
//! | `SolidFill` | position·color(24) | `color × tint` | — |
//! | `Texture` | position·uv(16) | `sample × tint` | 필요 |
//! | `LayerComposite` | position·uv(16) | `sample × tint × mask`(blend는 pipeline premultiplied over) | 필요 |
//! | `BlendComposite(mode)` | position·uv(16) | `composite(mode, sample × tint × mask, backdrop)`(framebuffer fetch, blend `Replace`) | 필요 |
//! | `GlyphMask` | position·uv(16) | `tint × sample.r`(coverage) | 필요 |
//! | `ClipStencil` | position·color(24) | 없음(color write mask 0, stencil만) | — |
//! | `ClipMask` | position·color(24) | `tint.a`(coverage, 모든 채널) | — |
//! | `GaussianBlur` | — | 미구현(XI.10.4, `Unsupported`) | — |
//!
//! - `mask`는 `DrawUniforms.mode`가 1이면 texture 1의 `.r`, 아니면 1입니다. mask를 쓰지 않는 draw에서도 slot 1이 비지 않도록
//!   backend가 slot 0 texture를 slot 1에 함께 bind합니다(`commands`).
//! - `BlendComposite`는 Apple GPU framebuffer fetch(`[[color(0)]]`, 선형 working 값)로 backdrop을 읽어
//!   `stellar_renderer_core::blend` 식(W3C separable, ADR-0102 결정 3)으로 합성합니다. shader가 합성을 끝내므로 pipeline
//!   blend state는 `Replace`여야 합니다(`PremultipliedOver`는 `InvalidUsage`).

use stellar_renderer_core::backend::ShaderProgram;
use stellar_renderer_core::blend::SceneBlendMode;
use stellar_renderer_core::effects::abi;

/// `DrawUniforms` byte 크기(공통 ABI, `stellar_renderer_core::effects::abi`).
pub(crate) const DRAW_UNIFORMS_SIZE: u32 = abi::DRAW_UNIFORMS_SIZE;

/// uniform 범위 offset 정렬(byte). `float4` 정렬과 같습니다.
pub(crate) const UNIFORM_OFFSET_ALIGNMENT: u64 = abi::UNIFORM_ALIGNMENT;

/// vertex·index buffer offset 정렬(byte).
pub(crate) const VERTEX_OFFSET_ALIGNMENT: u64 = 4;

/// position·color vertex stride(byte).
pub(crate) const SOLID_VERTEX_STRIDE: u32 = abi::SOLID_VERTEX_STRIDE;

/// position·uv vertex stride(byte).
pub(crate) const TEXTURED_VERTEX_STRIDE: u32 = abi::TEXTURED_VERTEX_STRIDE;

/// fragment texture slot 수.
pub(crate) const MAX_TEXTURE_SLOTS: u8 = 4;

/// 내장 shader library 소스(MSL).
pub(crate) const BUILTIN_SOURCE: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct TargetUniforms { float2 size; };
struct DrawUniforms { float4 tint; float depth; float mode; float2 param; };

static float4 stellar_clip(float2 px, float depth, constant TargetUniforms& target) {
    float2 ndc = float2(px.x / target.size.x * 2.0 - 1.0, 1.0 - px.y / target.size.y * 2.0);
    return float4(ndc, depth, 1.0);
}

struct SolidVertex { packed_float2 position; packed_float4 color; };
struct SolidOut { float4 position [[position]]; float4 color; };

vertex SolidOut stellar_solid_vertex(uint vid [[vertex_id]],
                                     const device SolidVertex* vertices [[buffer(0)]],
                                     constant DrawUniforms& draw [[buffer(1)]],
                                     constant TargetUniforms& target [[buffer(2)]]) {
    SolidVertex v = vertices[vid];
    SolidOut out;
    out.position = stellar_clip(float2(v.position), draw.depth, target);
    out.color = float4(v.color) * draw.tint;
    return out;
}

fragment float4 stellar_solid_fragment(SolidOut in [[stage_in]]) {
    return in.color;
}

fragment float4 stellar_clip_stencil_fragment(SolidOut in [[stage_in]]) {
    return float4(0.0);
}

fragment float4 stellar_clip_mask_fragment(SolidOut in [[stage_in]],
                                           constant DrawUniforms& draw [[buffer(1)]]) {
    return float4(draw.tint.a);
}

struct TexturedVertex { packed_float2 position; packed_float2 uv; };
struct TexturedOut { float4 position [[position]]; float2 uv; };

vertex TexturedOut stellar_textured_vertex(uint vid [[vertex_id]],
                                           const device TexturedVertex* vertices [[buffer(0)]],
                                           constant DrawUniforms& draw [[buffer(1)]],
                                           constant TargetUniforms& target [[buffer(2)]]) {
    TexturedVertex v = vertices[vid];
    TexturedOut out;
    out.position = stellar_clip(float2(v.position), draw.depth, target);
    out.uv = float2(v.uv);
    return out;
}

fragment float4 stellar_texture_fragment(TexturedOut in [[stage_in]],
                                         constant DrawUniforms& draw [[buffer(1)]],
                                         texture2d<float> source [[texture(0)]],
                                         sampler source_sampler [[sampler(0)]]) {
    return source.sample(source_sampler, in.uv) * draw.tint;
}

static float4 stellar_layer_source(TexturedOut in, constant DrawUniforms& draw,
                                   texture2d<float> source, sampler source_sampler,
                                   texture2d<float> mask, sampler mask_sampler) {
    float4 value = source.sample(source_sampler, in.uv) * draw.tint;
    if (draw.mode > 0.5) {
        value *= mask.sample(mask_sampler, in.uv).r;
    }
    return value;
}

fragment float4 stellar_layer_fragment(TexturedOut in [[stage_in]],
                                       constant DrawUniforms& draw [[buffer(1)]],
                                       texture2d<float> source [[texture(0)]],
                                       sampler source_sampler [[sampler(0)]],
                                       texture2d<float> mask [[texture(1)]],
                                       sampler mask_sampler [[sampler(1)]]) {
    return stellar_layer_source(in, draw, source, source_sampler, mask, mask_sampler);
}

// W3C separable blend 함수 B(cb, cs). mode: 0 normal, 1 multiply, 2 screen, 3 overlay, 4 darken, 5 lighten
// (SceneBlendMode tag).
static float stellar_blend_channel(uint mode, float cb, float cs) {
    switch (mode) {
        case 1: return cb * cs;
        case 2: return cb + cs - cb * cs;
        case 3: return cb <= 0.5 ? 2.0 * cb * cs : cs + (2.0 * cb - 1.0) - cs * (2.0 * cb - 1.0);
        case 4: return min(cb, cs);
        case 5: return max(cb, cs);
        default: return cs;
    }
}

static float4 stellar_composite(uint mode, float4 src, float4 dst) {
    float as = src.a;
    float ab = dst.a;
    float3 cs = as > 0.0 ? src.rgb / as : float3(0.0);
    float3 cb = ab > 0.0 ? dst.rgb / ab : float3(0.0);
    float3 b = float3(stellar_blend_channel(mode, cb.r, cs.r),
                      stellar_blend_channel(mode, cb.g, cs.g),
                      stellar_blend_channel(mode, cb.b, cs.b));
    float3 co = src.rgb * (1.0 - ab) + dst.rgb * (1.0 - as) + as * ab * b;
    return float4(co, as + ab * (1.0 - as));
}

#define STELLAR_BLEND_FRAGMENT(NAME, MODE)                                                      \
fragment float4 NAME(TexturedOut in [[stage_in]],                                               \
                     float4 backdrop [[color(0)]],                                              \
                     constant DrawUniforms& draw [[buffer(1)]],                                 \
                     texture2d<float> source [[texture(0)]],                                    \
                     sampler source_sampler [[sampler(0)]],                                     \
                     texture2d<float> mask [[texture(1)]],                                      \
                     sampler mask_sampler [[sampler(1)]]) {                                     \
    float4 src = stellar_layer_source(in, draw, source, source_sampler, mask, mask_sampler);   \
    return stellar_composite(MODE, src, backdrop);                                              \
}

STELLAR_BLEND_FRAGMENT(stellar_blend_normal_fragment, 0)
STELLAR_BLEND_FRAGMENT(stellar_blend_multiply_fragment, 1)
STELLAR_BLEND_FRAGMENT(stellar_blend_screen_fragment, 2)
STELLAR_BLEND_FRAGMENT(stellar_blend_overlay_fragment, 3)
STELLAR_BLEND_FRAGMENT(stellar_blend_darken_fragment, 4)
STELLAR_BLEND_FRAGMENT(stellar_blend_lighten_fragment, 5)

fragment float4 stellar_glyph_fragment(TexturedOut in [[stage_in]],
                                       constant DrawUniforms& draw [[buffer(1)]],
                                       texture2d<float> source [[texture(0)]],
                                       sampler source_sampler [[sampler(0)]]) {
    return draw.tint * source.sample(source_sampler, in.uv).r;
}
"#;

/// program 하나의 shader 변형.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ProgramLayout {
    /// vertex 함수 이름.
    pub(crate) vertex: &'static str,
    /// fragment 함수 이름.
    pub(crate) fragment: &'static str,
    /// vertex stride(byte).
    pub(crate) stride: u32,
    /// fragment texture slot 0이 필요하면 참.
    pub(crate) needs_texture: bool,
    /// color를 쓰면 참(거짓이면 write mask 0).
    pub(crate) writes_color: bool,
    /// texture slot 1 coverage mask를 선언하면 참(비어 있으면 backend가 slot 0을 함께 bind).
    pub(crate) mask_slot: bool,
    /// uniform 최소 byte 수.
    pub(crate) uniform_size: u32,
    /// shader가 backdrop과 합성을 끝내 pipeline blend가 `Replace`여야 하면 참.
    pub(crate) shader_blend: bool,
}

/// program의 shader 변형. 아직 구현하지 않은 program은 `None`.
pub(crate) const fn program_layout(program: ShaderProgram) -> Option<ProgramLayout> {
    const fn solid(fragment: &'static str, writes_color: bool) -> ProgramLayout {
        ProgramLayout {
            vertex: "stellar_solid_vertex",
            fragment,
            stride: SOLID_VERTEX_STRIDE,
            needs_texture: false,
            writes_color,
            mask_slot: false,
            uniform_size: DRAW_UNIFORMS_SIZE,
            shader_blend: false,
        }
    }
    const fn textured(fragment: &'static str) -> ProgramLayout {
        ProgramLayout {
            vertex: "stellar_textured_vertex",
            fragment,
            stride: TEXTURED_VERTEX_STRIDE,
            needs_texture: true,
            writes_color: true,
            mask_slot: false,
            uniform_size: DRAW_UNIFORMS_SIZE,
            shader_blend: false,
        }
    }
    const fn layer(fragment: &'static str, shader_blend: bool) -> ProgramLayout {
        ProgramLayout {
            mask_slot: true,
            shader_blend,
            ..textured(fragment)
        }
    }
    match program {
        ShaderProgram::SolidFill => Some(solid("stellar_solid_fragment", true)),
        ShaderProgram::ClipStencil => Some(solid("stellar_clip_stencil_fragment", false)),
        ShaderProgram::ClipMask => Some(solid("stellar_clip_mask_fragment", true)),
        ShaderProgram::Texture => Some(textured("stellar_texture_fragment")),
        ShaderProgram::LayerComposite => Some(layer("stellar_layer_fragment", false)),
        ShaderProgram::BlendComposite(mode) => Some(layer(
            match mode {
                SceneBlendMode::Normal => "stellar_blend_normal_fragment",
                SceneBlendMode::Multiply => "stellar_blend_multiply_fragment",
                SceneBlendMode::Screen => "stellar_blend_screen_fragment",
                SceneBlendMode::Overlay => "stellar_blend_overlay_fragment",
                SceneBlendMode::Darken => "stellar_blend_darken_fragment",
                SceneBlendMode::Lighten => "stellar_blend_lighten_fragment",
            },
            true,
        )),
        ShaderProgram::GlyphMask => Some(textured("stellar_glyph_fragment")),
        ShaderProgram::GaussianBlur => None,
    }
}

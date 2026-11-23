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
//!
//! - 좌표는 attachment pixel 좌표(좌상단 원점, y 아래로)입니다. backend가 clip 좌표로 바꿉니다.
//! - 색은 working space 값입니다: surface primaries 선형광, premultiplied alpha
//!   (`stellar_renderer_core::color`, XI.10.1). fragment는 encode하지 않고, attachment format이 저장 방식을
//!   정합니다(`*UnormSrgb`는 하드웨어 transfer encode, 나머지는 선형 그대로).
//! - `DrawUniforms { float4 tint; float depth; float pad[3]; }` = 32 byte. clip z = depth.
//!
//! | program | vertex layout(stride) | fragment 출력 | texture 0 |
//! |---|---|---|---|
//! | `SolidFill` | position·color(24) | `color × tint` | — |
//! | `Texture`·`LayerComposite` | position·uv(16) | `sample × tint` | 필요 |
//! | `GlyphMask` | position·uv(16) | `tint × sample.r`(coverage) | 필요 |
//! | `ClipStencil` | position·color(24) | 없음(color write mask 0, stencil만) | — |
//! | `ClipMask` | position·color(24) | `tint.a`(coverage, 모든 채널) | — |
//! | `BlendComposite`·`GaussianBlur` | — | 미구현(XI.10.2·XI.10.4, `Unsupported`) | — |

use stellar_renderer_core::backend::ShaderProgram;

/// `DrawUniforms` byte 크기.
pub(crate) const DRAW_UNIFORMS_SIZE: u32 = 32;

/// uniform 범위 offset 정렬(byte). `float4` 정렬과 같습니다.
pub(crate) const UNIFORM_OFFSET_ALIGNMENT: u64 = 16;

/// vertex·index buffer offset 정렬(byte).
pub(crate) const VERTEX_OFFSET_ALIGNMENT: u64 = 4;

/// position·color vertex stride(byte).
pub(crate) const SOLID_VERTEX_STRIDE: u32 = 24;

/// position·uv vertex stride(byte).
pub(crate) const TEXTURED_VERTEX_STRIDE: u32 = 16;

/// fragment texture slot 수.
pub(crate) const MAX_TEXTURE_SLOTS: u8 = 4;

/// 내장 shader library 소스(MSL).
pub(crate) const BUILTIN_SOURCE: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct TargetUniforms { float2 size; };
struct DrawUniforms { float4 tint; float depth; float pad0; float pad1; float pad2; };

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
        }
    }
    const fn textured(fragment: &'static str) -> ProgramLayout {
        ProgramLayout {
            vertex: "stellar_textured_vertex",
            fragment,
            stride: TEXTURED_VERTEX_STRIDE,
            needs_texture: true,
            writes_color: true,
        }
    }
    match program {
        ShaderProgram::SolidFill => Some(solid("stellar_solid_fragment", true)),
        ShaderProgram::ClipStencil => Some(solid("stellar_clip_stencil_fragment", false)),
        ShaderProgram::ClipMask => Some(solid("stellar_clip_mask_fragment", true)),
        ShaderProgram::Texture | ShaderProgram::LayerComposite => {
            Some(textured("stellar_texture_fragment"))
        }
        ShaderProgram::GlyphMask => Some(textured("stellar_glyph_fragment")),
        ShaderProgram::BlendComposite(_) | ShaderProgram::GaussianBlur => None,
    }
}

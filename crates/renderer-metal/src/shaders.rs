//! Metal shader 소스와 shader ABI.
//!
//! ## shader ABI(공통 계층 ↔ Metal backend 계약)
//!
//! | slot | 단계 | 내용 |
//! |---|---|---|
//! | buffer 0 | vertex | vertex 배열(`SetVertexBuffer`, program별 layout) |
//! | buffer 1 | vertex·fragment | [`DRAW_UNIFORMS_SIZE`] byte `DrawUniforms`(`SetUniforms`) |
//! | buffer 2 | vertex | backend가 pass마다 넣는 target 크기(`float2`, pixel) |
//!
//! - 좌표는 attachment pixel 좌표(좌상단 원점, y 아래로)입니다. backend가 clip 좌표로 바꿉니다.
//! - 색은 premultiplied alpha 선형 값입니다(XI.10.1에서 encode 규칙 고정).
//! - `DrawUniforms { float4 tint; float depth; float pad[3]; }` = 32 byte. 최종 색 = vertex 색 × tint,
//!   clip z = depth.
//! - `SolidFill` vertex = `{ packed_float2 position; packed_float4 color; }` = [`SOLID_VERTEX_STRIDE`]
//!   byte.

use stellar_renderer_core::backend::ShaderProgram;

/// `DrawUniforms` byte 크기.
pub(crate) const DRAW_UNIFORMS_SIZE: u32 = 32;

/// uniform 범위 offset 정렬(byte). `float4` 정렬과 같습니다.
pub(crate) const UNIFORM_OFFSET_ALIGNMENT: u64 = 16;

/// vertex·index buffer offset 정렬(byte).
pub(crate) const VERTEX_OFFSET_ALIGNMENT: u64 = 4;

/// `SolidFill` vertex stride(byte).
pub(crate) const SOLID_VERTEX_STRIDE: u32 = 24;

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
"#;

/// program의 vertex·fragment 함수 이름. 아직 없는 program은 `None`.
pub(crate) const fn entry_points(program: ShaderProgram) -> Option<(&'static str, &'static str)> {
    match program {
        ShaderProgram::SolidFill => Some(("stellar_solid_vertex", "stellar_solid_fragment")),
        _ => None,
    }
}

/// program의 vertex stride(byte).
pub(crate) const fn vertex_stride(program: ShaderProgram) -> Option<u32> {
    match program {
        ShaderProgram::SolidFill => Some(SOLID_VERTEX_STRIDE),
        _ => None,
    }
}

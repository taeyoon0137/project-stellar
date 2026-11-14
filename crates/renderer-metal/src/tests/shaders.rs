//! XI.8.4: pipeline state·shader library cache, 구조화 shader 오류, known shader fixture.

use stellar_renderer_core::backend::{
    BackendErrorKind, BlendState, BufferUsage, CallOrderState, DepthMode, DrawCommand,
    GpuTextureDesc, PipelineKey, RendererBackend, ShaderProgram, StencilMode, TextureRegion,
    TextureSource, TextureUsage,
};
use stellar_renderer_core::placeholder::{
    placeholder_texture, rasterize_placeholder_reference, PlaceholderKind, SamplerDesc,
    TextureFilter, TextureWrap, PLACEHOLDER_SAMPLER, PLACEHOLDER_TILE,
};
use stellar_renderer_core::render_graph::{
    LoadOp, ResourceId, ResourceVersion, SampleCount, TextureFormat,
};

use super::geometry::{draw_uniforms, pixel, quad, single_pass, solid_key, Offscreen};
use super::*;
use crate::ShaderErrorStage;

/// `{ float2 position; float2 uv; }` vertex 사각형(6 vertex).
fn textured_quad(x1: f32, y1: f32, u1: f32, v1: f32) -> Vec<u8> {
    let corners = [
        ([0.0, 0.0], [0.0, 0.0]),
        ([x1, 0.0], [u1, 0.0]),
        ([0.0, y1], [0.0, v1]),
        ([0.0, y1], [0.0, v1]),
        ([x1, 0.0], [u1, 0.0]),
        ([x1, y1], [u1, v1]),
    ];
    corners
        .iter()
        .flat_map(|(position, uv)| position.iter().chain(uv.iter()))
        .flat_map(|value: &f32| value.to_le_bytes())
        .collect()
}

fn key(program: ShaderProgram, format: TextureFormat) -> PipelineKey {
    PipelineKey {
        program,
        blend: BlendState::Replace,
        depth: DepthMode::Disabled,
        stencil: StencilMode::Disabled,
        color_format: format,
        samples: SampleCount::X1,
    }
}

fn sampled(width: u32, height: u32, format: TextureFormat) -> GpuTextureDesc {
    GpuTextureDesc {
        width,
        height,
        format,
        mip_levels: 1,
        samples: SampleCount::X1,
        usage: TextureUsage {
            sampled: true,
            render_target: false,
        },
    }
}

#[test]
fn known_shader_fixture_matches_placeholder_cpu_reference() {
    // 검증 기준(XI.8.4): known shader fixture 렌더 — Texture program이 XI.7.4 placeholder checker를
    // Nearest + Repeat로 그린 결과가 CPU 기준 screenshot과 byte 단위로 같습니다.
    if !metal_available() {
        return;
    }
    let (width, height) = (40, 24);
    let mut env = Offscreen::new(width, height);
    let tile = placeholder_texture(PlaceholderKind::Missing);
    let texture = env
        .backend
        .create_texture(&sampled(
            PLACEHOLDER_TILE,
            PLACEHOLDER_TILE,
            TextureFormat::Rgba8Unorm,
        ))
        .expect("tile texture");
    env.backend
        .update_texture(
            texture,
            TextureRegion {
                mip_level: 0,
                x: 0,
                y: 0,
                width: PLACEHOLDER_TILE,
                height: PLACEHOLDER_TILE,
            },
            tile.pixels(),
        )
        .expect("upload tile");
    let tile_size = PLACEHOLDER_TILE as f32;
    let vertices = env.buffer(
        BufferUsage::Vertex,
        &textured_quad(
            width as f32,
            height as f32,
            width as f32 / tile_size,
            height as f32 / tile_size,
        ),
    );
    let uniforms = env.buffer(BufferUsage::Uniform, &draw_uniforms([1.0; 4], 0.0));
    let graph = external_clear_graph(
        width,
        height,
        TextureFormat::Rgba8Unorm,
        unit_clear([false; 4]),
    );
    let pixels = env.render(
        &graph,
        vec![
            DrawCommand::SetPipeline(key(ShaderProgram::Texture, TextureFormat::Rgba8Unorm)),
            DrawCommand::SetVertexBuffer {
                buffer: vertices,
                offset: 0,
            },
            DrawCommand::SetUniforms {
                buffer: uniforms,
                offset: 0,
                size: 32,
            },
            DrawCommand::BindTexture {
                slot: 0,
                texture: TextureSource::Handle(texture),
                sampler: PLACEHOLDER_SAMPLER,
            },
            DrawCommand::Draw {
                vertex_start: 0,
                vertex_count: 6,
                instance_count: 1,
            },
        ],
    );
    let reference =
        rasterize_placeholder_reference(PlaceholderKind::Missing, width, height).expect("ref");
    assert_eq!(pixels, reference.pixels());
}

#[test]
fn glyph_mask_and_clip_mask_programs_render_coverage() {
    if !metal_available() {
        return;
    }
    // GlyphMask: R8 coverage 2×2 × green tint → (0, c, 0, c).
    let mut env = Offscreen::new(2, 2);
    let coverage = env
        .backend
        .create_texture(&sampled(2, 2, TextureFormat::R8Unorm))
        .expect("coverage");
    env.backend
        .update_texture(
            coverage,
            TextureRegion {
                mip_level: 0,
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
            &[0, 255, 128, 64],
        )
        .expect("upload");
    let vertices = env.buffer(BufferUsage::Vertex, &textured_quad(2.0, 2.0, 1.0, 1.0));
    let uniforms = env.buffer(
        BufferUsage::Uniform,
        &draw_uniforms([0.0, 1.0, 0.0, 1.0], 0.0),
    );
    let graph = external_clear_graph(2, 2, TextureFormat::Rgba8Unorm, unit_clear([false; 4]));
    let nearest_clamp = SamplerDesc {
        filter: TextureFilter::Nearest,
        wrap: TextureWrap::ClampToEdge,
        mipmaps: false,
    };
    let pixels = env.render(
        &graph,
        vec![
            DrawCommand::SetPipeline(key(ShaderProgram::GlyphMask, TextureFormat::Rgba8Unorm)),
            DrawCommand::SetVertexBuffer {
                buffer: vertices,
                offset: 0,
            },
            DrawCommand::SetUniforms {
                buffer: uniforms,
                offset: 0,
                size: 32,
            },
            DrawCommand::BindTexture {
                slot: 0,
                texture: TextureSource::Handle(coverage),
                sampler: nearest_clamp,
            },
            DrawCommand::Draw {
                vertex_start: 0,
                vertex_count: 6,
                instance_count: 1,
            },
        ],
    );
    assert_eq!(
        pixels,
        vec![0, 0, 0, 0, 0, 255, 0, 255, 0, 128, 0, 128, 0, 64, 0, 64]
    );

    // ClipMask: R8 mask target에 도형 안쪽만 coverage 1.
    let mut backend =
        stellar_renderer_core::backend::CheckedBackend::new(crate::MetalBackend::headless());
    backend
        .init(&device_config(8, 8, TextureFormat::Bgra8Unorm))
        .expect("init");
    let mask = backend
        .create_texture(&target_desc(8, 8, TextureFormat::R8Unorm))
        .expect("mask");
    let rect = backend
        .create_buffer(&stellar_renderer_core::backend::BufferDesc {
            size: 6 * 24,
            usage: BufferUsage::Vertex,
        })
        .expect("vertices");
    backend
        .update_buffer(rect, 0, &quad(2.0, 2.0, 6.0, 5.0, [1.0; 4]))
        .expect("update");
    let mask_uniforms = backend
        .create_buffer(&stellar_renderer_core::backend::BufferDesc {
            size: 32,
            usage: BufferUsage::Uniform,
        })
        .expect("uniforms");
    backend
        .update_buffer(mask_uniforms, 0, &draw_uniforms([1.0; 4], 0.0))
        .expect("update");
    let graph = external_clear_graph(8, 8, TextureFormat::R8Unorm, unit_clear([false; 4]));
    let imports = bind_first_import(&graph, mask);
    let passes = [stellar_renderer_core::backend::PassCommands {
        pass: stellar_renderer_core::render_graph::PassId(0),
        commands: vec![
            DrawCommand::SetPipeline(key(ShaderProgram::ClipMask, TextureFormat::R8Unorm)),
            DrawCommand::SetVertexBuffer {
                buffer: rect,
                offset: 0,
            },
            DrawCommand::SetUniforms {
                buffer: mask_uniforms,
                offset: 0,
                size: 32,
            },
            DrawCommand::Draw {
                vertex_start: 0,
                vertex_count: 6,
                instance_count: 1,
            },
        ],
    }];
    let request = stellar_renderer_core::backend::FrameEncodeRequest {
        frame: stellar_renderer_core::backend::FrameId(1),
        graph: &graph,
        passes: &passes,
        imports: &imports,
    };
    let stellar_renderer_core::backend::EncodeOutcome::Encoded(frame) =
        backend.encode(&request).expect("encode")
    else {
        panic!("encode");
    };
    backend.submit(frame).expect("submit");
    backend.inner_mut().wait_idle().expect("wait");
    let mask_pixels = backend.inner_mut().read_texture(mask).expect("readback");
    for y in 0..8u32 {
        for x in 0..8u32 {
            let inside = (2..6).contains(&x) && (2..5).contains(&y);
            assert_eq!(
                mask_pixels[(y * 8 + x) as usize],
                if inside { 255 } else { 0 },
                "({x}, {y})"
            );
        }
    }
}

#[test]
fn library_and_pipelines_are_compiled_once_per_key() {
    if !metal_available() {
        return;
    }
    let mut env = Offscreen::new(16, 16);
    assert_eq!(env.backend.inner().shader_compile_count(), 0);
    let vertices = env.buffer(
        BufferUsage::Vertex,
        &quad(0.0, 0.0, 16.0, 16.0, [1.0, 0.0, 0.0, 1.0]),
    );
    let uniforms = env.buffer(BufferUsage::Uniform, &draw_uniforms([1.0; 4], 0.0));
    let graph = single_pass(16, LoadOp::Clear, unit_clear([false; 4]));
    let draw = |blend| {
        vec![
            DrawCommand::SetPipeline(solid_key(TextureFormat::Rgba8Unorm, blend)),
            DrawCommand::SetVertexBuffer {
                buffer: vertices,
                offset: 0,
            },
            DrawCommand::SetUniforms {
                buffer: uniforms,
                offset: 0,
                size: 32,
            },
            DrawCommand::Draw {
                vertex_start: 0,
                vertex_count: 6,
                instance_count: 1,
            },
        ]
    };
    for _ in 0..3 {
        env.render(&graph, draw(BlendState::Replace));
        env.render(&graph, draw(BlendState::PremultipliedOver));
    }
    assert_eq!(env.backend.inner().shader_compile_count(), 1);
    assert_eq!(env.backend.inner().pipeline_count(), 2);
    assert!(env.backend.inner().last_shader_error().is_none());
}

#[test]
fn shader_compile_failure_is_a_structured_error() {
    // 검증 기준(XI.8.4): shader compile 실패는 구조화 오류로 반환.
    if !metal_available() {
        return;
    }
    let mut env = Offscreen::new(4, 4);
    env.backend
        .inner_mut()
        .use_shader_source("this is not metal shading language {");
    let vertices = env.buffer(BufferUsage::Vertex, &quad(0.0, 0.0, 4.0, 4.0, [1.0; 4]));
    let uniforms = env.buffer(BufferUsage::Uniform, &draw_uniforms([1.0; 4], 0.0));
    let graph = single_pass(4, LoadOp::Clear, unit_clear([false; 4]));
    let commands = vec![
        DrawCommand::SetPipeline(solid_key(TextureFormat::Rgba8Unorm, BlendState::Replace)),
        DrawCommand::SetVertexBuffer {
            buffer: vertices,
            offset: 0,
        },
        DrawCommand::SetUniforms {
            buffer: uniforms,
            offset: 0,
            size: 32,
        },
        DrawCommand::Draw {
            vertex_start: 0,
            vertex_count: 6,
            instance_count: 1,
        },
    ];
    for _ in 0..2 {
        let error = env.try_render(&graph, commands.clone()).unwrap_err();
        assert_eq!(error.kind(), BackendErrorKind::PipelineCreation);
        assert!(!error.is_recoverable(), "PipelineCreation은 비활성화 정책");
        assert_eq!(env.backend.validator().state(), CallOrderState::Idle);
        let shader = env.backend.inner().last_shader_error().expect("structured");
        assert_eq!(shader.stage, ShaderErrorStage::Compile);
        assert_eq!(shader.program, None);
        assert!(shader.log.contains("error"), "{}", shader.log);
        assert!(shader.log.len() <= 4096);
    }
    // 실패한 소스도 cache하므로 두 번째 frame은 다시 compile하지 않습니다.
    assert_eq!(env.backend.inner().shader_compile_count(), 1);
    // clear만 하는 frame은 shader 없이 계속 그릴 수 있습니다.
    assert!(all_pixels(&env.render(&graph, Vec::new()), [0; 4]));
}

#[test]
fn missing_entry_point_is_a_structured_error() {
    if !metal_available() {
        return;
    }
    let mut env = Offscreen::new(4, 4);
    env.backend
        .inner_mut()
        .use_shader_source("#include <metal_stdlib>\nusing namespace metal;\n");
    let graph = single_pass(4, LoadOp::Clear, unit_clear([false; 4]));
    let error = env
        .try_render(
            &graph,
            vec![DrawCommand::SetPipeline(solid_key(
                TextureFormat::Rgba8Unorm,
                BlendState::Replace,
            ))],
        )
        .unwrap_err();
    assert_eq!(error.kind(), BackendErrorKind::PipelineCreation);
    let shader = env.backend.inner().last_shader_error().expect("structured");
    assert_eq!(shader.stage, ShaderErrorStage::MissingEntryPoint);
    assert_eq!(shader.program, Some(ShaderProgram::SolidFill));
    assert_eq!(shader.log, "stellar_solid_vertex");
    assert!(shader.to_string().contains("MissingEntryPoint"));
}

#[test]
fn texture_binding_and_program_rules_are_checked() {
    if !metal_available() {
        return;
    }
    let mut env = Offscreen::new(4, 4);
    let vertices = env.buffer(BufferUsage::Vertex, &textured_quad(4.0, 4.0, 1.0, 1.0));
    let uniforms = env.buffer(BufferUsage::Uniform, &draw_uniforms([1.0; 4], 0.0));
    let render_only = env
        .backend
        .create_texture(&GpuTextureDesc {
            usage: TextureUsage {
                sampled: false,
                render_target: true,
            },
            ..target_desc(4, 4, TextureFormat::Rgba8Unorm)
        })
        .expect("render-only");
    let graph = single_pass(4, LoadOp::Clear, unit_clear([false; 4]));
    let base = |program| {
        vec![
            DrawCommand::SetPipeline(key(program, TextureFormat::Rgba8Unorm)),
            DrawCommand::SetVertexBuffer {
                buffer: vertices,
                offset: 0,
            },
            DrawCommand::SetUniforms {
                buffer: uniforms,
                offset: 0,
                size: 32,
            },
        ]
    };
    let draw = DrawCommand::Draw {
        vertex_start: 0,
        vertex_count: 6,
        instance_count: 1,
    };
    let bind = |slot, texture| DrawCommand::BindTexture {
        slot,
        texture,
        sampler: PLACEHOLDER_SAMPLER,
    };
    let mut no_texture = base(ShaderProgram::Texture);
    no_texture.push(draw);
    let mut not_sampled = base(ShaderProgram::Texture);
    not_sampled.push(bind(0, TextureSource::Handle(render_only)));
    let mut bad_slot = base(ShaderProgram::Texture);
    bad_slot.push(bind(4, TextureSource::Handle(render_only)));
    let mut graph_source = base(ShaderProgram::Texture);
    graph_source.push(bind(
        0,
        TextureSource::Graph(ResourceVersion::new(ResourceId(0), 0)),
    ));
    let mut depth_key = base(ShaderProgram::SolidFill);
    depth_key[0] = DrawCommand::SetPipeline(PipelineKey {
        depth: DepthMode::TestAndWrite,
        ..key(ShaderProgram::SolidFill, TextureFormat::Rgba8Unorm)
    });
    let cases = [
        (no_texture, BackendErrorKind::InvalidUsage),
        (not_sampled, BackendErrorKind::InvalidUsage),
        (bad_slot, BackendErrorKind::InvalidUsage),
        (graph_source, BackendErrorKind::Unsupported),
        (depth_key, BackendErrorKind::InvalidUsage),
        (
            base(ShaderProgram::GaussianBlur),
            BackendErrorKind::Unsupported,
        ),
    ];
    for (commands, kind) in cases {
        let error = env.try_render(&graph, commands.clone()).unwrap_err();
        assert_eq!(error.kind(), kind, "{commands:?}");
        assert_eq!(env.backend.validator().state(), CallOrderState::Idle);
    }
    // 오류 뒤에도 같은 backend로 그릴 수 있습니다.
    let pixels = env.render(&graph, Vec::new());
    assert_eq!(pixel(&pixels, 4, 0, 0), [0; 4]);
}

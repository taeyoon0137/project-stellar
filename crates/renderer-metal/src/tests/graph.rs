//! XI.8.5: RenderGraph pass encoding·submit·present 예약·CPU timestamp.

use std::cell::Cell;
use std::rc::Rc;

use stellar_renderer_core::backend::{
    BackendErrorKind, BlendState, BufferHandle, BufferUsage, CallOrderState, CheckedBackend,
    DepthMode, DrawCommand, EncodeOutcome, PassCommands, PipelineKey, RendererBackend,
    ShaderProgram, StencilMode, SurfaceSize, TextureSource,
};
use stellar_renderer_core::graph_trace::{MonotonicClock, TraceClock};
use stellar_renderer_core::placeholder::{SamplerDesc, TextureFilter, TextureWrap};
use stellar_renderer_core::render_graph::{
    AttachmentSize, CompiledRenderGraph, ImportKind, LoadOp, PassId, PassKind, RenderGraphBuilder,
    SampleCount, StoreOp, TextureDesc, TextureFormat,
};

use super::geometry::{draw_uniforms, pixel, quad, solid_key, Offscreen};
use super::shaders::key;
use super::*;
use crate::{LayerOptions, MetalBackend, MetalFrameTrace, MAX_FRAME_TRACES};

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

const NEAREST: SamplerDesc = SamplerDesc {
    filter: TextureFilter::Nearest,
    wrap: TextureWrap::ClampToEdge,
    mipmaps: false,
};

fn fixed(format: TextureFormat, size: u32) -> TextureDesc {
    TextureDesc {
        format,
        size: AttachmentSize::Fixed {
            width: size,
            height: size,
        },
        samples: SampleCount::X1,
    }
}

/// vertex·uniform을 bind하고 그리는 command.
fn draw(
    pipeline: PipelineKey,
    vertices: BufferHandle,
    uniforms: BufferHandle,
    uniform_offset: u64,
    count: u32,
) -> Vec<DrawCommand> {
    vec![
        DrawCommand::SetPipeline(pipeline),
        DrawCommand::SetVertexBuffer {
            buffer: vertices,
            offset: 0,
        },
        DrawCommand::SetUniforms {
            buffer: uniforms,
            offset: uniform_offset,
            size: 32,
        },
        DrawCommand::Draw {
            vertex_start: 0,
            vertex_count: count,
            instance_count: 1,
        },
    ]
}

#[test]
fn transient_layer_pass_feeds_composite_pass() {
    // 검증 기준(XI.8.5): 여러 pass graph encoding — transient layer를 그리고 다음 pass가 sampling.
    if !metal_available() {
        return;
    }
    let size = 8;
    let mut env = Offscreen::new(size, size);
    let mut builder = RenderGraphBuilder::new();
    let target = builder.import(
        "target",
        fixed(TextureFormat::Rgba8Unorm, size),
        ImportKind::External,
    );
    let layer = builder.transient("layer", fixed(TextureFormat::Rgba8Unorm, size));
    // 선언 순서를 거꾸로(composite 먼저) 두어도 resource 흐름이 실행 순서를 정합니다.
    let composite = builder.pass("composite", PassKind::Raster);
    let paint = builder.pass("layer", PassKind::Raster);
    let layer_v1 = builder
        .color(
            paint,
            layer,
            LoadOp::Clear,
            unit_clear([true, false, false, true]),
            StoreOp::Store,
        )
        .expect("layer");
    builder.read(composite, layer_v1).expect("read");
    let written = builder
        .color(
            composite,
            target,
            LoadOp::Clear,
            unit_clear([false, false, true, true]),
            StoreOp::Store,
        )
        .expect("target");
    builder.set_output(written);
    let graph = builder.finish().compile().expect("compile");
    assert_eq!(graph.order(), &[PassId(1), PassId(0)]);

    let green_left = env.buffer(
        BufferUsage::Vertex,
        &quad(0.0, 0.0, 4.0, 8.0, [0.0, 1.0, 0.0, 1.0]),
    );
    // composite: layer를 오른쪽 6열에만 덮습니다(x ∈ [2, 8)).
    let composite_quad: Vec<u8> = {
        let corners = [
            ([2.0f32, 0.0f32], [0.25f32, 0.0f32]),
            ([8.0, 0.0], [1.0, 0.0]),
            ([2.0, 8.0], [0.25, 1.0]),
            ([2.0, 8.0], [0.25, 1.0]),
            ([8.0, 0.0], [1.0, 0.0]),
            ([8.0, 8.0], [1.0, 1.0]),
        ];
        corners
            .iter()
            .flat_map(|(p, uv)| p.iter().chain(uv.iter()))
            .flat_map(|v| v.to_le_bytes())
            .collect()
    };
    let composite_vertices = env.buffer(BufferUsage::Vertex, &composite_quad);
    let uniforms = env.buffer(BufferUsage::Uniform, &draw_uniforms([1.0; 4], 0.0));
    let mut composite_commands = vec![DrawCommand::BindTexture {
        slot: 0,
        texture: TextureSource::Graph(layer_v1),
        sampler: NEAREST,
    }];
    composite_commands.extend(draw(
        key(ShaderProgram::LayerComposite, TextureFormat::Rgba8Unorm),
        composite_vertices,
        uniforms,
        0,
        6,
    ));
    let pixels = env
        .try_render_passes(
            &graph,
            vec![
                PassCommands {
                    pass: PassId(1),
                    commands: draw(
                        solid_key(TextureFormat::Rgba8Unorm, BlendState::Replace),
                        green_left,
                        uniforms,
                        0,
                        6,
                    ),
                },
                PassCommands {
                    pass: PassId(0),
                    commands: composite_commands,
                },
            ],
        )
        .expect("render");
    for y in 0..size {
        for x in 0..size {
            // x < 2: target clear(blue). x ∈ [2, 4): layer의 green. x ≥ 4: layer clear(red).
            let expected = match x {
                0 | 1 => BLUE,
                2 | 3 => GREEN,
                _ => RED,
            };
            assert_eq!(pixel(&pixels, size, x, y), expected, "({x}, {y})");
        }
    }
    assert_eq!(env.backend.inner().transient_texture_count(), 1);
}

#[test]
fn depth_attachment_keeps_the_nearest_draw() {
    if !metal_available() {
        return;
    }
    let size = 4;
    let mut env = Offscreen::new(size, size);
    let mut builder = RenderGraphBuilder::new();
    let target = builder.import(
        "target",
        fixed(TextureFormat::Rgba8Unorm, size),
        ImportKind::External,
    );
    let depth = builder.transient("depth", fixed(TextureFormat::Depth32FloatStencil8, size));
    let pass = builder.pass("opaque", PassKind::Raster);
    let written = builder
        .color(
            pass,
            target,
            LoadOp::Clear,
            unit_clear([false, false, false, true]),
            StoreOp::Store,
        )
        .expect("color");
    builder
        .depth_stencil(pass, depth, LoadOp::Clear, StoreOp::Discard)
        .expect("depth");
    builder.set_output(written);
    let graph = builder.finish().compile().expect("compile");
    let red = env.buffer(
        BufferUsage::Vertex,
        &quad(0.0, 0.0, 4.0, 4.0, [1.0, 0.0, 0.0, 1.0]),
    );
    let green = env.buffer(
        BufferUsage::Vertex,
        &quad(0.0, 0.0, 4.0, 4.0, [0.0, 1.0, 0.0, 1.0]),
    );
    let mut depths = draw_uniforms([1.0; 4], 0.2);
    depths.extend(draw_uniforms([1.0; 4], 0.8));
    let uniforms = env.buffer(BufferUsage::Uniform, &depths);
    let opaque = PipelineKey {
        depth: DepthMode::TestAndWrite,
        ..solid_key(TextureFormat::Rgba8Unorm, BlendState::Replace)
    };
    // 가까운 red(0.2) → 먼 green(0.8), 그리고 반대 순서 모두 red가 남습니다.
    for order in [[(red, 0), (green, 32)], [(green, 32), (red, 0)]] {
        let mut commands = Vec::new();
        for (vertices, offset) in order {
            commands.extend(draw(opaque, vertices, uniforms, offset, 6));
        }
        let pixels = env.render(&graph, commands);
        assert!(all_pixels(&pixels, RED), "{order:?}");
    }
}

#[test]
fn stencil_clip_restricts_drawing_to_the_clip_shape() {
    if !metal_available() {
        return;
    }
    let size = 8;
    let mut env = Offscreen::new(size, size);
    let mut builder = RenderGraphBuilder::new();
    let target = builder.import(
        "target",
        fixed(TextureFormat::Rgba8Unorm, size),
        ImportKind::External,
    );
    let stencil = builder.transient("stencil", fixed(TextureFormat::Depth32FloatStencil8, size));
    let pass = builder.pass("clip", PassKind::Raster);
    let written = builder
        .color(
            pass,
            target,
            LoadOp::Clear,
            unit_clear([false, false, false, true]),
            StoreOp::Store,
        )
        .expect("color");
    builder
        .depth_stencil(pass, stencil, LoadOp::Clear, StoreOp::Discard)
        .expect("stencil");
    builder.set_output(written);
    let graph = builder.finish().compile().expect("compile");
    let clip_shape = env.buffer(BufferUsage::Vertex, &quad(2.0, 3.0, 6.0, 5.0, [1.0; 4]));
    let full_green = env.buffer(
        BufferUsage::Vertex,
        &quad(0.0, 0.0, 8.0, 8.0, [0.0, 1.0, 0.0, 1.0]),
    );
    let full_red = env.buffer(
        BufferUsage::Vertex,
        &quad(0.0, 0.0, 8.0, 8.0, [1.0, 0.0, 0.0, 1.0]),
    );
    let uniforms = env.buffer(BufferUsage::Uniform, &draw_uniforms([1.0; 4], 0.0));
    let stencil_key = |program, stencil| PipelineKey {
        stencil,
        ..key(program, TextureFormat::Rgba8Unorm)
    };
    let mut commands = vec![DrawCommand::SetStencilReference(0)];
    // clip 진입: stencil 0인 곳(전체)에서 도형 영역을 1로.
    commands.extend(draw(
        stencil_key(ShaderProgram::ClipStencil, StencilMode::IncrementWhereEqual),
        clip_shape,
        uniforms,
        0,
        6,
    ));
    // clip 안쪽(stencil == 1)에만 green.
    commands.push(DrawCommand::SetStencilReference(1));
    commands.extend(draw(
        stencil_key(ShaderProgram::SolidFill, StencilMode::TestEqual),
        full_green,
        uniforms,
        0,
        6,
    ));
    // clip 해제: reference 0, DecrementWhereEqual은 stencil == 1을 0으로.
    commands.push(DrawCommand::SetStencilReference(0));
    commands.extend(draw(
        stencil_key(ShaderProgram::ClipStencil, StencilMode::DecrementWhereEqual),
        clip_shape,
        uniforms,
        0,
        6,
    ));
    // 해제 뒤 stencil == 1인 곳이 없으므로 red는 그려지지 않습니다.
    commands.push(DrawCommand::SetStencilReference(1));
    commands.extend(draw(
        stencil_key(ShaderProgram::SolidFill, StencilMode::TestEqual),
        full_red,
        uniforms,
        0,
        6,
    ));
    let pixels = env.render(&graph, commands);
    for y in 0..size {
        for x in 0..size {
            let inside = (2..6).contains(&x) && (3..5).contains(&y);
            let expected = if inside { GREEN } else { [0, 0, 0, 255] };
            assert_eq!(pixel(&pixels, size, x, y), expected, "({x}, {y})");
        }
    }
    // reference 255에서는 "reference + 1" 비교가 불가능합니다.
    let mut overflow = vec![DrawCommand::SetStencilReference(255)];
    overflow.extend(draw(
        stencil_key(ShaderProgram::ClipStencil, StencilMode::DecrementWhereEqual),
        clip_shape,
        uniforms,
        0,
        6,
    ));
    assert_eq!(
        env.try_render(&graph, overflow).unwrap_err().kind(),
        BackendErrorKind::InvalidUsage
    );
}

/// 두 transient가 수명이 겹치지 않으면 같은 alias slot texture 하나를 씁니다.
fn aliased_graph() -> CompiledRenderGraph {
    let mut builder = RenderGraphBuilder::new();
    let target = builder.import(
        "target",
        TextureDesc::surface(TextureFormat::Rgba8Unorm),
        ImportKind::External,
    );
    let first = builder.transient("first", TextureDesc::surface(TextureFormat::Rgba8Unorm));
    let second = builder.transient("second", TextureDesc::surface(TextureFormat::Rgba8Unorm));
    let clear = unit_clear([false, false, false, true]);
    let a = builder.pass("a", PassKind::Raster);
    let first_v1 = builder
        .color(a, first, LoadOp::Clear, clear, StoreOp::Store)
        .expect("a");
    let b = builder.pass("b", PassKind::Raster);
    builder.read(b, first_v1).expect("read");
    let target_v1 = builder
        .color(b, target, LoadOp::Clear, clear, StoreOp::Store)
        .expect("b");
    let c = builder.pass("c", PassKind::Raster);
    let second_v1 = builder
        .color(c, second, LoadOp::Clear, clear, StoreOp::Store)
        .expect("c");
    let d = builder.pass("d", PassKind::Raster);
    builder.read(d, second_v1).expect("read");
    let target_v2 = builder
        .color(d, target_v1, LoadOp::Load, clear, StoreOp::Store)
        .expect("d");
    builder.set_output(target_v2);
    builder.finish().compile().expect("compile")
}

#[test]
fn transient_pool_aliases_slots_and_follows_surface_resize() {
    if !metal_available() {
        return;
    }
    let mut backend = CheckedBackend::new(MetalBackend::headless());
    backend
        .init(&device_config(8, 8, TextureFormat::Bgra8Unorm))
        .expect("init");
    let graph = aliased_graph();
    assert_eq!(graph.alias_slot_count(), 1);
    for (frame, size) in (1..).zip([8, 8, 16, 4]) {
        backend
            .resize(SurfaceSize {
                width: size,
                height: size,
            })
            .expect("resize");
        let target = backend
            .create_texture(&target_desc(size, size, TextureFormat::Rgba8Unorm))
            .expect("target");
        let imports = bind_first_import(&graph, target);
        let EncodeOutcome::Encoded(encoded) = backend
            .encode(&request(frame, &graph, &imports))
            .expect("encode")
        else {
            panic!("encode");
        };
        backend.submit(encoded).expect("submit");
        backend.inner_mut().wait_idle().expect("wait");
        let pixels = backend.inner_mut().read_texture(target).expect("readback");
        assert_eq!(pixels.len(), (size * size * 4) as usize);
        assert!(all_pixels(&pixels, [0, 0, 0, 255]));
        // 겹치지 않는 두 transient는 slot 하나를 공유하고, resize 뒤에도 slot 수는 같습니다.
        assert_eq!(backend.inner().transient_texture_count(), 1);
        backend.destroy_texture(target).expect("destroy");
    }
}

#[test]
fn compute_and_transfer_passes_are_unsupported() {
    if !metal_available() {
        return;
    }
    let mut env = Offscreen::new(4, 4);
    let mut builder = RenderGraphBuilder::new();
    let target = builder.import(
        "target",
        fixed(TextureFormat::Rgba8Unorm, 4),
        ImportKind::External,
    );
    let pass = builder.pass("copy", PassKind::Transfer);
    let written = builder.write(pass, target).expect("write");
    builder.set_output(written);
    let graph = builder.finish().compile().expect("compile");
    assert_eq!(
        env.try_render(&graph, Vec::new()).unwrap_err().kind(),
        BackendErrorKind::Unsupported
    );
}

#[test]
fn attachments_without_render_target_usage_are_rejected() {
    // render target 용도가 없는 external texture를 attachment로 붙이면 Metal에 넘기기 전에
    // InvalidUsage입니다(Metal API validation을 켜면 그대로 넘긴 descriptor는 process를 끝냄).
    if !metal_available() {
        return;
    }
    let size = 4;
    let mut env = Offscreen::new(size, size);
    let sampled_only = |format| GpuTextureDesc {
        usage: TextureUsage {
            sampled: true,
            render_target: false,
        },
        ..target_desc(size, size, format)
    };
    let color = env
        .backend
        .create_texture(&sampled_only(TextureFormat::Rgba8Unorm))
        .expect("sampled-only color");
    let graph = external_clear_graph(size, size, TextureFormat::Rgba8Unorm, unit_clear([true; 4]));
    let imports = bind_first_import(&graph, color);
    let error = env
        .backend
        .encode(&request(100, &graph, &imports))
        .unwrap_err();
    assert_eq!(error.kind(), BackendErrorKind::InvalidUsage);
    assert_eq!(env.backend.validator().state(), CallOrderState::Idle);

    let depth = env
        .backend
        .create_texture(&sampled_only(TextureFormat::Depth32FloatStencil8))
        .expect("sampled-only depth");
    let mut builder = RenderGraphBuilder::new();
    let target = builder.import(
        "target",
        fixed(TextureFormat::Rgba8Unorm, size),
        ImportKind::External,
    );
    let depth_import = builder.import(
        "depth",
        fixed(TextureFormat::Depth32FloatStencil8, size),
        ImportKind::External,
    );
    let pass = builder.pass("opaque", PassKind::Raster);
    let written = builder
        .color(
            pass,
            target,
            LoadOp::Clear,
            unit_clear([true; 4]),
            StoreOp::Store,
        )
        .expect("color");
    builder
        .depth_stencil(pass, depth_import, LoadOp::Clear, StoreOp::Discard)
        .expect("depth");
    builder.set_output(written);
    let graph = builder.finish().compile().expect("compile");
    let imports = [
        ImportBinding {
            resource: target.resource,
            target: ImportTarget::Texture(env.target),
        },
        ImportBinding {
            resource: depth_import.resource,
            target: ImportTarget::Texture(depth),
        },
    ];
    let error = env
        .backend
        .encode(&request(101, &graph, &imports))
        .unwrap_err();
    assert_eq!(error.kind(), BackendErrorKind::InvalidUsage);
    assert_eq!(env.backend.validator().state(), CallOrderState::Idle);
    // 거부 뒤에도 같은 backend로 그릴 수 있습니다.
    let pixels = env.render(
        &external_clear_graph(size, size, TextureFormat::Rgba8Unorm, unit_clear([true; 4])),
        Vec::new(),
    );
    assert!(all_pixels(&pixels, [255; 4]));
}

#[test]
fn sampling_a_texture_the_pass_renders_into_is_rejected() {
    // pass가 그리는 attachment texture를 같은 pass에서 sampling하면 결과가 정의되지 않습니다(feedback
    // loop). 자기 attachment version 읽기는 graph compile이 막고(`FeedbackLoop`), backend는 handle bind와
    // 같은 texture에 묶인 다른 import 읽기를 거부합니다.
    if !metal_available() {
        return;
    }
    let size = 4;
    let mut env = Offscreen::new(size, size);
    let mut builder = RenderGraphBuilder::new();
    let target = builder.import(
        "target",
        fixed(TextureFormat::Rgba8Unorm, size),
        ImportKind::External,
    );
    let source = builder.import(
        "source",
        fixed(TextureFormat::Rgba8Unorm, size),
        ImportKind::External,
    );
    let pass = builder.pass("draw", PassKind::Raster);
    builder.read(pass, source).expect("read source");
    let written = builder
        .color(
            pass,
            target,
            LoadOp::Clear,
            unit_clear([true; 4]),
            StoreOp::Store,
        )
        .expect("color");
    builder.set_output(written);
    let graph = builder.finish().compile().expect("compile");
    let imports = [
        ImportBinding {
            resource: target.resource,
            target: ImportTarget::Texture(env.target),
        },
        ImportBinding {
            resource: source.resource,
            target: ImportTarget::Texture(env.target),
        },
    ];
    let bind = |texture| {
        vec![PassCommands {
            pass,
            commands: vec![DrawCommand::BindTexture {
                slot: 0,
                texture,
                sampler: NEAREST,
            }],
        }]
    };
    for (frame, texture) in (200..).zip([
        TextureSource::Handle(env.target),
        TextureSource::Graph(source),
    ]) {
        let passes = bind(texture);
        let request = FrameEncodeRequest {
            frame: FrameId(frame),
            graph: &graph,
            passes: &passes,
            imports: &imports,
        };
        let error = env.backend.encode(&request).unwrap_err();
        assert_eq!(error.kind(), BackendErrorKind::InvalidUsage, "{texture:?}");
        assert_eq!(env.backend.validator().state(), CallOrderState::Idle);
    }
    // 다른 texture를 sampling하는 것은 그대로 허용됩니다.
    let other = env
        .backend
        .create_texture(&target_desc(size, size, TextureFormat::Rgba8Unorm))
        .expect("other");
    let passes = bind(TextureSource::Handle(other));
    let request = FrameEncodeRequest {
        frame: FrameId(300),
        graph: &graph,
        passes: &passes,
        imports: &imports,
    };
    let EncodeOutcome::Encoded(encoded) = env.backend.encode(&request).expect("encode") else {
        panic!("offscreen frame은 skip하지 않습니다");
    };
    env.backend.discard(encoded).expect("discard");
}

/// 호출마다 10 ns씩 증가하는 시계.
#[derive(Debug, Clone)]
struct StepClock(Rc<Cell<u64>>);

impl TraceClock for StepClock {
    fn now_nanos(&self) -> u64 {
        let next = self.0.get() + 10;
        self.0.set(next);
        next
    }
}

#[test]
fn frame_trace_has_submit_and_present_timestamps() {
    // 검증 기준(XI.8.5): T0-T5 trace에 submit/present timestamp 존재.
    if !metal_available() {
        return;
    }
    let readback = LayerOptions {
        framebuffer_only: false,
        ..LayerOptions::DEFAULT
    };
    let mut backend =
        CheckedBackend::new(MetalBackend::with_detached_layer(readback).expect("layer"));
    backend
        .init(&device_config(4, 4, TextureFormat::Bgra8Unorm))
        .expect("init");
    let ticks = Rc::new(Cell::new(0));
    backend
        .inner_mut()
        .set_trace_clock(Box::new(StepClock(ticks.clone())));
    let graph = surface_clear_graph(TextureFormat::Bgra8Unorm, unit_clear([true; 4]));
    let imports = bind_surface(&graph);
    let EncodeOutcome::Encoded(frame) = backend.encode(&request(1, &graph, &imports)).unwrap()
    else {
        panic!("encode");
    };
    backend.schedule_present(&frame).expect("present");
    let submission = backend.submit(frame).expect("submit");
    let traces = backend.inner_mut().drain_frame_traces();
    let expected = MetalFrameTrace {
        frame: 1,
        submission: submission.0,
        encode_start_nanos: 10,
        encode_end_nanos: 20,
        present_nanos: Some(30),
        submit_nanos: 40,
        pass_count: 1,
        draw_count: 0,
    };
    assert_eq!(traces, vec![expected]);
    assert_eq!(expected.t5_nanos(), 40);
    assert_eq!(
        expected.to_line(),
        "renderer.metal.frame frame=1 submission=1 encode_start_ns=10 encode_end_ns=20 \
         present=1 present_ns=30 submit_ns=40 encode_cpu_ns=10 passes=1 draws=0"
    );
    // skip한 frame은 trace를 남기지 않습니다.
    backend.inner_mut().drawable_failures = 1;
    assert!(matches!(
        backend.encode(&request(2, &graph, &imports)).unwrap(),
        EncodeOutcome::Skipped(_)
    ));
    assert!(backend.inner_mut().drain_frame_traces().is_empty());
    backend.inner_mut().wait_idle().expect("wait");
}

#[test]
fn real_clock_orders_t4_encode_present_and_t5() {
    if !metal_available() {
        return;
    }
    let mut env = Offscreen::new(4, 4);
    let clock = MonotonicClock::new();
    env.backend.inner_mut().set_trace_clock(Box::new(clock));
    let vertices = env.buffer(BufferUsage::Vertex, &quad(0.0, 0.0, 4.0, 4.0, [1.0; 4]));
    let uniforms = env.buffer(BufferUsage::Uniform, &draw_uniforms([1.0; 4], 0.0));
    let graph = super::geometry::single_pass(4, LoadOp::Clear, unit_clear([false; 4]));
    // 공통 계층이 graph를 준비한 시각(T4 후보)을 같은 시계로 잽니다.
    let t4 = clock.now_nanos();
    env.render(
        &graph,
        draw(
            solid_key(TextureFormat::Rgba8Unorm, BlendState::Replace),
            vertices,
            uniforms,
            0,
            6,
        ),
    );
    let traces = env.backend.inner_mut().drain_frame_traces();
    assert_eq!(traces.len(), 1);
    let trace = traces[0];
    assert!(t4 <= trace.encode_start_nanos);
    assert!(trace.encode_start_nanos <= trace.encode_end_nanos);
    assert_eq!(trace.present_nanos, None, "offscreen frame");
    assert!(trace.encode_end_nanos <= trace.t5_nanos());
    assert_eq!((trace.pass_count, trace.draw_count), (1, 1));
    // 보관 수는 상한을 넘지 않고 오래된 것부터 버립니다.
    for _ in 0..(MAX_FRAME_TRACES + 6) {
        env.render(&graph, Vec::new());
    }
    let traces = env.backend.inner_mut().drain_frame_traces();
    assert_eq!(traces.len(), MAX_FRAME_TRACES);
    assert_eq!(
        traces.last().map(|trace| trace.submission),
        Some(MAX_FRAME_TRACES as u64 + 7)
    );
}
